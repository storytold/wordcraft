//! A control key given at launch (#5047), with PhotoCraft's flags and rules: `--control-token`
//! (64 hex digits) or `--control-token-file` (a private file holding them). The app's server
//! takes it through [`server_token`], `wordcraft-cli mcp --connect` through [`client_token`].

use std::io::{Read, Write};
use std::path::Path;

/// A control key's length: 32 bytes as hex.
pub const TOKEN_HEX_LEN: usize = 64;

/// A token file is read up to this many bytes; a longer one is not a token.
const MAX_TOKEN_FILE: u64 = 4096;

/// A token given at launch must be exactly 64 hexadecimal digits (either case), as the app
/// generates them.
pub fn validate_token(token: &str) -> Result<(), String> {
    if token.len() != TOKEN_HEX_LEN || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("a control token must be exactly {TOKEN_HEX_LEN} hexadecimal digits"));
    }
    Ok(())
}

/// `token` checked and lowercased, as the app keeps it.
fn checked(token: &str) -> Result<String, String> {
    validate_token(token)?;
    Ok(token.to_ascii_lowercase())
}

/// Read a control-token file: a regular file (links followed) that, on Unix, no other user can
/// read (no group or other permission bits), holding the token with optional surrounding
/// whitespace. Returns the token lowercased.
pub fn read_token_file(path: &Path) -> Result<String, String> {
    let shown = path.display();
    let file = std::fs::File::open(path).map_err(|e| format!("control token file {shown}: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("control token file {shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("control token file {shown}: not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "control token file {shown}: readable by group or others (mode {:o}); restrict it to its owner (chmod 600)",
                mode & 0o777
            ));
        }
    }
    let mut text = String::new();
    file.take(MAX_TOKEN_FILE).read_to_string(&mut text).map_err(|e| format!("control token file {shown}: {e}"))?;
    checked(text.trim()).map_err(|e| format!("control token file {shown}: {e}"))
}

/// Create a token file holding `token` (mode 0600 on Unix), never replacing an existing file.
fn create_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    writeln!(open.open(path)?, "{token}")
}

/// The key the app's control server uses: the given `token`, or the one in `file` (created with
/// a `fresh` token when nothing has that name, then reused by later launches), or else a `fresh`
/// one. A token and a file together are an error, as are a bad token and an unusable file.
pub fn server_token(token: Option<&str>, file: Option<&Path>, fresh: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    match (token, file) {
        (Some(_), Some(_)) => Err(NOT_BOTH.into()),
        (Some(token), None) => checked(token),
        (None, None) => fresh(),
        (None, Some(path)) => {
            // Only a name that is not there at all is created: a link that leads nowhere is an
            // error, not a way to create its target.
            if path.symlink_metadata().is_ok() {
                return read_token_file(path);
            }
            let token = checked(&fresh()?)?;
            match create_token_file(path, &token) {
                Ok(()) => Ok(token),
                // Another launch created it first: use that one.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_token_file(path),
                Err(e) => Err(format!("control token file {}: {e}", path.display())),
            }
        }
    }
}

/// The key a client of the control server (`wordcraft-cli mcp --connect`) sends when one was
/// given: the `token`, or the one in `file`, which must exist (a client never makes one up).
/// `None` when neither was given. A token and a file together are an error.
pub fn client_token(token: Option<&str>, file: Option<&Path>) -> Result<Option<String>, String> {
    match (token, file) {
        (Some(_), Some(_)) => Err(NOT_BOTH.into()),
        (Some(token), None) => checked(token).map(Some),
        (None, Some(path)) => read_token_file(path).map(Some),
        (None, None) => Ok(None),
    }
}

const NOT_BOTH: &str = "use either a control token or a control token file, not both";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-control-token-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn chmod(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Written private, as a person following the docs would.
    fn private(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        #[cfg(unix)]
        chmod(path, 0o600);
    }

    #[test]
    fn a_token_is_64_hex_digits() {
        assert!(validate_token(TOKEN).is_ok());
        assert!(validate_token(&TOKEN.to_uppercase()).is_ok());
        for bad in ["", "abcd", &TOKEN[..63], &format!("{TOKEN}0"), &TOKEN.replace('a', "g"), &format!(" {}", &TOKEN[1..])] {
            assert!(validate_token(bad).is_err(), "{bad:?}");
        }
        // Multi-byte text of the right byte length is not hex either.
        assert!(validate_token(&"é".repeat(32)).is_err());
    }

    #[test]
    fn a_given_token_is_used_lowercased() {
        assert_eq!(server_token(Some(&TOKEN.to_uppercase()), None, || Err("generated".into())).unwrap(), TOKEN);
        assert!(server_token(Some("abcd"), None, || Ok(TOKEN.into())).is_err());
        assert_eq!(server_token(None, None, || Ok("generated".into())).unwrap(), "generated");
        assert_eq!(client_token(Some(&TOKEN.to_uppercase()), None).unwrap().as_deref(), Some(TOKEN));
        assert_eq!(client_token(None, None).unwrap(), None);
        assert!(client_token(Some("abcd"), None).is_err());
    }

    #[test]
    fn a_token_and_a_token_file_together_are_an_error() {
        let file = Path::new("/nonexistent/token");
        assert!(server_token(Some(TOKEN), Some(file), || Ok(TOKEN.into())).unwrap_err().contains("not both"));
        assert!(client_token(Some(TOKEN), Some(file)).unwrap_err().contains("not both"));
    }

    #[test]
    fn a_missing_token_file_is_created_by_the_server_and_reused() {
        let dir = temp("create");
        let path = dir.join("sub/control.token");
        // A client never creates one.
        assert!(client_token(None, Some(&path)).is_err());
        assert!(!path.exists());
        assert_eq!(server_token(None, Some(&path), || Ok(TOKEN.to_uppercase())).unwrap(), TOKEN);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), TOKEN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // The next launch reads it rather than making a new one; the client reads the same.
        assert_eq!(server_token(None, Some(&path), || Ok("f".repeat(64))).unwrap(), TOKEN);
        assert_eq!(client_token(None, Some(&path)).unwrap().as_deref(), Some(TOKEN));
        // A generator that fails leaves no file behind.
        let other = dir.join("other.token");
        assert!(server_token(None, Some(&other), || Err("no randomness".into())).is_err());
        assert!(!other.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unusable_token_file_is_an_error() {
        let dir = temp("bad");
        let junk = dir.join("junk");
        private(&junk, "not a token\n");
        assert!(server_token(None, Some(&junk), || Ok(TOKEN.into())).unwrap_err().contains("hexadecimal"));
        // A huge file is read only up to the cap, and is not a token.
        let huge = dir.join("huge");
        private(&huge, &"a".repeat(1 << 20));
        assert!(read_token_file(&huge).is_err());
        // Surrounding whitespace is fine (`echo … > file`).
        let padded = dir.join("padded");
        private(&padded, &format!("  {}\r\n", TOKEN.to_uppercase()));
        assert_eq!(read_token_file(&padded).unwrap(), TOKEN);
        assert!(server_token(None, Some(&dir), || Ok(TOKEN.into())).unwrap_err().contains("not a regular file"));
        #[cfg(unix)]
        {
            let open = dir.join("open");
            std::fs::write(&open, TOKEN).unwrap();
            chmod(&open, 0o644);
            let e = server_token(None, Some(&open), || Ok(TOKEN.into())).unwrap_err();
            assert!(e.contains("readable by group or others"), "{e}");
            assert!(client_token(None, Some(&open)).is_err());
            chmod(&open, 0o600);
            assert_eq!(client_token(None, Some(&open)).unwrap().as_deref(), Some(TOKEN));
            // A link that leads nowhere is not followed to create its target.
            let dangling = dir.join("dangling");
            std::os::unix::fs::symlink(dir.join("nowhere"), &dangling).unwrap();
            assert!(server_token(None, Some(&dangling), || Ok(TOKEN.into())).is_err());
            assert!(!dir.join("nowhere").exists());
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
