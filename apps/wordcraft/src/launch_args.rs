//! The control channel's launch options (#5047), named and read as PhotoCraft's, so one setup
//! works across the Craft apps:
//!
//! - `--control-token HEX` (`WORDCRAFT_CONTROL_TOKEN`): the key, 64 hex digits, instead of a new
//!   random one each start.
//! - `--control-token-file PATH` (`WORDCRAFT_CONTROL_TOKEN_FILE`): a private file holding the key;
//!   made with a new key when nothing has that name, and reused by later starts.
//! - `--automation-read-root DIR` / `--automation-write-root DIR`
//!   (`WORDCRAFT_AUTOMATION_READ_ROOT` / `WORDCRAFT_AUTOMATION_WRITE_ROOT`): the folders the
//!   control channel's requests may read and write (`wordcraft_engine::file_access`).
//!
//! A flag wins over its variable, and an empty or blank variable counts as unset. Each flag is
//! given once, as `--flag VALUE` or `--flag=VALUE` (a value starting with `-` needs the `=`).
//! The values are used only with the control server on (`--control`): then a token together with
//! a token file, from flags or variables, is an error, and so is a bad token, an unusable token
//! file or a root that isn't a folder. Without it they are ignored, so a stale exported variable
//! doesn't stop a launch that needs none.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use wordcraft_engine::file_access::AutomationRoots;

/// Each option's flag and its variable.
const TOKEN: (&str, &str) = ("--control-token", "WORDCRAFT_CONTROL_TOKEN");
const TOKEN_FILE: (&str, &str) = ("--control-token-file", "WORDCRAFT_CONTROL_TOKEN_FILE");
const READ_ROOT: (&str, &str) = ("--automation-read-root", "WORDCRAFT_AUTOMATION_READ_ROOT");
const WRITE_ROOT: (&str, &str) = ("--automation-write-root", "WORDCRAFT_AUTOMATION_WRITE_ROOT");

/// The flags that take a value (with `--control`): the value after one must be text.
pub const VALUE_FLAGS: [&str; 5] = ["--control", TOKEN.0, TOKEN_FILE.0, READ_ROOT.0, WRITE_ROOT.0];

/// A value, and the flag or variable it came from (for messages).
#[derive(Clone, Debug, PartialEq)]
pub struct Given<T> {
    pub value: T,
    pub from: &'static str,
}

/// The control token and automation roots of a launch.
#[derive(Debug, Default, PartialEq)]
pub struct ControlArgs {
    pub token: Option<Given<String>>,
    pub token_file: Option<Given<PathBuf>>,
    pub read_root: Option<Given<PathBuf>>,
    pub write_root: Option<Given<PathBuf>>,
}

/// The control server's key and the roots its requests are confined to.
pub struct ControlSetup {
    pub key: String,
    pub roots: Option<Arc<AutomationRoots>>,
    /// Where the key came from, for the log: the flag or variable, or `None` for a new one.
    pub key_from: Option<&'static str>,
}

impl ControlArgs {
    /// Take `arg` when it is one of the options, with its value (inline after `=`, or the next
    /// argument). `Ok(false)` for any other argument. A missing or empty value, or an option given
    /// twice, is an error naming it.
    pub fn consume(&mut self, arg: &str, args: &mut std::iter::Peekable<impl Iterator<Item = String>>) -> Result<bool, String> {
        let (flag, inline) = arg.split_once('=').map_or((arg, None), |(flag, value)| (flag, Some(value)));
        let known = [TOKEN.0, TOKEN_FILE.0, READ_ROOT.0, WRITE_ROOT.0];
        let Some(&flag) = known.iter().find(|f| **f == flag) else { return Ok(false) };
        let value = match inline {
            Some(v) => v.to_owned(),
            None => match args.next_if(|v| !v.starts_with('-')) {
                Some(v) => v,
                None if args.peek().is_some() => return Err(format!("{flag} needs a value; write {flag}=<value> for one starting with '-'")),
                None => return Err(format!("{flag} needs a value")),
            },
        };
        if value.trim().is_empty() {
            return Err(format!("{flag} needs a non-empty value"));
        }
        let twice = || format!("{flag} given twice");
        match flag {
            f if f == TOKEN.0 => set(&mut self.token, value, f).ok_or_else(twice)?,
            f if f == TOKEN_FILE.0 => set(&mut self.token_file, value.into(), f).ok_or_else(twice)?,
            f if f == READ_ROOT.0 => set(&mut self.read_root, value.into(), f).ok_or_else(twice)?,
            f => set(&mut self.write_root, value.into(), f).ok_or_else(twice)?,
        }
        Ok(true)
    }

    /// Fill the options not given as flags from their variables (`var` reads one). An empty or
    /// blank variable counts as unset; a token that isn't text is an error naming the variable.
    pub fn or_env(mut self, var: impl Fn(&str) -> Option<OsString>) -> Result<Self, String> {
        let read = |name: &'static str| var(name).filter(|v| !v.to_str().is_some_and(|t| t.trim().is_empty()));
        if self.token.is_none()
            && let Some(v) = read(TOKEN.1)
        {
            let value = v.into_string().map_err(|_| format!("{} is not valid Unicode", TOKEN.1))?;
            self.token = Some(Given { value, from: TOKEN.1 });
        }
        for (slot, (_, name)) in [(&mut self.token_file, TOKEN_FILE), (&mut self.read_root, READ_ROOT), (&mut self.write_root, WRITE_ROOT)] {
            if slot.is_none()
                && let Some(v) = read(name)
            {
                *slot = Some(Given { value: v.into(), from: name });
            }
        }
        Ok(self)
    }

    /// Whether any option is set (they do nothing without the control server).
    pub fn any(&self) -> bool {
        self.token.is_some() || self.token_file.is_some() || self.read_root.is_some() || self.write_root.is_some()
    }

    /// For the control server: its key (the token, the token file's, or `fresh`) and the roots.
    pub fn resolve(&self, fresh: impl FnOnce() -> Result<String, String>) -> Result<ControlSetup, String> {
        if let (Some(token), Some(file)) = (&self.token, &self.token_file) {
            return Err(format!("{} cannot be used with {}", token.from, file.from));
        }
        let token = self.token.as_ref().map(|t| t.value.as_str());
        let file = self.token_file.as_ref().map(|f| f.value.as_path());
        let key_from = self.token.as_ref().map(|t| t.from).or(self.token_file.as_ref().map(|f| f.from));
        let key = wordcraft_control_key::server_token(token, file, fresh).map_err(|e| match key_from {
            Some(from) => format!("{from}: {e}"),
            None => format!("control key: {e}"),
        })?;
        let roots = AutomationRoots::new(self.read_root.as_ref().map(|r| r.value.as_path()), self.write_root.as_ref().map(|r| r.value.as_path()))?;
        Ok(ControlSetup { key, roots, key_from })
    }
}

/// Fill an unset `slot`; `None` when it was set already.
fn set<T>(slot: &mut Option<Given<T>>, value: T, from: &'static str) -> Option<()> {
    if slot.is_some() {
        return None;
    }
    *slot = Some(Given { value, from });
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// The options taken from `args`, and the arguments left.
    fn parse(args: &[&str]) -> Result<(ControlArgs, Vec<String>), String> {
        let mut parsed = ControlArgs::default();
        let mut left = Vec::new();
        let mut it = args.iter().map(|a| a.to_string()).peekable();
        while let Some(a) = it.next() {
            if !parsed.consume(&a, &mut it)? {
                left.push(a);
            }
        }
        Ok((parsed, left))
    }

    fn given<T>(value: T, from: &'static str) -> Option<Given<T>> {
        Some(Given { value, from })
    }

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    #[test]
    fn the_four_flags_are_taken_in_either_spelling() {
        let (a, left) = parse(&[
            "--sample",
            "--control-token",
            KEY,
            "--control-token-file=/p/token",
            "--automation-read-root",
            "/r",
            "--automation-write-root=-w",
            "a.docx",
        ])
        .unwrap();
        assert_eq!(left, ["--sample", "a.docx"]);
        assert_eq!(a.token, given(KEY.to_string(), "--control-token"));
        assert_eq!(a.token_file, given("/p/token".into(), "--control-token-file"));
        assert_eq!(a.read_root, given("/r".into(), "--automation-read-root"));
        assert_eq!(a.write_root, given("-w".into(), "--automation-write-root"));
        // Another flag, or nothing, is not a value.
        assert!(parse(&[]).unwrap().0 == ControlArgs::default());
    }

    #[test]
    fn a_flag_without_its_value_or_given_twice_is_an_error() {
        for flag in ["--control-token", "--control-token-file", "--automation-read-root", "--automation-write-root"] {
            assert_eq!(parse(&[flag]).unwrap_err(), format!("{flag} needs a value"));
            assert!(parse(&[flag, "--sample"]).unwrap_err().contains(&format!("write {flag}=<value>")), "{flag}");
            assert_eq!(parse(&[&format!("{flag}=")]).unwrap_err(), format!("{flag} needs a non-empty value"));
            assert_eq!(parse(&[flag, "a", &format!("{flag}=b")]).unwrap_err(), format!("{flag} given twice"));
        }
        // A look-alike is not one of them.
        assert_eq!(parse(&["--control-tokens", "x"]).unwrap().1, ["--control-tokens", "x"]);
    }

    #[test]
    fn a_flag_wins_over_its_variable_and_a_blank_variable_is_unset() {
        let env = |name: &str| -> Option<OsString> {
            match name {
                "WORDCRAFT_CONTROL_TOKEN" => Some(KEY.into()),
                "WORDCRAFT_CONTROL_TOKEN_FILE" => Some(" ".into()),
                "WORDCRAFT_AUTOMATION_READ_ROOT" => Some("/env-read".into()),
                "WORDCRAFT_AUTOMATION_WRITE_ROOT" => Some("/env-write".into()),
                _ => None,
            }
        };
        let (a, _) = parse(&["--automation-write-root", "/flag-write"]).unwrap();
        let a = a.or_env(env).unwrap();
        assert_eq!(a.token, given(KEY.to_string(), "WORDCRAFT_CONTROL_TOKEN"));
        assert_eq!(a.token_file, None);
        assert_eq!(a.read_root, given("/env-read".into(), "WORDCRAFT_AUTOMATION_READ_ROOT"));
        assert_eq!(a.write_root, given("/flag-write".into(), "--automation-write-root"));
        // A flag for one of the pair doesn't hide the other's variable.
        let (a, _) = parse(&["--control-token-file", "/t"]).unwrap();
        let a = a.or_env(env).unwrap();
        assert_eq!(a.resolve(|| Ok(KEY.into())).err().unwrap(), "WORDCRAFT_CONTROL_TOKEN cannot be used with --control-token-file");
    }

    #[test]
    fn the_key_is_the_token_given_or_a_new_one() {
        let (a, _) = parse(&["--control-token", &KEY.to_uppercase()]).unwrap();
        let setup = a.or_env(no_env).unwrap().resolve(|| Err("not generated".into())).unwrap();
        assert_eq!((setup.key.as_str(), setup.key_from), (KEY, Some("--control-token")));
        assert!(setup.roots.is_none());
        let setup = ControlArgs::default().resolve(|| Ok("fresh".into())).unwrap();
        assert_eq!((setup.key.as_str(), setup.key_from), ("fresh", None));
        // A bad token names where it came from.
        let (a, _) = parse(&["--control-token", "abcd"]).unwrap();
        let e = a.resolve(|| Ok(KEY.into())).err().unwrap();
        assert!(e.starts_with("--control-token: ") && e.contains("hexadecimal"), "{e}");
        // Both: an error naming both.
        let (a, _) = parse(&["--control-token", KEY, "--control-token-file", "/t"]).unwrap();
        assert_eq!(a.resolve(|| Ok(KEY.into())).err().unwrap(), "--control-token cannot be used with --control-token-file");
    }

    #[test]
    fn the_token_file_is_made_then_reused_and_the_roots_opened() {
        let dir = std::env::temp_dir().join(format!("wordcraft-launch-args-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let file = dir.join("control.token");
        let work = dir.join("work").to_string_lossy().into_owned();
        let flag = format!("--control-token-file={}", file.display());
        let (a, _) = parse(&[&flag, "--automation-read-root", &work, "--automation-write-root", &work]).unwrap();
        let setup = a.resolve(|| Ok(KEY.into())).unwrap();
        assert_eq!((setup.key.as_str(), setup.key_from), (KEY, Some("--control-token-file")));
        let roots = setup.roots.unwrap();
        assert!(roots.read_root().is_some() && roots.write_root().is_some());
        // The next start reads the same key.
        assert_eq!(a.resolve(|| Ok("f".repeat(64))).unwrap().key, KEY);
        // A root that isn't a folder stops the launch.
        let (a, _) = parse(&["--automation-read-root", "/definitely/not/a/folder"]).unwrap();
        assert!(a.resolve(|| Ok(KEY.into())).err().unwrap().contains("automation read root"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_token_variable_that_isnt_text_is_an_error() {
        use std::os::unix::ffi::OsStringExt;
        let env = |name: &str| (name == "WORDCRAFT_CONTROL_TOKEN").then(|| OsString::from_vec(b"caf\xe9".to_vec()));
        assert_eq!(ControlArgs::default().or_env(env).unwrap_err(), "WORDCRAFT_CONTROL_TOKEN is not valid Unicode");
        // A root path need not be.
        let env = |name: &str| (name == "WORDCRAFT_AUTOMATION_READ_ROOT").then(|| OsString::from_vec(b"/caf\xe9".to_vec()));
        assert!(ControlArgs::default().or_env(env).unwrap().read_root.is_some());
    }
}
