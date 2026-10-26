//! Parse `--automation-read-root` / `--automation-write-root` before the server starts. Each may be
//! given more than once, for several folders (#3582). A malformed flag must not silently remove a
//! root and leave the server unconfined.

use std::path::PathBuf;

/// The root folders found on the command line so far, in the order given.
pub struct AutomationArgs {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
}

impl AutomationArgs {
    /// Starts from `read_roots` and `write_roots` (empty when parsing a command line).
    pub fn new(read_roots: Vec<PathBuf>, write_roots: Vec<PathBuf>) -> Self {
        Self { read_roots, write_roots }
    }

    /// Returns whether this argument is a root flag. Each occurrence adds a folder, in either
    /// spelling (`--flag DIR`, `--flag=DIR`).
    pub fn consume(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        let (flag, inline) = arg.split_once('=').map_or((arg, None), |(flag, value)| (flag, Some(value)));
        let roots = match flag {
            "--automation-read-root" => &mut self.read_roots,
            "--automation-write-root" => &mut self.write_roots,
            _ => return Ok(false),
        };
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
        roots.push(value.into());
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both spellings of both flags are taken; other arguments are left for the caller.
    #[test]
    fn roots_are_taken_and_other_arguments_are_left() {
        let mut roots = AutomationArgs::new(vec![], vec![]);
        let mut args = ["a folder".to_owned(), "report.docx".to_owned()].into_iter();
        assert!(roots.consume("--automation-read-root", &mut args).unwrap());
        assert!(roots.consume("--automation-write-root=out", &mut args).unwrap());
        assert!(!roots.consume("--author", &mut args).unwrap());
        assert_eq!(roots.read_roots, [PathBuf::from("a folder")]);
        assert_eq!(roots.write_roots, [PathBuf::from("out")]);
        assert_eq!(args.next().as_deref(), Some("report.docx"));
    }

    /// A flag given again adds a folder instead of being an error (#3582).
    #[test]
    fn a_root_flag_given_again_adds_a_folder() {
        let mut roots = AutomationArgs::new(vec![], vec![]);
        let mut args = ["b".to_owned(), "out 2".to_owned()].into_iter();
        assert!(roots.consume("--automation-read-root=a", &mut args).unwrap());
        assert!(roots.consume("--automation-read-root", &mut args).unwrap());
        assert!(roots.consume("--automation-write-root=out", &mut args).unwrap());
        assert!(roots.consume("--automation-write-root", &mut args).unwrap());
        assert_eq!(roots.read_roots, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(roots.write_roots, [PathBuf::from("out"), PathBuf::from("out 2")]);
    }

    /// A missing, empty or flag-like operand is an error and leaves the roots found so far as they were.
    #[test]
    fn invalid_operands_leave_roots_unchanged() {
        for flag in ["--automation-read-root", "--automation-write-root"] {
            for operand in [None, Some(""), Some("--version"), Some("--connect"), Some("-folder")] {
                let mut roots = AutomationArgs::new(vec!["read".into()], vec!["write".into()]);
                let mut args = operand.map(str::to_owned).into_iter();
                assert!(roots.consume(flag, &mut args).is_err(), "{flag} {operand:?}");
                assert_eq!(roots.read_roots, [PathBuf::from("read")]);
                assert_eq!(roots.write_roots, [PathBuf::from("write")]);
            }
            let mut roots = AutomationArgs::new(vec![], vec![]);
            assert!(roots.consume(&format!("{flag}="), &mut std::iter::empty()).is_err());
        }
    }

    /// A folder named with a leading `-` is taken when given as `--flag=-dir` or `./-dir`.
    #[test]
    fn explicit_dash_folder_and_relative_dash_folder_are_valid() {
        let mut roots = AutomationArgs::new(vec![], vec![]);
        assert!(roots.consume("--automation-read-root=-folder", &mut std::iter::empty()).unwrap());
        assert!(roots.consume("--automation-write-root", &mut ["./-folder".to_owned()].into_iter()).unwrap());
        assert_eq!(roots.read_roots, [PathBuf::from("-folder")]);
        assert_eq!(roots.write_roots, [PathBuf::from("./-folder")]);
    }
}
