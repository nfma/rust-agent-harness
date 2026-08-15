use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, ErrorKind, Write};
use std::process::ExitCode;

use harness_openai_codex_auth::{ConnectedAccount, LoginError, LoginProgress};

const HELP: &str = "A local coding-agent harness

Usage: harness [OPTIONS] [COMMAND]

Commands:
  auth  Manage account authentication

Options:
  -h, --help     Print help
  -V, --version  Print version
";
const AUTH_HELP: &str = "Manage account authentication

Usage: harness auth [OPTIONS] [COMMAND]

Commands:
  login  Connect an account

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
const ROOT_USAGE: &str = "harness [OPTIONS] [COMMAND]";
const AUTH_USAGE: &str = "harness auth [OPTIONS] [COMMAND]";
const LOGIN_USAGE: &str = "harness auth login [OPTIONS] <PROVIDER>";
const MAX_DIAGNOSTIC_ARGUMENT_CHARS: usize = 256;

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let mut login = ProductionLogin;
    let completion = run_application(&arguments, &mut io::stdout(), &mut io::stderr(), &mut login);
    complete(completion)
}

struct ProductionLogin;

trait LoginAction {
    fn login(
        &mut self,
        progress: &mut dyn FnMut(CliLoginProgress),
    ) -> Result<ConnectedAccount, LoginError>;
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

enum Command {
    RootHelp,
    Version,
    AuthHelp,
    LoginHelp,
    LoginOpenAiCodex,
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
) -> Completion {
    match parse_command(arguments) {
        Command::RootHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{HELP}")),
        Command::Version => Completion::new(
            ExitCode::SUCCESS,
            writeln!(stdout, "harness {}", env!("CARGO_PKG_VERSION")),
        ),
        Command::AuthHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{AUTH_HELP}")),
        Command::LoginHelp => Completion::new(ExitCode::SUCCESS, write!(stdout, "{LOGIN_HELP}")),
        Command::LoginOpenAiCodex => run_login(stdout, stderr, login),
        Command::UsageError { argument, usage } => Completion::new(
            ExitCode::from(2),
            print_usage_error(stderr, &argument, usage),
        ),
    }
}

fn parse_command(arguments: &[OsString]) -> Command {
    let Some(first) = arguments.first() else {
        return Command::RootHelp;
    };
    if is_help(first) {
        return Command::RootHelp;
    }
    if first == OsStr::new("-V") || first == OsStr::new("--version") {
        return Command::Version;
    }
    if first != OsStr::new("auth") {
        return usage_error(first, ROOT_USAGE);
    }

    let Some(second) = arguments.get(1) else {
        return Command::AuthHelp;
    };
    if is_help(second) {
        return Command::AuthHelp;
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

fn is_help(argument: &OsStr) -> bool {
    argument == OsStr::new("-h") || argument == OsStr::new("--help")
}

fn usage_error(argument: &OsStr, usage: &'static str) -> Command {
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

    struct FakeLogin {
        result: Result<ConnectedAccount, LoginError>,
        calls: usize,
        browser_failure: bool,
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

    fn run(arguments: &[&str], login: &mut FakeLogin) -> (ExitCode, String, String) {
        let arguments: Vec<_> = arguments.iter().map(OsString::from).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let completion = run_application(&arguments, &mut stdout, &mut stderr, login);
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
        let completion =
            run_application(&arguments, &mut Vec::new(), &mut BrokenWriter, &mut success);
        assert_eq!(complete(completion), ExitCode::SUCCESS);

        let mut failure = FakeLogin {
            result: Err(LoginError::TimedOut),
            calls: 0,
            browser_failure: false,
        };
        let completion =
            run_application(&arguments, &mut Vec::new(), &mut BrokenWriter, &mut failure);
        assert_eq!(complete(completion), ExitCode::FAILURE);

        let mut success = fake_success();
        let completion =
            run_application(&arguments, &mut BrokenWriter, &mut Vec::new(), &mut success);
        assert_eq!(complete(completion), ExitCode::SUCCESS);
    }
}
