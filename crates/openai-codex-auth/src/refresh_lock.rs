use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant;

use fs4::TryLockError;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

const LOCK_DIRECTORY: &str = "locks";
const LOCK_FILE: &str = "openai-codex-refresh.lock";
const WAIT_TIMEOUT: Duration = Duration::from_secs(65);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(test)]
pub(crate) static FILE_LOCK_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) trait RefreshLock {
    type Guard;

    fn acquire(&self) -> Result<Self::Guard, LockError>;
}

pub(crate) trait WaitClock {
    fn now(&self) -> Duration;
}

pub(crate) trait Sleeper {
    fn sleep(&self, duration: Duration);
}

pub(crate) struct FileRefreshLock<C, S> {
    private_root: PathBuf,
    lock_path: PathBuf,
    clock: C,
    sleeper: S,
    wait_timeout: Duration,
    poll_interval: Duration,
}

impl<C, S> FileRefreshLock<C, S> {
    #[cfg(test)]
    pub(crate) fn with_root(private_root: PathBuf, clock: C, sleeper: S) -> Self {
        let lock_path = private_root.join(LOCK_DIRECTORY).join(LOCK_FILE);
        Self {
            private_root,
            lock_path,
            clock,
            sleeper,
            wait_timeout: WAIT_TIMEOUT,
            poll_interval: POLL_INTERVAL,
        }
    }

    #[cfg(test)]
    fn with_limits(mut self, wait_timeout: Duration, poll_interval: Duration) -> Self {
        self.wait_timeout = wait_timeout;
        self.poll_interval = poll_interval;
        self
    }

    #[cfg(test)]
    fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

#[cfg(target_os = "macos")]
impl FileRefreshLock<SystemWaitClock, ThreadSleeper> {
    pub(crate) fn production() -> Result<Self, LockError> {
        let project = directories::ProjectDirs::from("", "", "rust-agent-harness")
            .ok_or(LockError::Unavailable)?;
        let private_root = project.data_local_dir().to_owned();
        Ok(Self {
            lock_path: private_root.join(LOCK_DIRECTORY).join(LOCK_FILE),
            private_root,
            clock: SystemWaitClock::new(),
            sleeper: ThreadSleeper,
            wait_timeout: WAIT_TIMEOUT,
            poll_interval: POLL_INTERVAL,
        })
    }
}

impl<C: WaitClock, S: Sleeper> RefreshLock for FileRefreshLock<C, S> {
    type Guard = FileLockGuard;

    fn acquire(&self) -> Result<Self::Guard, LockError> {
        prepare_private_directory(&self.private_root)?;
        let lock_directory = self.lock_path.parent().ok_or(LockError::Unavailable)?;
        prepare_private_directory(lock_directory)?;
        let file = open_lock_file(&self.lock_path)?;
        let started = self.clock.now();

        loop {
            match fs4::FileExt::try_lock(&file) {
                Ok(()) => return Ok(FileLockGuard { _file: file }),
                Err(TryLockError::WouldBlock) => {
                    let elapsed = self.clock.now().saturating_sub(started);
                    if elapsed >= self.wait_timeout {
                        return Err(LockError::Busy);
                    }
                    self.sleeper.sleep(self.poll_interval);
                }
                Err(TryLockError::Error(_)) => return Err(LockError::Unavailable),
            }
        }
    }
}

pub(crate) struct FileLockGuard {
    _file: File,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LockError {
    Busy,
    Unavailable,
}

#[cfg(target_os = "macos")]
pub(crate) struct SystemWaitClock {
    started: Instant,
}

#[cfg(target_os = "macos")]
impl SystemWaitClock {
    fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

#[cfg(target_os = "macos")]
impl WaitClock for SystemWaitClock {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }
}

pub(crate) struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

fn prepare_private_directory(path: &Path) -> Result<(), LockError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(LockError::Unavailable);
        }
    } else {
        create_private_directory(path)?;
    }

    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| LockError::Unavailable)?;
    Ok(())
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), LockError> {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path).map_err(|_| LockError::Unavailable)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<(), LockError> {
    fs::create_dir_all(path).map_err(|_| LockError::Unavailable)
}

fn open_lock_file(path: &Path) -> Result<File, LockError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 0 {
            return Err(LockError::Unavailable);
        }
    }

    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path).map_err(|_| LockError::Unavailable)?;

    let file_metadata = file.metadata().map_err(|_| LockError::Unavailable)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| LockError::Unavailable)?;
    if path_metadata.file_type().is_symlink()
        || !file_metadata.is_file()
        || !path_metadata.is_file()
        || file_metadata.len() != 0
        || path_metadata.len() != 0
    {
        return Err(LockError::Unavailable);
    }

    #[cfg(unix)]
    {
        if file_metadata.dev() != path_metadata.dev() || file_metadata.ino() != path_metadata.ino()
        {
            return Err(LockError::Unavailable);
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| LockError::Unavailable)?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "rust-agent-harness-refresh-lock-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[derive(Clone, Default)]
    struct FakeClock(Arc<AtomicU64>);

    impl WaitClock for FakeClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }
    }

    #[derive(Clone)]
    struct AdvancingSleeper {
        clock: FakeClock,
        sleeps: Arc<Mutex<Vec<Duration>>>,
    }

    impl Sleeper for AdvancingSleeper {
        fn sleep(&self, duration: Duration) {
            self.sleeps.lock().unwrap().push(duration);
            self.clock
                .0
                .fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
        }
    }

    fn fake_lock(root: PathBuf) -> FileRefreshLock<FakeClock, AdvancingSleeper> {
        let clock = FakeClock::default();
        FileRefreshLock::with_root(
            root,
            clock.clone(),
            AdvancingSleeper {
                clock,
                sleeps: Arc::default(),
            },
        )
    }

    #[test]
    fn stable_file_and_directories_are_private_empty_and_reused() {
        let _serial = FILE_LOCK_TEST_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temporary = TempDirectory::new();
        let lock = fake_lock(temporary.0.join("rust-agent-harness"));
        let path = lock.lock_path().to_owned();

        let first = lock.acquire().unwrap();
        let first_metadata = fs::metadata(&path).unwrap();
        assert_eq!(first_metadata.len(), 0);
        #[cfg(unix)]
        {
            assert_eq!(first_metadata.permissions().mode() & 0o777, 0o600);
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        drop(first);

        let second = lock.acquire().unwrap();
        let second_metadata = fs::metadata(&path).unwrap();
        #[cfg(unix)]
        {
            assert_eq!(first_metadata.dev(), second_metadata.dev());
            assert_eq!(first_metadata.ino(), second_metadata.ino());
        }
        drop(second);
        assert_eq!(fs::read(&path).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn independent_handles_contend_until_the_owner_drops() {
        let _serial = FILE_LOCK_TEST_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temporary = TempDirectory::new();
        let root = temporary.0.join("rust-agent-harness");
        let first_lock = fake_lock(root.clone());
        let first = first_lock.acquire().unwrap();
        let second_lock = fake_lock(root).with_limits(Duration::ZERO, POLL_INTERVAL);

        assert_eq!(second_lock.acquire().err(), Some(LockError::Busy));
        drop(first);
        assert!(second_lock.acquire().is_ok());
    }

    #[test]
    fn injected_wait_reaches_the_responsiveness_cap_without_cancelling_owner() {
        let _serial = FILE_LOCK_TEST_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temporary = TempDirectory::new();
        let root = temporary.0.join("rust-agent-harness");
        let owner_lock = fake_lock(root.clone());
        let owner = owner_lock.acquire().unwrap();
        let waiter = fake_lock(root);

        assert_eq!(waiter.acquire().err(), Some(LockError::Busy));
        assert_eq!(waiter.clock.now(), WAIT_TIMEOUT);
        assert_eq!(waiter.sleeper.sleeps.lock().unwrap().len(), 1_300);

        drop(owner);
        assert!(waiter.acquire().is_ok());
    }

    #[test]
    fn path_failures_are_sanitized_and_reject_symlink_or_nonempty_leaves() {
        let _serial = FILE_LOCK_TEST_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temporary = TempDirectory::new();
        let file_root = temporary.0.join("not-a-directory");
        fs::write(&file_root, b"not a directory").unwrap();
        assert_eq!(
            fake_lock(file_root).acquire().err(),
            Some(LockError::Unavailable)
        );

        let root = temporary.0.join("rust-agent-harness");
        let lock = fake_lock(root.clone());
        fs::create_dir_all(lock.lock_path().parent().unwrap()).unwrap();
        fs::write(lock.lock_path(), b"must stay empty").unwrap();
        assert_eq!(lock.acquire().err(), Some(LockError::Unavailable));

        #[cfg(unix)]
        {
            fs::remove_file(lock.lock_path()).unwrap();
            std::os::unix::fs::symlink(temporary.0.join("target"), lock.lock_path()).unwrap();
            assert_eq!(lock.acquire().err(), Some(LockError::Unavailable));
        }
        assert_eq!(format!("{:?}", LockError::Unavailable), "Unavailable");
    }

    #[test]
    fn child_process_holds_the_same_file_against_an_independent_parent_handle() {
        let _serial = FILE_LOCK_TEST_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temporary = TempDirectory::new();
        let lock = fake_lock(temporary.0.join("rust-agent-harness"));
        let path = lock.lock_path().to_owned();
        prepare_private_directory(path.parent().unwrap()).unwrap();
        let mut child = Command::new(env::current_exe().unwrap())
            .arg("--exact")
            .arg("refresh_lock::tests::child_process_lock_helper")
            .arg("--nocapture")
            .env("HARNESS_REFRESH_LOCK_CHILD_PATH", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert_ne!(stdout.read_line(&mut line).unwrap(), 0);
            if line.contains("refresh-lock-acquired") {
                break;
            }
            line.clear();
        }

        let contender = fake_lock(temporary.0.join("rust-agent-harness"))
            .with_limits(Duration::ZERO, POLL_INTERVAL);
        assert_eq!(contender.acquire().err(), Some(LockError::Busy));
        child.stdin.take().unwrap().write_all(b"release\n").unwrap();
        assert!(child.wait().unwrap().success());
        assert!(contender.acquire().is_ok());
    }

    #[test]
    fn child_process_lock_helper() {
        let Ok(path) = env::var("HARNESS_REFRESH_LOCK_CHILD_PATH") else {
            return;
        };
        let file = open_lock_file(Path::new(&path)).unwrap();
        fs4::FileExt::lock(&file).unwrap();
        println!("refresh-lock-acquired");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn production_path_uses_standard_application_support_data() {
        let lock = FileRefreshLock::production().unwrap();
        let rendered = lock.lock_path().to_string_lossy();

        assert!(rendered.contains("/Library/Application Support/rust-agent-harness/locks/"));
        assert!(rendered.ends_with(LOCK_FILE));
    }
}
