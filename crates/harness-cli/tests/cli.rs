use std::process::{Command, Output};

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
        &["auth", "login"][..],
        &["auth", "login", "-h"][..],
        &["auth", "login", "--help", "ignored"][..],
    ] {
        let output = run(arguments);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), LOGIN_HELP);
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
}
