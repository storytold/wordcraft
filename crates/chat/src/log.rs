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

/// A document's log file name: a readable prefix from its file name and a stable id, the
/// FNV-1a hash of its canonical path (`Contract v2.docx` -> `Contract_v2.docx-1f0c..e9.jsonl`).
pub fn file_name(canonical_doc: &Path) -> String {
    let raw = canonical_doc.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // `take` counts chars, so the cut is always at a char boundary.
    let safe: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(48).collect();
    let safe = safe.trim_start_matches('.');
    let prefix = if safe.trim_matches(|c| c == '.' || c == '_').is_empty() { "document" } else { safe };
    let hash = canonical_doc
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    format!("{prefix}-{hash:016x}.jsonl")
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
    fn file_name_is_stable_readable_and_safe() {
        let a = file_name(Path::new("/docs/Contract v2.docx"));
        assert!(a.starts_with("Contract_v2.docx-") && a.ends_with(".jsonl"), "{a}");
        assert_eq!(a.len(), "Contract_v2.docx-".len() + 16 + ".jsonl".len());
        assert_eq!(a, file_name(Path::new("/docs/Contract v2.docx")), "stable");
        assert_ne!(a, file_name(Path::new("/other/Contract v2.docx")), "the folder counts");
        assert!(file_name(Path::new("/x/R\u{e9}sum\u{e9}.docx")).starts_with("R_sum_.docx-"));
        assert!(file_name(Path::new("/")).starts_with("document-"));
        assert!(!file_name(Path::new("/x/.hidden")).starts_with('.'));
        assert!(file_name(Path::new(&format!("/x/{}.docx", "a".repeat(300)))).len() <= 48 + 1 + 16 + 6);
        let wide = format!("/x/{}.docx", "\u{e9}".repeat(100));
        assert!(file_name(Path::new(&wide)).starts_with("document-"), "multi-byte names are cut at a char boundary");
    }

    #[cfg(unix)]
    #[test]
    fn the_log_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wc-chat-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        private_dir(&dir).unwrap();
        let p = dir.join("t.jsonl");
        let m = Message { seq: 1, ts_ms: 5, from: "OWNER".into(), role: crate::Role::Owner, text: "x".into(), mentions: vec![] };
        append(&p, &m).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
