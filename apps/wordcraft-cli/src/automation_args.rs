//! Parse `--automation-read-root` / `--automation-write-root` and `--control-token` /
//! `--control-token-file` (#5047, as PhotoCraft names them) before the server starts. A malformed
//! flag must not silently remove a root and leave the server unconfined, nor fall back to another
//! key.

use std::path::PathBuf;

pub struct AutomationArgs {
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    /// `--control-token`: the key `mcp --connect` sends.
    pub control_token: Option<String>,
    /// `--control-token-file`: the file holding that key.
    pub control_token_file: Option<PathBuf>,
}

impl AutomationArgs {
    pub fn new(read_root: Option<PathBuf>, write_root: Option<PathBuf>) -> Self {
        Self { read_root, write_root, control_token: None, control_token_file: None }
    }

    /// Returns whether this argument is one of the flags. A second occurrence of a flag is an
    /// error, including when the two spellings (`--flag VALUE`, `--flag=VALUE`) are mixed.
    pub fn consume(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        let (flag, inline) = arg.split_once('=').map_or((arg, None), |(flag, value)| (flag, Some(value)));
        let (noun, placeholder) = match flag {
            "--automation-read-root" | "--automation-write-root" => ("folder", "<dir>"),
            "--control-token" | "--control-token-file" => ("value", "<value>"),
            _ => return Ok(false),
        };
        let given = match flag {
            "--automation-read-root" => self.read_root.is_some(),
            "--automation-write-root" => self.write_root.is_some(),
            "--control-token" => self.control_token.is_some(),
            _ => self.control_token_file.is_some(),
        };
        if given {
            return Err(format!("{flag} given twice"));
        }
        let value = match inline {
            Some(value) => value.to_owned(),
            None => {
                let value = args.next().ok_or_else(|| format!("{flag} needs a {noun}"))?;
                if value.starts_with('-') {
                    return Err(format!("{flag} needs a {noun}; use {flag}={placeholder} for one starting with '-'"));
                }
                value
            }
        };
        if value.is_empty() {
            return Err(format!("{flag} needs a non-empty {noun}"));
        }
        match flag {
            "--automation-read-root" => self.read_root = Some(value.into()),
            "--automation-write-root" => self.write_root = Some(value.into()),
            "--control-token" => self.control_token = Some(value),
            _ => self.control_token_file = Some(value.into()),
        }
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
    fn the_control_token_flags_are_taken_once() {
        let mut a = AutomationArgs::new(None, None);
        let mut args = ["abc".to_owned(), "x".to_owned()].into_iter();
        assert!(a.consume("--control-token", &mut args).unwrap());
        assert!(a.consume("--control-token-file=/p/token", &mut args).unwrap());
        assert_eq!(a.control_token.as_deref(), Some("abc"));
        assert_eq!(a.control_token_file, Some("/p/token".into()));
        assert_eq!(a.consume("--control-token=again", &mut args).unwrap_err(), "--control-token given twice");
        assert_eq!(a.consume("--control-token-file", &mut args).unwrap_err(), "--control-token-file given twice");
        assert_eq!(args.next().as_deref(), Some("x"));
        assert!(!a.consume("--control-tokens", &mut args).unwrap());
    }

    #[test]
    fn invalid_operands_leave_roots_unchanged() {
        for flag in ["--automation-read-root", "--automation-write-root", "--control-token", "--control-token-file"] {
            for operand in [None, Some(""), Some("--version"), Some("--connect"), Some("-folder")] {
                let mut roots = AutomationArgs::new(None, None);
                let mut args = operand.map(str::to_owned).into_iter();
                assert!(roots.consume(flag, &mut args).is_err(), "{flag} {operand:?}");
                assert_eq!((roots.read_root, roots.write_root), (None, None));
                assert_eq!((roots.control_token, roots.control_token_file), (None, None));
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
