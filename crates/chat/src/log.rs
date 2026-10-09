//! The chat log: one JSON message per line (the app keeps it in the settings folder, see `file_name`).

use std::io::Write;
use std::path::Path;

use crate::hub::Message;

/// Append one message. A new log file is private to the user (0600 on Unix).
pub fn append(path: &Path, m: &Message) -> std::io::Result<()> {
    let mut open = std::fs::OpenOptions::new();
    open.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut f = open.open(path)?;
    let line = serde_json::to_string(m).map_err(std::io::Error::other)?;
    writeln!(f, "{line}")
}

/// Create `dir` (and missing parents) and make it private to the user (0700 on Unix).
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Every message in the log; unreadable lines are skipped.
pub fn load(path: &Path) -> std::io::Result<Vec<Message>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(text.lines().filter_map(|l| serde_json::from_str::<Message>(l).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::{Message, Role};

    #[test]
    fn append_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("wc-chat-log-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("doc.jsonl");
        let _ = std::fs::remove_file(&p);
        let m = Message { seq: 1, ts_ms: 5, from: "OWNER".into(), role: Role::Owner, text: "hello ✅".into(), mentions: vec![] };
        assert!(append(&p, &m).is_ok());
        let back = load(&p).unwrap_or_default();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].text, "hello ✅");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
            assert_eq!(mode, 0o600, "only the user can read the chat");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_dir_is_0700() {
        let dir = std::env::temp_dir().join(format!("wc-chat-log-dir-{}", std::process::id())).join("chats");
        assert!(private_dir(&dir).is_ok());
        assert!(dir.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
            assert_eq!(mode, 0o700);
        }
        if let Some(parent) = dir.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}
