#[cfg(any(target_os = "macos", test))]
mod callback;
#[cfg(any(target_os = "macos", test))]
mod credential;
#[cfg(any(target_os = "macos", test))]
mod keychain;
#[cfg(any(target_os = "macos", test))]
mod oauth;
#[cfg(any(target_os = "macos", test))]
mod pkce;
#[cfg(any(target_os = "macos", test))]
mod refresh;
#[cfg(any(target_os = "macos", test))]
mod refresh_lock;

use std::fmt;
#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

#[cfg(any(target_os = "macos", test))]
use callback::{CallbackError, CallbackListener, CallbackOutcome};
#[cfg(target_os = "macos")]
use credential::SystemClock;
#[cfg(any(target_os = "macos", test))]
use credential::{Clock, CredentialRecord};
#[cfg(any(target_os = "macos", test))]
use keychain::{CredentialDeleter, CredentialReader, CredentialStore, DeleteOutcome};
#[cfg(any(target_os = "macos", test))]
use oauth::{ExchangeConfig, ExchangeError};
#[cfg(any(target_os = "macos", test))]
use pkce::AuthSecrets;
#[cfg(target_os = "macos")]
use refresh::HttpRefreshClient;
#[cfg(any(target_os = "macos", test))]
use refresh::{RefreshClient, RefreshError};
#[cfg(any(target_os = "macos", test))]
use refresh_lock::{LockError, RefreshLock};
#[cfg(any(target_os = "macos", test))]
use url::Url;

#[cfg(any(target_os = "macos", test))]
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
#[cfg(any(target_os = "macos", test))]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(any(target_os = "macos", test))]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(any(target_os = "macos", test))]
const TOKEN_RESPONSE_LIMIT: usize = 64 * 1024;
#[cfg(any(target_os = "macos", test))]
const REDIRECT_PORTS: [u16; 2] = [1455, 1457];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedAccount {
    pub email: Option<String>,
    pub plan: Option<String>,
}

pub struct AuthorizationUrl(String);

impl AuthorizationUrl {
    #[doc(hidden)]
    pub fn from_string(url: String) -> Self {
        Self(url)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AuthorizationUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationUrl([REDACTED])")
    }
}

pub enum LoginProgress {
    OpeningBrowser,
    AuthorizationUrl(AuthorizationUrl),
    BrowserLaunchFailed,
}

impl fmt::Debug for LoginProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpeningBrowser => formatter.write_str("OpeningBrowser"),
            Self::AuthorizationUrl(_) => formatter.write_str("AuthorizationUrl([REDACTED])"),
            Self::BrowserLaunchFailed => formatter.write_str("BrowserLaunchFailed"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginError {
    UnsupportedPlatform,
    CallbackPortsUnavailable,
    AuthorizationDenied,
    AuthorizationTemporarilyUnavailable,
    AuthorizationFailed,
    InvalidCallback,
    TimedOut,
    TokenExchangeUnavailable,
    TokenExchangeRejected,
    TokenResponseMalformed,
    CredentialStoreUnavailable,
}

impl fmt::Display for LoginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "OpenAI Codex login is supported only on macOS",
            Self::CallbackPortsUnavailable => {
                "OpenAI Codex login could not bind callback ports 1455 or 1457"
            }
            Self::AuthorizationDenied => "OpenAI Codex authorization was denied",
            Self::AuthorizationTemporarilyUnavailable => {
                "OpenAI Codex authorization is temporarily unavailable; try again"
            }
            Self::AuthorizationFailed => "OpenAI Codex authorization failed; try again",
            Self::InvalidCallback => "OpenAI Codex returned an invalid authorization callback",
            Self::TimedOut => "OpenAI Codex login timed out; try again",
            Self::TokenExchangeUnavailable => {
                "OpenAI Codex token exchange is unavailable; try again"
            }
            Self::TokenExchangeRejected => {
                "OpenAI Codex rejected the authorization; try logging in again"
            }
            Self::TokenResponseMalformed => "OpenAI Codex returned an invalid account credential",
            Self::CredentialStoreUnavailable => {
                "the OpenAI Codex credential could not be saved to macOS Keychain"
            }
        })
    }
}

impl std::error::Error for LoginError {}

pub struct AuthorizedCredential {
    access_token: String,
    account_id: String,
}

impl AuthorizedCredential {
    pub fn authorize(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        let mut account_id = reqwest::header::HeaderValue::try_from(self.account_id.as_str())
            .expect("validated account id must remain a valid header value");
        account_id.set_sensitive(true);
        request
            .bearer_auth(&self.access_token)
            .header("ChatGPT-Account-ID", account_id)
    }
}

impl fmt::Debug for AuthorizedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizedCredential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialReadError {
    UnsupportedPlatform,
    NotConnected,
    StoreUnavailable,
    UnsupportedVersion,
    InvalidCredential,
    Expired,
}

impl fmt::Display for CredentialReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "OpenAI Codex credentials are supported only on macOS",
            Self::NotConnected => {
                "OpenAI Codex is not connected; run 'harness auth login openai-codex'"
            }
            Self::StoreUnavailable => "the OpenAI Codex credential store is unavailable",
            Self::UnsupportedVersion | Self::InvalidCredential => {
                "the stored OpenAI Codex connection is invalid; run 'harness auth login openai-codex'"
            }
            Self::Expired => {
                "the OpenAI Codex connection expired; run 'harness auth login openai-codex'"
            }
        })
    }
}

impl std::error::Error for CredentialReadError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialUseError {
    UnsupportedPlatform,
    NotConnected,
    StoreUnavailable,
    UnsupportedVersion,
    InvalidCredential,
    RefreshCoordinationBusy,
    RefreshTemporarilyUnavailable,
    ReauthenticationRequired,
}

impl fmt::Display for CredentialUseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "OpenAI Codex credentials are supported only on macOS",
            Self::NotConnected => {
                "OpenAI Codex is not connected; run 'harness auth login openai-codex'"
            }
            Self::StoreUnavailable => "the OpenAI Codex credential store is unavailable",
            Self::UnsupportedVersion | Self::InvalidCredential => {
                "the stored OpenAI Codex connection is invalid; run 'harness auth login openai-codex'"
            }
            Self::RefreshCoordinationBusy => {
                "OpenAI Codex credential refresh is already in progress; try again"
            }
            Self::RefreshTemporarilyUnavailable => {
                "OpenAI Codex credential refresh is temporarily unavailable; try again"
            }
            Self::ReauthenticationRequired => {
                "OpenAI Codex rejected the connection; run 'harness auth login openai-codex'"
            }
        })
    }
}

impl std::error::Error for CredentialUseError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogoutError {
    UnsupportedPlatform,
    StoreUnavailable,
}

impl fmt::Display for LogoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "OpenAI Codex credentials are supported only on macOS",
            Self::StoreUnavailable => "the OpenAI Codex credential store is unavailable",
        })
    }
}

impl std::error::Error for LogoutError {}

pub fn logout() -> Result<(), LogoutError> {
    #[cfg(target_os = "macos")]
    {
        logout_with(&keychain::MacOsKeychain)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(LogoutError::UnsupportedPlatform)
    }
}

#[cfg(any(target_os = "macos", test))]
fn logout_with(deleter: &dyn CredentialDeleter) -> Result<(), LogoutError> {
    match deleter
        .delete()
        .map_err(|_| LogoutError::StoreUnavailable)?
    {
        DeleteOutcome::Deleted | DeleteOutcome::Absent => Ok(()),
    }
}

pub fn with_authorized_credential<T>(
    operation: impl FnOnce(&AuthorizedCredential) -> T,
) -> Result<T, CredentialReadError> {
    #[cfg(target_os = "macos")]
    {
        with_authorized_credential_from(&keychain::MacOsKeychain, &SystemClock, operation)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = operation;
        Err(CredentialReadError::UnsupportedPlatform)
    }
}

#[cfg(any(target_os = "macos", test))]
fn with_authorized_credential_from<T>(
    reader: &dyn CredentialReader,
    clock: &dyn Clock,
    operation: impl FnOnce(&AuthorizedCredential) -> T,
) -> Result<T, CredentialReadError> {
    let serialized = reader
        .read()
        .map_err(|_| CredentialReadError::StoreUnavailable)?
        .ok_or(CredentialReadError::NotConnected)?;
    let record =
        CredentialRecord::deserialize(&serialized, clock.unix_seconds()).map_err(|error| {
            match error {
                credential::StoredCredentialError::UnsupportedVersion => {
                    CredentialReadError::UnsupportedVersion
                }
                credential::StoredCredentialError::Malformed => {
                    CredentialReadError::InvalidCredential
                }
                credential::StoredCredentialError::Expired => CredentialReadError::Expired,
            }
        })?;
    let (access_token, account_id) = record.into_authorization();
    let credential = AuthorizedCredential {
        access_token,
        account_id,
    };
    Ok(operation(&credential))
}

pub fn with_renewable_authorized_credential<T, E>(
    operation: impl FnMut(&AuthorizedCredential) -> Result<T, E>,
    is_unauthorized: impl Fn(&E) -> bool,
) -> Result<Result<T, E>, CredentialUseError> {
    #[cfg(target_os = "macos")]
    {
        let mut store = keychain::MacOsKeychain;
        let refresh_lock =
            refresh_lock::FileRefreshLock::production().map_err(map_refresh_lock_error)?;
        with_renewable_authorized_credential_from(
            &mut store,
            &SystemClock,
            &refresh_lock,
            &HttpRefreshClient::production(),
            operation,
            is_unauthorized,
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = operation;
        let _ = is_unauthorized;
        Err(CredentialUseError::UnsupportedPlatform)
    }
}

#[cfg(any(target_os = "macos", test))]
fn with_renewable_authorized_credential_from<T, E, S, L, R>(
    store: &mut S,
    clock: &dyn Clock,
    refresh_lock: &L,
    refresher: &R,
    mut operation: impl FnMut(&AuthorizedCredential) -> Result<T, E>,
    is_unauthorized: impl Fn(&E) -> bool,
) -> Result<Result<T, E>, CredentialUseError>
where
    S: CredentialReader + CredentialStore,
    L: RefreshLock,
    R: RefreshClient,
{
    let (mut attempted_bytes, mut record) = read_renewable_record(store)?;
    let mut contacted_refresh_endpoint = false;

    if record.refresh_due(clock.unix_seconds()) {
        let _guard = refresh_lock.acquire().map_err(map_refresh_lock_error)?;
        let (reread_bytes, reread_record) = read_renewable_record(store)?;
        if reread_bytes != attempted_bytes {
            attempted_bytes = reread_bytes;
            record = reread_record;
        } else {
            contacted_refresh_endpoint = true;
            let response = refresher
                .refresh(record.refresh_token())
                .map_err(map_refresh_error)?;
            let replacement = record
                .apply_refresh(response, clock.unix_seconds())
                .map_err(|_| CredentialUseError::InvalidCredential)?;
            let replacement_bytes = replacement
                .serialize()
                .map_err(|_| CredentialUseError::InvalidCredential)?;
            store
                .replace(&replacement_bytes)
                .map_err(|_| CredentialUseError::StoreUnavailable)?;
            attempted_bytes = replacement_bytes;
            record = replacement;
        }
    }

    let first = operation(&authorized_credential(&record));
    let Err(first_error) = first else {
        return Ok(first);
    };
    if !is_unauthorized(&first_error) {
        return Ok(Err(first_error));
    }

    let replay_record = {
        let _guard = refresh_lock.acquire().map_err(map_refresh_lock_error)?;
        let (reread_bytes, reread_record) = read_renewable_record(store)?;
        if reread_bytes != attempted_bytes {
            reread_record
        } else if contacted_refresh_endpoint {
            return Ok(Err(first_error));
        } else {
            let response = refresher
                .refresh(record.refresh_token())
                .map_err(map_refresh_error)?;
            let replacement = record
                .apply_refresh(response, clock.unix_seconds())
                .map_err(|_| CredentialUseError::InvalidCredential)?;
            let replacement_bytes = replacement
                .serialize()
                .map_err(|_| CredentialUseError::InvalidCredential)?;
            store
                .replace(&replacement_bytes)
                .map_err(|_| CredentialUseError::StoreUnavailable)?;
            replacement
        }
    };

    Ok(operation(&authorized_credential(&replay_record)))
}

#[cfg(any(target_os = "macos", test))]
fn read_renewable_record(
    reader: &dyn CredentialReader,
) -> Result<(Vec<u8>, CredentialRecord), CredentialUseError> {
    let serialized = reader
        .read()
        .map_err(|_| CredentialUseError::StoreUnavailable)?
        .ok_or(CredentialUseError::NotConnected)?;
    let record =
        CredentialRecord::deserialize_structural(&serialized).map_err(|error| match error {
            credential::StoredCredentialError::UnsupportedVersion => {
                CredentialUseError::UnsupportedVersion
            }
            credential::StoredCredentialError::Malformed
            | credential::StoredCredentialError::Expired => CredentialUseError::InvalidCredential,
        })?;
    Ok((serialized, record))
}

#[cfg(any(target_os = "macos", test))]
fn authorized_credential(record: &CredentialRecord) -> AuthorizedCredential {
    let (access_token, account_id) = record.authorization();
    AuthorizedCredential {
        access_token,
        account_id,
    }
}

#[cfg(any(target_os = "macos", test))]
fn map_refresh_lock_error(error: LockError) -> CredentialUseError {
    match error {
        LockError::Busy => CredentialUseError::RefreshCoordinationBusy,
        LockError::Unavailable => CredentialUseError::StoreUnavailable,
    }
}

#[cfg(any(target_os = "macos", test))]
fn map_refresh_error(error: RefreshError) -> CredentialUseError {
    match error {
        RefreshError::ReauthenticationRequired => CredentialUseError::ReauthenticationRequired,
        RefreshError::Unavailable => CredentialUseError::RefreshTemporarilyUnavailable,
    }
}

pub fn login(progress: impl FnMut(LoginProgress)) -> Result<ConnectedAccount, LoginError> {
    #[cfg(target_os = "macos")]
    {
        let config = LoginConfig::production();
        let mut browser = SystemBrowser;
        let mut store = keychain::MacOsKeychain;
        login_with(
            &config,
            &mut browser,
            &mut store,
            &SystemClock,
            AuthSecrets::generate(),
            progress,
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = progress;
        Err(LoginError::UnsupportedPlatform)
    }
}

#[cfg(any(target_os = "macos", test))]
struct LoginConfig {
    authorization_endpoint: Url,
    exchange: ExchangeConfig,
    redirect_ports: Vec<u16>,
    login_timeout: Duration,
}

#[cfg(any(target_os = "macos", test))]
impl LoginConfig {
    fn production() -> Self {
        Self {
            authorization_endpoint: Url::parse("https://auth.openai.com/oauth/authorize")
                .expect("production authorization endpoint must be valid"),
            exchange: ExchangeConfig {
                token_endpoint: Url::parse("https://auth.openai.com/oauth/token")
                    .expect("production token endpoint must be valid"),
                connect_timeout: CONNECT_TIMEOUT,
                request_timeout: REQUEST_TIMEOUT,
                response_body_limit: TOKEN_RESPONSE_LIMIT,
            },
            redirect_ports: REDIRECT_PORTS.to_vec(),
            login_timeout: LOGIN_TIMEOUT,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
trait BrowserLauncher {
    fn open(&mut self, url: &str) -> Result<(), ()>;
}

#[cfg(target_os = "macos")]
struct SystemBrowser;

#[cfg(target_os = "macos")]
impl BrowserLauncher for SystemBrowser {
    fn open(&mut self, url: &str) -> Result<(), ()> {
        webbrowser::open(url).map(|_| ()).map_err(|_| ())
    }
}

#[cfg(any(target_os = "macos", test))]
fn login_with(
    config: &LoginConfig,
    browser: &mut dyn BrowserLauncher,
    store: &mut dyn CredentialStore,
    clock: &dyn Clock,
    secrets: AuthSecrets,
    mut progress: impl FnMut(LoginProgress),
) -> Result<ConnectedAccount, LoginError> {
    let listener = CallbackListener::bind(&config.redirect_ports).map_err(map_callback_error)?;
    let redirect_uri = listener.redirect_uri();
    let authorization_url =
        oauth::authorization_url(&config.authorization_endpoint, &redirect_uri, &secrets);

    progress(LoginProgress::OpeningBrowser);
    progress(LoginProgress::AuthorizationUrl(
        AuthorizationUrl::from_string(authorization_url.to_string()),
    ));
    if browser.open(authorization_url.as_str()).is_err() {
        progress(LoginProgress::BrowserLaunchFailed);
    }

    let code = match listener
        .wait(&secrets.state, config.login_timeout)
        .map_err(map_callback_error)?
    {
        CallbackOutcome::Code(code) => code,
        CallbackOutcome::Denied => return Err(LoginError::AuthorizationDenied),
        CallbackOutcome::AuthorizationTemporarilyUnavailable => {
            return Err(LoginError::AuthorizationTemporarilyUnavailable);
        }
        CallbackOutcome::AuthorizationFailed => return Err(LoginError::AuthorizationFailed),
    };
    let response = oauth::exchange_code(&config.exchange, &code, &redirect_uri, &secrets.verifier)
        .map_err(map_exchange_error)?;
    let credential = CredentialRecord::validate(response, clock.unix_seconds())
        .map_err(|_| LoginError::TokenResponseMalformed)?;
    let account = ConnectedAccount {
        email: credential.email().map(str::to_owned),
        plan: credential.plan().map(str::to_owned),
    };
    let serialized = credential
        .serialize()
        .map_err(|_| LoginError::TokenResponseMalformed)?;
    store
        .replace(&serialized)
        .map_err(|_| LoginError::CredentialStoreUnavailable)?;

    Ok(account)
}

#[cfg(any(target_os = "macos", test))]
fn map_callback_error(error: CallbackError) -> LoginError {
    match error {
        CallbackError::PortsUnavailable => LoginError::CallbackPortsUnavailable,
        CallbackError::TimedOut => LoginError::TimedOut,
        CallbackError::ListenerFailed => LoginError::InvalidCallback,
    }
}

#[cfg(any(target_os = "macos", test))]
fn map_exchange_error(error: ExchangeError) -> LoginError {
    match error {
        ExchangeError::Unavailable => LoginError::TokenExchangeUnavailable,
        ExchangeError::Rejected => LoginError::TokenExchangeRejected,
        ExchangeError::ResponseTooLarge | ExchangeError::MalformedResponse => {
            LoginError::TokenResponseMalformed
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, VecDeque};
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use std::time::Instant;

    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};
    use tiny_http::{Response, Server};
    use url::form_urlencoded;

    use super::*;
    use crate::keychain::StoreError;

    const STATE: &str = "fixed-state-sentinel";
    const VERIFIER: &str = "fixed-verifier-sentinel";
    const CODE: &str = "fixed-code-sentinel";
    const ACCESS: &str = "fixed-access-token-sentinel";
    const REFRESH: &str = "fixed-refresh-token-sentinel";
    const ID_PAYLOAD_ACCOUNT: &str = "account-one";
    static NEXT_REFRESH_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct FixedClock;

    impl Clock for FixedClock {
        fn unix_seconds(&self) -> u64 {
            1_000
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        record: Option<Vec<u8>>,
        fail: bool,
        writes: usize,
        unrelated: Vec<u8>,
    }

    impl CredentialStore for MemoryStore {
        fn replace(&mut self, record: &[u8]) -> Result<(), StoreError> {
            self.writes += 1;
            if self.fail {
                return Err(StoreError);
            }
            self.record = Some(record.to_vec());
            Ok(())
        }
    }

    struct MemoryReader {
        record: Option<Vec<u8>>,
        fail: bool,
        reads: Cell<usize>,
    }

    #[derive(Default)]
    struct SharedStoreState {
        record: Option<Vec<u8>>,
        fail_read: bool,
        fail_write: bool,
        reads: usize,
        writes: usize,
    }

    #[derive(Clone, Default)]
    struct SharedStore(Arc<Mutex<SharedStoreState>>);

    impl SharedStore {
        fn with_record(record: Vec<u8>) -> Self {
            Self(Arc::new(Mutex::new(SharedStoreState {
                record: Some(record),
                ..SharedStoreState::default()
            })))
        }

        fn record(&self) -> Option<Vec<u8>> {
            self.0.lock().unwrap().record.clone()
        }

        fn replace_without_counting(&self, record: Vec<u8>) {
            self.0.lock().unwrap().record = Some(record);
        }

        fn reads(&self) -> usize {
            self.0.lock().unwrap().reads
        }

        fn writes(&self) -> usize {
            self.0.lock().unwrap().writes
        }
    }

    impl CredentialReader for SharedStore {
        fn read(&self) -> Result<Option<Vec<u8>>, keychain::StoreError> {
            let mut state = self.0.lock().unwrap();
            state.reads += 1;
            if state.fail_read {
                Err(keychain::StoreError)
            } else {
                Ok(state.record.clone())
            }
        }
    }

    impl CredentialStore for SharedStore {
        fn replace(&mut self, record: &[u8]) -> Result<(), keychain::StoreError> {
            let mut state = self.0.lock().unwrap();
            state.writes += 1;
            if state.fail_write {
                Err(keychain::StoreError)
            } else {
                state.record = Some(record.to_vec());
                Ok(())
            }
        }
    }

    struct FakeRefreshLock {
        result: Result<(), LockError>,
        acquisitions: Cell<usize>,
    }

    impl FakeRefreshLock {
        fn available() -> Self {
            Self {
                result: Ok(()),
                acquisitions: Cell::new(0),
            }
        }
    }

    impl RefreshLock for FakeRefreshLock {
        type Guard = ();

        fn acquire(&self) -> Result<Self::Guard, LockError> {
            self.acquisitions.set(self.acquisitions.get() + 1);
            self.result
        }
    }

    struct MutatingRefreshLock {
        replacement: Vec<u8>,
        store: SharedStore,
        acquisitions: Cell<usize>,
    }

    impl RefreshLock for MutatingRefreshLock {
        type Guard = ();

        fn acquire(&self) -> Result<Self::Guard, LockError> {
            let acquisitions = self.acquisitions.get() + 1;
            self.acquisitions.set(acquisitions);
            if acquisitions == 1 {
                self.store
                    .replace_without_counting(self.replacement.clone());
            }
            Ok(())
        }
    }

    struct FakeRefresher {
        responses: RefCell<VecDeque<Result<refresh::RefreshResponse, RefreshError>>>,
        calls: Cell<usize>,
        refresh_tokens: RefCell<Vec<String>>,
    }

    impl FakeRefresher {
        fn new(responses: Vec<Result<refresh::RefreshResponse, RefreshError>>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
                calls: Cell::new(0),
                refresh_tokens: RefCell::new(Vec::new()),
            }
        }
    }

    impl RefreshClient for FakeRefresher {
        fn refresh(&self, refresh_token: &str) -> Result<refresh::RefreshResponse, RefreshError> {
            self.calls.set(self.calls.get() + 1);
            self.refresh_tokens
                .borrow_mut()
                .push(refresh_token.to_owned());
            self.responses
                .borrow_mut()
                .pop_front()
                .expect("a configured refresh response")
        }
    }

    struct BarrierStore {
        store: SharedStore,
        first_read_barrier: Arc<Barrier>,
        reads: AtomicUsize,
    }

    impl CredentialReader for BarrierStore {
        fn read(&self) -> Result<Option<Vec<u8>>, keychain::StoreError> {
            let result = self.store.read();
            if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                self.first_read_barrier.wait();
            }
            result
        }
    }

    impl CredentialStore for BarrierStore {
        fn replace(&mut self, record: &[u8]) -> Result<(), keychain::StoreError> {
            self.store.replace(record)
        }
    }

    struct ConcurrentRefresher {
        response: refresh::RefreshResponse,
        calls: AtomicUsize,
        refresh_tokens: Mutex<Vec<String>>,
    }

    impl RefreshClient for ConcurrentRefresher {
        fn refresh(&self, refresh_token: &str) -> Result<refresh::RefreshResponse, RefreshError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.refresh_tokens
                .lock()
                .unwrap()
                .push(refresh_token.to_owned());
            Ok(self.response.clone())
        }
    }

    struct MonotonicClock(Instant);

    impl refresh_lock::WaitClock for MonotonicClock {
        fn now(&self) -> Duration {
            self.0.elapsed()
        }
    }

    struct FakeDeleter {
        result: Result<DeleteOutcome, StoreError>,
        calls: Cell<usize>,
        unrelated_record: Vec<u8>,
    }

    impl CredentialDeleter for FakeDeleter {
        fn delete(&self) -> Result<DeleteOutcome, StoreError> {
            self.calls.set(self.calls.get() + 1);
            self.result
        }
    }

    impl CredentialReader for MemoryReader {
        fn read(&self) -> Result<Option<Vec<u8>>, keychain::StoreError> {
            self.reads.set(self.reads.get() + 1);
            if self.fail {
                return Err(keychain::StoreError);
            }
            Ok(self.record.clone())
        }
    }

    fn stored_credential(
        schema_version: u8,
        access_token: &str,
        account_id: &str,
        expiry: Option<u64>,
    ) -> Vec<u8> {
        let access_token = jwt_with_signature(
            json!({
                "https://api.openai.com/auth": {"chatgpt_account_id": account_id},
                "exp": expiry
            }),
            access_token,
        );
        serde_json::to_vec(&json!({
            "schema_version": schema_version,
            "access_token": access_token,
            "refresh_token": "stored-refresh-token-sentinel",
            "id_token": jwt_with_signature(json!({
                "https://api.openai.com/auth": {"chatgpt_account_id": account_id}
            }), "stored-id-token-sentinel"),
            "chatgpt_account_id": account_id,
            "access_token_expires_at": expiry,
            "email": null,
            "plan": null
        }))
        .unwrap()
    }

    fn memory_reader(record: Option<Vec<u8>>) -> MemoryReader {
        MemoryReader {
            record,
            fail: false,
            reads: Cell::new(0),
        }
    }

    fn fake_deleter(result: Result<DeleteOutcome, StoreError>) -> FakeDeleter {
        FakeDeleter {
            result,
            calls: Cell::new(0),
            unrelated_record: b"unrelated-credential-sentinel".to_vec(),
        }
    }

    #[test]
    fn deleted_and_absent_logout_outcomes_succeed_once_without_touching_neighbours() {
        for outcome in [DeleteOutcome::Deleted, DeleteOutcome::Absent] {
            let deleter = fake_deleter(Ok(outcome));

            assert_eq!(logout_with(&deleter), Ok(()));
            assert_eq!(deleter.calls.get(), 1);
            assert_eq!(deleter.unrelated_record, b"unrelated-credential-sentinel");
        }
    }

    #[test]
    fn logout_store_failure_is_redacted_and_attempted_once() {
        let deleter = fake_deleter(Err(StoreError));

        let result = logout_with(&deleter);

        assert_eq!(result, Err(LogoutError::StoreUnavailable));
        assert_eq!(deleter.calls.get(), 1);
        let rendered = format!("{:?} {}", result.unwrap_err(), result.unwrap_err());
        assert!(!rendered.contains("unrelated-credential-sentinel"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn public_logout_is_unsupported_without_macos_keychain_access() {
        assert_eq!(logout(), Err(LogoutError::UnsupportedPlatform));
    }

    enum BrowserBehavior {
        Callback(Vec<CallbackRequest>),
        NoCallback,
    }

    struct CallbackRequest {
        method: &'static str,
        path_and_query: String,
    }

    struct FakeBrowser {
        behavior: Option<BrowserBehavior>,
        fail_to_open: bool,
        opened_urls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeBrowser {
        fn success() -> Self {
            Self {
                behavior: Some(BrowserBehavior::Callback(vec![CallbackRequest {
                    method: "GET",
                    path_and_query: format!("/auth/callback?code={CODE}&state={STATE}"),
                }])),
                fail_to_open: false,
                opened_urls: Arc::default(),
            }
        }
    }

    impl BrowserLauncher for FakeBrowser {
        fn open(&mut self, url: &str) -> Result<(), ()> {
            self.opened_urls.lock().unwrap().push(url.to_owned());
            if let Some(BrowserBehavior::Callback(requests)) = self.behavior.take() {
                let redirect_uri = Url::parse(url)
                    .unwrap()
                    .query_pairs()
                    .find_map(|(name, value)| (name == "redirect_uri").then(|| value.into_owned()))
                    .unwrap();
                let port = Url::parse(&redirect_uri).unwrap().port().unwrap();
                thread::spawn(move || {
                    for request in requests {
                        send_callback(port, request.method, &request.path_and_query);
                    }
                });
            }
            if self.fail_to_open { Err(()) } else { Ok(()) }
        }
    }

    fn send_callback(port: u16, method: &str, target: &str) {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
    }

    struct TokenServer {
        endpoint: Url,
        request: Arc<Mutex<Option<HashMap<String, String>>>>,
        thread: thread::JoinHandle<()>,
    }

    impl TokenServer {
        fn respond(status: u16, body: Vec<u8>) -> Self {
            let server = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = server.server_addr().to_ip().unwrap().port();
            let request_body = Arc::new(Mutex::new(None));
            let captured = Arc::clone(&request_body);
            let thread = thread::spawn(move || {
                let mut request = server.recv().unwrap();
                let mut body_buffer = String::new();
                request
                    .as_reader()
                    .read_to_string(&mut body_buffer)
                    .unwrap();
                let form = form_urlencoded::parse(body_buffer.as_bytes())
                    .into_owned()
                    .collect();
                *captured.lock().unwrap() = Some(form);
                request
                    .respond(Response::from_data(body).with_status_code(status))
                    .unwrap();
            });
            Self {
                endpoint: Url::parse(&format!("http://127.0.0.1:{port}/oauth/token")).unwrap(),
                request: request_body,
                thread,
            }
        }

        fn finish(self) -> HashMap<String, String> {
            self.thread.join().unwrap();
            self.request.lock().unwrap().take().unwrap()
        }
    }

    fn jwt(payload: Value) -> String {
        jwt_with_signature(payload, "signature")
    }

    fn jwt_with_signature(payload: Value, signature: &str) -> String {
        format!(
            "{}.{}.{signature}",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        )
    }

    fn valid_token_body() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "access_token": jwt(json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": ID_PAYLOAD_ACCOUNT
                },
                "exp": 5_000
            })),
            "refresh_token": REFRESH,
            "id_token": jwt(json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": ID_PAYLOAD_ACCOUNT,
                    "chatgpt_plan_type": "plus"
                },
                "email": "nuno@example.com"
            })),
            "expires_in": 3600
        }))
        .unwrap()
    }

    fn refresh_response(
        access_signature: &str,
        account_id: Option<&str>,
        access_expiry: Option<u64>,
        refresh_token: Option<&str>,
        id_token: Option<&str>,
        expires_in: Option<i64>,
    ) -> refresh::RefreshResponse {
        let mut access_claims = json!({});
        if let Some(account_id) = account_id {
            access_claims["https://api.openai.com/auth"] =
                json!({"chatgpt_account_id": account_id});
        }
        if let Some(expiry) = access_expiry {
            access_claims["exp"] = json!(expiry);
        }
        refresh::RefreshResponse {
            access_token: Some(jwt_with_signature(access_claims, access_signature)),
            refresh_token: refresh_token.map(str::to_owned),
            id_token: id_token.map(str::to_owned),
            expires_in,
        }
    }

    fn bearer_token(credential: &AuthorizedCredential) -> String {
        let request = credential
            .authorize(reqwest::blocking::Client::new().get("http://127.0.0.1/"))
            .build()
            .unwrap();
        request.headers()[reqwest::header::AUTHORIZATION]
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap()
            .to_owned()
    }

    fn config(token_endpoint: Url) -> LoginConfig {
        LoginConfig {
            authorization_endpoint: Url::parse("http://auth.example/oauth/authorize").unwrap(),
            exchange: ExchangeConfig {
                token_endpoint,
                connect_timeout: Duration::from_millis(200),
                request_timeout: Duration::from_secs(2),
                response_body_limit: TOKEN_RESPONSE_LIMIT,
            },
            redirect_ports: vec![0],
            login_timeout: Duration::from_millis(200),
        }
    }

    fn run_login(
        token_server: &TokenServer,
        browser: &mut FakeBrowser,
        store: &mut MemoryStore,
        progress: &mut Vec<String>,
    ) -> Result<ConnectedAccount, LoginError> {
        login_with(
            &config(token_server.endpoint.clone()),
            browser,
            store,
            &FixedClock,
            AuthSecrets::fixed(STATE, VERIFIER),
            |event| progress.push(format!("{event:?}")),
        )
    }

    #[test]
    fn complete_fake_login_exchanges_and_persists_a_versioned_record() {
        let token_server = TokenServer::respond(200, valid_token_body());
        let mut browser = FakeBrowser::success();
        let mut store = MemoryStore {
            unrelated: b"leave-this-alone".to_vec(),
            ..MemoryStore::default()
        };
        let mut progress = Vec::new();

        let account = run_login(&token_server, &mut browser, &mut store, &mut progress).unwrap();
        let form = token_server.finish();

        assert_eq!(account.email.as_deref(), Some("nuno@example.com"));
        assert_eq!(account.plan.as_deref(), Some("plus"));
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some(oauth::CLIENT_ID)
        );
        assert_eq!(form.get("code").map(String::as_str), Some(CODE));
        assert_eq!(
            form.get("code_verifier").map(String::as_str),
            Some(VERIFIER)
        );
        assert!(form["redirect_uri"].starts_with("http://localhost:"));
        assert!(form["redirect_uri"].ends_with("/auth/callback"));
        let stored: Value = serde_json::from_slice(store.record.as_ref().unwrap()).unwrap();
        assert_eq!(stored["schema_version"], 1);
        assert_eq!(stored["refresh_token"], REFRESH);
        assert_eq!(store.unrelated, b"leave-this-alone");
        assert_eq!(store.writes, 1);
        assert_eq!(progress[0], "OpeningBrowser");
        assert_eq!(progress[1], "AuthorizationUrl([REDACTED])");
        assert_eq!(progress.len(), 2);
    }

    #[test]
    fn credential_read_applies_headers_through_an_opaque_redacted_capability() {
        let access = "stored-access-token-sentinel";
        let account = "stored-account-id-sentinel";
        let serialized = stored_credential(1, access, account, Some(2_000));
        let stored: Value = serde_json::from_slice(&serialized).unwrap();
        let expected_access = stored["access_token"].as_str().unwrap().to_owned();
        let reader = memory_reader(Some(serialized));

        let request = with_authorized_credential_from(&reader, &FixedClock, |credential| {
            assert_eq!(
                format!("{credential:?}"),
                "AuthorizedCredential([REDACTED])"
            );
            credential
                .authorize(reqwest::blocking::Client::new().get("http://127.0.0.1/"))
                .build()
                .unwrap()
        })
        .unwrap();

        assert_eq!(
            request.headers()[reqwest::header::AUTHORIZATION],
            format!("Bearer {expected_access}")
        );
        assert_eq!(request.headers()["ChatGPT-Account-ID"], account);
        assert!(request.headers()["ChatGPT-Account-ID"].is_sensitive());
        assert_eq!(reader.reads.get(), 1);
    }

    #[test]
    fn credential_read_failures_are_typed_and_never_invoke_the_operation() {
        let empty_access_record = {
            let mut record: Value = serde_json::from_slice(&stored_credential(
                1,
                "unused-access-signature",
                "account-one",
                Some(2_000),
            ))
            .unwrap();
            record["access_token"] = json!("");
            serde_json::to_vec(&record).unwrap()
        };
        let cases = [
            (None, false, CredentialReadError::NotConnected),
            (None, true, CredentialReadError::StoreUnavailable),
            (
                Some(stored_credential(
                    2,
                    "access-token",
                    "account-one",
                    Some(2_000),
                )),
                false,
                CredentialReadError::UnsupportedVersion,
            ),
            (
                Some(b"malformed-keychain-json-sentinel".to_vec()),
                false,
                CredentialReadError::InvalidCredential,
            ),
            (
                Some(empty_access_record),
                false,
                CredentialReadError::InvalidCredential,
            ),
            (
                Some(stored_credential(1, "access-token", "", Some(2_000))),
                false,
                CredentialReadError::InvalidCredential,
            ),
            (
                Some(stored_credential(
                    1,
                    "access-token",
                    "account-one",
                    Some(1_000),
                )),
                false,
                CredentialReadError::Expired,
            ),
        ];

        for (record, fail, expected) in cases {
            let mut reader = memory_reader(record);
            reader.fail = fail;
            let invoked = Cell::new(false);

            let result = with_authorized_credential_from(&reader, &FixedClock, |_| {
                invoked.set(true);
            });

            assert_eq!(result, Err(expected));
            assert!(!invoked.get());
            assert_eq!(reader.reads.get(), 1);
        }
    }

    #[test]
    fn credential_read_errors_and_debug_output_do_not_expose_stored_secrets() {
        let sentinels = [
            "stored-access-token-sentinel",
            "stored-refresh-token-sentinel",
            "stored-id-token-sentinel",
            "stored-account-id-sentinel",
            "malformed-keychain-json-sentinel",
        ];
        let reader = memory_reader(Some(stored_credential(
            1,
            sentinels[0],
            sentinels[3],
            Some(2_000),
        )));
        let debug = with_authorized_credential_from(&reader, &FixedClock, |credential| {
            format!("{credential:?}")
        })
        .unwrap();
        let mut rendered = debug;
        for error in [
            CredentialReadError::UnsupportedPlatform,
            CredentialReadError::NotConnected,
            CredentialReadError::StoreUnavailable,
            CredentialReadError::UnsupportedVersion,
            CredentialReadError::InvalidCredential,
            CredentialReadError::Expired,
        ] {
            rendered.push_str(&format!("{error:?} {error}"));
        }

        for sentinel in sentinels {
            assert!(!rendered.contains(sentinel));
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn public_renewable_operation_is_unsupported_without_keychain_access() {
        let invoked = Cell::new(false);
        let result = with_renewable_authorized_credential(
            |_| {
                invoked.set(true);
                Ok::<_, ()>(())
            },
            |_| false,
        );

        assert_eq!(result, Err(CredentialUseError::UnsupportedPlatform));
        assert!(!invoked.get());
    }

    #[test]
    fn fresh_credential_uses_one_operation_without_lock_or_refresh() {
        let original = stored_credential(1, "fresh-access", ID_PAYLOAD_ACCOUNT, Some(2_000));
        let mut store = SharedStore::with_record(original.clone());
        let refresh_lock = FakeRefreshLock::available();
        let refresher = FakeRefresher::new(Vec::new());
        let attempts = Cell::new(0);

        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |credential| {
                attempts.set(attempts.get() + 1);
                assert!(bearer_token(credential).ends_with(".fresh-access"));
                Ok::<_, bool>("ok")
            },
            |error| *error,
        );

        assert_eq!(result, Ok(Ok("ok")));
        assert_eq!(attempts.get(), 1);
        assert_eq!(refresh_lock.acquisitions.get(), 0);
        assert_eq!(refresher.calls.get(), 0);
        assert_eq!(store.reads(), 1);
        assert_eq!(store.writes(), 0);
        assert_eq!(store.record().as_deref(), Some(original.as_slice()));
    }

    #[test]
    fn near_expired_and_unknown_credentials_refresh_once_before_use() {
        for expiry in [Some(1_300), Some(999), None] {
            let original = stored_credential(1, "old-access", ID_PAYLOAD_ACCOUNT, expiry);
            let mut store = SharedStore::with_record(original.clone());
            let refresh_lock = FakeRefreshLock::available();
            let refresher = FakeRefresher::new(vec![Ok(refresh_response(
                "new-access",
                None,
                Some(5_000),
                Some("rotated-refresh"),
                None,
                Some(3_600),
            ))]);
            let store_observer = store.clone();

            let result = with_renewable_authorized_credential_from(
                &mut store,
                &FixedClock,
                &refresh_lock,
                &refresher,
                |credential| {
                    assert_eq!(
                        store_observer.writes(),
                        1,
                        "replacement must precede operation"
                    );
                    assert!(bearer_token(credential).ends_with(".new-access"));
                    Ok::<_, bool>(())
                },
                |error| *error,
            );

            assert_eq!(result, Ok(Ok(())));
            assert_eq!(refresher.calls.get(), 1);
            assert_eq!(refresher.refresh_tokens.borrow().len(), 1);
            assert_eq!(refresh_lock.acquisitions.get(), 1);
            assert_eq!(store.reads(), 2);
            assert_eq!(store.writes(), 1);
            assert_ne!(store.record().as_deref(), Some(original.as_slice()));
            let stored: Value = serde_json::from_slice(&store.record().unwrap()).unwrap();
            assert_eq!(stored["refresh_token"], "rotated-refresh");
        }
    }

    #[test]
    fn determinate_short_refresh_is_persisted_and_used() {
        let original = stored_credential(1, "old-short-access", ID_PAYLOAD_ACCOUNT, Some(1_300));
        let mut store = SharedStore::with_record(original);
        let refresh_lock = FakeRefreshLock::available();
        let refresher = FakeRefresher::new(vec![Ok(refresh_response(
            "short-access",
            None,
            Some(1_000),
            Some("rotated-short-refresh"),
            None,
            Some(300),
        ))]);

        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |credential| Ok::<_, bool>(bearer_token(credential)),
            |error| *error,
        );

        assert!(matches!(result, Ok(Ok(token)) if token.ends_with(".short-access")));
        let stored: Value = serde_json::from_slice(&store.record().unwrap()).unwrap();
        assert_eq!(stored["refresh_token"], "rotated-short-refresh");
        assert_eq!(stored["access_token_expires_at"], 1_000);
        assert_eq!(store.writes(), 1);
    }

    #[test]
    fn failed_persistence_never_exposes_the_unpersisted_access_token() {
        let original = stored_credential(1, "old-access", ID_PAYLOAD_ACCOUNT, Some(1_300));
        let mut store = SharedStore::with_record(original.clone());
        store.0.lock().unwrap().fail_write = true;
        let refresh_lock = FakeRefreshLock::available();
        let refresher = FakeRefresher::new(vec![Ok(refresh_response(
            "must-not-be-used",
            None,
            Some(5_000),
            Some("must-not-be-persisted"),
            None,
            Some(3_600),
        ))]);
        let invoked = Cell::new(false);

        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |_| {
                invoked.set(true);
                Ok::<_, bool>(())
            },
            |error| *error,
        );

        assert_eq!(result, Err(CredentialUseError::StoreUnavailable));
        assert!(!invoked.get());
        assert_eq!(store.record().as_deref(), Some(original.as_slice()));
        assert_eq!(store.writes(), 1);
    }

    #[test]
    fn first_unauthorized_refreshes_and_replays_once_but_second_unauthorized_stops() {
        for second_succeeds in [true, false] {
            let mut store = SharedStore::with_record(stored_credential(
                1,
                "old-access",
                ID_PAYLOAD_ACCOUNT,
                Some(5_000),
            ));
            let refresh_lock = FakeRefreshLock::available();
            let refresher = FakeRefresher::new(vec![Ok(refresh_response(
                "reactive-access",
                None,
                Some(5_000),
                Some("reactive-refresh"),
                None,
                Some(3_600),
            ))]);
            let attempts = Cell::new(0);
            let seen = RefCell::new(Vec::new());

            let result = with_renewable_authorized_credential_from(
                &mut store,
                &FixedClock,
                &refresh_lock,
                &refresher,
                |credential| {
                    attempts.set(attempts.get() + 1);
                    seen.borrow_mut().push(bearer_token(credential));
                    if attempts.get() == 1 || !second_succeeds {
                        Err(true)
                    } else {
                        Ok("replayed")
                    }
                },
                |error| *error,
            );

            if second_succeeds {
                assert_eq!(result, Ok(Ok("replayed")));
            } else {
                assert_eq!(result, Ok(Err(true)));
            }
            assert_eq!(attempts.get(), 2);
            assert!(seen.borrow()[0].ends_with(".old-access"));
            assert!(seen.borrow()[1].ends_with(".reactive-access"));
            assert_eq!(refresher.calls.get(), 1);
            assert_eq!(store.writes(), 1);
            assert_eq!(refresh_lock.acquisitions.get(), 1);
        }
    }

    #[test]
    fn non_unauthorized_model_error_is_never_refreshed_or_replayed() {
        let mut store = SharedStore::with_record(stored_credential(
            1,
            "fresh-access",
            ID_PAYLOAD_ACCOUNT,
            Some(5_000),
        ));
        let refresh_lock = FakeRefreshLock::available();
        let refresher = FakeRefresher::new(Vec::new());
        let attempts = Cell::new(0);

        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |_| {
                attempts.set(attempts.get() + 1);
                Err::<(), _>(false)
            },
            |error| *error,
        );

        assert_eq!(result, Ok(Err(false)));
        assert_eq!(attempts.get(), 1);
        assert_eq!(refresher.calls.get(), 0);
        assert_eq!(refresh_lock.acquisitions.get(), 0);
    }

    #[test]
    fn proactive_refresh_then_unauthorized_does_not_refresh_or_replay_unchanged_bytes() {
        let mut store = SharedStore::with_record(stored_credential(
            1,
            "near-access",
            ID_PAYLOAD_ACCOUNT,
            Some(1_300),
        ));
        let refresh_lock = FakeRefreshLock::available();
        let refresher = FakeRefresher::new(vec![Ok(refresh_response(
            "proactive-access",
            None,
            Some(5_000),
            Some("proactive-refresh"),
            None,
            Some(3_600),
        ))]);
        let attempts = Cell::new(0);

        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |_| {
                attempts.set(attempts.get() + 1);
                Err::<(), _>(true)
            },
            |error| *error,
        );

        assert_eq!(result, Ok(Err(true)));
        assert_eq!(attempts.get(), 1);
        assert_eq!(refresher.calls.get(), 1);
        assert_eq!(store.writes(), 1);
        assert_eq!(refresh_lock.acquisitions.get(), 2);
    }

    #[test]
    fn proactive_refresh_then_unauthorized_replays_only_byte_different_winner() {
        let mut store = SharedStore::with_record(stored_credential(
            1,
            "near-access",
            ID_PAYLOAD_ACCOUNT,
            Some(1_300),
        ));
        let store_handle = store.clone();
        let winner = stored_credential(1, "winner-access", ID_PAYLOAD_ACCOUNT, Some(5_000));
        let refresh_lock = FakeRefreshLock::available();
        let refresher = FakeRefresher::new(vec![Ok(refresh_response(
            "proactive-access",
            None,
            Some(5_000),
            Some("proactive-refresh"),
            None,
            Some(3_600),
        ))]);
        let attempts = Cell::new(0);

        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |credential| {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    assert!(bearer_token(credential).ends_with(".proactive-access"));
                    store_handle.replace_without_counting(winner.clone());
                    Err(true)
                } else {
                    assert!(bearer_token(credential).ends_with(".winner-access"));
                    Ok("adopted")
                }
            },
            |error| *error,
        );

        assert_eq!(result, Ok(Ok("adopted")));
        assert_eq!(attempts.get(), 2);
        assert_eq!(refresher.calls.get(), 1);
        assert_eq!(store.writes(), 1);
        assert_eq!(refresh_lock.acquisitions.get(), 2);
    }

    #[test]
    fn lock_reread_adopts_a_structurally_valid_winner_without_refreshing() {
        let mut store = SharedStore::with_record(stored_credential(
            1,
            "loser-access",
            ID_PAYLOAD_ACCOUNT,
            Some(1_300),
        ));
        let winner = stored_credential(1, "winner-access", ID_PAYLOAD_ACCOUNT, Some(5_000));
        let refresh_lock = MutatingRefreshLock {
            replacement: winner.clone(),
            store: store.clone(),
            acquisitions: Cell::new(0),
        };
        let refresher = FakeRefresher::new(Vec::new());

        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |credential| Ok::<_, bool>(bearer_token(credential)),
            |error| *error,
        );

        assert!(matches!(result, Ok(Ok(token)) if token.ends_with(".winner-access")));
        assert_eq!(refresher.calls.get(), 0);
        assert_eq!(store.writes(), 0);
        assert_eq!(store.record().as_deref(), Some(winner.as_slice()));
        assert_eq!(store.reads(), 2);
    }

    #[test]
    fn lock_failures_and_invalid_rereads_stop_before_refresh_or_model_use() {
        let original = stored_credential(1, "near-access", ID_PAYLOAD_ACCOUNT, Some(1_300));
        for (lock_error, expected) in [
            (LockError::Busy, CredentialUseError::RefreshCoordinationBusy),
            (LockError::Unavailable, CredentialUseError::StoreUnavailable),
        ] {
            let mut store = SharedStore::with_record(original.clone());
            let refresh_lock = FakeRefreshLock {
                result: Err(lock_error),
                acquisitions: Cell::new(0),
            };
            let refresher = FakeRefresher::new(Vec::new());
            let invoked = Cell::new(false);
            let result = with_renewable_authorized_credential_from(
                &mut store,
                &FixedClock,
                &refresh_lock,
                &refresher,
                |_| {
                    invoked.set(true);
                    Ok::<_, bool>(())
                },
                |error| *error,
            );

            assert_eq!(result, Err(expected));
            assert!(!invoked.get());
            assert_eq!(refresher.calls.get(), 0);
            assert_eq!(store.record().as_deref(), Some(original.as_slice()));
        }

        let mut store = SharedStore::with_record(original);
        let refresh_lock = MutatingRefreshLock {
            replacement: b"malformed-winner-bytes".to_vec(),
            store: store.clone(),
            acquisitions: Cell::new(0),
        };
        let refresher = FakeRefresher::new(Vec::new());
        let result = with_renewable_authorized_credential_from(
            &mut store,
            &FixedClock,
            &refresh_lock,
            &refresher,
            |_| Ok::<_, bool>(()),
            |error| *error,
        );
        assert_eq!(result, Err(CredentialUseError::InvalidCredential));
        assert_eq!(refresher.calls.get(), 0);
        assert_eq!(store.writes(), 0);
    }

    #[test]
    fn concurrent_callers_use_independent_file_handles_and_spend_the_old_token_once() {
        let _serial = refresh_lock::FILE_LOCK_TEST_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sequence = NEXT_REFRESH_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let temporary = std::env::temp_dir().join(format!(
            "rust-agent-harness-refresh-concurrency-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&temporary).unwrap();
        let private_root = temporary.join("rust-agent-harness");
        let shared_store = SharedStore::with_record(stored_credential(
            1,
            "concurrent-old-access",
            ID_PAYLOAD_ACCOUNT,
            Some(1_300),
        ));
        let first_read_barrier = Arc::new(Barrier::new(2));
        let refresher = Arc::new(ConcurrentRefresher {
            response: refresh_response(
                "concurrent-winner-access",
                None,
                Some(5_000),
                Some("concurrent-rotated-refresh"),
                None,
                Some(3_600),
            ),
            calls: AtomicUsize::new(0),
            refresh_tokens: Mutex::new(Vec::new()),
        });

        let mut workers = Vec::new();
        for _ in 0..2 {
            let mut store = BarrierStore {
                store: shared_store.clone(),
                first_read_barrier: Arc::clone(&first_read_barrier),
                reads: AtomicUsize::new(0),
            };
            let refresher = Arc::clone(&refresher);
            let private_root = private_root.clone();
            workers.push(thread::spawn(move || {
                let refresh_lock = refresh_lock::FileRefreshLock::with_root(
                    private_root,
                    MonotonicClock(Instant::now()),
                    refresh_lock::ThreadSleeper,
                );
                with_renewable_authorized_credential_from(
                    &mut store,
                    &FixedClock,
                    &refresh_lock,
                    refresher.as_ref(),
                    |credential| Ok::<_, bool>(bearer_token(credential)),
                    |error| *error,
                )
            }));
        }

        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();

        for result in results {
            assert!(
                matches!(result, Ok(Ok(token)) if token.ends_with(".concurrent-winner-access"))
            );
        }
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            refresher.refresh_tokens.lock().unwrap().as_slice(),
            ["stored-refresh-token-sentinel"]
        );
        assert_eq!(shared_store.writes(), 1);
        let stored: Value = serde_json::from_slice(&shared_store.record().unwrap()).unwrap();
        assert_eq!(stored["refresh_token"], "concurrent-rotated-refresh");
        std::fs::remove_dir_all(&temporary).unwrap();
    }

    #[test]
    fn refresh_and_coordination_failures_are_fieldless_redacted_and_preserve_bytes() {
        let original = stored_credential(1, "old-access", ID_PAYLOAD_ACCOUNT, Some(1_300));
        for (refresh_result, expected) in [
            (
                Err(RefreshError::Unavailable),
                CredentialUseError::RefreshTemporarilyUnavailable,
            ),
            (
                Err(RefreshError::ReauthenticationRequired),
                CredentialUseError::ReauthenticationRequired,
            ),
            (
                Ok(refresh::RefreshResponse {
                    access_token: Some(jwt(json!({}))),
                    refresh_token: None,
                    id_token: None,
                    expires_in: None,
                }),
                CredentialUseError::InvalidCredential,
            ),
        ] {
            let mut store = SharedStore::with_record(original.clone());
            let refresh_lock = FakeRefreshLock::available();
            let refresher = FakeRefresher::new(vec![refresh_result]);
            let result = with_renewable_authorized_credential_from(
                &mut store,
                &FixedClock,
                &refresh_lock,
                &refresher,
                |_| Ok::<_, bool>(()),
                |error| *error,
            );

            assert_eq!(result, Err(expected));
            assert_eq!(store.record().as_deref(), Some(original.as_slice()));
            assert_eq!(store.writes(), 0);
        }

        let all_errors = [
            CredentialUseError::UnsupportedPlatform,
            CredentialUseError::NotConnected,
            CredentialUseError::StoreUnavailable,
            CredentialUseError::UnsupportedVersion,
            CredentialUseError::InvalidCredential,
            CredentialUseError::RefreshCoordinationBusy,
            CredentialUseError::RefreshTemporarilyUnavailable,
            CredentialUseError::ReauthenticationRequired,
        ];
        for error in all_errors {
            let rendered = format!("{error:?} {error}");
            for sentinel in [ACCESS, REFRESH, "old-access", "stored-id-token-sentinel"] {
                assert!(!rendered.contains(sentinel));
            }
        }
    }

    #[test]
    fn successful_fake_login_replaces_an_existing_record_once() {
        let token_server = TokenServer::respond(200, valid_token_body());
        let mut browser = FakeBrowser::success();
        let original = serde_json::to_vec(&json!({
            "schema_version": 1,
            "access_token": "old-access-token",
            "refresh_token": "old-refresh-token",
            "id_token": "old-id-token",
            "chatgpt_account_id": "old-account",
            "access_token_expires_at": 4_000,
            "email": "old@example.com",
            "plan": "plus"
        }))
        .unwrap();
        let mut store = MemoryStore {
            record: Some(original.clone()),
            ..MemoryStore::default()
        };
        let mut progress = Vec::new();

        let result = run_login(&token_server, &mut browser, &mut store, &mut progress);
        token_server.finish();

        assert!(result.is_ok());
        assert_eq!(store.writes, 1);
        let replacement = store.record.as_ref().unwrap();
        assert_ne!(replacement, &original);
        let replacement: Value = serde_json::from_slice(replacement).unwrap();
        assert_eq!(replacement["schema_version"], 1);
        assert_eq!(replacement["refresh_token"], REFRESH);
        assert_eq!(replacement["chatgpt_account_id"], ID_PAYLOAD_ACCOUNT);
    }

    #[test]
    fn browser_failure_warns_but_fallback_flow_succeeds() {
        let token_server = TokenServer::respond(200, valid_token_body());
        let mut browser = FakeBrowser::success();
        browser.fail_to_open = true;
        let mut store = MemoryStore::default();
        let mut progress = Vec::new();

        let result = run_login(&token_server, &mut browser, &mut store, &mut progress);
        token_server.finish();

        assert!(result.is_ok());
        assert_eq!(
            progress.last().map(String::as_str),
            Some("BrowserLaunchFailed")
        );
    }

    #[test]
    fn callback_failures_leave_existing_and_absent_credentials_unchanged() {
        let unused_endpoint = Url::parse("http://127.0.0.1:1/oauth/token").unwrap();
        let original = b"existing-record".to_vec();
        for initial in [None, Some(original)] {
            for (behavior, expected) in [
                (
                    BrowserBehavior::Callback(vec![CallbackRequest {
                        method: "GET",
                        path_and_query: format!("/auth/callback?error=access_denied&state={STATE}"),
                    }]),
                    LoginError::AuthorizationDenied,
                ),
                (
                    BrowserBehavior::Callback(vec![CallbackRequest {
                        method: "GET",
                        path_and_query: format!("/auth/callback?error=server_error&state={STATE}"),
                    }]),
                    LoginError::AuthorizationTemporarilyUnavailable,
                ),
                (
                    BrowserBehavior::Callback(vec![CallbackRequest {
                        method: "GET",
                        path_and_query: format!(
                            "/auth/callback?error=temporarily_unavailable&state={STATE}"
                        ),
                    }]),
                    LoginError::AuthorizationTemporarilyUnavailable,
                ),
                (
                    BrowserBehavior::Callback(vec![CallbackRequest {
                        method: "GET",
                        path_and_query: format!("/auth/callback?error=invalid_scope&state={STATE}"),
                    }]),
                    LoginError::AuthorizationFailed,
                ),
                (BrowserBehavior::NoCallback, LoginError::TimedOut),
                (
                    BrowserBehavior::Callback(vec![CallbackRequest {
                        method: "GET",
                        path_and_query: "/auth/callback?code=code&state=wrong-state".to_owned(),
                    }]),
                    LoginError::TimedOut,
                ),
            ] {
                let mut browser = FakeBrowser {
                    behavior: Some(behavior),
                    fail_to_open: false,
                    opened_urls: Arc::default(),
                };
                let mut store = MemoryStore {
                    record: initial.clone(),
                    ..MemoryStore::default()
                };
                let result = login_with(
                    &config(unused_endpoint.clone()),
                    &mut browser,
                    &mut store,
                    &FixedClock,
                    AuthSecrets::fixed(STATE, VERIFIER),
                    |_| {},
                );

                assert_eq!(result, Err(expected));
                assert_eq!(store.record.as_deref(), initial.as_deref());
                assert_eq!(store.writes, 0);
            }
        }
    }

    #[test]
    fn unavailable_callback_ports_fail_before_browser_or_store_access() {
        let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut config = config(Url::parse("http://127.0.0.1:1/oauth/token").unwrap());
        config.redirect_ports = vec![
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port(),
        ];
        let opened_urls = Arc::new(Mutex::new(Vec::new()));
        let mut browser = FakeBrowser {
            behavior: Some(BrowserBehavior::NoCallback),
            fail_to_open: false,
            opened_urls: Arc::clone(&opened_urls),
        };
        let original = b"existing-record".to_vec();
        let mut store = MemoryStore {
            record: Some(original.clone()),
            ..MemoryStore::default()
        };

        let result = login_with(
            &config,
            &mut browser,
            &mut store,
            &FixedClock,
            AuthSecrets::fixed(STATE, VERIFIER),
            |_| {},
        );

        assert_eq!(result, Err(LoginError::CallbackPortsUnavailable));
        assert!(opened_urls.lock().unwrap().is_empty());
        assert_eq!(store.record, Some(original));
        assert_eq!(store.writes, 0);
    }

    #[test]
    fn exchange_and_validation_failures_leave_credentials_unchanged() {
        for (status, body, expected) in [
            (
                503,
                b"provider body with secrets".to_vec(),
                LoginError::TokenExchangeUnavailable,
            ),
            (
                401,
                b"provider rejection body with secrets".to_vec(),
                LoginError::TokenExchangeRejected,
            ),
            (
                200,
                b"not-json".to_vec(),
                LoginError::TokenResponseMalformed,
            ),
            (
                200,
                serde_json::to_vec(&json!({
                    "access_token": ACCESS,
                    "refresh_token": REFRESH,
                    "id_token": "malformed-id-token",
                    "expires_in": 3600
                }))
                .unwrap(),
                LoginError::TokenResponseMalformed,
            ),
            (
                200,
                serde_json::to_vec(&json!({
                    "access_token": jwt(json!({"exp": 5_000})),
                    "refresh_token": REFRESH,
                    "id_token": jwt(json!({"email": "nuno@example.com"})),
                    "expires_in": 3600
                }))
                .unwrap(),
                LoginError::TokenResponseMalformed,
            ),
            (
                200,
                serde_json::to_vec(&json!({
                    "access_token": jwt(json!({
                        "https://api.openai.com/auth": {
                            "chatgpt_account_id": "account-two"
                        },
                        "exp": 5_000
                    })),
                    "refresh_token": REFRESH,
                    "id_token": jwt(json!({
                        "https://api.openai.com/auth": {
                            "chatgpt_account_id": "account-one"
                        }
                    })),
                    "expires_in": 3600
                }))
                .unwrap(),
                LoginError::TokenResponseMalformed,
            ),
            (
                200,
                vec![b'x'; TOKEN_RESPONSE_LIMIT + 1],
                LoginError::TokenResponseMalformed,
            ),
        ] {
            let original = b"existing-record".to_vec();
            for initial in [None, Some(original)] {
                let token_server = TokenServer::respond(status, body.clone());
                let mut browser = FakeBrowser::success();
                let mut store = MemoryStore {
                    record: initial.clone(),
                    ..MemoryStore::default()
                };
                let mut progress = Vec::new();

                let result = run_login(&token_server, &mut browser, &mut store, &mut progress);
                token_server.finish();

                assert_eq!(result, Err(expected));
                assert_eq!(store.record, initial);
                assert_eq!(store.writes, 0);
            }
        }
    }

    #[test]
    fn unavailable_token_endpoint_does_not_touch_credentials() {
        let endpoint_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = endpoint_listener.local_addr().unwrap().port();
        drop(endpoint_listener);
        let endpoint = Url::parse(&format!("http://127.0.0.1:{port}/oauth/token")).unwrap();
        let original = b"existing-record".to_vec();

        for initial in [None, Some(original)] {
            let mut browser = FakeBrowser::success();
            let mut store = MemoryStore {
                record: initial.clone(),
                ..MemoryStore::default()
            };
            let result = login_with(
                &config(endpoint.clone()),
                &mut browser,
                &mut store,
                &FixedClock,
                AuthSecrets::fixed(STATE, VERIFIER),
                |_| {},
            );

            assert_eq!(result, Err(LoginError::TokenExchangeUnavailable));
            assert_eq!(store.record, initial);
            assert_eq!(store.writes, 0);
        }
    }

    #[test]
    fn failed_store_replacement_preserves_existing_and_absent_records() {
        let original = b"existing-record".to_vec();
        for initial in [None, Some(original)] {
            let token_server = TokenServer::respond(200, valid_token_body());
            let mut browser = FakeBrowser::success();
            let mut store = MemoryStore {
                record: initial.clone(),
                fail: true,
                ..MemoryStore::default()
            };
            let mut progress = Vec::new();

            let result = run_login(&token_server, &mut browser, &mut store, &mut progress);
            token_server.finish();

            assert_eq!(result, Err(LoginError::CredentialStoreUnavailable));
            assert_eq!(store.record, initial);
            assert_eq!(store.writes, 1);
        }
    }

    #[test]
    fn generated_state_appears_only_in_the_usable_url_and_debug_is_redacted() {
        let secrets = AuthSecrets::fixed(STATE, VERIFIER);
        let url = oauth::authorization_url(
            &Url::parse("https://auth.openai.com/oauth/authorize").unwrap(),
            "http://localhost:1455/auth/callback",
            &secrets,
        );
        let progress_debug = [
            format!("{:?}", LoginProgress::OpeningBrowser),
            format!(
                "{:?}",
                LoginProgress::AuthorizationUrl(AuthorizationUrl(url.to_string()))
            ),
            format!("{:?}", LoginProgress::BrowserLaunchFailed),
        ];

        assert_eq!(progress_debug[0], "OpeningBrowser");
        assert_eq!(progress_debug[1], "AuthorizationUrl([REDACTED])");
        assert_eq!(progress_debug[2], "BrowserLaunchFailed");
        for debug in progress_debug {
            for secret in [STATE, VERIFIER, CODE, ACCESS, REFRESH] {
                assert!(!debug.contains(secret));
            }
        }
        assert!(url.as_str().contains(STATE));
        assert!(!url.as_str().contains(VERIFIER));
        for error in [
            LoginError::UnsupportedPlatform,
            LoginError::CallbackPortsUnavailable,
            LoginError::AuthorizationDenied,
            LoginError::AuthorizationTemporarilyUnavailable,
            LoginError::AuthorizationFailed,
            LoginError::InvalidCallback,
            LoginError::TimedOut,
            LoginError::TokenExchangeUnavailable,
            LoginError::TokenExchangeRejected,
            LoginError::TokenResponseMalformed,
            LoginError::CredentialStoreUnavailable,
        ] {
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains(STATE));
            assert!(!rendered.contains(VERIFIER));
            assert!(!rendered.contains(CODE));
            assert!(!rendered.contains(ACCESS));
            assert!(!rendered.contains(REFRESH));
        }
    }

    #[test]
    fn authorization_error_displays_are_categorized_without_provider_codes() {
        let cases = [
            (
                LoginError::AuthorizationDenied,
                "OpenAI Codex authorization was denied",
            ),
            (
                LoginError::AuthorizationTemporarilyUnavailable,
                "OpenAI Codex authorization is temporarily unavailable; try again",
            ),
            (
                LoginError::AuthorizationFailed,
                "OpenAI Codex authorization failed; try again",
            ),
        ];

        for (error, expected) in cases {
            let display = error.to_string();
            assert_eq!(display, expected);
            for provider_code in [
                "access_denied",
                "server_error",
                "temporarily_unavailable",
                "invalid_scope",
            ] {
                assert!(!display.contains(provider_code));
            }
        }
    }

    #[test]
    fn production_configuration_keeps_registered_ports_and_limits() {
        let config = LoginConfig::production();

        assert_eq!(config.redirect_ports, [1455, 1457]);
        assert_eq!(config.login_timeout, Duration::from_secs(300));
        assert_eq!(config.exchange.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.exchange.request_timeout, Duration::from_secs(30));
        assert_eq!(config.exchange.response_body_limit, 64 * 1024);
    }
}
