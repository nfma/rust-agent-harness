use std::env;
use std::ffi::OsStr;
use std::io::{self, ErrorKind, Write};
use std::process::ExitCode;

const HELP: &str = "A local coding-agent harness

Usage: harness [OPTIONS]

Options:
  -h, --help     Print help
  -V, --version  Print version
";
const MAX_DIAGNOSTIC_ARGUMENT_CHARS: usize = 256;

fn main() -> ExitCode {
    let argument = env::args_os().nth(1);

    match argument.as_deref() {
        None => complete(print_help(&mut io::stdout()), ExitCode::SUCCESS),
        Some(argument) if argument == OsStr::new("-h") || argument == OsStr::new("--help") => {
            complete(print_help(&mut io::stdout()), ExitCode::SUCCESS)
        }
        Some(argument) if argument == OsStr::new("-V") || argument == OsStr::new("--version") => {
            complete(print_version(&mut io::stdout()), ExitCode::SUCCESS)
        }
        Some(argument) => complete(print_error(&mut io::stderr(), argument), ExitCode::from(2)),
    }
}

fn print_help(output: &mut impl Write) -> io::Result<()> {
    write!(output, "{HELP}")
}

fn print_version(output: &mut impl Write) -> io::Result<()> {
    writeln!(output, "harness {}", env!("CARGO_PKG_VERSION"))
}

fn print_error(output: &mut impl Write, argument: &OsStr) -> io::Result<()> {
    let argument = format_argument(argument);

    writeln!(
        output,
        "error: unexpected argument '{argument}'\n\nUsage: harness [OPTIONS]\n\nFor more information, try '--help'."
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

fn complete(result: io::Result<()>, status: ExitCode) -> ExitCode {
    match result {
        Ok(()) => status,
        Err(error) if error.kind() == ErrorKind::BrokenPipe => status,
        Err(_) => ExitCode::FAILURE,
    }
}
