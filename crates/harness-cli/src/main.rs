use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, ErrorKind, Write};
use std::process::ExitCode;

use harness_openai_codex_auth::{
    ConnectedAccount, CredentialReadError, LoginError, LoginProgress, LogoutError,
};
use harness_openai_codex_model::ModelError;
use harness_session_log::{
    AppendError, CreateError, FailureCode, ProjectedTerminal, RollbackError, SessionLog,
    SessionStatus, SessionWriter, ShowError, ShowResult, TRAILING_FRAGMENT_WARNING,
};

const HELP: &str = "A local coding-agent harness

Usage: harness [OPTIONS] [COMMAND]

Commands:
  ask    Ask OpenAI Codex one question
  auth   Manage account authentication
  model  Exercise model connections
  session  Inspect a retained session

Retention:
  Successful and failed valid asks are retained locally by default, including the
  prompt, answer or fixed failure, timestamps, and working directory when available.
  Files remain until manually removed from:
  macOS: ~/Library/Application Support/rust-agent-harness/sessions/
  Linux: $XDG_DATA_HOME/rust-agent-harness/sessions/ or
         ~/.local/share/rust-agent-harness/sessions/

Options:
  -h, --help     Print help
  -V, --version  Print version
";
const AUTH_HELP: &str = "Manage account authentication

Usage: harness auth [OPTIONS] [COMMAND]

Commands:
  login   Connect an account
  logout  Disconnect an account
  status  Inspect an account connection

Options:
  -h, --help  Print help
";
const LOGOUT_HELP: &str = "Disconnect an account

Usage: harness auth logout [OPTIONS] <PROVIDER>

Providers:
  openai-codex  OpenAI Codex through a ChatGPT subscription

Options:
  -h, --help  Print help
";
const STATUS_HELP: &str = "Inspect an account connection

Usage: harness auth status [OPTIONS] <PROVIDER>

Providers:
  openai-codex  OpenAI Codex through a ChatGPT subscription

Options:
  -h, --help  Print help
";
const LOGIN_HELP: &str = "Connect an account

Usage: harness auth login [OPTIONS] <PROVIDER>

Providers:
  openai-codex  OpenAI Codex through a ChatGPT subscription

Options:
  -h, --help  Print help
";
const MODEL_HELP: &str = "Exercise model connections

Usage: harness model [OPTIONS] [COMMAND]

Commands:
  test  Test a model connection

Options:
  -h, --help  Print help
";
const MODEL_TEST_HELP: &str = "Test a model connection

Usage: harness model test [OPTIONS] <PROVIDER>

Providers:
  openai-codex  OpenAI Codex through a ChatGPT subscription

Options:
  -h, --help  Print help
";
const ASK_HELP: &str = "Ask OpenAI Codex one question

Usage: harness ask [OPTIONS] <PROVIDER> <PROMPT>

Providers:
  openai-codex  OpenAI Codex through a ChatGPT subscription

Arguments:
  <PROMPT>  One UTF-8 prompt of at most 32768 bytes

Output:
  Terminal and bidi controls become visible escapes; all other Unicode is preserved,
  including other invisible format characters

Retention:
  Successful and failed valid asks are retained locally by default, including the
  prompt, answer or fixed failure, timestamps, and working directory when available.
  The stderr Session line is the only CLI handle. Files remain until manually removed;
  there is no opt-out, list, delete, retention, or automatic-cleanup command.
  macOS: ~/Library/Application Support/rust-agent-harness/sessions/
  Linux: $XDG_DATA_HOME/rust-agent-harness/sessions/ or
         ~/.local/share/rust-agent-harness/sessions/

Options:
  -h, --help  Print help
";
const SESSION_HELP: &str = "Inspect a retained session

Usage: harness session [OPTIONS] [COMMAND]

Commands:
  show  Show one retained session by identifier

Retention:
  Successful and failed valid asks are retained locally by default, including the
  prompt, answer or fixed failure, timestamps, and working directory when available.
  The stderr Session line is the only CLI handle. Files remain until manually removed;
  there is no opt-out, list, delete, retention, or automatic-cleanup command.
  macOS: ~/Library/Application Support/rust-agent-harness/sessions/
  Linux: $XDG_DATA_HOME/rust-agent-harness/sessions/ or
         ~/.local/share/rust-agent-harness/sessions/

Options:
  -h, --help  Print help
";
const SESSION_SHOW_HELP: &str = "Show one retained session

Usage: harness session show [OPTIONS] <SESSION_ID>

Arguments:
  <SESSION_ID>  Exactly 32 lowercase hexadecimal characters

Output:
  Prompt and answer text use the same terminal-safe rendering as ask

Options:
  -h, --help  Print help
";
const ROOT_USAGE: &str = "harness [OPTIONS] [COMMAND]";
const AUTH_USAGE: &str = "harness auth [OPTIONS] [COMMAND]";
const LOGIN_USAGE: &str = "harness auth login [OPTIONS] <PROVIDER>";
const LOGOUT_USAGE: &str = "harness auth logout [OPTIONS] <PROVIDER>";
const STATUS_USAGE: &str = "harness auth status [OPTIONS] <PROVIDER>";
const MODEL_USAGE: &str = "harness model [OPTIONS] [COMMAND]";
const MODEL_TEST_USAGE: &str = "harness model test [OPTIONS] <PROVIDER>";
const ASK_USAGE: &str = "harness ask [OPTIONS] <PROVIDER> <PROMPT>";
const SESSION_USAGE: &str = "harness session [OPTIONS] [COMMAND]";
const SESSION_SHOW_USAGE: &str = "harness session show [OPTIONS] <SESSION_ID>";
const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_DIAGNOSTIC_ARGUMENT_CHARS: usize = 256;

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let mut login = ProductionLogin;
    let mut logout = ProductionLogout;
    let mut status = ProductionStatus;
    let mut model = ProductionModel;
    let mut session = ProductionSession::new();
    let mut actions = ApplicationActions {
        login: &mut login,
        logout: &mut logout,
        status: &mut status,
        model: &mut model,
        session: &mut session,
    };
    let completion = run_application(
        &arguments,
        &mut io::stdout(),
        &mut io::stderr(),
        &mut actions,
    );
    complete(completion)
}

struct ProductionLogin;
struct ProductionLogout;
struct ProductionStatus;
struct ProductionModel;
struct ProductionSession {
    log: SessionLog,
}

struct ProductionSessionWriter {
    writer: SessionWriter,
}

trait LoginAction {
    fn login(
        &mut self,
        progress: &mut dyn FnMut(CliLoginProgress),
    ) -> Result<ConnectedAccount, LoginError>;
}

trait StatusAction {
    fn status_openai_codex(&mut self) -> Result<(), CredentialReadError>;
}

trait LogoutAction {
    fn logout_openai_codex(&mut self) -> Result<(), LogoutError>;
}

trait ModelAction {
    fn test_openai_codex(&mut self) -> Result<String, ModelError>;
    fn ask_openai_codex(&mut self, prompt: &str) -> Result<String, ModelError>;
}

trait SessionAction {
    fn start(&mut self, prompt: &str) -> Result<Box<dyn SessionWriteAction>, CreateError>;
    fn show(&mut self, session_id: &str) -> Result<ShowResult, ShowError>;
}

trait SessionWriteAction {
    fn session_id(&self) -> &str;
    fn append_assistant(self: Box<Self>, text: &str) -> Result<(), AppendError>;
    fn append_failure(self: Box<Self>, failure_code: FailureCode) -> Result<(), AppendError>;
    fn rollback(self: Box<Self>) -> Result<(), RollbackError>;
}

struct ApplicationActions<'a> {
    login: &'a mut dyn LoginAction,
    logout: &'a mut dyn LogoutAction,
    status: &'a mut dyn StatusAction,
    model: &'a mut dyn ModelAction,
    session: &'a mut dyn SessionAction,
}

enum CliLoginProgress {
    OpeningBrowser,
    AuthorizationUrl(String),
    BrowserLaunchFailed,
}

impl LoginAction for ProductionLogin {
    fn login(
        &mut self,
        progress: &mut dyn FnMut(CliLoginProgress),
    ) -> Result<ConnectedAccount, LoginError> {
        harness_openai_codex_auth::login(|event| progress(map_login_progress(event)))
    }
}

impl StatusAction for ProductionStatus {
    fn status_openai_codex(&mut self) -> Result<(), CredentialReadError> {
        harness_openai_codex_auth::with_authorized_credential(|_| ())
    }
}

impl LogoutAction for ProductionLogout {
    fn logout_openai_codex(&mut self) -> Result<(), LogoutError> {
        harness_openai_codex_auth::logout()
    }
}

impl ModelAction for ProductionModel {
    fn test_openai_codex(&mut self) -> Result<String, ModelError> {
        harness_openai_codex_model::test_connection()
    }

    fn ask_openai_codex(&mut self, prompt: &str) -> Result<String, ModelError> {
        harness_openai_codex_model::ask(prompt)
    }
}

impl ProductionSession {
    fn new() -> Self {
        Self {
            log: SessionLog::production(),
        }
    }

    #[cfg(test)]
    fn with_log(log: SessionLog) -> Self {
        Self { log }
    }
}

impl SessionAction for ProductionSession {
    fn start(&mut self, prompt: &str) -> Result<Box<dyn SessionWriteAction>, CreateError> {
        self.log.start(prompt).map(|writer| {
            Box::new(ProductionSessionWriter { writer }) as Box<dyn SessionWriteAction>
        })
    }

    fn show(&mut self, session_id: &str) -> Result<ShowResult, ShowError> {
        self.log.show(session_id)
    }
}

impl SessionWriteAction for ProductionSessionWriter {
    fn session_id(&self) -> &str {
        self.writer.session_id()
    }

    fn append_assistant(self: Box<Self>, text: &str) -> Result<(), AppendError> {
        let Self { writer } = *self;
        writer.append_assistant(text)
    }

    fn append_failure(self: Box<Self>, failure_code: FailureCode) -> Result<(), AppendError> {
        let Self { writer } = *self;
        writer.append_failure(failure_code)
    }

    fn rollback(self: Box<Self>) -> Result<(), RollbackError> {
        let Self { writer } = *self;
        writer.rollback()
    }
}

fn map_login_progress(progress: LoginProgress) -> CliLoginProgress {
    match progress {
        LoginProgress::OpeningBrowser => CliLoginProgress::OpeningBrowser,
        LoginProgress::AuthorizationUrl(url) => {
            CliLoginProgress::AuthorizationUrl(url.as_str().to_owned())
        }
        LoginProgress::BrowserLaunchFailed => CliLoginProgress::BrowserLaunchFailed,
    }
}

struct Completion {
    status: ExitCode,
    stdout: io::Result<()>,
    stderr: io::Result<()>,
}

impl Completion {
    fn stdout(status: ExitCode, stdout: io::Result<()>) -> Self {
        Self {
            status,
            stdout,
            stderr: Ok(()),
        }
    }

    fn stderr(status: ExitCode, stderr: io::Result<()>) -> Self {
        Self {
            status,
            stdout: Ok(()),
            stderr,
        }
    }

    fn streams(status: ExitCode, stdout: io::Result<()>, stderr: io::Result<()>) -> Self {
        Self {
            status,
            stdout,
            stderr,
        }
    }
}

enum Command<'a> {
    RootHelp,
    Version,
    AskHelp,
    AskOpenAiCodex(&'a str),
    InvalidPrompt,
    SessionHelp,
    SessionShowHelp,
    SessionShow(&'a str),
    AuthHelp,
    LoginHelp,
    LoginOpenAiCodex,
    LogoutHelp,
    LogoutOpenAiCodex,
    StatusHelp,
    StatusOpenAiCodex,
    ModelHelp,
    ModelTestHelp,
    ModelTestOpenAiCodex,
    UsageError {
        argument: OsString,
        usage: &'static str,
    },
}

fn run_application(
    arguments: &[OsString],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    actions: &mut ApplicationActions<'_>,
) -> Completion {
    match parse_command(arguments) {
        Command::RootHelp => Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{HELP}")),
        Command::Version => Completion::stdout(
            ExitCode::SUCCESS,
            writeln!(stdout, "harness {}", env!("CARGO_PKG_VERSION")),
        ),
        Command::AskHelp => Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{ASK_HELP}")),
        Command::AskOpenAiCodex(prompt) => run_ask(stdout, stderr, actions, prompt),
        Command::InvalidPrompt => Completion::stderr(ExitCode::from(2), print_prompt_error(stderr)),
        Command::SessionHelp => {
            Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{SESSION_HELP}"))
        }
        Command::SessionShowHelp => {
            Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{SESSION_SHOW_HELP}"))
        }
        Command::SessionShow(session_id) => {
            run_session_show(stdout, stderr, actions.session, session_id)
        }
        Command::AuthHelp => Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{AUTH_HELP}")),
        Command::LoginHelp => Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{LOGIN_HELP}")),
        Command::LoginOpenAiCodex => run_login(stdout, stderr, actions.login),
        Command::LogoutHelp => {
            Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{LOGOUT_HELP}"))
        }
        Command::LogoutOpenAiCodex => run_logout(stdout, stderr, actions.logout),
        Command::StatusHelp => {
            Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{STATUS_HELP}"))
        }
        Command::StatusOpenAiCodex => run_status(stdout, stderr, actions.status),
        Command::ModelHelp => Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{MODEL_HELP}")),
        Command::ModelTestHelp => {
            Completion::stdout(ExitCode::SUCCESS, write!(stdout, "{MODEL_TEST_HELP}"))
        }
        Command::ModelTestOpenAiCodex => run_model_test(stdout, stderr, actions.model),
        Command::UsageError { argument, usage } => Completion::stderr(
            ExitCode::from(2),
            print_usage_error(stderr, &argument, usage),
        ),
    }
}

fn parse_command(arguments: &[OsString]) -> Command<'_> {
    let Some(first) = arguments.first() else {
        return Command::RootHelp;
    };
    if is_help(first) {
        return Command::RootHelp;
    }
    if first == OsStr::new("-V") || first == OsStr::new("--version") {
        return Command::Version;
    }
    if first == OsStr::new("auth") {
        return parse_auth(arguments);
    }
    if first == OsStr::new("ask") {
        return parse_ask(arguments);
    }
    if first == OsStr::new("model") {
        return parse_model(arguments);
    }
    if first == OsStr::new("session") {
        return parse_session(arguments);
    }
    usage_error(first, ROOT_USAGE)
}

fn parse_session(arguments: &[OsString]) -> Command<'_> {
    let Some(command) = arguments.get(1) else {
        return Command::SessionHelp;
    };
    if is_help(command) {
        return arguments.get(2).map_or(Command::SessionHelp, |trailing| {
            usage_error(trailing, SESSION_USAGE)
        });
    }
    if command != OsStr::new("show") {
        return usage_error(command, SESSION_USAGE);
    }

    let Some(session_id) = arguments.get(2) else {
        return Command::UsageError {
            argument: OsString::from("<SESSION_ID>"),
            usage: SESSION_SHOW_USAGE,
        };
    };
    if is_help(session_id) {
        return arguments
            .get(3)
            .map_or(Command::SessionShowHelp, |trailing| {
                usage_error(trailing, SESSION_SHOW_USAGE)
            });
    }
    if let Some(trailing) = arguments.get(3) {
        return usage_error(trailing, SESSION_SHOW_USAGE);
    }
    let Some(session_id) = session_id.to_str() else {
        return usage_error(session_id, SESSION_SHOW_USAGE);
    };
    if !valid_session_id(session_id) {
        return usage_error(OsStr::new(session_id), SESSION_SHOW_USAGE);
    }
    Command::SessionShow(session_id)
}

fn valid_session_id(session_id: &str) -> bool {
    session_id.len() == 32
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn parse_auth(arguments: &[OsString]) -> Command<'_> {
    let Some(second) = arguments.get(1) else {
        return Command::AuthHelp;
    };
    if is_help(second) {
        return Command::AuthHelp;
    }
    if second == OsStr::new("logout") {
        return parse_auth_provider(
            arguments,
            Command::LogoutHelp,
            LOGOUT_USAGE,
            Command::LogoutOpenAiCodex,
        );
    }
    if second == OsStr::new("status") {
        return parse_auth_provider(
            arguments,
            Command::StatusHelp,
            STATUS_USAGE,
            Command::StatusOpenAiCodex,
        );
    }
    if second != OsStr::new("login") {
        return usage_error(second, AUTH_USAGE);
    }

    parse_auth_provider(
        arguments,
        Command::LoginHelp,
        LOGIN_USAGE,
        Command::LoginOpenAiCodex,
    )
}

fn parse_auth_provider<'a>(
    arguments: &'a [OsString],
    help: Command<'a>,
    usage: &'static str,
    success: Command<'a>,
) -> Command<'a> {
    let Some(provider) = arguments.get(2) else {
        return help;
    };
    if is_help(provider) {
        return help;
    }
    if provider != OsStr::new("openai-codex") {
        return usage_error(provider, usage);
    }
    if let Some(trailing) = arguments.get(3) {
        return usage_error(trailing, usage);
    }
    success
}

fn parse_model(arguments: &[OsString]) -> Command<'_> {
    let Some(second) = arguments.get(1) else {
        return Command::ModelHelp;
    };
    if is_help(second) {
        return Command::ModelHelp;
    }
    if second != OsStr::new("test") {
        return usage_error(second, MODEL_USAGE);
    }

    let Some(third) = arguments.get(2) else {
        return Command::ModelTestHelp;
    };
    if is_help(third) {
        return Command::ModelTestHelp;
    }
    if third != OsStr::new("openai-codex") {
        return usage_error(third, MODEL_TEST_USAGE);
    }
    if let Some(trailing) = arguments.get(3) {
        return usage_error(trailing, MODEL_TEST_USAGE);
    }

    Command::ModelTestOpenAiCodex
}

fn parse_ask(arguments: &[OsString]) -> Command<'_> {
    let Some(provider) = arguments.get(1) else {
        return Command::AskHelp;
    };
    if is_help(provider) {
        return Command::AskHelp;
    }
    if provider != OsStr::new("openai-codex") {
        return usage_error(provider, ASK_USAGE);
    }

    let Some(prompt) = arguments.get(2) else {
        return Command::InvalidPrompt;
    };
    if is_help(prompt) {
        return Command::AskHelp;
    }
    if prompt == OsStr::new("--") {
        return usage_error(prompt, ASK_USAGE);
    }
    if let Some(trailing) = arguments.get(3) {
        return usage_error(trailing, ASK_USAGE);
    }
    let Some(prompt) = prompt.to_str() else {
        return Command::InvalidPrompt;
    };
    if !valid_prompt(prompt) {
        return Command::InvalidPrompt;
    }

    Command::AskOpenAiCodex(prompt)
}

fn valid_prompt(prompt: &str) -> bool {
    !prompt.trim().is_empty() && prompt.len() <= MAX_PROMPT_BYTES
}

fn is_help(argument: &OsStr) -> bool {
    argument == OsStr::new("-h") || argument == OsStr::new("--help")
}

fn usage_error(argument: &OsStr, usage: &'static str) -> Command<'static> {
    Command::UsageError {
        argument: argument.to_owned(),
        usage,
    }
}

fn run_login(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    login: &mut dyn LoginAction,
) -> Completion {
    let mut output_error = None;
    let result = login.login(&mut |progress| {
        if output_error.is_some() {
            return;
        }
        let result = match progress {
            CliLoginProgress::OpeningBrowser => {
                writeln!(stderr, "Opening your browser to connect OpenAI Codex.")
            }
            CliLoginProgress::AuthorizationUrl(url) => {
                writeln!(stderr, "If it does not open, visit: {url}")
            }
            CliLoginProgress::BrowserLaunchFailed => writeln!(
                stderr,
                "The browser did not open; use the authorization URL above."
            ),
        };
        if let Err(error) = result {
            output_error = Some(error);
        }
    });

    let status = if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
    if let Some(error) = output_error {
        return Completion::stderr(status, Err(error));
    }

    match result {
        Ok(account) => Completion::stdout(
            status,
            writeln!(
                stdout,
                "Connected to OpenAI Codex as {}.",
                account.email.as_deref().unwrap_or("your account")
            ),
        ),
        Err(error) => Completion::stderr(status, writeln!(stderr, "error: {error}")),
    }
}

fn run_status(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    status: &mut dyn StatusAction,
) -> Completion {
    match status.status_openai_codex() {
        Ok(()) => Completion::stdout(
            ExitCode::SUCCESS,
            writeln!(stdout, "OpenAI Codex is connected."),
        ),
        Err(error) => Completion::stderr(ExitCode::FAILURE, writeln!(stderr, "error: {error}")),
    }
}

fn run_logout(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    logout: &mut dyn LogoutAction,
) -> Completion {
    match logout.logout_openai_codex() {
        Ok(()) => Completion::stdout(
            ExitCode::SUCCESS,
            writeln!(stdout, "OpenAI Codex is disconnected."),
        ),
        Err(error) => Completion::stderr(ExitCode::FAILURE, writeln!(stderr, "error: {error}")),
    }
}

fn run_model_test(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    model: &mut dyn ModelAction,
) -> Completion {
    match model.test_openai_codex() {
        Ok(text) => write_model_text(stdout, stderr, &text),
        Err(error) => Completion::stderr(ExitCode::FAILURE, writeln!(stderr, "error: {error}")),
    }
}

fn run_ask(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    actions: &mut ApplicationActions<'_>,
    prompt: &str,
) -> Completion {
    let writer = match actions.session.start(prompt) {
        Ok(writer) => writer,
        Err(error) => {
            return Completion::stderr(ExitCode::FAILURE, writeln!(stderr, "error: {error}"));
        }
    };
    let session_id = writer.session_id().to_owned();

    match actions.model.ask_openai_codex(prompt) {
        Ok(text) => {
            let append = writer.append_assistant(&text);
            let status = if append.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
            let rendered = render_model_text(&text);
            let stdout_result = stdout.write_all(rendered.as_bytes());
            let mut stderr_result = Ok(());
            if stdout_result
                .as_ref()
                .is_err_and(|error| error.kind() != ErrorKind::BrokenPipe)
            {
                retain_first_error(
                    &mut stderr_result,
                    writeln!(stderr, "error: unable to write model output"),
                );
            }
            if let Err(error) = append {
                retain_first_error(&mut stderr_result, writeln!(stderr, "error: {error}"));
            }
            retain_first_error(
                &mut stderr_result,
                writeln!(stderr, "Session: {session_id}"),
            );
            Completion::streams(status, stdout_result, stderr_result)
        }
        Err(ModelError::InvalidPrompt) => match writer.rollback() {
            Ok(()) => Completion::stderr(ExitCode::from(2), print_prompt_error(stderr)),
            Err(error) => Completion::stderr(ExitCode::FAILURE, writeln!(stderr, "error: {error}")),
        },
        Err(error) => {
            let append = writer.append_failure(model_failure_code(error));
            let mut stderr_result = writeln!(stderr, "error: {error}");
            if let Err(error) = append {
                retain_first_error(&mut stderr_result, writeln!(stderr, "error: {error}"));
            }
            retain_first_error(
                &mut stderr_result,
                writeln!(stderr, "Session: {session_id}"),
            );
            Completion::stderr(ExitCode::FAILURE, stderr_result)
        }
    }
}

fn model_failure_code(error: ModelError) -> FailureCode {
    match error {
        ModelError::InvalidPrompt => unreachable!("invalid prompts are rolled back"),
        ModelError::UnsupportedPlatform => FailureCode::UnsupportedPlatform,
        ModelError::NotConnected => FailureCode::NotConnected,
        ModelError::CredentialStoreUnavailable => FailureCode::CredentialStoreUnavailable,
        ModelError::InvalidCredential => FailureCode::InvalidCredential,
        ModelError::ExpiredCredential => FailureCode::ExpiredCredential,
        ModelError::CredentialRefreshBusy => FailureCode::CredentialRefreshBusy,
        ModelError::CredentialRefreshUnavailable => FailureCode::CredentialRefreshUnavailable,
        ModelError::Unavailable => FailureCode::Unavailable,
        ModelError::UnexpectedProviderResponse => FailureCode::UnexpectedProviderResponse,
        ModelError::ConnectionRejected => FailureCode::ConnectionRejected,
        ModelError::AccessDenied => FailureCode::AccessDenied,
        ModelError::RateLimited => FailureCode::RateLimited,
        ModelError::TemporarilyUnavailable => FailureCode::TemporarilyUnavailable,
        ModelError::Rejected => FailureCode::Rejected,
        ModelError::InvalidProviderResponse => FailureCode::InvalidProviderResponse,
        ModelError::CallTimedOut => FailureCode::CallTimedOut,
        ModelError::ResponseTooLarge => FailureCode::ResponseTooLarge,
        ModelError::ModelCallFailed => FailureCode::ModelCallFailed,
        ModelError::EmptyResponse => FailureCode::EmptyResponse,
    }
}

fn run_session_show(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    session: &mut dyn SessionAction,
    session_id: &str,
) -> Completion {
    let shown = match session.show(session_id) {
        Ok(shown) => shown,
        Err(error) => {
            return Completion::stderr(ExitCode::FAILURE, writeln!(stderr, "error: {error}"));
        }
    };
    let rendered = render_session(&shown);
    let stdout_result = stdout.write_all(rendered.as_bytes());
    let mut stderr_result = Ok(());
    if shown.has_trailing_fragment() {
        retain_first_error(
            &mut stderr_result,
            writeln!(stderr, "warning: {TRAILING_FRAGMENT_WARNING}"),
        );
    }
    if stdout_result
        .as_ref()
        .is_err_and(|error| error.kind() != ErrorKind::BrokenPipe)
    {
        retain_first_error(
            &mut stderr_result,
            writeln!(stderr, "error: unable to write session output"),
        );
    }
    Completion::streams(ExitCode::SUCCESS, stdout_result, stderr_result)
}

fn render_session(shown: &ShowResult) -> String {
    let projection = shown.projection();
    let status = match projection.status() {
        SessionStatus::Completed => "completed",
        SessionStatus::Failed => "failed",
        SessionStatus::Interrupted => "interrupted",
    };
    let mut rendered = format!(
        "Session: {}\nStatus: {status}\nProvider: openai-codex\n\nUser:\n{}",
        projection.session_id(),
        render_model_text(projection.prompt())
    );
    match projection.terminal() {
        Some(ProjectedTerminal::Assistant(answer)) => {
            rendered.push_str("\nAssistant:\n");
            rendered.push_str(&render_model_text(answer));
        }
        Some(ProjectedTerminal::Failure(failure)) => {
            rendered.push_str("\nFailure: ");
            rendered.push_str(&render_model_text(failure.diagnostic()));
        }
        None => {}
    }
    rendered
}

fn retain_first_error(result: &mut io::Result<()>, next: io::Result<()>) {
    if result.is_ok() {
        *result = next;
    }
}

fn write_model_text(stdout: &mut dyn Write, stderr: &mut dyn Write, text: &str) -> Completion {
    let rendered = render_model_text(text);
    match stdout.write_all(rendered.as_bytes()) {
        Ok(()) => Completion::stdout(ExitCode::SUCCESS, Ok(())),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => {
            Completion::stdout(ExitCode::SUCCESS, Err(error))
        }
        Err(error) => Completion::streams(
            ExitCode::FAILURE,
            Err(error),
            writeln!(stderr, "error: unable to write model output"),
        ),
    }
}

fn render_model_text(text: &str) -> String {
    let capacity = text
        .len()
        .checked_mul(6)
        .and_then(|size| size.checked_add(1))
        .expect("bounded model output capacity must fit in usize");
    let mut rendered = String::with_capacity(capacity);
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                rendered.push('\n');
            }
            '\n' | '\t' | ' '..='~' => rendered.push(character),
            character if should_escape(character) => {
                rendered.extend(character.escape_unicode());
            }
            character => rendered.push(character),
        }
    }
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

fn should_escape(character: char) -> bool {
    character.is_control() || is_bidi_control(character)
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{61c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn print_prompt_error(output: &mut dyn Write) -> io::Result<()> {
    writeln!(
        output,
        "error: prompt must contain non-whitespace text and be at most 32768 bytes\n\nUsage: {ASK_USAGE}\n\nFor more information, try '--help'."
    )
}

fn print_usage_error(output: &mut dyn Write, argument: &OsStr, usage: &str) -> io::Result<()> {
    let argument = format_argument(argument);

    writeln!(
        output,
        "error: unexpected argument '{argument}'\n\nUsage: {usage}\n\nFor more information, try '--help'."
    )
}

fn format_argument(argument: &OsStr) -> String {
    let argument = argument.to_string_lossy();
    let mut characters = argument.chars();
    let prefix: String = characters
        .by_ref()
        .take(MAX_DIAGNOSTIC_ARGUMENT_CHARS)
        .collect();
    let truncated = characters.next().is_some();
    let mut formatted: String = prefix.escape_debug().collect();

    if truncated {
        formatted.push('…');
    }

    formatted
}

fn complete(completion: Completion) -> ExitCode {
    let failed_output = [completion.stdout, completion.stderr]
        .into_iter()
        .any(|result| result.is_err_and(|error| error.kind() != ErrorKind::BrokenPipe));
    if failed_output && completion.status == ExitCode::SUCCESS {
        ExitCode::FAILURE
    } else {
        completion.status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    const STATE: &str = "state-sentinel";
    const ACCESS_TOKEN: &str = "access-token-sentinel";
    const REFRESH_TOKEN: &str = "refresh-token-sentinel";
    const ID_TOKEN: &str = "id-token-sentinel";
    const CALLBACK_CODE: &str = "callback-code-sentinel";
    const VERIFIER: &str = "pkce-verifier-sentinel";
    const ACCOUNT_ID: &str = "account-id-sentinel";
    const EMAIL: &str = "email-sentinel";
    const PLAN: &str = "plan-sentinel";
    const EXPIRY: &str = "expiry-sentinel";
    const SESSION_ID: &str = "11111111111111111111111111111111";

    proptest! {
        #[test]
        fn fuzz_terminal_renderer_never_emits_raw_controls(input in any::<String>()) {
            let rendered = render_model_text(&input);
            prop_assert!(rendered.ends_with('\n'));
            prop_assert!(rendered.len() <= input.len().saturating_mul(6).saturating_add(1));
            let contains_only_safe_characters = rendered.chars().all(|character| {
                character == '\n'
                    || character == '\t'
                    || (!character.is_control() && !is_bidi_control(character))
            });
            prop_assert!(contains_only_safe_characters);
        }
    }

    struct FakeLogin {
        result: Result<ConnectedAccount, LoginError>,
        calls: usize,
        browser_failure: bool,
    }

    struct FakeModel {
        result: Result<String, ModelError>,
        calls: usize,
        ask_result: Result<String, ModelError>,
        ask_calls: usize,
        prompts: Vec<String>,
        events: Option<Rc<RefCell<Vec<&'static str>>>>,
    }

    struct FakeStatus {
        result: Result<(), CredentialReadError>,
        calls: usize,
    }

    struct FakeLogout {
        result: Result<(), LogoutError>,
        calls: usize,
    }

    #[derive(Default)]
    struct FakeSessionState {
        starts: usize,
        shows: usize,
        rollbacks: usize,
        prompts: Vec<String>,
        answers: Vec<String>,
        failures: Vec<FailureCode>,
    }

    struct FakeSession {
        state: Rc<RefCell<FakeSessionState>>,
        start_error: Option<CreateError>,
        append_error: Option<AppendError>,
        rollback_error: Option<RollbackError>,
        show_result: Result<ShowResult, ShowError>,
        events: Option<Rc<RefCell<Vec<&'static str>>>>,
    }

    struct FakeSessionWriter {
        state: Rc<RefCell<FakeSessionState>>,
        append_error: Option<AppendError>,
        rollback_error: Option<RollbackError>,
        events: Option<Rc<RefCell<Vec<&'static str>>>>,
    }

    impl SessionAction for FakeSession {
        fn start(&mut self, prompt: &str) -> Result<Box<dyn SessionWriteAction>, CreateError> {
            if let Some(events) = &self.events {
                events.borrow_mut().push("session-start");
            }
            self.state.borrow_mut().starts += 1;
            if let Some(error) = self.start_error {
                return Err(error);
            }
            self.state.borrow_mut().prompts.push(prompt.to_owned());
            Ok(Box::new(FakeSessionWriter {
                state: Rc::clone(&self.state),
                append_error: self.append_error,
                rollback_error: self.rollback_error,
                events: self.events.as_ref().map(Rc::clone),
            }))
        }

        fn show(&mut self, _session_id: &str) -> Result<ShowResult, ShowError> {
            self.state.borrow_mut().shows += 1;
            self.show_result.clone()
        }
    }

    impl SessionWriteAction for FakeSessionWriter {
        fn session_id(&self) -> &str {
            SESSION_ID
        }

        fn append_assistant(self: Box<Self>, text: &str) -> Result<(), AppendError> {
            if let Some(events) = &self.events {
                events.borrow_mut().push("session-terminal");
            }
            self.state.borrow_mut().answers.push(text.to_owned());
            self.append_error.map_or(Ok(()), Err)
        }

        fn append_failure(self: Box<Self>, failure_code: FailureCode) -> Result<(), AppendError> {
            if let Some(events) = &self.events {
                events.borrow_mut().push("session-terminal");
            }
            self.state.borrow_mut().failures.push(failure_code);
            self.append_error.map_or(Ok(()), Err)
        }

        fn rollback(self: Box<Self>) -> Result<(), RollbackError> {
            if let Some(events) = &self.events {
                events.borrow_mut().push("session-rollback");
            }
            self.state.borrow_mut().rollbacks += 1;
            self.rollback_error.map_or(Ok(()), Err)
        }
    }

    impl LogoutAction for FakeLogout {
        fn logout_openai_codex(&mut self) -> Result<(), LogoutError> {
            self.calls += 1;
            self.result
        }
    }

    impl StatusAction for FakeStatus {
        fn status_openai_codex(&mut self) -> Result<(), CredentialReadError> {
            self.calls += 1;
            self.result
        }
    }

    impl ModelAction for FakeModel {
        fn test_openai_codex(&mut self) -> Result<String, ModelError> {
            self.calls += 1;
            self.result.clone()
        }

        fn ask_openai_codex(&mut self, prompt: &str) -> Result<String, ModelError> {
            if let Some(events) = &self.events {
                events.borrow_mut().push("model");
            }
            self.ask_calls += 1;
            self.prompts.push(prompt.to_owned());
            self.ask_result.clone()
        }
    }

    impl LoginAction for FakeLogin {
        fn login(
            &mut self,
            progress: &mut dyn FnMut(CliLoginProgress),
        ) -> Result<ConnectedAccount, LoginError> {
            self.calls += 1;
            progress(CliLoginProgress::OpeningBrowser);
            progress(CliLoginProgress::AuthorizationUrl(format!(
                "https://auth.openai.com/oauth/authorize?state={STATE}&code_challenge=challenge"
            )));
            if self.browser_failure {
                progress(CliLoginProgress::BrowserLaunchFailed);
            }
            self.result.clone()
        }
    }

    fn fake_success() -> FakeLogin {
        FakeLogin {
            result: Ok(ConnectedAccount {
                email: Some("nuno@example.com".to_owned()),
                plan: Some("plus".to_owned()),
            }),
            calls: 0,
            browser_failure: false,
        }
    }

    fn fake_model_success() -> FakeModel {
        FakeModel {
            result: Ok("OpenAI Codex connection verified.".to_owned()),
            calls: 0,
            ask_result: Ok("One-shot answer.".to_owned()),
            ask_calls: 0,
            prompts: Vec::new(),
            events: None,
        }
    }

    fn fake_status_success() -> FakeStatus {
        FakeStatus {
            result: Ok(()),
            calls: 0,
        }
    }

    fn fake_logout_success() -> FakeLogout {
        FakeLogout {
            result: Ok(()),
            calls: 0,
        }
    }

    fn fake_session_success() -> FakeSession {
        FakeSession {
            state: Rc::new(RefCell::new(FakeSessionState::default())),
            start_error: None,
            append_error: None,
            rollback_error: None,
            show_result: Err(ShowError::Missing),
            events: None,
        }
    }

    fn run(arguments: &[&str], login: &mut FakeLogin) -> (ExitCode, String, String) {
        run_with_model(arguments, login, &mut fake_model_success())
    }

    fn run_with_model(
        arguments: &[&str],
        login: &mut FakeLogin,
        model: &mut FakeModel,
    ) -> (ExitCode, String, String) {
        run_with_actions(
            arguments,
            login,
            &mut fake_logout_success(),
            &mut fake_status_success(),
            model,
        )
    }

    fn run_with_status(arguments: &[&str], status: &mut FakeStatus) -> (ExitCode, String, String) {
        run_with_actions(
            arguments,
            &mut fake_success(),
            &mut fake_logout_success(),
            status,
            &mut fake_model_success(),
        )
    }

    fn run_with_logout(arguments: &[&str], logout: &mut FakeLogout) -> (ExitCode, String, String) {
        run_with_actions(
            arguments,
            &mut fake_success(),
            logout,
            &mut fake_status_success(),
            &mut fake_model_success(),
        )
    }

    fn run_with_actions(
        arguments: &[&str],
        login: &mut FakeLogin,
        logout: &mut FakeLogout,
        status: &mut FakeStatus,
        model: &mut FakeModel,
    ) -> (ExitCode, String, String) {
        run_with_actions_and_session(
            arguments,
            login,
            logout,
            status,
            model,
            &mut fake_session_success(),
        )
    }

    fn run_with_actions_and_session(
        arguments: &[&str],
        login: &mut FakeLogin,
        logout: &mut FakeLogout,
        status: &mut FakeStatus,
        model: &mut FakeModel,
        session: &mut FakeSession,
    ) -> (ExitCode, String, String) {
        let arguments: Vec<_> = arguments.iter().map(OsString::from).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let completion = run_application_with_session(
            &arguments,
            &mut stdout,
            &mut stderr,
            test_actions(login, logout, status, model, session),
        );
        let status = complete(completion);
        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    fn run_application(
        arguments: &[OsString],
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
        login: &mut dyn LoginAction,
        logout: &mut dyn LogoutAction,
        status: &mut dyn StatusAction,
        model: &mut dyn ModelAction,
    ) -> Completion {
        run_application_with_session(
            arguments,
            stdout,
            stderr,
            test_actions(login, logout, status, model, &mut fake_session_success()),
        )
    }

    struct TestApplicationActions<'a> {
        login: &'a mut dyn LoginAction,
        logout: &'a mut dyn LogoutAction,
        status: &'a mut dyn StatusAction,
        model: &'a mut dyn ModelAction,
        session: &'a mut dyn SessionAction,
    }

    fn test_actions<'a>(
        login: &'a mut dyn LoginAction,
        logout: &'a mut dyn LogoutAction,
        status: &'a mut dyn StatusAction,
        model: &'a mut dyn ModelAction,
        session: &'a mut dyn SessionAction,
    ) -> TestApplicationActions<'a> {
        TestApplicationActions {
            login,
            logout,
            status,
            model,
            session,
        }
    }

    fn run_application_with_session(
        arguments: &[OsString],
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
        actions: TestApplicationActions<'_>,
    ) -> Completion {
        let TestApplicationActions {
            login,
            logout,
            status,
            model,
            session,
        } = actions;
        let mut actions = ApplicationActions {
            login,
            logout,
            status,
            model,
            session,
        };
        super::run_application(arguments, stdout, stderr, &mut actions)
    }

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "closed"))
        }
    }

    struct FailedWriter;

    impl Write for FailedWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("failed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("failed"))
        }
    }

    struct EventWriter {
        events: Rc<RefCell<Vec<&'static str>>>,
        event: &'static str,
        recorded: bool,
    }

    impl Write for EventWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if !self.recorded {
                self.events.borrow_mut().push(self.event);
                self.recorded = true;
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};

            static NEXT: AtomicU64 = AtomicU64::new(0);
            Self(env::temp_dir().join(format!(
                "rust-agent-harness-cli-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            if self
                .0
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rust-agent-harness-cli-"))
            {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn session_durability_boundaries_complete_before_model_and_output() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut model = fake_model_success();
        model.events = Some(Rc::clone(&events));
        let mut session = fake_session_success();
        session.events = Some(Rc::clone(&events));
        let arguments: Vec<_> = ["ask", "openai-codex", "prompt"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let completion = run_application_with_session(
            &arguments,
            &mut EventWriter {
                events: Rc::clone(&events),
                event: "stdout",
                recorded: false,
            },
            &mut EventWriter {
                events: Rc::clone(&events),
                event: "stderr",
                recorded: false,
            },
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );

        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert_eq!(
            *events.borrow(),
            [
                "session-start",
                "model",
                "session-terminal",
                "stdout",
                "stderr"
            ]
        );
    }

    #[test]
    fn shell_invalid_prompts_never_create_and_defensive_invalid_prompt_rolls_back() {
        for arguments in [
            &["ask", "openai-codex"][..],
            &["ask", "openai-codex", ""][..],
            &["ask", "openai-codex", "   "][..],
            &["ask", "openai-codex", "prompt", "extra"][..],
        ] {
            let mut session = fake_session_success();
            let _ = run_with_actions_and_session(
                arguments,
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
                &mut session,
            );
            assert_eq!(session.state.borrow().starts, 0, "{arguments:?}");
        }

        let mut model = fake_model_success();
        model.ask_result = Err(ModelError::InvalidPrompt);
        let mut session = fake_session_success();
        let (status, stdout, stderr) = run_with_actions_and_session(
            &["ask", "openai-codex", "valid"],
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut model,
            &mut session,
        );
        assert_eq!(status, ExitCode::from(2));
        assert!(stdout.is_empty());
        assert!(!stderr.contains("Session:"));
        let state = session.state.borrow();
        assert_eq!(state.starts, 1);
        assert_eq!(state.rollbacks, 1);
        assert!(state.answers.is_empty());
        assert!(state.failures.is_empty());
        assert_eq!(model.ask_calls, 1);
        assert_eq!(model.calls, 0);
    }

    #[test]
    fn session_start_and_rollback_failures_stop_without_publishing_an_identifier() {
        let mut session = fake_session_success();
        session.start_error = Some(CreateError::StoreUnavailable);
        let mut model = fake_model_success();
        let (status, stdout, stderr) = run_with_actions_and_session(
            &["ask", "openai-codex", "prompt"],
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut model,
            &mut session,
        );
        assert_eq!(status, ExitCode::FAILURE);
        assert!(stdout.is_empty());
        assert_eq!(stderr, "error: unable to create the local session\n");
        assert_eq!(model.ask_calls, 0);
        assert_eq!(session.state.borrow().starts, 1);

        let mut session = fake_session_success();
        session.rollback_error = Some(RollbackError::StoreUnavailable);
        let mut model = fake_model_success();
        model.ask_result = Err(ModelError::InvalidPrompt);
        let (status, stdout, stderr) = run_with_actions_and_session(
            &["ask", "openai-codex", "prompt"],
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut model,
            &mut session,
        );
        assert_eq!(status, ExitCode::FAILURE);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            "error: unable to roll back the invalid local session\n"
        );
        assert!(!stderr.contains("Session:"));
        assert_eq!(session.state.borrow().rollbacks, 1);
    }

    #[test]
    fn every_persistable_model_error_maps_to_one_closed_failure() {
        for (error, failure) in [
            (
                ModelError::UnsupportedPlatform,
                FailureCode::UnsupportedPlatform,
            ),
            (ModelError::NotConnected, FailureCode::NotConnected),
            (
                ModelError::CredentialStoreUnavailable,
                FailureCode::CredentialStoreUnavailable,
            ),
            (
                ModelError::InvalidCredential,
                FailureCode::InvalidCredential,
            ),
            (
                ModelError::ExpiredCredential,
                FailureCode::ExpiredCredential,
            ),
            (
                ModelError::CredentialRefreshBusy,
                FailureCode::CredentialRefreshBusy,
            ),
            (
                ModelError::CredentialRefreshUnavailable,
                FailureCode::CredentialRefreshUnavailable,
            ),
            (ModelError::Unavailable, FailureCode::Unavailable),
            (
                ModelError::UnexpectedProviderResponse,
                FailureCode::UnexpectedProviderResponse,
            ),
            (
                ModelError::ConnectionRejected,
                FailureCode::ConnectionRejected,
            ),
            (ModelError::AccessDenied, FailureCode::AccessDenied),
            (ModelError::RateLimited, FailureCode::RateLimited),
            (
                ModelError::TemporarilyUnavailable,
                FailureCode::TemporarilyUnavailable,
            ),
            (ModelError::Rejected, FailureCode::Rejected),
            (
                ModelError::InvalidProviderResponse,
                FailureCode::InvalidProviderResponse,
            ),
            (ModelError::CallTimedOut, FailureCode::CallTimedOut),
            (ModelError::ResponseTooLarge, FailureCode::ResponseTooLarge),
            (ModelError::ModelCallFailed, FailureCode::ModelCallFailed),
            (ModelError::EmptyResponse, FailureCode::EmptyResponse),
        ] {
            let mut model = fake_model_success();
            model.ask_result = Err(error);
            let mut session = fake_session_success();
            let (status, stdout, stderr) = run_with_actions_and_session(
                &["ask", "openai-codex", "prompt"],
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            );
            assert_eq!(status, ExitCode::FAILURE);
            assert!(stdout.is_empty());
            assert_eq!(stderr, format!("error: {error}\nSession: {SESSION_ID}\n"));
            assert_eq!(session.state.borrow().failures, [failure]);
            assert_eq!(model.ask_calls, 1);
        }
    }

    #[test]
    fn terminal_append_failure_preserves_model_result_without_retry() {
        let mut session = fake_session_success();
        session.append_error = Some(AppendError::StoreUnavailable);
        let mut model = fake_model_success();
        let (status, stdout, stderr) = run_with_actions_and_session(
            &["ask", "openai-codex", "prompt"],
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut model,
            &mut session,
        );
        assert_eq!(status, ExitCode::FAILURE);
        assert_eq!(stdout, "One-shot answer.\n");
        assert_eq!(
            stderr,
            format!("error: unable to persist the local session result\nSession: {SESSION_ID}\n")
        );
        assert_eq!(model.ask_calls, 1);
        assert_eq!(session.state.borrow().answers, ["One-shot answer."]);

        let mut session = fake_session_success();
        session.append_error = Some(AppendError::StoreUnavailable);
        let mut model = fake_model_success();
        model.ask_result = Err(ModelError::RateLimited);
        let (status, stdout, stderr) = run_with_actions_and_session(
            &["ask", "openai-codex", "prompt"],
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut model,
            &mut session,
        );
        assert_eq!(status, ExitCode::FAILURE);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            format!(
                "error: OpenAI Codex usage or rate limit reached\nerror: unable to persist the local session result\nSession: {SESSION_ID}\n"
            )
        );
        assert_eq!(model.ask_calls, 1);
        assert_eq!(session.state.borrow().failures, [FailureCode::RateLimited]);
    }

    #[test]
    fn ask_stream_failures_keep_status_and_side_effects_separate() {
        let arguments: Vec<_> = ["ask", "openai-codex", "prompt"]
            .into_iter()
            .map(OsString::from)
            .collect();

        let mut model = fake_model_success();
        let mut session = fake_session_success();
        let completion = run_application_with_session(
            &arguments,
            &mut BrokenWriter,
            &mut Vec::new(),
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert_eq!(model.ask_calls, 1);
        assert_eq!(session.state.borrow().answers.len(), 1);

        let mut model = fake_model_success();
        let mut session = fake_session_success();
        let completion = run_application_with_session(
            &arguments,
            &mut Vec::new(),
            &mut BrokenWriter,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert_eq!(model.ask_calls, 1);
        assert_eq!(session.state.borrow().answers.len(), 1);

        let mut model = fake_model_success();
        let mut session = fake_session_success();
        let completion = run_application_with_session(
            &arguments,
            &mut Vec::new(),
            &mut FailedWriter,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        assert_eq!(model.ask_calls, 1);
        assert_eq!(session.state.borrow().answers.len(), 1);

        let mut model = fake_model_success();
        model.ask_result = Err(ModelError::Unavailable);
        let mut session = fake_session_success();
        let completion = run_application_with_session(
            &arguments,
            &mut Vec::new(),
            &mut BrokenWriter,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        assert_eq!(model.ask_calls, 1);
        assert_eq!(session.state.borrow().failures.len(), 1);
    }

    #[test]
    fn session_command_grammar_is_narrow_and_non_session_commands_are_ephemeral() {
        for arguments in [
            &["session", "show"][..],
            &["session", "show", "ABCDEFABCDEFABCDEFABCDEFABCDEFAB"][..],
            &["session", "show", "../../etc/passwd"][..],
            &["session", "show", "--"][..],
            &["session", "show", SESSION_ID, "extra"][..],
            &["session", "--help", "extra"][..],
            &["session", "show", "--help", "extra"][..],
        ] {
            let mut session = fake_session_success();
            let (status, _, _) = run_with_actions_and_session(
                arguments,
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
                &mut session,
            );
            assert_eq!(status, ExitCode::from(2), "{arguments:?}");
            assert_eq!(session.state.borrow().shows, 0);
        }

        for arguments in [
            &["session"][..],
            &["session", "--help"][..],
            &["session", "show", "--help"][..],
        ] {
            let mut session = fake_session_success();
            let (status, stdout, stderr) = run_with_actions_and_session(
                arguments,
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
                &mut session,
            );
            assert_eq!(status, ExitCode::SUCCESS);
            assert!(!stdout.is_empty());
            assert!(stderr.is_empty());
            assert_eq!(session.state.borrow().shows, 0);
        }

        let mut session = fake_session_success();
        let (status, stdout, stderr) = run_with_actions_and_session(
            &["session", "show", SESSION_ID],
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut fake_model_success(),
            &mut session,
        );
        assert_eq!(status, ExitCode::FAILURE);
        assert!(stdout.is_empty());
        assert_eq!(stderr, "error: the session was not found\n");
        assert_eq!(session.state.borrow().shows, 1);

        for arguments in [
            &["--help"][..],
            &["--version"][..],
            &["auth", "status", "openai-codex"][..],
            &["model", "test", "openai-codex"][..],
        ] {
            let mut session = fake_session_success();
            let _ = run_with_actions_and_session(
                arguments,
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
                &mut session,
            );
            let state = session.state.borrow();
            assert_eq!(state.starts, 0, "{arguments:?}");
            assert_eq!(state.shows, 0, "{arguments:?}");
        }
    }

    #[test]
    fn origin_aware_boundary_persists_only_semantic_content_and_show_sanitizes_it() {
        struct SecretBoundaryModel {
            forbidden: [&'static str; 9],
            answer: String,
            calls: usize,
        }

        impl ModelAction for SecretBoundaryModel {
            fn test_openai_codex(&mut self) -> Result<String, ModelError> {
                unreachable!()
            }

            fn ask_openai_codex(&mut self, _prompt: &str) -> Result<String, ModelError> {
                self.calls += 1;
                std::hint::black_box(self.forbidden);
                Ok(self.answer.clone())
            }
        }

        let forbidden = [
            "credential-origin-sentinel",
            "authorization-origin-sentinel",
            "account-origin-sentinel",
            "callback-origin-sentinel",
            "state-origin-sentinel",
            "verifier-origin-sentinel",
            "correlation-origin-sentinel",
            "environment-origin-sentinel",
            "raw-provider-origin-sentinel",
        ];
        let prompt = "prompt-origin-sk-semantic\u{1b}[31m";
        let answer = "answer-origin-token-semantic\u{202e}";
        let root = TempRoot::new("origin-boundary");
        let mut session =
            ProductionSession::with_log(SessionLog::at_data_local_dir(root.0.clone()));
        let mut model = SecretBoundaryModel {
            forbidden,
            answer: answer.to_owned(),
            calls: 0,
        };
        let arguments: Vec<_> = ["ask", "openai-codex", prompt]
            .into_iter()
            .map(OsString::from)
            .collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let completion = run_application_with_session(
            &arguments,
            &mut stdout,
            &mut stderr,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert_eq!(model.calls, 1);
        let stderr_text = String::from_utf8(stderr).unwrap();
        let session_id = stderr_text
            .strip_prefix("Session: ")
            .and_then(|value| value.strip_suffix('\n'))
            .unwrap();
        assert!(valid_session_id(session_id));
        let path = root.0.join("sessions").join(format!("{session_id}.jsonl"));
        let canonical = std::fs::read(&path).unwrap();
        let canonical_text = String::from_utf8(canonical).unwrap();
        assert!(canonical_text.contains("prompt-origin-sk-semantic"));
        assert!(canonical_text.contains("\\u001b"));
        assert!(canonical_text.contains(answer));

        let show_arguments: Vec<_> = ["session", "show", session_id]
            .into_iter()
            .map(OsString::from)
            .collect();
        let mut show_stdout = Vec::new();
        let mut show_stderr = Vec::new();
        let completion = run_application_with_session(
            &show_arguments,
            &mut show_stdout,
            &mut show_stderr,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert!(show_stderr.is_empty());
        let show_stdout = String::from_utf8(show_stdout).unwrap();
        assert_eq!(
            show_stdout,
            format!(
                "Session: {session_id}\nStatus: completed\nProvider: openai-codex\n\nUser:\nprompt-origin-sk-semantic\\u{{1b}}[31m\n\nAssistant:\nanswer-origin-token-semantic\\u{{202e}}\n"
            )
        );

        let combined = format!(
            "{canonical_text}{show_stdout}{stderr_text}{:?}{:?}",
            ShowError::StoreUnavailable,
            CreateError::StoreUnavailable
        );
        for sentinel in forbidden {
            assert!(!combined.contains(sentinel));
        }
    }

    #[test]
    fn session_show_renders_interrupted_projection_exactly() {
        let prompt = "interrupted prompt";
        let root = TempRoot::new("show-interrupted");
        let mut log = SessionLog::at_data_local_dir(root.0.clone());
        let writer = log.start(prompt).unwrap();
        let session_id = writer.session_id().to_owned();
        drop(writer);
        let arguments: Vec<_> = ["session", "show", session_id.as_str()]
            .into_iter()
            .map(OsString::from)
            .collect();
        let mut session = ProductionSession::with_log(log);
        let mut model = fake_model_success();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let completion = run_application_with_session(
            &arguments,
            &mut stdout,
            &mut stderr,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert!(stderr.is_empty());
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!(
                "Session: {session_id}\nStatus: interrupted\nProvider: openai-codex\n\nUser:\n{prompt}\n"
            )
        );
        assert_eq!(model.calls, 0);
        assert_eq!(model.ask_calls, 0);
    }

    #[test]
    fn session_show_stream_rules_do_not_repeat_reads_or_touch_model_actions() {
        let root = TempRoot::new("show-streams");
        let mut log = SessionLog::at_data_local_dir(root.0.clone());
        let writer = log.start("prompt").unwrap();
        let session_id = writer.session_id().to_owned();
        writer.append_assistant("answer").unwrap();
        let path = root.0.join("sessions").join(format!("{session_id}.jsonl"));
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(b"torn")
            .unwrap();
        let arguments: Vec<_> = ["session", "show", session_id.as_str()]
            .into_iter()
            .map(OsString::from)
            .collect();
        let mut session = ProductionSession::with_log(log);
        let mut model = fake_model_success();
        let mut stderr = Vec::new();
        let completion = run_application_with_session(
            &arguments,
            &mut BrokenWriter,
            &mut stderr,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            format!("warning: {TRAILING_FRAGMENT_WARNING}\n")
        );
        assert_eq!(model.calls, 0);
        assert_eq!(model.ask_calls, 0);

        let mut stderr = Vec::new();
        let completion = run_application_with_session(
            &arguments,
            &mut FailedWriter,
            &mut stderr,
            test_actions(
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut model,
                &mut session,
            ),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            format!(
                "warning: {TRAILING_FRAGMENT_WARNING}\nerror: unable to write session output\n"
            )
        );
        assert_eq!(model.calls, 0);
        assert_eq!(model.ask_calls, 0);
    }

    #[test]
    fn only_the_exact_login_command_invokes_the_action() {
        for arguments in [
            &[][..],
            &["auth"][..],
            &["auth", "login"][..],
            &["auth", "login", "other"][..],
            &["auth", "login", "openai-codex", "extra"][..],
        ] {
            let mut login = fake_success();
            let _ = run(arguments, &mut login);
            assert_eq!(login.calls, 0, "unexpected invocation for {arguments:?}");
        }

        let mut login = fake_success();
        let (status, stdout, stderr) = run(&["auth", "login", "openai-codex"], &mut login);
        assert_eq!(login.calls, 1);
        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(stdout, "Connected to OpenAI Codex as nuno@example.com.\n");
        assert_eq!(
            stderr,
            format!(
                "Opening your browser to connect OpenAI Codex.\nIf it does not open, visit: https://auth.openai.com/oauth/authorize?state={STATE}&code_challenge=challenge\n"
            )
        );
    }

    #[test]
    fn only_the_exact_logout_command_invokes_the_action() {
        for arguments in [
            &[][..],
            &["auth"][..],
            &["auth", "logout"][..],
            &["auth", "logout", "-h"][..],
            &["auth", "logout", "--help", "ignored"][..],
            &["auth", "logout", "other"][..],
            &["auth", "logout", "--"][..],
            &["auth", "logout", "openai-codex", "extra"][..],
            &["auth", "logout", "openai-codex", "--help"][..],
        ] {
            let mut logout = fake_logout_success();
            let _ = run_with_logout(arguments, &mut logout);
            assert_eq!(logout.calls, 0, "unexpected invocation for {arguments:?}");
        }

        let mut logout = fake_logout_success();
        let (exit, stdout, stderr) =
            run_with_logout(&["auth", "logout", "openai-codex"], &mut logout);
        assert_eq!(logout.calls, 1);
        assert_eq!(exit, ExitCode::SUCCESS);
        assert_eq!(stdout, "OpenAI Codex is disconnected.\n");
        assert!(stderr.is_empty());
        for secret in [
            ACCESS_TOKEN,
            REFRESH_TOKEN,
            ID_TOKEN,
            CALLBACK_CODE,
            VERIFIER,
            ACCOUNT_ID,
            EMAIL,
            PLAN,
            EXPIRY,
        ] {
            assert!(!stdout.contains(secret));
        }
    }

    #[test]
    fn logout_errors_preserve_the_complete_redacted_taxonomy() {
        for (error, diagnostic) in [
            (
                LogoutError::UnsupportedPlatform,
                "error: OpenAI Codex credentials are supported only on macOS\n",
            ),
            (
                LogoutError::StoreUnavailable,
                "error: the OpenAI Codex credential store is unavailable\n",
            ),
        ] {
            let mut logout = FakeLogout {
                result: Err(error),
                calls: 0,
            };
            let (exit, stdout, stderr) =
                run_with_logout(&["auth", "logout", "openai-codex"], &mut logout);

            assert_eq!(logout.calls, 1);
            assert_eq!(exit, ExitCode::FAILURE);
            assert!(stdout.is_empty());
            assert_eq!(stderr, diagnostic);
            for secret in [
                ACCESS_TOKEN,
                REFRESH_TOKEN,
                ID_TOKEN,
                CALLBACK_CODE,
                VERIFIER,
                ACCOUNT_ID,
                EMAIL,
                PLAN,
                EXPIRY,
            ] {
                assert!(!stderr.contains(secret));
            }
        }
    }

    #[test]
    fn only_the_exact_status_command_invokes_the_action() {
        for arguments in [
            &[][..],
            &["auth"][..],
            &["auth", "status"][..],
            &["auth", "status", "-h"][..],
            &["auth", "status", "--help", "ignored"][..],
            &["auth", "status", "other"][..],
            &["auth", "status", "--"][..],
            &["auth", "status", "openai-codex", "extra"][..],
            &["auth", "status", "openai-codex", "--help"][..],
        ] {
            let mut status = fake_status_success();
            let _ = run_with_status(arguments, &mut status);
            assert_eq!(status.calls, 0, "unexpected invocation for {arguments:?}");
        }

        let mut status = fake_status_success();
        let (exit, stdout, stderr) =
            run_with_status(&["auth", "status", "openai-codex"], &mut status);
        assert_eq!(status.calls, 1);
        assert_eq!(exit, ExitCode::SUCCESS);
        assert_eq!(stdout, "OpenAI Codex is connected.\n");
        assert!(stderr.is_empty());
        for secret in [
            ACCESS_TOKEN,
            REFRESH_TOKEN,
            ID_TOKEN,
            CALLBACK_CODE,
            VERIFIER,
            ACCOUNT_ID,
            EMAIL,
            PLAN,
            EXPIRY,
        ] {
            assert!(!stdout.contains(secret));
        }
    }

    #[test]
    fn status_errors_preserve_the_complete_redacted_taxonomy() {
        for (error, diagnostic) in [
            (
                CredentialReadError::UnsupportedPlatform,
                "error: OpenAI Codex credentials are supported only on macOS\n",
            ),
            (
                CredentialReadError::NotConnected,
                "error: OpenAI Codex is not connected; run 'harness auth login openai-codex'\n",
            ),
            (
                CredentialReadError::StoreUnavailable,
                "error: the OpenAI Codex credential store is unavailable\n",
            ),
            (
                CredentialReadError::UnsupportedVersion,
                "error: the stored OpenAI Codex connection is invalid; run 'harness auth login openai-codex'\n",
            ),
            (
                CredentialReadError::InvalidCredential,
                "error: the stored OpenAI Codex connection is invalid; run 'harness auth login openai-codex'\n",
            ),
            (
                CredentialReadError::Expired,
                "error: the OpenAI Codex connection expired; run 'harness auth login openai-codex'\n",
            ),
        ] {
            let mut status = FakeStatus {
                result: Err(error),
                calls: 0,
            };
            let (exit, stdout, stderr) =
                run_with_status(&["auth", "status", "openai-codex"], &mut status);

            assert_eq!(status.calls, 1);
            assert_eq!(exit, ExitCode::FAILURE);
            assert!(stdout.is_empty());
            assert_eq!(stderr, diagnostic);
            for secret in [
                ACCESS_TOKEN,
                REFRESH_TOKEN,
                ID_TOKEN,
                CALLBACK_CODE,
                VERIFIER,
                ACCOUNT_ID,
                EMAIL,
                PLAN,
                EXPIRY,
            ] {
                assert!(!stderr.contains(secret));
            }
        }
    }

    #[test]
    fn only_the_exact_model_test_command_invokes_the_action() {
        for arguments in [
            &[][..],
            &["model"][..],
            &["model", "test"][..],
            &["model", "other"][..],
            &["model", "test", "other"][..],
            &["model", "test", "openai-codex", "extra"][..],
            &["auth", "login", "openai-codex"][..],
        ] {
            let mut login = fake_success();
            let mut model = fake_model_success();
            let _ = run_with_model(arguments, &mut login, &mut model);
            assert_eq!(model.calls, 0, "unexpected invocation for {arguments:?}");
        }

        let mut login = fake_success();
        let mut model = fake_model_success();
        let (status, stdout, stderr) =
            run_with_model(&["model", "test", "openai-codex"], &mut login, &mut model);

        assert_eq!(model.calls, 1);
        assert_eq!(login.calls, 0);
        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(stdout, "OpenAI Codex connection verified.\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn only_the_exact_ask_command_invokes_the_action_with_the_original_prompt() {
        let oversized = "x".repeat(MAX_PROMPT_BYTES + 1);
        for arguments in [
            &[][..],
            &["ask"][..],
            &["ask", "other", "question"][..],
            &["ask", "openai-codex"][..],
            &["ask", "openai-codex", "--"][..],
            &["ask", "openai-codex", ""][..],
            &["ask", "openai-codex", " \t\n"][..],
            &["ask", "openai-codex", &oversized][..],
            &["ask", "openai-codex", "question", "extra"][..],
            &["model", "test", "openai-codex"][..],
        ] {
            let mut login = fake_success();
            let mut model = fake_model_success();
            let _ = run_with_model(arguments, &mut login, &mut model);
            assert_eq!(
                model.ask_calls, 0,
                "unexpected invocation for {arguments:?}"
            );
        }

        let prompt = "  Explain café ownership.\nKeep this line.  ";
        let mut login = fake_success();
        let mut model = fake_model_success();
        let (status, stdout, stderr) =
            run_with_model(&["ask", "openai-codex", prompt], &mut login, &mut model);

        assert_eq!(model.ask_calls, 1);
        assert_eq!(model.calls, 0);
        assert_eq!(login.calls, 0);
        assert_eq!(model.prompts, [prompt]);
        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(stdout, "One-shot answer.\n");
        assert_eq!(stderr, format!("Session: {SESSION_ID}\n"));

        let mut login = fake_success();
        let mut model = fake_model_success();
        let (status, _, _) = run_with_model(
            &["ask", "openai-codex", "-leading-dash"],
            &mut login,
            &mut model,
        );
        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(model.prompts, ["-leading-dash"]);
    }

    #[test]
    fn prompt_validation_accepts_the_byte_limit_and_defensive_error_is_usage() {
        let boundary = "é".repeat(MAX_PROMPT_BYTES / 2);
        let mut login = fake_success();
        let mut model = fake_model_success();
        let (status, _, stderr) =
            run_with_model(&["ask", "openai-codex", &boundary], &mut login, &mut model);
        assert_eq!(boundary.len(), MAX_PROMPT_BYTES);
        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(stderr, format!("Session: {SESSION_ID}\n"));
        assert_eq!(model.prompts, [boundary]);

        let mut login = fake_success();
        let mut model = fake_model_success();
        model.ask_result = Err(ModelError::InvalidPrompt);
        let (status, stdout, stderr) =
            run_with_model(&["ask", "openai-codex", "valid"], &mut login, &mut model);
        assert_eq!(status, ExitCode::from(2));
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            "error: prompt must contain non-whitespace text and be at most 32768 bytes\n\nUsage: harness ask [OPTIONS] <PROVIDER> <PROMPT>\n\nFor more information, try '--help'.\n"
        );
    }

    #[test]
    fn model_errors_use_stderr_exit_one_and_never_expose_credential_sentinels() {
        for error in [
            ModelError::InvalidPrompt,
            ModelError::UnsupportedPlatform,
            ModelError::NotConnected,
            ModelError::CredentialStoreUnavailable,
            ModelError::InvalidCredential,
            ModelError::ExpiredCredential,
            ModelError::CredentialRefreshBusy,
            ModelError::CredentialRefreshUnavailable,
            ModelError::Unavailable,
            ModelError::UnexpectedProviderResponse,
            ModelError::ConnectionRejected,
            ModelError::AccessDenied,
            ModelError::RateLimited,
            ModelError::TemporarilyUnavailable,
            ModelError::Rejected,
            ModelError::InvalidProviderResponse,
            ModelError::CallTimedOut,
            ModelError::ResponseTooLarge,
            ModelError::ModelCallFailed,
            ModelError::EmptyResponse,
        ] {
            let mut login = fake_success();
            let mut model = FakeModel {
                result: Err(error),
                calls: 0,
                ask_result: Ok("unused".to_owned()),
                ask_calls: 0,
                prompts: Vec::new(),
                events: None,
            };
            let (status, stdout, stderr) =
                run_with_model(&["model", "test", "openai-codex"], &mut login, &mut model);
            let combined = format!("{stdout}{stderr}");

            assert_eq!(status, ExitCode::FAILURE);
            assert!(stdout.is_empty());
            assert!(stderr.starts_with("error: "));
            assert_eq!(model.calls, 1);
            for secret in [
                ACCESS_TOKEN,
                REFRESH_TOKEN,
                ID_TOKEN,
                CALLBACK_CODE,
                VERIFIER,
            ] {
                assert!(!combined.contains(secret));
            }
        }
    }

    #[test]
    fn credential_refresh_errors_have_exact_diagnostics_for_both_model_commands() {
        for (error, message) in [
            (
                ModelError::CredentialRefreshBusy,
                "OpenAI Codex credential refresh is already in progress; try again",
            ),
            (
                ModelError::CredentialRefreshUnavailable,
                "OpenAI Codex credential refresh is temporarily unavailable; try again",
            ),
        ] {
            let mut model = FakeModel {
                result: Err(error),
                calls: 0,
                ask_result: Err(error),
                ask_calls: 0,
                prompts: Vec::new(),
                events: None,
            };
            let (status, stdout, stderr) = run_with_model(
                &["model", "test", "openai-codex"],
                &mut fake_success(),
                &mut model,
            );
            assert_eq!(status, ExitCode::FAILURE);
            assert!(stdout.is_empty());
            assert_eq!(stderr, format!("error: {message}\n"));

            let (status, stdout, stderr) = run_with_model(
                &["ask", "openai-codex", "safe prompt"],
                &mut fake_success(),
                &mut model,
            );
            assert_eq!(status, ExitCode::FAILURE);
            assert!(stdout.is_empty());
            assert_eq!(stderr, format!("error: {message}\nSession: {SESSION_ID}\n"));
        }
    }

    #[test]
    fn ask_failures_do_not_echo_prompt_or_answer_sentinels() {
        let prompt = "prompt-private-sentinel";
        let answer = "answer-private-sentinel";
        let mut model = fake_model_success();
        model.ask_result = Err(ModelError::Unavailable);
        let (status, stdout, stderr) = run_with_model(
            &["ask", "openai-codex", prompt],
            &mut fake_success(),
            &mut model,
        );
        assert_eq!(status, ExitCode::FAILURE);
        assert!(stdout.is_empty());
        assert!(!stderr.contains(prompt));
        assert!(!stderr.contains(answer));

        let arguments: Vec<_> = ["ask", "openai-codex", prompt]
            .into_iter()
            .map(OsString::from)
            .collect();
        let mut model = fake_model_success();
        model.ask_result = Ok(answer.to_owned());
        let mut stderr = Vec::new();
        let completion = run_application(
            &arguments,
            &mut FailedWriter,
            &mut stderr,
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut model,
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        let stderr = String::from_utf8(stderr).unwrap();
        assert_eq!(
            stderr,
            format!("error: unable to write model output\nSession: {SESSION_ID}\n")
        );
        assert!(!stderr.contains(prompt));
        assert!(!stderr.contains(answer));
    }

    #[test]
    fn login_outputs_never_contain_non_url_secret_sentinels() {
        let mut results = vec![Ok(ConnectedAccount {
            email: Some("nuno@example.com".to_owned()),
            plan: None,
        })];
        results.extend(
            [
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
            ]
            .into_iter()
            .map(Err),
        );

        for result in results {
            let mut login = FakeLogin {
                result,
                calls: 0,
                browser_failure: true,
            };
            let (status, stdout, stderr) = run(&["auth", "login", "openai-codex"], &mut login);
            let combined = format!("{stdout}{stderr}");
            let state_count = combined.matches(STATE).count();

            assert!(status == ExitCode::SUCCESS || status == ExitCode::FAILURE);
            assert_eq!(state_count, 1);
            for secret in [
                ACCESS_TOKEN,
                REFRESH_TOKEN,
                ID_TOKEN,
                CALLBACK_CODE,
                VERIFIER,
            ] {
                assert!(!combined.contains(secret));
            }
        }
    }

    #[test]
    fn browser_failure_is_a_non_terminal_warning() {
        let mut login = fake_success();
        login.browser_failure = true;

        let (status, stdout, stderr) = run(&["auth", "login", "openai-codex"], &mut login);

        assert_eq!(status, ExitCode::SUCCESS);
        assert!(stdout.contains("Connected to OpenAI Codex"));
        assert!(stderr.ends_with("The browser did not open; use the authorization URL above.\n"));
    }

    #[test]
    fn production_progress_mapping_covers_every_auth_variant() {
        assert!(matches!(
            map_login_progress(LoginProgress::OpeningBrowser),
            CliLoginProgress::OpeningBrowser
        ));

        let mapped = map_login_progress(LoginProgress::AuthorizationUrl(
            harness_openai_codex_auth::AuthorizationUrl::from_string(
                "https://auth.openai.com/oauth/authorize?state=state-sentinel".to_owned(),
            ),
        ));
        assert!(matches!(
            mapped,
            CliLoginProgress::AuthorizationUrl(url)
                if url == "https://auth.openai.com/oauth/authorize?state=state-sentinel"
        ));

        assert!(matches!(
            map_login_progress(LoginProgress::BrowserLaunchFailed),
            CliLoginProgress::BrowserLaunchFailed
        ));
    }

    #[test]
    fn operational_errors_use_stderr_and_exit_one() {
        let mut login = FakeLogin {
            result: Err(LoginError::TimedOut),
            calls: 0,
            browser_failure: false,
        };

        let (status, stdout, stderr) = run(&["auth", "login", "openai-codex"], &mut login);

        assert_eq!(status, ExitCode::FAILURE);
        assert!(stdout.is_empty());
        assert!(stderr.ends_with("error: OpenAI Codex login timed out; try again\n"));
    }

    #[test]
    fn safe_fallback_is_used_when_email_is_absent() {
        let mut login = FakeLogin {
            result: Ok(ConnectedAccount {
                email: None,
                plan: None,
            }),
            calls: 0,
            browser_failure: false,
        };

        let (_, stdout, _) = run(&["auth", "login", "openai-codex"], &mut login);

        assert_eq!(stdout, "Connected to OpenAI Codex as your account.\n");
    }

    #[test]
    fn closed_login_streams_preserve_the_intended_status_without_panicking() {
        let arguments: Vec<_> = ["auth", "login", "openai-codex"]
            .into_iter()
            .map(OsString::from)
            .collect();

        let mut success = fake_success();
        let completion = run_application(
            &arguments,
            &mut Vec::new(),
            &mut BrokenWriter,
            &mut success,
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);

        let mut failure = FakeLogin {
            result: Err(LoginError::TimedOut),
            calls: 0,
            browser_failure: false,
        };
        let completion = run_application(
            &arguments,
            &mut Vec::new(),
            &mut BrokenWriter,
            &mut failure,
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);

        let mut success = fake_success();
        let completion = run_application(
            &arguments,
            &mut BrokenWriter,
            &mut Vec::new(),
            &mut success,
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
    }

    #[test]
    fn logout_output_failures_preserve_the_intended_status_and_single_delete() {
        let arguments: Vec<_> = ["auth", "logout", "openai-codex"]
            .into_iter()
            .map(OsString::from)
            .collect();

        let mut logout = fake_logout_success();
        let mut stderr = Vec::new();
        let completion = run_application(
            &arguments,
            &mut BrokenWriter,
            &mut stderr,
            &mut fake_success(),
            &mut logout,
            &mut fake_status_success(),
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert_eq!(logout.calls, 1);
        assert!(stderr.is_empty());

        let mut logout = fake_logout_success();
        let mut stderr = Vec::new();
        let completion = run_application(
            &arguments,
            &mut FailedWriter,
            &mut stderr,
            &mut fake_success(),
            &mut logout,
            &mut fake_status_success(),
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        assert_eq!(logout.calls, 1);
        assert!(stderr.is_empty());

        let mut logout = FakeLogout {
            result: Err(LogoutError::StoreUnavailable),
            calls: 0,
        };
        let mut stdout = Vec::new();
        let completion = run_application(
            &arguments,
            &mut stdout,
            &mut BrokenWriter,
            &mut fake_success(),
            &mut logout,
            &mut fake_status_success(),
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        assert_eq!(logout.calls, 1);
        assert!(stdout.is_empty());
    }

    #[test]
    fn status_output_failures_preserve_the_intended_status_and_single_read() {
        let arguments: Vec<_> = ["auth", "status", "openai-codex"]
            .into_iter()
            .map(OsString::from)
            .collect();

        let mut status = fake_status_success();
        let mut stderr = Vec::new();
        let completion = run_application(
            &arguments,
            &mut BrokenWriter,
            &mut stderr,
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut status,
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
        assert_eq!(status.calls, 1);
        assert!(stderr.is_empty());

        let mut status = fake_status_success();
        let mut stderr = Vec::new();
        let completion = run_application(
            &arguments,
            &mut FailedWriter,
            &mut stderr,
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut status,
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        assert_eq!(status.calls, 1);
        assert!(stderr.is_empty());

        let mut status = FakeStatus {
            result: Err(CredentialReadError::Expired),
            calls: 0,
        };
        let mut stdout = Vec::new();
        let completion = run_application(
            &arguments,
            &mut stdout,
            &mut BrokenWriter,
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut status,
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        assert_eq!(status.calls, 1);
        assert!(stdout.is_empty());
    }

    #[test]
    fn model_output_failures_preserve_broken_pipe_status_only() {
        for arguments in [
            &["model", "test", "openai-codex"][..],
            &["ask", "openai-codex", "question"][..],
        ] {
            let arguments: Vec<_> = arguments.iter().map(OsString::from).collect();
            let completion = run_application(
                &arguments,
                &mut BrokenWriter,
                &mut Vec::new(),
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
            );
            assert_eq!(complete(completion), ExitCode::SUCCESS);

            let mut stderr = Vec::new();
            let completion = run_application(
                &arguments,
                &mut FailedWriter,
                &mut stderr,
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
            );
            assert_eq!(complete(completion), ExitCode::FAILURE);
            let expected = if arguments.first() == Some(&OsString::from("ask")) {
                format!("error: unable to write model output\nSession: {SESSION_ID}\n")
            } else {
                "error: unable to write model output\n".to_owned()
            };
            assert_eq!(stderr, expected.as_bytes());

            let completion = run_application(
                &arguments,
                &mut FailedWriter,
                &mut FailedWriter,
                &mut fake_success(),
                &mut fake_logout_success(),
                &mut fake_status_success(),
                &mut fake_model_success(),
            );
            assert_eq!(complete(completion), ExitCode::FAILURE);
        }

        let mut failed_model = FakeModel {
            result: Err(ModelError::Unavailable),
            calls: 0,
            ask_result: Ok("unused".to_owned()),
            ask_calls: 0,
            prompts: Vec::new(),
            events: None,
        };
        let arguments: Vec<_> = ["model", "test", "openai-codex"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let completion = run_application(
            &arguments,
            &mut Vec::new(),
            &mut BrokenWriter,
            &mut fake_success(),
            &mut fake_logout_success(),
            &mut fake_status_success(),
            &mut failed_model,
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
    }

    #[test]
    fn renderer_preserves_ascii_and_normalizes_carriage_returns() {
        assert_eq!(
            render_model_text("ASCII \"quotes\" and \\ code\tcafé 🙂"),
            "ASCII \"quotes\" and \\ code\tcafé 🙂\n"
        );
        assert_eq!(
            render_model_text("one\r\ntwo\rthree\n"),
            "one\ntwo\nthree\n"
        );
        assert_eq!(render_model_text("already\n"), "already\n");
    }

    #[test]
    fn renderer_escapes_every_remaining_c0_and_c1_control() {
        let mut controls = String::new();
        let mut expected = String::new();
        for value in 0..=0x9f {
            let character = char::from_u32(value).unwrap();
            if character.is_control() && !matches!(character, '\n' | '\t' | '\r') {
                controls.push(character);
                expected.extend(character.escape_unicode());
            }
        }
        controls.push('\n');
        expected.push('\n');

        assert_eq!(render_model_text(&controls), expected);
    }

    #[test]
    fn renderer_escapes_exactly_the_unicode_bidi_control_set() {
        let bidi_controls = [
            '\u{61c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
            '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ];
        let mut input: String = bidi_controls.into_iter().collect();
        let mut expected = String::new();
        for character in bidi_controls {
            assert!(is_bidi_control(character));
            expected.extend(character.escape_unicode());
        }
        input.push('\n');
        expected.push('\n');

        assert_eq!(render_model_text(&input), expected);
        for character in [
            '\u{61b}', '\u{61d}', '\u{200c}', '\u{2010}', '\u{2029}', '\u{202f}', '\u{2060}',
            '\u{2065}', '\u{206a}',
        ] {
            assert!(!is_bidi_control(character));
        }

        for value in 0..=0x10ffff {
            let Some(character) = char::from_u32(value) else {
                continue;
            };
            let expected = matches!(
                character,
                '\u{0}'..='\u{1f}'
                    | '\u{7f}'..='\u{9f}'
                    | '\u{61c}'
                    | '\u{200e}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            );
            assert_eq!(should_escape(character), expected, "U+{value:04X}");
        }
    }

    #[test]
    fn renderer_preserves_every_other_unicode_scalar_byte_for_byte() {
        let preserved = concat!(
            "NFD: e\u{301}; Hebrew: שָׁ; Indic: नमस्ते; ",
            "ZWJ: 👩\u{200d}💻; ZWNJ: क्\u{200c}ष; VS: ✈\u{fe0f}; ",
            "spaces:\u{a0}\u{2003}; separators:\u{2028}\u{2029}; ",
            "format:\u{200b}\u{2060}\u{feff}; private:\u{e000}; unassigned:\u{378}\n"
        );

        assert_eq!(render_model_text(preserved), preserved);
        assert_eq!(
            render_model_text("\u{a0}\u{2028}\u{200b}\u{e000}\u{378}"),
            "\u{a0}\u{2028}\u{200b}\u{e000}\u{378}\n"
        );
    }

    #[test]
    fn renderer_reachable_worst_case_respects_the_six_times_bound() {
        let text = "\u{1b}".repeat(256 * 1024);
        let rendered = render_model_text(&text);

        assert_eq!(rendered.len(), 6 * 262_144 + 1);
        assert!(rendered.ends_with("\\u{1b}\n"));
    }

    #[test]
    fn ask_and_model_test_share_the_terminal_safe_renderer() {
        let unsafe_text = "answer\u{1b}[31m\rnext\u{202e}";
        let expected = "answer\\u{1b}[31m\nnext\\u{202e}\n";

        let mut model = fake_model_success();
        model.result = Ok(unsafe_text.to_owned());
        let (_, stdout, stderr) = run_with_model(
            &["model", "test", "openai-codex"],
            &mut fake_success(),
            &mut model,
        );
        assert_eq!(stdout, expected);
        assert!(stderr.is_empty());

        let mut model = fake_model_success();
        model.ask_result = Ok(unsafe_text.to_owned());
        let (_, stdout, stderr) = run_with_model(
            &["ask", "openai-codex", "question"],
            &mut fake_success(),
            &mut model,
        );
        assert_eq!(stdout, expected);
        assert_eq!(stderr, format!("Session: {SESSION_ID}\n"));
    }
}
