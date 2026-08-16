use std::process::{Command, Output};

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

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(arguments)
        .output()
        .expect("harness should run")
}

#[test]
fn no_arguments_prints_help() {
    let output = run(&[]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), HELP);
    assert!(output.stderr.is_empty());
}

#[test]
fn help_flag_prints_help() {
    for flag in ["-h", "--help"] {
        let output = run(&[flag]);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), HELP);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn nested_help_is_available_and_terminal() {
    for arguments in [
        &["ask"][..],
        &["ask", "-h"][..],
        &["ask", "--help", "ignored"][..],
        &["ask", "openai-codex", "-h"][..],
        &["ask", "openai-codex", "--help", "ignored"][..],
    ] {
        let output = run(arguments);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), ASK_HELP);
        assert!(output.stderr.is_empty());
    }

    for arguments in [
        &["auth"][..],
        &["auth", "-h"][..],
        &["auth", "--help", "ignored"][..],
    ] {
        let output = run(arguments);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), AUTH_HELP);
        assert!(output.stderr.is_empty());
    }

    for arguments in [
        &["model"][..],
        &["model", "-h"][..],
        &["model", "--help", "ignored"][..],
    ] {
        let output = run(arguments);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), MODEL_HELP);
        assert!(output.stderr.is_empty());
    }

    for arguments in [
        &["model", "test"][..],
        &["model", "test", "-h"][..],
        &["model", "test", "--help", "ignored"][..],
    ] {
        let output = run(arguments);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), MODEL_TEST_HELP);
        assert!(output.stderr.is_empty());
    }

    for arguments in [
        &["auth", "login"][..],
        &["auth", "login", "-h"][..],
        &["auth", "login", "--help", "ignored"][..],
    ] {
        let output = run(arguments);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), LOGIN_HELP);
        assert!(output.stderr.is_empty());
    }

    for arguments in [
        &["auth", "status"][..],
        &["auth", "status", "-h"][..],
        &["auth", "status", "--help", "ignored"][..],
    ] {
        let output = run(arguments);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), STATUS_HELP);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn version_flag_prints_version() {
    for flag in ["-V", "--version"] {
        let output = run(&[flag]);

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("harness {}\n", env!("CARGO_PKG_VERSION"))
        );
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn invalid_command_shapes_print_bounded_usage_diagnostics() {
    for (arguments, argument, usage) in [
        (&["unknown"][..], "unknown", "harness [OPTIONS] [COMMAND]"),
        (
            &["auth", "unknown"][..],
            "unknown",
            "harness auth [OPTIONS] [COMMAND]",
        ),
        (
            &["auth", "login", "unknown"][..],
            "unknown",
            "harness auth login [OPTIONS] <PROVIDER>",
        ),
        (
            &["auth", "login", "openai-codex", "extra"][..],
            "extra",
            "harness auth login [OPTIONS] <PROVIDER>",
        ),
        (
            &["auth", "status", "unknown"][..],
            "unknown",
            "harness auth status [OPTIONS] <PROVIDER>",
        ),
        (
            &["auth", "status", "openai-codex", "extra"][..],
            "extra",
            "harness auth status [OPTIONS] <PROVIDER>",
        ),
        (
            &["auth", "status", "openai-codex", "--help"][..],
            "--help",
            "harness auth status [OPTIONS] <PROVIDER>",
        ),
        (
            &["model", "unknown"][..],
            "unknown",
            "harness model [OPTIONS] [COMMAND]",
        ),
        (
            &["model", "test", "unknown"][..],
            "unknown",
            "harness model test [OPTIONS] <PROVIDER>",
        ),
        (
            &["model", "test", "openai-codex", "extra"][..],
            "extra",
            "harness model test [OPTIONS] <PROVIDER>",
        ),
        (
            &["ask", "unknown", "question"][..],
            "unknown",
            "harness ask [OPTIONS] <PROVIDER> <PROMPT>",
        ),
        (
            &["ask", "openai-codex", "question", "extra"][..],
            "extra",
            "harness ask [OPTIONS] <PROVIDER> <PROMPT>",
        ),
    ] {
        let output = run(arguments);

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "error: unexpected argument '{argument}'\n\nUsage: {usage}\n\nFor more information, try '--help'.\n"
            )
        );
    }
}

#[test]
fn invalid_ask_prompts_use_one_fixed_redacted_diagnostic() {
    let oversized = "prompt-sentinel".repeat(3000);
    for arguments in [
        &["ask", "openai-codex"][..],
        &["ask", "openai-codex", ""][..],
        &["ask", "openai-codex", " \t\n"][..],
        &["ask", "openai-codex", &oversized][..],
    ] {
        let output = run(arguments);

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "error: prompt must contain non-whitespace text and be at most 32768 bytes\n\nUsage: harness ask [OPTIONS] <PROVIDER> <PROMPT>\n\nFor more information, try '--help'.\n"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("prompt-sentinel"));
    }
}

#[test]
fn invalid_command_escapes_control_characters() {
    let output = run(&["oops\nerror: forged\u{1b}[31m"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: unexpected argument 'oops\\nerror: forged\\u{1b}[31m'\n\nUsage: harness [OPTIONS] [COMMAND]\n\nFor more information, try '--help'.\n"
    );
}

#[test]
fn invalid_command_truncates_oversized_arguments() {
    let argument = "x".repeat(300);
    let output = run(&[&argument]);
    let displayed_argument = format!("{}…", "x".repeat(256));

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!(
            "error: unexpected argument '{displayed_argument}'\n\nUsage: harness [OPTIONS] [COMMAND]\n\nFor more information, try '--help'.\n"
        )
    );
}

#[test]
fn global_help_and_version_flags_are_terminal() {
    let help = run(&["--help", "extra"]);
    assert!(help.status.success());
    assert_eq!(String::from_utf8_lossy(&help.stdout), HELP);
    assert!(help.stderr.is_empty());

    let version = run(&["-V", "--help"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("harness {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(version.stderr.is_empty());
}

#[test]
fn double_dash_is_reported_as_invalid_at_each_level() {
    for arguments in [
        &["--"][..],
        &["auth", "--"][..],
        &["auth", "login", "--"][..],
        &["auth", "status", "--"][..],
        &["model", "--"][..],
        &["model", "test", "--"][..],
        &["ask", "--"][..],
        &["ask", "openai-codex", "--"][..],
    ] {
        let output = run(arguments);

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("error: unexpected argument '--'")
        );
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_prints_a_diagnostic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["auth", "login"])
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .expect("harness should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: unexpected argument '�'\n\nUsage: harness auth login [OPTIONS] <PROVIDER>\n\nFor more information, try '--help'.\n"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["auth", "status"])
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .expect("harness should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: unexpected argument '�'\n\nUsage: harness auth status [OPTIONS] <PROVIDER>\n\nFor more information, try '--help'.\n"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["ask", "openai-codex"])
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .expect("harness should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: prompt must contain non-whitespace text and be at most 32768 bytes\n\nUsage: harness ask [OPTIONS] <PROVIDER> <PROMPT>\n\nFor more information, try '--help'.\n"
    );
}

#[cfg(unix)]
#[test]
fn closed_output_streams_do_not_panic() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;

    fn closed_stream() -> Stdio {
        let (reader, writer) = UnixStream::pair().expect("socket pair should open");
        drop(reader);
        Stdio::from(OwnedFd::from(writer))
    }

    let help = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("--help")
        .stdout(closed_stream())
        .status()
        .expect("harness should run");
    assert!(help.success());

    let invalid = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("unknown")
        .stderr(closed_stream())
        .status()
        .expect("harness should run");
    assert_eq!(invalid.code(), Some(2));

    let model_help = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["model", "--help"])
        .stdout(closed_stream())
        .status()
        .expect("harness should run");
    assert!(model_help.success());

    let ask_help = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["ask", "--help"])
        .stdout(closed_stream())
        .status()
        .expect("harness should run");
    assert!(ask_help.success());

    let status_help = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["auth", "status", "--help"])
        .stdout(closed_stream())
        .status()
        .expect("harness should run");
    assert!(status_help.success());
}

#[test]
fn production_status_discards_the_opaque_credential_capability() {
    let source = include_str!("../src/main.rs");
    let (_, implementation) = source
        .split_once("impl StatusAction for ProductionStatus {")
        .expect("production status implementation should exist");
    let (implementation, _) = implementation
        .split_once("\n}\n")
        .expect("production status implementation should be bounded");

    assert_eq!(
        source.matches("with_authorized_credential(|_| ())").count(),
        1
    );
    assert_eq!(
        implementation
            .matches("with_authorized_credential(|_| ())")
            .count(),
        1
    );
    assert!(!implementation.contains("authorize("));
}
