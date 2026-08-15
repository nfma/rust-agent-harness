use std::process::{Command, Output};

const HELP: &str = "A local coding-agent harness

Usage: harness [OPTIONS]

Options:
  -h, --help     Print help
  -V, --version  Print version
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
fn invalid_command_prints_a_diagnostic() {
    let output = run(&["unknown"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: unexpected argument 'unknown'\n\nUsage: harness [OPTIONS]\n\nFor more information, try '--help'.\n"
    );
}

#[test]
fn help_and_version_flags_are_terminal() {
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
fn double_dash_is_reported_as_invalid() {
    let output = run(&["--"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: unexpected argument '--'"));
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_prints_a_diagnostic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .expect("harness should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: unexpected argument '�'\n\nUsage: harness [OPTIONS]\n\nFor more information, try '--help'.\n"
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
