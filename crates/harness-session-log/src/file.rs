#[cfg(test)]
use std::collections::VecDeque;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use fs4::{FileExt, TryLockError};

use crate::format::{
    FailureCode, MAX_SESSION_BYTES, Record, assistant_entry, failure_entry, header, serialize_line,
    user_entry,
};
use crate::projection::{ProjectionError, ShowResult, project, valid_id};
use crate::{
    AppendError, CreateError, ListError, ListResult, ListedSessionStatus, RollbackError,
    SessionSummary, ShowError,
};

const CREATE_RETRIES: usize = 8;
const MAX_LIST_CANDIDATES: usize = 4096;
const MAX_LIST_DIRECTORY_ENTRIES: usize = 4096;
const MAX_LIST_READ_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
struct ListLimits {
    candidates: usize,
    directory_entries: usize,
    read_bytes: u64,
}

const LIST_LIMITS: ListLimits = ListLimits {
    candidates: MAX_LIST_CANDIDATES,
    directory_entries: MAX_LIST_DIRECTORY_ENTRIES,
    read_bytes: MAX_LIST_READ_BYTES,
};

pub struct SessionLog {
    location: Location,
    sources: Arc<Mutex<Sources>>,
}

enum Location {
    File {
        application_dir: PathBuf,
        sessions_dir: PathBuf,
    },
    Unavailable,
    #[cfg(test)]
    Memory(Arc<Mutex<MemoryStore>>),
}

struct Sources {
    ids: IdSource,
    times: TimeSource,
    working_directory: WorkingDirectorySource,
    harness_version: String,
}

struct InitialRecords {
    session_id: String,
    operation_id: String,
    user_entry_id: String,
    header: Vec<u8>,
    user: Vec<u8>,
}

enum IdSource {
    Random,
    #[cfg(test)]
    Fixed(VecDeque<String>),
}

enum TimeSource {
    System,
    #[cfg(test)]
    Fixed(VecDeque<u64>),
}

enum WorkingDirectorySource {
    Current,
    #[cfg(test)]
    Fixed(Option<String>),
}

#[cfg(test)]
#[derive(Default)]
struct MemoryStore {
    files: std::collections::HashMap<String, MemoryFile>,
}

#[cfg(test)]
struct MemoryFile {
    bytes: Vec<u8>,
    locked: bool,
}

pub struct SessionWriter {
    backend: Option<WriterBackend>,
    sources: Arc<Mutex<Sources>>,
    session_id: String,
    user_entry_id: String,
    operation_id: String,
}

enum WriterBackend {
    File {
        file: File,
        path: PathBuf,
    },
    #[cfg(test)]
    Memory {
        store: Arc<Mutex<MemoryStore>>,
    },
}

impl SessionLog {
    pub fn production() -> Self {
        let location = ProjectDirs::from("", "", "rust-agent-harness").map_or(
            Location::Unavailable,
            |directories| {
                let application_dir = directories.data_local_dir().to_owned();
                let sessions_dir = application_dir.join("sessions");
                Location::File {
                    application_dir,
                    sessions_dir,
                }
            },
        );
        Self {
            location,
            sources: Arc::new(Mutex::new(Sources::production())),
        }
    }

    #[doc(hidden)]
    pub fn at_data_local_dir(application_dir: PathBuf) -> Self {
        let sessions_dir = application_dir.join("sessions");
        Self {
            location: Location::File {
                application_dir,
                sessions_dir,
            },
            sources: Arc::new(Mutex::new(Sources::production())),
        }
    }

    pub fn start(&mut self, prompt: &str) -> Result<SessionWriter, CreateError> {
        match &self.location {
            Location::Unavailable => Err(CreateError::StoreUnavailable),
            Location::File {
                application_dir,
                sessions_dir,
            } => start_file(
                application_dir,
                sessions_dir,
                prompt,
                Arc::clone(&self.sources),
            ),
            #[cfg(test)]
            Location::Memory(store) => start_memory(store, prompt, Arc::clone(&self.sources)),
        }
    }

    pub fn show(&self, session_id: &str) -> Result<ShowResult, ShowError> {
        if !valid_id(session_id) {
            return Err(ShowError::InvalidSessionId);
        }
        match &self.location {
            Location::Unavailable => Err(ShowError::StoreUnavailable),
            Location::File {
                application_dir,
                sessions_dir,
            } => show_file(application_dir, sessions_dir, session_id),
            #[cfg(test)]
            Location::Memory(store) => show_memory(store, session_id),
        }
    }

    pub fn list(&self) -> Result<ListResult, ListError> {
        self.list_with_limits(LIST_LIMITS)
    }

    fn list_with_limits(&self, limits: ListLimits) -> Result<ListResult, ListError> {
        match &self.location {
            Location::Unavailable => Err(ListError::StoreUnavailable),
            Location::File {
                application_dir,
                sessions_dir,
            } => list_file(application_dir, sessions_dir, limits),
            #[cfg(test)]
            Location::Memory(store) => list_memory(store, limits),
        }
    }

    #[cfg(test)]
    fn memory(ids: &[&str], times: &[u64], working_directory: Option<&str>) -> Self {
        Self {
            location: Location::Memory(Arc::new(Mutex::new(MemoryStore::default()))),
            sources: Arc::new(Mutex::new(Sources::fixed(
                ids,
                times,
                working_directory.map(str::to_owned),
            ))),
        }
    }

    #[cfg(test)]
    fn fixed_file(
        application_dir: PathBuf,
        ids: &[&str],
        times: &[u64],
        working_directory: Option<&str>,
    ) -> Self {
        let sessions_dir = application_dir.join("sessions");
        Self {
            location: Location::File {
                application_dir,
                sessions_dir,
            },
            sources: Arc::new(Mutex::new(Sources::fixed(
                ids,
                times,
                working_directory.map(str::to_owned),
            ))),
        }
    }

    #[cfg(test)]
    fn memory_bytes(&self, session_id: &str) -> Vec<u8> {
        let Location::Memory(store) = &self.location else {
            panic!("memory session log required");
        };
        store.lock().unwrap().files[session_id].bytes.clone()
    }
}

impl Sources {
    fn production() -> Self {
        Self {
            ids: IdSource::Random,
            times: TimeSource::System,
            working_directory: WorkingDirectorySource::Current,
            harness_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    #[cfg(test)]
    fn fixed(ids: &[&str], times: &[u64], working_directory: Option<String>) -> Self {
        Self {
            ids: IdSource::Fixed(ids.iter().map(|value| (*value).to_owned()).collect()),
            times: TimeSource::Fixed(times.iter().copied().collect()),
            working_directory: WorkingDirectorySource::Fixed(working_directory),
            harness_version: "0.1.0".to_owned(),
        }
    }

    fn next_id(&mut self) -> Result<String, ()> {
        let id = match &mut self.ids {
            IdSource::Random => random_id(),
            #[cfg(test)]
            IdSource::Fixed(ids) => ids.pop_front().ok_or(())?,
        };
        valid_id(&id).then_some(id).ok_or(())
    }

    fn now(&mut self) -> Result<u64, ()> {
        match &mut self.times {
            TimeSource::System => Ok(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX)),
            #[cfg(test)]
            TimeSource::Fixed(times) => times.pop_front().ok_or(()),
        }
    }

    fn working_directory(&self) -> Option<String> {
        match &self.working_directory {
            WorkingDirectorySource::Current => env::current_dir()
                .ok()
                .and_then(|path| path.into_os_string().into_string().ok()),
            #[cfg(test)]
            WorkingDirectorySource::Fixed(directory) => directory.clone(),
        }
    }
}

fn random_id() -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes: [u8; 16] = rand::random();
    let mut id = String::with_capacity(32);
    for byte in bytes {
        id.push(char::from(HEX[usize::from(byte >> 4)]));
        id.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    id
}

fn initial_records(
    sources: &Arc<Mutex<Sources>>,
    prompt: &str,
) -> Result<InitialRecords, CreateError> {
    let mut sources = sources.lock().map_err(|_| CreateError::StoreUnavailable)?;
    let session_id = sources
        .next_id()
        .map_err(|_| CreateError::StoreUnavailable)?;
    let operation_id = sources
        .next_id()
        .map_err(|_| CreateError::StoreUnavailable)?;
    let user_entry_id = sources
        .next_id()
        .map_err(|_| CreateError::StoreUnavailable)?;
    if [
        session_id.as_str(),
        operation_id.as_str(),
        user_entry_id.as_str(),
    ]
    .into_iter()
    .collect::<std::collections::HashSet<_>>()
    .len()
        != 3
    {
        return Err(CreateError::StoreUnavailable);
    }
    let created_at = sources.now().map_err(|_| CreateError::StoreUnavailable)?;
    let user_timestamp = sources.now().map_err(|_| CreateError::StoreUnavailable)?;
    let header = header(
        session_id.clone(),
        created_at,
        sources.working_directory(),
        sources.harness_version.clone(),
    );
    let user = user_entry(
        user_entry_id.clone(),
        user_timestamp,
        operation_id.clone(),
        prompt.to_owned(),
    );
    let header = serialize_line(&header).map_err(|_| CreateError::StoreUnavailable)?;
    let user = serialize_line(&user).map_err(|_| CreateError::StoreUnavailable)?;
    if header
        .len()
        .checked_add(user.len())
        .is_none_or(|size| size > MAX_SESSION_BYTES)
    {
        return Err(CreateError::StoreUnavailable);
    }
    Ok(InitialRecords {
        session_id,
        operation_id,
        user_entry_id,
        header,
        user,
    })
}

fn start_file(
    application_dir: &Path,
    sessions_dir: &Path,
    prompt: &str,
    sources: Arc<Mutex<Sources>>,
) -> Result<SessionWriter, CreateError> {
    ensure_private_directory(application_dir).map_err(|_| CreateError::StoreUnavailable)?;
    ensure_private_directory(sessions_dir).map_err(|_| CreateError::StoreUnavailable)?;

    for _ in 0..CREATE_RETRIES {
        let initial = initial_records(&sources, prompt)?;
        let session_id = initial.session_id;
        let operation_id = initial.operation_id;
        let user_entry_id = initial.user_entry_id;
        let path = sessions_dir.join(format!("{session_id}.jsonl"));
        let mut file = match create_private_file(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(CreateError::StoreUnavailable),
        };
        if FileExt::lock(&file).is_err()
            || file.write_all(&initial.header).is_err()
            || file.write_all(&initial.user).is_err()
            || file.sync_all().is_err()
            || File::open(sessions_dir)
                .and_then(|directory| directory.sync_all())
                .is_err()
        {
            rollback_created_file(file, &path);
            return Err(CreateError::StoreUnavailable);
        }
        return Ok(SessionWriter {
            backend: Some(WriterBackend::File { file, path }),
            sources,
            session_id,
            user_entry_id,
            operation_id,
        });
    }
    Err(CreateError::StoreUnavailable)
}

#[cfg(test)]
fn start_memory(
    store: &Arc<Mutex<MemoryStore>>,
    prompt: &str,
    sources: Arc<Mutex<Sources>>,
) -> Result<SessionWriter, CreateError> {
    for _ in 0..CREATE_RETRIES {
        let initial = initial_records(&sources, prompt)?;
        let session_id = initial.session_id;
        let operation_id = initial.operation_id;
        let user_entry_id = initial.user_entry_id;
        let mut bytes = initial.header;
        let mut memory = store.lock().map_err(|_| CreateError::StoreUnavailable)?;
        if memory.files.contains_key(&session_id) {
            continue;
        }
        bytes.extend(initial.user);
        memory.files.insert(
            session_id.clone(),
            MemoryFile {
                bytes,
                locked: true,
            },
        );
        drop(memory);
        return Ok(SessionWriter {
            backend: Some(WriterBackend::Memory {
                store: Arc::clone(store),
            }),
            sources,
            session_id,
            user_entry_id,
            operation_id,
        });
    }
    Err(CreateError::StoreUnavailable)
}

impl SessionWriter {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn append_assistant(mut self, text: &str) -> Result<(), AppendError> {
        let user_entry_id = self.user_entry_id.clone();
        let operation_id = self.operation_id.clone();
        let record = self.terminal_record(|entry_id, timestamp| {
            assistant_entry(
                entry_id,
                user_entry_id,
                timestamp,
                operation_id,
                text.to_owned(),
            )
        })?;
        self.append_terminal(record)
    }

    pub fn append_failure(mut self, failure_code: FailureCode) -> Result<(), AppendError> {
        let user_entry_id = self.user_entry_id.clone();
        let operation_id = self.operation_id.clone();
        let record = self.terminal_record(|entry_id, timestamp| {
            failure_entry(
                entry_id,
                user_entry_id,
                timestamp,
                operation_id,
                failure_code,
            )
        })?;
        self.append_terminal(record)
    }

    pub fn rollback(mut self) -> Result<(), RollbackError> {
        let Some(backend) = self.backend.take() else {
            return Err(RollbackError::StoreUnavailable);
        };
        match backend {
            WriterBackend::File { file, path } => {
                if !identity_matches(&file, &path) {
                    return Err(RollbackError::StoreUnavailable);
                }
                drop(file);
                let sessions_dir = path.parent().ok_or(RollbackError::StoreUnavailable)?;
                fs::remove_file(&path).map_err(|_| RollbackError::StoreUnavailable)?;
                File::open(sessions_dir)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|_| RollbackError::StoreUnavailable)
            }
            #[cfg(test)]
            WriterBackend::Memory { store } => store
                .lock()
                .map_err(|_| RollbackError::StoreUnavailable)?
                .files
                .remove(&self.session_id)
                .map(|_| ())
                .ok_or(RollbackError::StoreUnavailable),
        }
    }

    fn terminal_record(
        &mut self,
        build: impl FnOnce(String, u64) -> Record,
    ) -> Result<Record, AppendError> {
        let mut sources = self
            .sources
            .lock()
            .map_err(|_| AppendError::StoreUnavailable)?;
        let entry_id = sources
            .next_id()
            .map_err(|_| AppendError::StoreUnavailable)?;
        if entry_id == self.session_id
            || entry_id == self.user_entry_id
            || entry_id == self.operation_id
        {
            return Err(AppendError::StoreUnavailable);
        }
        let timestamp = sources.now().map_err(|_| AppendError::StoreUnavailable)?;
        Ok(build(entry_id, timestamp))
    }

    fn append_terminal(mut self, record: Record) -> Result<(), AppendError> {
        let line = serialize_line(&record).map_err(|_| AppendError::StoreUnavailable)?;
        let Some(backend) = self.backend.take() else {
            return Err(AppendError::StoreUnavailable);
        };
        match backend {
            WriterBackend::File { mut file, path } => {
                if !identity_matches(&file, &path)
                    || file
                        .metadata()
                        .ok()
                        .and_then(|metadata| {
                            usize::try_from(metadata.len())
                                .ok()
                                .and_then(|size| size.checked_add(line.len()))
                        })
                        .is_none_or(|size| size > MAX_SESSION_BYTES)
                    || file.write_all(&line).is_err()
                    || file.sync_data().is_err()
                {
                    return Err(AppendError::StoreUnavailable);
                }
                Ok(())
            }
            #[cfg(test)]
            WriterBackend::Memory { store } => {
                let mut store = store.lock().map_err(|_| AppendError::StoreUnavailable)?;
                let file = store
                    .files
                    .get_mut(&self.session_id)
                    .ok_or(AppendError::StoreUnavailable)?;
                let size = file
                    .bytes
                    .len()
                    .checked_add(line.len())
                    .ok_or(AppendError::StoreUnavailable)?;
                if size > MAX_SESSION_BYTES {
                    return Err(AppendError::StoreUnavailable);
                }
                file.bytes.extend(line);
                file.locked = false;
                Ok(())
            }
        }
    }
}

impl Drop for SessionWriter {
    fn drop(&mut self) {
        #[cfg(test)]
        let Some(WriterBackend::Memory { store }) = &self.backend else {
            return;
        };
        #[cfg(test)]
        let Ok(mut store) = store.lock() else {
            return;
        };
        #[cfg(test)]
        if let Some(file) = store.files.get_mut(&self.session_id) {
            file.locked = false;
        }
    }
}

fn list_file(
    application_dir: &Path,
    sessions_dir: &Path,
    limits: ListLimits,
) -> Result<ListResult, ListError> {
    if !list_directory_exists(application_dir)? || !list_directory_exists(sessions_dir)? {
        return Ok(ListResult::new(Vec::new()));
    }
    let directory = fs::read_dir(sessions_dir).map_err(|_| ListError::StoreUnavailable)?;
    let candidates = collect_candidate_ids(
        directory.map(|entry| entry.map(|entry| entry.file_name())),
        limits,
    )?;
    let mut budget = ListReadBudget {
        used: 0,
        limit: limits.read_bytes,
    };
    summarize_candidates(candidates, |session_id| {
        inspect_file(
            &sessions_dir.join(format!("{session_id}.jsonl")),
            session_id,
            ReadPolicy::List(&mut budget),
        )
    })
}

fn list_directory_exists(path: &Path) -> Result<bool, ListError> {
    match validate_private_directory(path) {
        Ok(()) => Ok(true),
        Err(ShowError::Missing) => Ok(false),
        Err(_) => Err(ListError::StoreUnavailable),
    }
}

fn collect_candidate_ids<I>(entries: I, limits: ListLimits) -> Result<Vec<String>, ListError>
where
    I: IntoIterator<Item = io::Result<OsString>>,
{
    let mut directory_entries = 0_usize;
    let mut candidates = Vec::new();
    for entry in entries {
        let file_name = entry.map_err(|_| ListError::StoreUnavailable)?;
        directory_entries = directory_entries
            .checked_add(1)
            .ok_or(ListError::InventoryTooLarge)?;
        if directory_entries > limits.directory_entries {
            return Err(ListError::InventoryTooLarge);
        }
        let Some(session_id) = canonical_session_id(&file_name) else {
            continue;
        };
        if candidates
            .len()
            .checked_add(1)
            .is_none_or(|count| count > limits.candidates)
        {
            return Err(ListError::InventoryTooLarge);
        }
        candidates.push(session_id);
    }
    candidates.sort_unstable();
    Ok(candidates)
}

fn canonical_session_id(file_name: &OsStr) -> Option<String> {
    let file_name = file_name.to_str()?;
    let session_id = file_name.strip_suffix(".jsonl")?;
    valid_id(session_id).then(|| session_id.to_owned())
}

fn summarize_candidates<F>(candidates: Vec<String>, mut inspect: F) -> Result<ListResult, ListError>
where
    F: FnMut(&str) -> Result<ShowResult, FileInspectionError>,
{
    let mut summaries = Vec::with_capacity(candidates.len());
    for session_id in candidates {
        match inspect(&session_id) {
            Ok(shown) => summaries.push(shown.into_summary()),
            Err(FileInspectionError::InventoryTooLarge) => {
                return Err(ListError::InventoryTooLarge);
            }
            Err(FileInspectionError::Show(ShowError::Missing)) => {}
            Err(FileInspectionError::Show(error)) => {
                let status = match error {
                    ShowError::Busy => ListedSessionStatus::Busy,
                    ShowError::UnsupportedVersion => ListedSessionStatus::Unsupported,
                    ShowError::Corrupt => ListedSessionStatus::Corrupt,
                    ShowError::StoreUnavailable => ListedSessionStatus::Unavailable,
                    ShowError::InvalidSessionId => {
                        unreachable!("canonical session filename produced an invalid identifier")
                    }
                    ShowError::Missing => unreachable!("missing candidates are handled above"),
                };
                summaries.push(SessionSummary::unprojectable(session_id, status));
            }
        }
    }
    summaries.sort_by(|left, right| {
        match (left.created_at_unix_ms(), right.created_at_unix_ms()) {
            (Some(left_time), Some(right_time)) => right_time
                .cmp(&left_time)
                .then_with(|| left.session_id().cmp(right.session_id())),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left.session_id().cmp(right.session_id()),
        }
    });
    Ok(ListResult::new(summaries))
}

#[cfg(test)]
fn list_memory(
    store: &Arc<Mutex<MemoryStore>>,
    limits: ListLimits,
) -> Result<ListResult, ListError> {
    let store = store.lock().map_err(|_| ListError::StoreUnavailable)?;
    let candidates = collect_candidate_ids(
        store
            .files
            .keys()
            .map(|session_id| Ok(OsString::from(format!("{session_id}.jsonl")))),
        limits,
    )?;
    let mut budget = ListReadBudget {
        used: 0,
        limit: limits.read_bytes,
    };
    summarize_candidates(candidates, |session_id| {
        let file = store
            .files
            .get(session_id)
            .ok_or(ShowError::Missing)
            .map_err(FileInspectionError::Show)?;
        if file.locked {
            return Err(FileInspectionError::Show(ShowError::Busy));
        }
        if file.bytes.len() > MAX_SESSION_BYTES {
            return Err(FileInspectionError::Show(ShowError::Corrupt));
        }
        let size = u64::try_from(file.bytes.len())
            .map_err(|_| FileInspectionError::Show(ShowError::Corrupt))?;
        budget.charge(size)?;
        project(&file.bytes, session_id)
            .map_err(|error| FileInspectionError::Show(map_projection_error(error)))
    })
}

fn show_file(
    application_dir: &Path,
    sessions_dir: &Path,
    session_id: &str,
) -> Result<ShowResult, ShowError> {
    validate_private_directory(application_dir)?;
    validate_private_directory(sessions_dir)?;
    let path = sessions_dir.join(format!("{session_id}.jsonl"));
    inspect_file(&path, session_id, ReadPolicy::Show).map_err(|error| match error {
        FileInspectionError::Show(error) => error,
        FileInspectionError::InventoryTooLarge => {
            unreachable!("show does not use the aggregate list budget")
        }
    })
}

struct ListReadBudget {
    used: u64,
    limit: u64,
}

impl ListReadBudget {
    fn charge(&mut self, size: u64) -> Result<(), FileInspectionError> {
        let next = self
            .used
            .checked_add(size)
            .ok_or(FileInspectionError::InventoryTooLarge)?;
        if next > self.limit {
            return Err(FileInspectionError::InventoryTooLarge);
        }
        self.used = next;
        Ok(())
    }
}

enum ReadPolicy<'a> {
    Show,
    List(&'a mut ListReadBudget),
}

enum FileInspectionError {
    Show(ShowError),
    InventoryTooLarge,
}

impl From<ShowError> for FileInspectionError {
    fn from(error: ShowError) -> Self {
        Self::Show(error)
    }
}

fn inspect_file(
    path: &Path,
    session_id: &str,
    policy: ReadPolicy<'_>,
) -> Result<ShowResult, FileInspectionError> {
    inspect_file_with_hook(path, session_id, policy, |_, _| {})
}

fn inspect_file_with_hook<F>(
    path: &Path,
    session_id: &str,
    policy: ReadPolicy<'_>,
    before_read: F,
) -> Result<ShowResult, FileInspectionError>
where
    F: FnOnce(&File, &Path),
{
    let path_metadata = fs::symlink_metadata(path).map_err(map_open_error)?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(ShowError::StoreUnavailable.into());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(map_open_error)?;
    if !identity_matches(&file, path) || !private_file_permissions(&file) {
        return Err(ShowError::StoreUnavailable.into());
    }
    match FileExt::try_lock_shared(&file) {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(ShowError::Busy.into()),
        Err(TryLockError::Error(_)) => return Err(ShowError::StoreUnavailable.into()),
    }
    let size = file
        .metadata()
        .map_err(|_| ShowError::StoreUnavailable)?
        .len();
    if size > MAX_SESSION_BYTES as u64 {
        return Err(ShowError::Corrupt.into());
    }
    before_read(&file, path);
    let bytes = match policy {
        ReadPolicy::Show => {
            let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(MAX_SESSION_BYTES));
            Read::by_ref(&mut file)
                .take(MAX_SESSION_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ShowError::StoreUnavailable)?;
            if bytes.len() > MAX_SESSION_BYTES || !identity_matches(&file, path) {
                return Err(ShowError::Corrupt.into());
            }
            bytes
        }
        ReadPolicy::List(budget) => {
            budget.charge(size)?;
            let expected_size =
                usize::try_from(size).map_err(|_| FileInspectionError::Show(ShowError::Corrupt))?;
            let bytes = read_captured_bytes(&mut file, size)?;
            let unchanged_size = file.metadata().is_ok_and(|metadata| metadata.len() == size);
            if bytes.len() != expected_size || !unchanged_size || !identity_matches(&file, path) {
                return Err(ShowError::Corrupt.into());
            }
            bytes
        }
    };
    project(&bytes, session_id)
        .map_err(|error| FileInspectionError::Show(map_projection_error(error)))
}

fn read_captured_bytes(reader: &mut dyn Read, size: u64) -> Result<Vec<u8>, ShowError> {
    let capacity = usize::try_from(size).map_err(|_| ShowError::Corrupt)?;
    let mut bytes = Vec::with_capacity(capacity);
    reader
        .take(size)
        .read_to_end(&mut bytes)
        .map_err(|_| ShowError::StoreUnavailable)?;
    Ok(bytes)
}

#[cfg(test)]
fn show_memory(store: &Arc<Mutex<MemoryStore>>, session_id: &str) -> Result<ShowResult, ShowError> {
    let store = store.lock().map_err(|_| ShowError::StoreUnavailable)?;
    let file = store.files.get(session_id).ok_or(ShowError::Missing)?;
    if file.locked {
        return Err(ShowError::Busy);
    }
    project(&file.bytes, session_id).map_err(map_projection_error)
}

fn map_projection_error(error: ProjectionError) -> ShowError {
    match error {
        ProjectionError::UnsupportedVersion => ShowError::UnsupportedVersion,
        ProjectionError::Corrupt => ShowError::Corrupt,
    }
}

fn map_open_error(error: io::Error) -> ShowError {
    if error.kind() == io::ErrorKind::NotFound {
        ShowError::Missing
    } else {
        ShowError::StoreUnavailable
    }
}

fn ensure_private_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(io::Error::other("unsafe session directory"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::other("unsafe session directory"));
    }
    set_private_directory_permissions(path)?;
    Ok(())
}

fn validate_private_directory(path: &Path) -> Result<(), ShowError> {
    let metadata = fs::symlink_metadata(path).map_err(map_open_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ShowError::StoreUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ShowError::StoreUnavailable);
        }
    }
    Ok(())
}

fn create_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    let file = options.open(path)?;
    if set_private_file_permissions(path).is_err()
        || !identity_matches(&file, path)
        || !private_file_permissions(&file)
    {
        rollback_created_file(file, path);
        return Err(io::Error::other("unsafe session file"));
    }
    Ok(file)
}

fn identity_matches(file: &File, path: &Path) -> bool {
    let Ok(handle) = file.metadata() else {
        return false;
    };
    let Ok(path_metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() || !handle.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        handle.dev() == path_metadata.dev()
            && handle.ino() == path_metadata.ino()
            && handle.nlink() == 1
            && path_metadata.nlink() == 1
    }
    #[cfg(not(unix))]
    true
}

fn rollback_created_file(file: File, path: &Path) {
    let matches = identity_matches(&file, path);
    drop(file);
    if matches {
        let _ = fs::remove_file(path);
    }
}

fn private_file_permissions(file: &File) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        file.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o077 == 0)
    }
    #[cfg(not(unix))]
    true
}

fn set_private_directory_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ListedSessionStatus, ProjectedTerminal, SessionStatus};

    const SESSION: &str = "00000000000000000000000000000001";
    const OPERATION: &str = "00000000000000000000000000000002";
    const USER: &str = "00000000000000000000000000000003";
    const TERMINAL: &str = "00000000000000000000000000000004";
    const SECOND_SESSION: &str = "00000000000000000000000000000005";
    const SECOND_OPERATION: &str = "00000000000000000000000000000006";
    const SECOND_USER: &str = "00000000000000000000000000000007";
    const SECOND_TERMINAL: &str = "00000000000000000000000000000008";
    const THIRD_SESSION: &str = "00000000000000000000000000000009";
    const THIRD_OPERATION: &str = "0000000000000000000000000000000a";
    const THIRD_USER: &str = "0000000000000000000000000000000b";
    const THIRD_TERMINAL: &str = "0000000000000000000000000000000f";
    const FOURTH_SESSION: &str = "0000000000000000000000000000000c";
    const FOURTH_OPERATION: &str = "0000000000000000000000000000000d";
    const FOURTH_USER: &str = "0000000000000000000000000000000e";

    fn memory_log() -> SessionLog {
        SessionLog::memory(&[SESSION, OPERATION, USER, TERMINAL], &[0, 0, 1], None)
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let path = env::temp_dir().join(format!(
                "rust-agent-harness-session-log-{label}-{}",
                random_id()
            ));
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            if self
                .0
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rust-agent-harness-session-log-"))
            {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn completed_exchange_has_exact_jsonl_and_projection() {
        let mut log = memory_log();
        let writer = log.start("prompt").unwrap();
        assert_eq!(writer.session_id(), SESSION);
        writer.append_assistant("answer").unwrap();

        let expected = concat!(
            "{\"record_type\":\"session_header\",\"format_version\":1,\"session_id\":\"00000000000000000000000000000001\",\"created_at_unix_ms\":0,\"working_directory\":null,\"harness_version\":\"0.1.0\"}\n",
            "{\"record_type\":\"entry\",\"entry_id\":\"00000000000000000000000000000003\",\"parent_entry_id\":null,\"sequence\":1,\"timestamp_unix_ms\":0,\"operation_id\":\"00000000000000000000000000000002\",\"payload\":{\"type\":\"user_message\",\"provider\":\"openai-codex\",\"text\":\"prompt\"}}\n",
            "{\"record_type\":\"entry\",\"entry_id\":\"00000000000000000000000000000004\",\"parent_entry_id\":\"00000000000000000000000000000003\",\"sequence\":2,\"timestamp_unix_ms\":1,\"operation_id\":\"00000000000000000000000000000002\",\"payload\":{\"type\":\"assistant_message\",\"provider\":\"openai-codex\",\"text\":\"answer\",\"operation_outcome\":\"completed\"}}\n"
        );
        assert_eq!(log.memory_bytes(SESSION), expected.as_bytes());
        let shown = log.show(SESSION).unwrap();
        assert!(!shown.has_trailing_fragment());
        assert_eq!(shown.projection().status(), &SessionStatus::Completed);
        assert_eq!(shown.projection().created_at_unix_ms(), 0);
        assert_eq!(shown.projection().prompt(), "prompt");
        assert_eq!(
            shown.projection().terminal(),
            Some(&ProjectedTerminal::Assistant("answer".to_owned()))
        );
    }

    #[test]
    fn list_orders_projected_metadata_and_redacts_all_session_content() {
        let corrupt_session = "00000000000000000000000000000010";
        let unsupported_session = "00000000000000000000000000000011";
        let mut log = SessionLog::memory(
            &[
                SESSION,
                OPERATION,
                USER,
                TERMINAL,
                SECOND_SESSION,
                SECOND_OPERATION,
                SECOND_USER,
                SECOND_TERMINAL,
                THIRD_SESSION,
                THIRD_OPERATION,
                THIRD_USER,
                FOURTH_SESSION,
                FOURTH_OPERATION,
                FOURTH_USER,
            ],
            &[100, 101, 102, 300, 301, 302, 200, 201, 400, 401],
            None,
        );
        log.start("prompt-origin-secret-sentinel")
            .unwrap()
            .append_assistant("answer-origin-secret-sentinel")
            .unwrap();
        log.start("failure-prompt-sentinel")
            .unwrap()
            .append_failure(FailureCode::RateLimited)
            .unwrap();
        drop(log.start("interrupted-prompt-sentinel").unwrap());
        let busy_writer = log.start("busy-prompt-sentinel").unwrap();

        let Location::Memory(store) = &log.location else {
            unreachable!();
        };
        let mut store = store.lock().unwrap();
        store
            .files
            .get_mut(SESSION)
            .unwrap()
            .bytes
            .extend_from_slice(b"trailing-content-sentinel");
        store.files.insert(
            corrupt_session.to_owned(),
            MemoryFile {
                bytes: b"corrupt-content-sentinel\n".to_vec(),
                locked: false,
            },
        );
        store.files.insert(
            unsupported_session.to_owned(),
            MemoryFile {
                bytes: format!(
                    "{{\"record_type\":\"session_header\",\"format_version\":2,\"session_id\":\"{unsupported_session}\",\"created_at_unix_ms\":999,\"working_directory\":null,\"harness_version\":\"0.1.0\"}}\n"
                )
                .into_bytes(),
                locked: false,
            },
        );
        drop(store);

        let listed = log.list().unwrap();
        let entries = listed.entries();
        assert_eq!(entries.len(), 6);
        assert_eq!(entries[0].session_id(), SECOND_SESSION);
        assert_eq!(entries[0].created_at_unix_ms(), Some(300));
        assert_eq!(entries[0].status(), ListedSessionStatus::Failed);
        assert_eq!(entries[1].session_id(), THIRD_SESSION);
        assert_eq!(entries[1].created_at_unix_ms(), Some(200));
        assert_eq!(entries[1].status(), ListedSessionStatus::Interrupted);
        assert_eq!(entries[2].session_id(), SESSION);
        assert_eq!(entries[2].created_at_unix_ms(), Some(100));
        assert_eq!(entries[2].status(), ListedSessionStatus::Completed);
        assert!(entries[2].has_trailing_fragment());
        assert_eq!(entries[3].session_id(), FOURTH_SESSION);
        assert_eq!(entries[3].created_at_unix_ms(), None);
        assert_eq!(entries[3].status(), ListedSessionStatus::Busy);
        assert_eq!(entries[4].session_id(), corrupt_session);
        assert_eq!(entries[4].status(), ListedSessionStatus::Corrupt);
        assert_eq!(entries[5].session_id(), unsupported_session);
        assert_eq!(entries[5].status(), ListedSessionStatus::Unsupported);

        let debug = format!("{listed:?}");
        for forbidden in [
            "prompt-origin-secret-sentinel",
            "answer-origin-secret-sentinel",
            "failure-prompt-sentinel",
            "interrupted-prompt-sentinel",
            "busy-prompt-sentinel",
            "trailing-content-sentinel",
            "corrupt-content-sentinel",
            FailureCode::RateLimited.diagnostic(),
        ] {
            assert!(!debug.contains(forbidden));
        }
        drop(busy_writer);
    }

    #[test]
    fn list_bounds_errors_and_missing_races_are_fail_closed() {
        let exact_candidates = collect_candidate_ids(
            (0..MAX_LIST_CANDIDATES).map(|index| Ok(OsString::from(format!("{index:032x}.jsonl")))),
            LIST_LIMITS,
        )
        .unwrap();
        assert_eq!(exact_candidates.len(), MAX_LIST_CANDIDATES);
        assert!(
            summarize_candidates(exact_candidates, |_| {
                Err(FileInspectionError::Show(ShowError::Missing))
            })
            .unwrap()
            .entries()
            .is_empty()
        );
        assert_eq!(
            collect_candidate_ids(
                (0..=MAX_LIST_CANDIDATES)
                    .map(|index| { Ok(OsString::from(format!("{index:032x}.jsonl"))) }),
                LIST_LIMITS,
            ),
            Err(ListError::InventoryTooLarge)
        );

        assert!(
            collect_candidate_ids(
                (0..MAX_LIST_DIRECTORY_ENTRIES)
                    .map(|index| Ok(OsString::from(format!("ignored-{index}")))),
                LIST_LIMITS,
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            collect_candidate_ids(
                (0..=MAX_LIST_DIRECTORY_ENTRIES)
                    .map(|index| Ok(OsString::from(format!("ignored-{index}")))),
                LIST_LIMITS,
            ),
            Err(ListError::InventoryTooLarge)
        );

        assert_eq!(
            collect_candidate_ids(
                [Err(io::Error::other("injected read_dir failure"))],
                LIST_LIMITS,
            ),
            Err(ListError::StoreUnavailable)
        );
        assert_eq!(
            collect_candidate_ids(
                [
                    Ok(OsString::from(format!("{SESSION}.jsonl"))),
                    Err(io::Error::other("injected later read_dir failure")),
                ],
                LIST_LIMITS,
            ),
            Err(ListError::StoreUnavailable)
        );

        let mut log = memory_log();
        drop(log.start("prompt").unwrap());
        let exact_bytes = u64::try_from(log.memory_bytes(SESSION).len()).unwrap();
        let exact_limits = ListLimits {
            read_bytes: exact_bytes,
            ..LIST_LIMITS
        };
        assert_eq!(
            log.list_with_limits(exact_limits).unwrap().entries().len(),
            1
        );
        assert_eq!(
            log.list_with_limits(ListLimits {
                read_bytes: exact_bytes - 1,
                ..LIST_LIMITS
            }),
            Err(ListError::InventoryTooLarge)
        );

        let shown = log.show(SESSION).unwrap();
        let missing_session = "ffffffffffffffffffffffffffffffff";
        let listed = summarize_candidates(
            vec![SESSION.to_owned(), missing_session.to_owned()],
            |session_id| {
                if session_id == SESSION {
                    Ok(shown.clone())
                } else {
                    Err(FileInspectionError::Show(ShowError::Missing))
                }
            },
        )
        .unwrap();
        assert_eq!(listed.entries().len(), 1);
        assert_eq!(listed.entries()[0].session_id(), SESSION);

        let candidates = collect_candidate_ids(
            [
                Ok(OsString::from("not-a-session.jsonl")),
                Ok(OsString::from(format!("{SESSION}.jsonl"))),
            ],
            LIST_LIMITS,
        )
        .unwrap();
        let mut inspections = 0;
        let _ = summarize_candidates(candidates, |_| {
            inspections += 1;
            Ok(shown.clone())
        })
        .unwrap();
        assert_eq!(inspections, 1);

        let mut overflow = ListReadBudget {
            used: u64::MAX,
            limit: u64::MAX,
        };
        assert!(matches!(
            overflow.charge(1),
            Err(FileInspectionError::InventoryTooLarge)
        ));

        let Location::Memory(store) = &log.location else {
            unreachable!();
        };
        store.lock().unwrap().files.get_mut(SESSION).unwrap().bytes =
            vec![b'x'; MAX_SESSION_BYTES + 1];
        let listed = log
            .list_with_limits(ListLimits {
                read_bytes: 0,
                ..LIST_LIMITS
            })
            .unwrap();
        assert_eq!(listed.entries()[0].status(), ListedSessionStatus::Corrupt);
    }

    #[test]
    fn every_failure_code_has_exact_closed_payload_and_projection() {
        let failures = [
            FailureCode::UnsupportedPlatform,
            FailureCode::NotConnected,
            FailureCode::CredentialStoreUnavailable,
            FailureCode::InvalidCredential,
            FailureCode::ExpiredCredential,
            FailureCode::CredentialRefreshBusy,
            FailureCode::CredentialRefreshUnavailable,
            FailureCode::Unavailable,
            FailureCode::UnexpectedProviderResponse,
            FailureCode::ConnectionRejected,
            FailureCode::AccessDenied,
            FailureCode::RateLimited,
            FailureCode::TemporarilyUnavailable,
            FailureCode::Rejected,
            FailureCode::InvalidProviderResponse,
            FailureCode::CallTimedOut,
            FailureCode::ResponseTooLarge,
            FailureCode::ModelCallFailed,
            FailureCode::EmptyResponse,
        ];

        for failure in failures {
            let mut log = memory_log();
            log.start("prompt")
                .unwrap()
                .append_failure(failure)
                .unwrap();
            let bytes = String::from_utf8(log.memory_bytes(SESSION)).unwrap();
            assert!(bytes.contains(&format!(
                "\"failure_code\":{}",
                serde_json::to_string(&failure).unwrap()
            )));
            assert!(bytes.contains(&format!(
                "\"diagnostic\":{}",
                serde_json::to_string(failure.diagnostic()).unwrap()
            )));
            let shown = log.show(SESSION).unwrap();
            assert_eq!(shown.projection().status(), &SessionStatus::Failed);
            assert_eq!(
                shown.projection().terminal(),
                Some(&ProjectedTerminal::Failure(failure))
            );
        }
    }

    #[test]
    fn unfinished_writer_is_busy_then_projects_interrupted_after_drop() {
        let mut log = memory_log();
        let writer = log.start("prompt").unwrap();
        assert_eq!(log.show(SESSION), Err(ShowError::Busy));
        drop(writer);
        let shown = log.show(SESSION).unwrap();
        assert_eq!(shown.projection().status(), &SessionStatus::Interrupted);
        assert_eq!(shown.projection().terminal(), None);
    }

    #[test]
    fn rollback_removes_only_the_creating_writer_file() {
        let mut log = memory_log();
        log.start("prompt").unwrap().rollback().unwrap();
        assert_eq!(log.show(SESSION), Err(ShowError::Missing));
    }

    #[test]
    fn trailing_terminal_fragment_warns_projects_prefix_and_never_mutates() {
        let mut log = memory_log();
        let writer = log.start("prompt").unwrap();
        drop(writer);
        let Location::Memory(store) = &log.location else {
            unreachable!();
        };
        store
            .lock()
            .unwrap()
            .files
            .get_mut(SESSION)
            .unwrap()
            .bytes
            .extend(b"{\"record_type\":");
        let before = log.memory_bytes(SESSION);
        let shown = log.show(SESSION).unwrap();
        assert!(shown.has_trailing_fragment());
        assert_eq!(shown.projection().status(), &SessionStatus::Interrupted);
        assert_eq!(log.memory_bytes(SESSION), before);

        let mut completed = memory_log();
        completed
            .start("prompt")
            .unwrap()
            .append_assistant("answer")
            .unwrap();
        let Location::Memory(store) = &completed.location else {
            unreachable!();
        };
        store
            .lock()
            .unwrap()
            .files
            .get_mut(SESSION)
            .unwrap()
            .bytes
            .extend(b"torn");
        let before = completed.memory_bytes(SESSION);
        let shown = completed.show(SESSION).unwrap();
        assert!(shown.has_trailing_fragment());
        assert_eq!(shown.projection().status(), &SessionStatus::Completed);
        assert_eq!(completed.memory_bytes(SESSION), before);
    }

    #[test]
    fn torn_header_header_only_and_malformed_complete_line_are_corrupt() {
        for bytes in [
            b"{\"record_type\":\"session_header\"".to_vec(),
            format!(
                "{{\"record_type\":\"session_header\",\"format_version\":1,\"session_id\":\"{SESSION}\",\"created_at_unix_ms\":0,\"working_directory\":null,\"harness_version\":\"0.1.0\"}}\n"
            )
            .into_bytes(),
            format!(
                "{{\"record_type\":\"session_header\",\"format_version\":1,\"session_id\":\"{SESSION}\",\"created_at_unix_ms\":0,\"working_directory\":null,\"harness_version\":\"0.1.0\"}}\n{{\"record_type\":\"entry\""
            )
            .into_bytes(),
            format!(
                "{{\"record_type\":\"session_header\",\"format_version\":1,\"session_id\":\"{SESSION}\",\"created_at_unix_ms\":0,\"working_directory\":null,\"harness_version\":\"0.1.0\"}}\nnot-json\n"
            )
            .into_bytes(),
        ] {
            let mut log = memory_log();
            let writer = log.start("prompt").unwrap();
            drop(writer);
            let Location::Memory(store) = &log.location else {
                unreachable!();
            };
            store.lock().unwrap().files.get_mut(SESSION).unwrap().bytes = bytes.clone();
            assert_eq!(log.show(SESSION), Err(ShowError::Corrupt));
            assert_eq!(log.memory_bytes(SESSION), bytes);
        }
    }

    #[test]
    fn schema_identity_sequence_parent_and_operation_mutations_are_rejected() {
        let mut log = memory_log();
        log.start("prompt")
            .unwrap()
            .append_assistant("answer")
            .unwrap();
        let golden = String::from_utf8(log.memory_bytes(SESSION)).unwrap();
        for mutated in [
            golden.replace("\"sequence\":1", "\"sequence\":2"),
            golden.replace(USER, SESSION),
            golden.replace(
                &format!("\"parent_entry_id\":\"{USER}\""),
                "\"parent_entry_id\":null",
            ),
            golden.replacen(OPERATION, TERMINAL, 1),
            golden.replace("\"assistant_message\"", "\"unknown_payload\""),
            golden.replacen(
                "\n{\"record_type\":\"entry\"",
                "\n\n{\"record_type\":\"entry\"",
                1,
            ),
            format!("{golden}{}", golden.lines().last().unwrap()) + "\n",
        ] {
            let mut fixture = memory_log();
            let writer = fixture.start("prompt").unwrap();
            drop(writer);
            let Location::Memory(store) = &fixture.location else {
                unreachable!();
            };
            store.lock().unwrap().files.get_mut(SESSION).unwrap().bytes = mutated.into_bytes();
            assert_eq!(fixture.show(SESSION), Err(ShowError::Corrupt));
        }
    }

    #[test]
    fn unsupported_version_and_filename_header_mismatch_are_distinct() {
        let mut log = memory_log();
        let writer = log.start("prompt").unwrap();
        drop(writer);
        let Location::Memory(store) = &log.location else {
            unreachable!();
        };
        let mut store = store.lock().unwrap();
        let file = store.files.get_mut(SESSION).unwrap();
        let original = String::from_utf8(file.bytes.clone()).unwrap();
        file.bytes = original
            .replace("\"format_version\":1", "\"format_version\":2")
            .into_bytes();
        drop(store);
        assert_eq!(log.show(SESSION), Err(ShowError::UnsupportedVersion));

        let mut log = memory_log();
        let writer = log.start("prompt").unwrap();
        drop(writer);
        let Location::Memory(store) = &log.location else {
            unreachable!();
        };
        let mut store = store.lock().unwrap();
        let file = store.files.get_mut(SESSION).unwrap();
        file.bytes = String::from_utf8(file.bytes.clone())
            .unwrap()
            .replacen(SESSION, TERMINAL, 1)
            .into_bytes();
        drop(store);
        assert_eq!(log.show(SESSION), Err(ShowError::Corrupt));
    }

    #[test]
    fn persisted_failure_diagnostic_must_match_its_closed_code() {
        let mut log = memory_log();
        log.start("prompt")
            .unwrap()
            .append_failure(FailureCode::RateLimited)
            .unwrap();
        let Location::Memory(store) = &log.location else {
            unreachable!();
        };
        let mut store = store.lock().unwrap();
        let file = store.files.get_mut(SESSION).unwrap();
        file.bytes = String::from_utf8(file.bytes.clone())
            .unwrap()
            .replace(
                FailureCode::RateLimited.diagnostic(),
                "raw provider body sentinel",
            )
            .into_bytes();
        drop(store);
        assert_eq!(log.show(SESSION), Err(ShowError::Corrupt));
    }

    #[cfg(unix)]
    #[test]
    fn file_mode_directories_mode_locking_and_round_trip_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempRoot::new("private");
        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[SESSION, OPERATION, USER, TERMINAL],
            &[0, 0, 1],
            None,
        );
        let writer = log.start("prompt").unwrap();
        assert_eq!(log.show(SESSION), Err(ShowError::Busy));
        writer.append_assistant("answer").unwrap();
        assert_eq!(
            log.show(SESSION).unwrap().projection().status(),
            &SessionStatus::Completed
        );
        assert_eq!(
            fs::metadata(&root.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.0.join("sessions"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.0.join("sessions").join(format!("{SESSION}.jsonl")))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn missing_show_does_not_create_directories() {
        let root = TempRoot::new("missing");
        let log = SessionLog::at_data_local_dir(root.0.clone());
        assert_eq!(log.show(SESSION), Err(ShowError::Missing));
        assert!(log.list().unwrap().entries().is_empty());
        assert!(!root.0.exists());

        fs::create_dir(&root.0).unwrap();
        set_private_directory_permissions(&root.0).unwrap();
        assert!(log.list().unwrap().entries().is_empty());
        assert!(!root.0.join("sessions").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(log.list(), Err(ListError::StoreUnavailable));
            fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(root.0.join("sessions"), b"not a directory").unwrap();
        assert_eq!(log.list(), Err(ListError::StoreUnavailable));

        let unavailable = SessionLog {
            location: Location::Unavailable,
            sources: Arc::new(Mutex::new(Sources::production())),
        };
        assert_eq!(unavailable.list(), Err(ListError::StoreUnavailable));
    }

    #[cfg(unix)]
    #[test]
    fn file_list_uses_header_time_and_preserves_every_canonical_byte() {
        use std::fs::FileTimes;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::time::Duration;

        let root = TempRoot::new("list-order");
        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[
                SESSION,
                OPERATION,
                USER,
                TERMINAL,
                SECOND_SESSION,
                SECOND_OPERATION,
                SECOND_USER,
                SECOND_TERMINAL,
                THIRD_SESSION,
                THIRD_OPERATION,
                THIRD_USER,
                THIRD_TERMINAL,
            ],
            &[900, 901, 902, 900, 903, 904, 100, 905, 906],
            None,
        );
        log.start("first prompt")
            .unwrap()
            .append_assistant("first answer")
            .unwrap();
        log.start("second prompt")
            .unwrap()
            .append_assistant("second answer")
            .unwrap();
        log.start("third prompt")
            .unwrap()
            .append_assistant("third answer")
            .unwrap();

        let sessions = root.0.join("sessions");
        let first = sessions.join(format!("{SESSION}.jsonl"));
        let second = sessions.join(format!("{SECOND_SESSION}.jsonl"));
        let third = sessions.join(format!("{THIRD_SESSION}.jsonl"));
        OpenOptions::new()
            .append(true)
            .open(&second)
            .unwrap()
            .write_all(b"trailing-fragment-sentinel")
            .unwrap();
        for (path, modified) in [
            (&first, UNIX_EPOCH + Duration::from_secs(1)),
            (&second, UNIX_EPOCH + Duration::from_secs(2)),
            (&third, UNIX_EPOCH + Duration::from_secs(3)),
        ] {
            File::open(path)
                .unwrap()
                .set_times(FileTimes::new().set_modified(modified))
                .unwrap();
        }

        let before = [&first, &second, &third].map(|path| {
            let metadata = fs::metadata(path).unwrap();
            (
                fs::read(path).unwrap(),
                metadata.modified().unwrap(),
                metadata.permissions().mode(),
                metadata.nlink(),
            )
        });
        let listed = log.list().unwrap();
        assert_eq!(
            listed
                .entries()
                .iter()
                .map(|entry| (
                    entry.session_id(),
                    entry.created_at_unix_ms(),
                    entry.status(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (SESSION, Some(900), ListedSessionStatus::Completed),
                (SECOND_SESSION, Some(900), ListedSessionStatus::Completed,),
                (THIRD_SESSION, Some(100), ListedSessionStatus::Completed,),
            ]
        );
        assert!(listed.entries()[1].has_trailing_fragment());
        let after = [&first, &second, &third].map(|path| {
            let metadata = fs::metadata(path).unwrap();
            (
                fs::read(path).unwrap(),
                metadata.modified().unwrap(),
                metadata.permissions().mode(),
                metadata.nlink(),
            )
        });
        assert_eq!(after, before);
    }

    #[cfg(unix)]
    #[test]
    fn file_list_maps_unhealthy_neighbors_without_mutation() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

        fn write_private(path: &Path, bytes: &[u8]) {
            fs::write(path, bytes).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let corrupt = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let unsupported = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let symlinked = "cccccccccccccccccccccccccccccccc";
        let hard_linked = "dddddddddddddddddddddddddddddddd";
        let non_regular = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let wrong_permissions = "ffffffffffffffffffffffffffffffff";
        let root = TempRoot::new("list-unhealthy");
        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[
                SECOND_SESSION,
                SECOND_OPERATION,
                SECOND_USER,
                SECOND_TERMINAL,
                SESSION,
                OPERATION,
                USER,
            ],
            &[500, 501, 502, 600, 601],
            None,
        );
        log.start("valid-neighbor-prompt")
            .unwrap()
            .append_assistant("valid-neighbor-answer")
            .unwrap();
        let busy_writer = log.start("busy-neighbor-prompt").unwrap();
        let sessions = root.0.join("sessions");

        let corrupt_path = sessions.join(format!("{corrupt}.jsonl"));
        write_private(&corrupt_path, b"corrupt-private-sentinel\n");
        let unsupported_path = sessions.join(format!("{unsupported}.jsonl"));
        write_private(
            &unsupported_path,
            format!(
                "{{\"record_type\":\"session_header\",\"format_version\":2,\"session_id\":\"{unsupported}\",\"created_at_unix_ms\":999,\"working_directory\":null,\"harness_version\":\"0.1.0\"}}\n"
            )
            .as_bytes(),
        );
        let target = root.0.join("link-target");
        write_private(&target, b"link-target-sentinel");
        symlink(&target, sessions.join(format!("{symlinked}.jsonl"))).unwrap();
        let hard_target = root.0.join("hard-target");
        write_private(&hard_target, b"hard-link-target-sentinel");
        fs::hard_link(&hard_target, sessions.join(format!("{hard_linked}.jsonl"))).unwrap();
        fs::create_dir(sessions.join(format!("{non_regular}.jsonl"))).unwrap();
        let wrong_path = sessions.join(format!("{wrong_permissions}.jsonl"));
        fs::write(&wrong_path, b"wrong-permission-sentinel").unwrap();
        fs::set_permissions(&wrong_path, fs::Permissions::from_mode(0o644)).unwrap();
        let ignored = sessions.join("not-a-session.jsonl");
        fs::write(&ignored, b"ignored-noncanonical-sentinel").unwrap();
        fs::set_permissions(&ignored, fs::Permissions::from_mode(0o600)).unwrap();

        let regular_paths = [
            sessions.join(format!("{SECOND_SESSION}.jsonl")),
            sessions.join(format!("{SESSION}.jsonl")),
            corrupt_path,
            unsupported_path,
            target.clone(),
            hard_target.clone(),
            wrong_path,
            ignored,
        ];
        let before = regular_paths
            .iter()
            .map(|path| {
                let metadata = fs::metadata(path).unwrap();
                (
                    fs::read(path).unwrap(),
                    metadata.modified().unwrap(),
                    metadata.permissions().mode(),
                    metadata.nlink(),
                )
            })
            .collect::<Vec<_>>();
        let listed = log.list().unwrap();
        assert_eq!(listed.entries().len(), 8);
        assert_eq!(listed.entries()[0].session_id(), SECOND_SESSION);
        assert_eq!(listed.entries()[0].status(), ListedSessionStatus::Completed);
        assert_eq!(listed.entries()[1].session_id(), SESSION);
        assert_eq!(listed.entries()[1].status(), ListedSessionStatus::Busy);
        for (entry, expected_id, expected_status) in [
            (&listed.entries()[2], corrupt, ListedSessionStatus::Corrupt),
            (
                &listed.entries()[3],
                unsupported,
                ListedSessionStatus::Unsupported,
            ),
            (
                &listed.entries()[4],
                symlinked,
                ListedSessionStatus::Unavailable,
            ),
            (
                &listed.entries()[5],
                hard_linked,
                ListedSessionStatus::Unavailable,
            ),
            (
                &listed.entries()[6],
                non_regular,
                ListedSessionStatus::Unavailable,
            ),
            (
                &listed.entries()[7],
                wrong_permissions,
                ListedSessionStatus::Unavailable,
            ),
        ] {
            assert_eq!(entry.session_id(), expected_id);
            assert_eq!(entry.status(), expected_status);
            assert_eq!(entry.created_at_unix_ms(), None);
        }
        let after = regular_paths
            .iter()
            .map(|path| {
                let metadata = fs::metadata(path).unwrap();
                (
                    fs::read(path).unwrap(),
                    metadata.modified().unwrap(),
                    metadata.permissions().mode(),
                    metadata.nlink(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(after, before);
        assert_eq!(
            fs::read_link(sessions.join(format!("{symlinked}.jsonl"))).unwrap(),
            target
        );
        assert!(sessions.join(format!("{non_regular}.jsonl")).is_dir());
        drop(busy_writer);
    }

    #[cfg(unix)]
    #[test]
    fn file_list_enforces_locked_aggregate_budget_before_the_next_read() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempRoot::new("list-aggregate");
        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[
                SESSION,
                OPERATION,
                USER,
                TERMINAL,
                SECOND_SESSION,
                SECOND_OPERATION,
                SECOND_USER,
                SECOND_TERMINAL,
            ],
            &[100, 101, 102, 200, 201, 202],
            None,
        );
        log.start("first")
            .unwrap()
            .append_assistant("answer-one")
            .unwrap();
        log.start("second")
            .unwrap()
            .append_assistant("answer-two")
            .unwrap();
        let sessions = root.0.join("sessions");
        let first = sessions.join(format!("{SESSION}.jsonl"));
        let second = sessions.join(format!("{SECOND_SESSION}.jsonl"));
        let exact = fs::metadata(&first).unwrap().len() + fs::metadata(&second).unwrap().len();
        let before = [fs::read(&first).unwrap(), fs::read(&second).unwrap()];
        let limits = ListLimits {
            read_bytes: exact,
            ..LIST_LIMITS
        };
        assert_eq!(log.list_with_limits(limits).unwrap().entries().len(), 2);
        assert_eq!(
            log.list_with_limits(ListLimits {
                read_bytes: exact - 1,
                ..LIST_LIMITS
            }),
            Err(ListError::InventoryTooLarge)
        );
        assert_eq!(
            [fs::read(&first).unwrap(), fs::read(&second).unwrap()],
            before
        );

        let oversized_id = "ffffffffffffffffffffffffffffffff";
        let oversized_path = sessions.join(format!("{oversized_id}.jsonl"));
        fs::write(&oversized_path, vec![b'x'; MAX_SESSION_BYTES + 1]).unwrap();
        fs::set_permissions(&oversized_path, fs::Permissions::from_mode(0o600)).unwrap();
        let listed = log.list_with_limits(limits).unwrap();
        assert_eq!(listed.entries().len(), 3);
        assert_eq!(listed.entries()[2].status(), ListedSessionStatus::Corrupt);
    }

    #[cfg(unix)]
    #[test]
    fn file_list_detects_path_replacement_and_maps_read_errors_to_unavailable() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempRoot::new("list-races");
        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[SESSION, OPERATION, USER, TERMINAL],
            &[100, 101, 102],
            None,
        );
        log.start("prompt")
            .unwrap()
            .append_assistant("answer")
            .unwrap();
        let path = root.0.join("sessions").join(format!("{SESSION}.jsonl"));
        let displaced = root.0.join("sessions/displaced.jsonl");
        let original = fs::read(&path).unwrap();
        let replacement = b"replacement-race-sentinel";
        let mut budget = ListReadBudget {
            used: 0,
            limit: MAX_LIST_READ_BYTES,
        };
        let result = inspect_file_with_hook(
            &path,
            SESSION,
            ReadPolicy::List(&mut budget),
            |_, canonical| {
                fs::rename(canonical, &displaced).unwrap();
                fs::write(canonical, replacement).unwrap();
                fs::set_permissions(canonical, fs::Permissions::from_mode(0o600)).unwrap();
            },
        );
        assert!(matches!(
            result,
            Err(FileInspectionError::Show(ShowError::Corrupt))
        ));
        assert_eq!(fs::read(&displaced).unwrap(), original);
        assert_eq!(fs::read(&path).unwrap(), replacement);

        struct FailingReader;

        impl Read for FailingReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected read failure"))
            }
        }

        let error = read_captured_bytes(&mut FailingReader, 1).unwrap_err();
        assert_eq!(error, ShowError::StoreUnavailable);
        let listed = summarize_candidates(vec![SESSION.to_owned()], |_| {
            Err(FileInspectionError::Show(error))
        })
        .unwrap();
        assert_eq!(listed.entries().len(), 1);
        assert_eq!(
            listed.entries()[0].status(),
            ListedSessionStatus::Unavailable
        );
    }

    #[cfg(unix)]
    #[test]
    fn oversized_file_is_rejected_without_mutation() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempRoot::new("oversized");
        let sessions = root.0.join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
        let path = sessions.join(format!("{SESSION}.jsonl"));
        let bytes = vec![b'x'; crate::format::MAX_SESSION_BYTES + 1];
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let log = SessionLog::at_data_local_dir(root.0.clone());
        assert_eq!(log.show(SESSION), Err(ShowError::Corrupt));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hard_link_files_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = TempRoot::new("links");
        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[SESSION, OPERATION, USER, TERMINAL],
            &[0, 0, 1],
            None,
        );
        log.start("prompt")
            .unwrap()
            .append_assistant("answer")
            .unwrap();
        let canonical = root.0.join("sessions").join(format!("{SESSION}.jsonl"));
        let linked_id = "00000000000000000000000000000005";
        let hard_link = root.0.join("sessions").join(format!("{linked_id}.jsonl"));
        fs::hard_link(&canonical, &hard_link).unwrap();
        assert_eq!(log.show(SESSION), Err(ShowError::StoreUnavailable));
        fs::remove_file(&hard_link).unwrap();

        let target = root.0.join("target.jsonl");
        fs::rename(&canonical, &target).unwrap();
        symlink(&target, &canonical).unwrap();
        assert_eq!(log.show(SESSION), Err(ShowError::StoreUnavailable));
        fs::remove_file(&canonical).unwrap();
        fs::create_dir(&canonical).unwrap();
        assert_eq!(log.show(SESSION), Err(ShowError::StoreUnavailable));
    }

    #[cfg(unix)]
    #[test]
    fn path_replacement_prevents_append_and_rollback_from_touching_the_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempRoot::new("replacement");
        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[SESSION, OPERATION, USER, TERMINAL],
            &[0, 0, 1],
            None,
        );
        let writer = log.start("prompt").unwrap();
        let canonical = root.0.join("sessions").join(format!("{SESSION}.jsonl"));
        let displaced = root.0.join("sessions/displaced.jsonl");
        fs::rename(&canonical, &displaced).unwrap();
        fs::write(&canonical, b"replacement").unwrap();
        fs::set_permissions(&canonical, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(writer.rollback(), Err(RollbackError::StoreUnavailable));
        assert_eq!(fs::read(&canonical).unwrap(), b"replacement");

        let mut log = SessionLog::fixed_file(
            root.0.clone(),
            &[
                "00000000000000000000000000000005",
                "00000000000000000000000000000006",
                "00000000000000000000000000000007",
                "00000000000000000000000000000008",
            ],
            &[0, 0, 1],
            None,
        );
        let writer = log.start("prompt").unwrap();
        let session_id = writer.session_id().to_owned();
        let canonical = root.0.join("sessions").join(format!("{session_id}.jsonl"));
        let displaced = root.0.join("sessions/displaced-two.jsonl");
        fs::rename(&canonical, displaced).unwrap();
        fs::write(&canonical, b"replacement-two").unwrap();
        fs::set_permissions(&canonical, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            writer.append_assistant("answer"),
            Err(AppendError::StoreUnavailable)
        );
        assert_eq!(fs::read(&canonical).unwrap(), b"replacement-two");
    }

    #[test]
    fn identifiers_must_be_exact_lowercase_hex() {
        let log = memory_log();
        for id in [
            "0000000000000000000000000000000",
            "000000000000000000000000000000000",
            "0000000000000000000000000000000A",
            "../../00000000000000000000000000",
            "0000000000000000000000000000000g",
        ] {
            assert_eq!(log.show(id), Err(ShowError::InvalidSessionId));
        }
    }

    #[test]
    fn nullable_working_directory_is_golden_and_non_load_bearing() {
        let mut log = SessionLog::memory(&[SESSION, OPERATION, USER, TERMINAL], &[0, 0, 1], None);
        log.start("prompt")
            .unwrap()
            .append_assistant("answer")
            .unwrap();
        assert!(
            String::from_utf8(log.memory_bytes(SESSION))
                .unwrap()
                .contains("\"working_directory\":null")
        );

        let mut log = SessionLog::memory(
            &[SESSION, OPERATION, USER, TERMINAL],
            &[0, 0, 1],
            Some("/workspace"),
        );
        log.start("prompt")
            .unwrap()
            .append_assistant("answer")
            .unwrap();
        assert!(
            String::from_utf8(log.memory_bytes(SESSION))
                .unwrap()
                .contains("\"working_directory\":\"/workspace\"")
        );
    }

    #[test]
    fn a_session_filename_collision_regenerates_the_complete_identity_set() {
        let second_session = "00000000000000000000000000000007";
        let mut log = SessionLog::memory(
            &[
                SESSION,
                OPERATION,
                USER,
                SESSION,
                "00000000000000000000000000000005",
                "00000000000000000000000000000006",
                second_session,
                "00000000000000000000000000000008",
                "00000000000000000000000000000009",
                "0000000000000000000000000000000a",
            ],
            &[0, 0, 1, 1, 2, 2, 3],
            None,
        );
        drop(log.start("first").unwrap());
        let writer = log.start("second").unwrap();
        assert_eq!(writer.session_id(), second_session);
        writer.append_assistant("answer").unwrap();
        assert_eq!(log.show(SESSION).unwrap().projection().prompt(), "first");
        assert_eq!(
            log.show(second_session).unwrap().projection().prompt(),
            "second"
        );
    }

    #[test]
    fn concurrent_creators_produce_distinct_complete_sessions() {
        let root = TempRoot::new("concurrent");
        let mut threads = Vec::new();
        for index in 0..24 {
            let application_dir = root.0.clone();
            threads.push(std::thread::spawn(move || {
                let mut log = SessionLog::at_data_local_dir(application_dir);
                let prompt = format!("prompt-{index}");
                let writer = log.start(&prompt).unwrap();
                let session_id = writer.session_id().to_owned();
                writer.append_assistant("answer").unwrap();
                (session_id, prompt)
            }));
        }
        let sessions = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        let unique = sessions
            .iter()
            .map(|(session_id, _)| session_id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), sessions.len());

        let log = SessionLog::at_data_local_dir(root.0.clone());
        for (session_id, prompt) in sessions {
            let shown = log.show(&session_id).unwrap();
            assert_eq!(shown.projection().prompt(), prompt);
            assert_eq!(shown.projection().status(), &SessionStatus::Completed);
        }
    }

    #[test]
    fn record_and_file_bounds_reject_oversized_input_without_unbounded_reads() {
        let mut log = memory_log();
        let oversized = "\u{1f}".repeat(crate::format::MAX_RECORD_BYTES + 1);
        assert!(matches!(
            log.start(&oversized),
            Err(CreateError::StoreUnavailable)
        ));

        let mut log = memory_log();
        let writer = log.start("prompt").unwrap();
        assert_eq!(
            writer.append_assistant(&"x".repeat(crate::format::MAX_RECORD_BYTES + 1)),
            Err(AppendError::StoreUnavailable)
        );

        let empty = assistant_entry(
            TERMINAL.to_owned(),
            USER.to_owned(),
            1,
            OPERATION.to_owned(),
            String::new(),
        );
        let overhead = serde_json::to_vec(&empty).unwrap().len();
        let exact = assistant_entry(
            TERMINAL.to_owned(),
            USER.to_owned(),
            1,
            OPERATION.to_owned(),
            "x".repeat(crate::format::MAX_RECORD_BYTES - overhead),
        );
        assert_eq!(
            serialize_line(&exact).unwrap().len(),
            crate::format::MAX_RECORD_BYTES + 1
        );
        let beyond = assistant_entry(
            TERMINAL.to_owned(),
            USER.to_owned(),
            1,
            OPERATION.to_owned(),
            "x".repeat(crate::format::MAX_RECORD_BYTES - overhead + 1),
        );
        assert!(serialize_line(&beyond).is_err());
        assert_eq!(
            project(&vec![b'x'; crate::format::MAX_SESSION_BYTES + 1], SESSION),
            Err(ProjectionError::Corrupt)
        );

        let mut expanded = memory_log();
        expanded
            .start(&"\u{1b}".repeat(32 * 1024))
            .unwrap()
            .append_assistant(&"\u{1b}".repeat(256 * 1024))
            .unwrap();
        assert_eq!(
            expanded.show(SESSION).unwrap().projection().status(),
            &SessionStatus::Completed
        );
    }
}
