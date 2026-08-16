use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, ErrorKind, Write};
use std::process::ExitCode;

use harness_openai_codex_auth::{ConnectedAccount, CredentialReadError, LoginError, LoginProgress};
use harness_openai_codex_model::ModelError;

const HELP: &str = "A local coding-agent harness

Usage: harness [OPTIONS] [COMMAND]

Commands:
  ask    Ask OpenAI Codex one question
  auth   Manage account authentication
  model  Exercise model connections

Options:
  -h, --help     Print help
  -V, --version  Print version
";
const AUTH_HELP: &str = "Manage account authentication

Usage: harness auth [OPTIONS] [COMMAND]

Commands:
  login   Connect an account
  status  Inspect an account connection

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

Options:
  -h, --help  Print help
";
const ROOT_USAGE: &str = "harness [OPTIONS] [COMMAND]";
const AUTH_USAGE: &str = "harness auth [OPTIONS] [COMMAND]";
const LOGIN_USAGE: &str = "harness auth login [OPTIONS] <PROVIDER>";
const STATUS_USAGE: &str = "harness auth status [OPTIONS] <PROVIDER>";
const MODEL_USAGE: &str = "harness model [OPTIONS] [COMMAND]";
const MODEL_TEST_USAGE: &str = "harness model test [OPTIONS] <PROVIDER>";
const ASK_USAGE: &str = "harness ask [OPTIONS] <PROVIDER> <PROMPT>";
const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_DIAGNOSTIC_ARGUMENT_CHARS: usize = 256;

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let mut login = ProductionLogin;
    let mut status = ProductionStatus;
    let mut model = ProductionModel;
    let completion = run_application(
        &arguments,
        &mut io::stdout(),
        &mut io::stderr(),
        &mut login,
        &mut status,
        &mut model,
    );
    complete(completion)
}

struct ProductionLogin;
struct ProductionStatus;
struct ProductionModel;

trait LoginAction {
    fn login(
        &mut self,
        progress: &mut dyn FnMut(CliLoginProgress),
    ) -> Result<ConnectedAccount, LoginError>;
}

trait StatusAction {
    fn status_openai_codex(&mut self) -> Result<(), CredentialReadError>;
}

trait ModelAction {
    fn test_openai_codex(&mut self) -> Result<String, ModelError>;
    fn ask_openai_codex(&mut self, prompt: &str) -> Result<String, ModelError>;
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

impl ModelAction for ProductionModel {
    fn test_openai_codex(&mut self) -> Result<String, ModelError> {
        harness_openai_codex_model::test_connection()
    }

    fn ask_openai_codex(&mut self, prompt: &str) -> Result<String, ModelError> {
        harness_openai_codex_model::ask(prompt)
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
    output: io::Result<()>,
}

impl Completion {
    fn new(status: ExitCode, output: io::Result<()>) -> Self {
        Self { status, output }
    }
}

enum Command<'a> {
    RootHelp,
    Version,
    AskHelp,
    AskOpenAiCodex(&'a str),
    InvalidPrompt,
    AuthHelp,
    LoginHelp,
    LoginOpenAiCodex,
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
    login: &mut dyn LoginAction,
    status: &mut dyn StatusAction,
    model: &mut dyn ModelAction,
) -> Completion {
    match parse_command(arguments) {
        Command::RootHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{HELP}")),
        Command::Version => Completion::new(
            ExitCode::SUCCESS,
            writeln!(stdout, "harness {}", env!("CARGO_PKG_VERSION")),
        ),
        Command::AskHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{ASK_HELP}")),
        Command::AskOpenAiCodex(prompt) => run_ask(stdout, stderr, model, prompt),
        Command::InvalidPrompt => Completion::new(ExitCode::from(2), print_prompt_error(stderr)),
        Command::AuthHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{AUTH_HELP}")),
        Command::LoginHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{LOGIN_HELP}")),
        Command::LoginOpenAiCodex => run_login(stdout, stderr, login),
        Command::StatusHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{STATUS_HELP}")),
        Command::StatusOpenAiCodex => run_status(stdout, stderr, status),
        Command::ModelHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{MODEL_HELP}")),
        Command::ModelTestHelp => {
            Completion::new(ExitCode::SUCCESS, write!(stdout, "{MODEL_TEST_HELP}"))
        }
        Command::ModelTestOpenAiCodex => run_model_test(stdout, stderr, model),
        Command::UsageError { argument, usage } => Completion::new(
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
    usage_error(first, ROOT_USAGE)
}

fn parse_auth(arguments: &[OsString]) -> Command<'_> {
    let Some(second) = arguments.get(1) else {
        return Command::AuthHelp;
    };
    if is_help(second) {
        return Command::AuthHelp;
    }
    if second == OsStr::new("status") {
        let Some(third) = arguments.get(2) else {
            return Command::StatusHelp;
        };
        if is_help(third) {
            return Command::StatusHelp;
        }
        if third != OsStr::new("openai-codex") {
            return usage_error(third, STATUS_USAGE);
        }
        if let Some(trailing) = arguments.get(3) {
            return usage_error(trailing, STATUS_USAGE);
        }
        return Command::StatusOpenAiCodex;
    }
    if second != OsStr::new("login") {
        return usage_error(second, AUTH_USAGE);
    }

    let Some(third) = arguments.get(2) else {
        return Command::LoginHelp;
    };
    if is_help(third) {
        return Command::LoginHelp;
    }
    if third != OsStr::new("openai-codex") {
        return usage_error(third, LOGIN_USAGE);
    }
    if let Some(trailing) = arguments.get(3) {
        return usage_error(trailing, LOGIN_USAGE);
    }

    Command::LoginOpenAiCodex
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
        return Completion::new(status, Err(error));
    }

    let output = match result {
        Ok(account) => writeln!(
            stdout,
            "Connected to OpenAI Codex as {}.",
            account.email.as_deref().unwrap_or("your account")
        ),
        Err(error) => writeln!(stderr, "error: {error}"),
    };
    Completion::new(status, output)
}

fn run_status(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    status: &mut dyn StatusAction,
) -> Completion {
    match status.status_openai_codex() {
        Ok(()) => Completion::new(
            ExitCode::SUCCESS,
            writeln!(stdout, "OpenAI Codex is connected."),
        ),
        Err(error) => Completion::new(ExitCode::FAILURE, writeln!(stderr, "error: {error}")),
    }
}

fn run_model_test(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    model: &mut dyn ModelAction,
) -> Completion {
    match model.test_openai_codex() {
        Ok(text) => write_model_text(stdout, stderr, &text),
        Err(error) => Completion::new(ExitCode::FAILURE, writeln!(stderr, "error: {error}")),
    }
}

fn run_ask(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    model: &mut dyn ModelAction,
    prompt: &str,
) -> Completion {
    match model.ask_openai_codex(prompt) {
        Ok(text) => write_model_text(stdout, stderr, &text),
        Err(ModelError::InvalidPrompt) => {
            Completion::new(ExitCode::from(2), print_prompt_error(stderr))
        }
        Err(error) => Completion::new(ExitCode::FAILURE, writeln!(stderr, "error: {error}")),
    }
}

fn write_model_text(stdout: &mut dyn Write, stderr: &mut dyn Write, text: &str) -> Completion {
    let rendered = render_model_text(text);
    match stdout.write_all(rendered.as_bytes()) {
        Ok(()) => Completion::new(ExitCode::SUCCESS, Ok(())),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => {
            Completion::new(ExitCode::SUCCESS, Err(error))
        }
        Err(_) => Completion::new(
            ExitCode::FAILURE,
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
    match completion.output {
        Ok(()) => completion.status,
        Err(error) if error.kind() == ErrorKind::BrokenPipe => completion.status,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    struct FakeStatus {
        result: Result<(), CredentialReadError>,
        calls: usize,
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
        }
    }

    fn fake_status_success() -> FakeStatus {
        FakeStatus {
            result: Ok(()),
            calls: 0,
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
        run_with_actions(arguments, login, &mut fake_status_success(), model)
    }

    fn run_with_status(arguments: &[&str], status: &mut FakeStatus) -> (ExitCode, String, String) {
        run_with_actions(
            arguments,
            &mut fake_success(),
            status,
            &mut fake_model_success(),
        )
    }

    fn run_with_actions(
        arguments: &[&str],
        login: &mut FakeLogin,
        status: &mut FakeStatus,
        model: &mut FakeModel,
    ) -> (ExitCode, String, String) {
        let arguments: Vec<_> = arguments.iter().map(OsString::from).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let completion =
            run_application(&arguments, &mut stdout, &mut stderr, login, status, model);
        let status = complete(completion);
        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
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
        assert!(stderr.is_empty());

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
        assert!(stderr.is_empty());
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
            &mut fake_status_success(),
            &mut model,
        );
        assert_eq!(complete(completion), ExitCode::FAILURE);
        let stderr = String::from_utf8(stderr).unwrap();
        assert_eq!(stderr, "error: unable to write model output\n");
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
            &mut fake_status_success(),
            &mut fake_model_success(),
        );
        assert_eq!(complete(completion), ExitCode::SUCCESS);
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
                &mut fake_status_success(),
                &mut fake_model_success(),
            );
            assert_eq!(complete(completion), ExitCode::FAILURE);
            assert_eq!(stderr, b"error: unable to write model output\n");

            let completion = run_application(
                &arguments,
                &mut FailedWriter,
                &mut FailedWriter,
                &mut fake_success(),
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
        assert!(stderr.is_empty());
    }
}
