//! The chat log: one JSON message per line, next to the document (`<doc>.chat.jsonl`).

use std::io::Write;
use std::path::Path;

use crate::hub::Message;

/// Append one message.
pub fn append(path: &Path, m: &Message) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
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
        let p = dir.join("doc.docx.chat.jsonl");
        let _ = std::fs::remove_file(&p);
        let m = Message { seq: 1, ts_ms: 5, from: "DONO".into(), role: Role::Owner, text: "olá ✅".into(), mentions: vec![] };
        assert!(append(&p, &m).is_ok());
        let back = load(&p).unwrap_or_default();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].text, "olá ✅");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
