//! The `wordcraft-cli` program: [`wordcraft_cli::run`] with the process's streams.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let code = wordcraft_cli::run(args, &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
