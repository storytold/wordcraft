//! Parse `--automation-read-root` / `--automation-write-root` before automation starts: for
//! `wordcraft-cli mcp` and the desktop app's control channel (`wordcraft --control`, #3695). A
//! malformed flag must not silently remove a root and leave automation unconfined.

use std::path::PathBuf;

/// The root folders given on a command line, before they are opened ([`super::AutomationRoots::new`]).
pub struct AutomationArgs {
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    read_given: bool,
    write_given: bool,
}

impl AutomationArgs {
    /// Start from `read_root` and `write_root` (usually none), each flag not yet given.
    pub fn new(read_root: Option<PathBuf>, write_root: Option<PathBuf>) -> Self {
        Self { read_root, write_root, read_given: false, write_given: false }
    }

    /// Whether either flag was given.
    pub fn any(&self) -> bool {
        self.read_given || self.write_given
    }

    /// Returns whether this argument is a root flag. A second occurrence of a flag is an error,
    /// including when the two spellings (`--flag DIR`, `--flag=DIR`) are mixed.
    pub fn consume(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        let (flag, inline) = arg.split_once('=').map_or((arg, None), |(flag, value)| (flag, Some(value)));
        let (root, given) = match flag {
            "--automation-read-root" => (&mut self.read_root, &mut self.read_given),
            "--automation-write-root" => (&mut self.write_root, &mut self.write_given),
            _ => return Ok(false),
        };
        if *given {
            return Err(format!("{flag} given twice"));
        }
        let value = match inline {
            Some(value) => value.to_owned(),
            None => {
                let value = args.next().ok_or_else(|| format!("{flag} needs a folder"))?;
                if value.starts_with('-') {
                    return Err(format!("{flag} needs a folder; use {flag}=<dir> for a folder starting with '-'"));
                }
                value
            }
        };
        if value.is_empty() {
            return Err(format!("{flag} needs a non-empty folder"));
        }
        *root = Some(value.into());
        *given = true;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_taken_once_and_other_arguments_are_left() {
        let mut roots = AutomationArgs::new(None, None);
        let mut args = ["a folder".to_owned(), "report.docx".to_owned()].into_iter();
        assert!(roots.consume("--automation-read-root", &mut args).unwrap());
        assert!(roots.consume("--automation-write-root=out", &mut args).unwrap());
        assert!(!roots.consume("--author", &mut args).unwrap());
        assert_eq!(roots.read_root, Some("a folder".into()));
        assert_eq!(roots.write_root, Some("out".into()));
        assert_eq!(args.next().as_deref(), Some("report.docx"));
        assert!(roots.consume("--automation-read-root=other", &mut args).is_err());
        assert_eq!(roots.read_root, Some("a folder".into()));
    }

    #[test]
    fn invalid_operands_leave_roots_unchanged() {
        for flag in ["--automation-read-root", "--automation-write-root"] {
            for operand in [None, Some(""), Some("--version"), Some("--connect"), Some("-folder")] {
                let mut roots = AutomationArgs::new(Some("read".into()), Some("write".into()));
                let mut args = operand.map(str::to_owned).into_iter();
                assert!(roots.consume(flag, &mut args).is_err(), "{flag} {operand:?}");
                assert_eq!(roots.read_root, Some("read".into()));
                assert_eq!(roots.write_root, Some("write".into()));
            }
            let mut roots = AutomationArgs::new(None, None);
            assert!(roots.consume(&format!("{flag}="), &mut std::iter::empty()).is_err());
        }
    }

    #[test]
    fn explicit_dash_folder_and_relative_dash_folder_are_valid() {
        let mut roots = AutomationArgs::new(None, None);
        assert!(roots.consume("--automation-read-root=-folder", &mut std::iter::empty()).unwrap());
        assert!(roots.consume("--automation-write-root", &mut ["./-folder".to_owned()].into_iter()).unwrap());
        assert_eq!(roots.read_root, Some("-folder".into()));
        assert_eq!(roots.write_root, Some("./-folder".into()));
    }
}
