//! Headless WordCraft (`wordcraft-cli`). [`run`] is the whole program, so it can be tested in-process as well as
//! through the binary; `wordcraft-cli --help` lists the subcommands. The conventions are RFC-0018's: exit 0, 1 (a
//! command failed) or 2 (a usage error), results on stdout, `error: <message>` on stderr.

mod args;
mod commands;
mod error;
mod script;

use std::ffi::OsString;
use std::io::Write;

use error::CliError;

pub use args::command;

/// Where a command writes: results to `out`, diagnostics and progress to `err`.
pub(crate) struct Io<'a> {
    pub(crate) out: &'a mut dyn Write,
    pub(crate) err: &'a mut dyn Write,
}

/// Writes a line of progress or a warning; a closed stderr is not worth an error.
macro_rules! errln {
    ($io:expr, $($arg:tt)*) => {
        drop(writeln!($io.err, $($arg)*))
    };
}

pub(crate) use errln;

/// The Launcher's Nightly bootstrap belongs to the desktop app: a CLI must not accept it, in the
/// separate-token or `=` form, so a mis-routed launch fails loudly instead of editing stable data.
fn launcher_profile_flag(args: &[OsString]) -> bool {
    args.iter().any(|arg| {
        let arg = arg.to_string_lossy();
        arg == "--launch-profile" || arg == "--profile-root" || arg.starts_with("--launch-profile=") || arg.starts_with("--profile-root=")
    })
}

/// Run the CLI on its arguments (without the program name), writing results to `out` and diagnostics to `err`.
/// Returns the process exit code: 0 on success, 1 when a command failed, 2 for a usage error. Help and the version go
/// to `out`, exit 0.
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if launcher_profile_flag(&args) {
        let _ = writeln!(err, "error: --launch-profile and --profile-root are the desktop app's Nightly bootstrap, not a CLI option");
        return 2;
    }
    // Help and the version (`help`, `-h`, `<subcommand> --help`, `-V`) come back as errors too.
    let cli = match args::parse(&args) {
        Ok(cli) => cli,
        Err(e) => {
            let to: &mut dyn Write = if e.use_stderr() { err } else { out };
            let _ = write!(to, "{}", e.render());
            return e.exit_code();
        }
    };
    let mut io = Io { out, err };
    // Language packs installed in the suite's font store are used; `proof.installPack` installs
    // only while the person has turned downloading on in the app.
    wordcraft_engine::packs::use_font_store(false);
    if let Some(source) = wordcraft_engine::packs::index_override() {
        errln!(io, "Language packs come from a test source: {source}");
    }
    match commands::dispatch(cli.command, &mut io) {
        Ok(code) => i32::from(code),
        // A reader that stopped early (a closed pipe) ends the program quietly, as ripgrep does.
        Err(CliError::StdoutClosed) => 0,
        Err(CliError::Usage(e)) => {
            errln!(io, "error: {e}");
            2
        }
        // The report's lines already start with `error:`.
        Err(CliError::Strict(report)) => {
            errln!(io, "{report}");
            1
        }
        Err(e) => {
            errln!(io, "error: {e}");
            1
        }
    }
}
