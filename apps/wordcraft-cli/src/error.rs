//! What stops a command, and how each case exits.

/// Why a CLI run stopped early.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CliError {
    /// The command failed (exit 1); the text is already safe to print.
    #[error("{0}")]
    Failed(String),
    /// A mistake in the command line found after parsing (exit 2).
    #[error("{0}")]
    Usage(String),
    /// A strict export refused (exit 1): the report's lines, then its JSON line, ready for stderr.
    #[error("{0}")]
    Strict(String),
    /// Whoever read stdout closed it (`wordcraft-cli commands --json | head`): nobody is left to write to, so a quiet
    /// exit 0 (`validate` keeps its verdict's status).
    #[error("stdout closed")]
    StdoutClosed,
}

impl From<String> for CliError {
    fn from(e: String) -> Self {
        CliError::Failed(e)
    }
}

impl From<&str> for CliError {
    fn from(e: &str) -> Self {
        CliError::Failed(e.into())
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::BrokenPipe { CliError::StdoutClosed } else { CliError::Failed(e.to_string()) }
    }
}
