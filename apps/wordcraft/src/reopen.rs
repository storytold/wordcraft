//! The documents to open again after Update and restart. The update helper starts the new copy
//! without them, so the app writes their paths to `reopen-after-update.json` in the settings
//! folder as it quits for the update, and the next start reads that list once and removes it.
//! A list older than [`MAX_AGE_SECS`] is from a start that never came, and is ignored.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FILE: &str = "reopen-after-update.json";
/// How long the new copy has to start: the helper waits up to a minute for the old one to exit,
/// then an installer may run.
const MAX_AGE_SECS: u64 = 15 * 60;
/// At most this many documents come back.
const MAX_PATHS: usize = 64;
/// A longer file isn't one this app wrote.
const MAX_BYTES: u64 = 256 * 1024;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn dir() -> Option<PathBuf> {
    if !crate::prefs_enabled() {
        return None;
    }
    wordcraft_control_key::settings_dir()
}

/// Records `paths` for the next start. Nothing is written when there are none.
pub(crate) fn save(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    if let Some(dir) = dir() {
        save_in(&dir, paths, now());
    }
}

/// The documents the last run left to reopen, once: the list is removed whatever it holds.
pub(crate) fn take() -> Vec<String> {
    dir().map(|dir| take_from(&dir, now())).unwrap_or_default()
}

fn save_in(dir: &Path, paths: &[PathBuf], written: u64) {
    // `file.open` takes its path as text; a name that isn't valid Unicode stays in Recent.
    let paths: Vec<&str> = paths.iter().filter_map(|p| p.to_str()).take(MAX_PATHS).collect();
    if paths.is_empty() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("reopen after update: cannot create {}: {e}", dir.display());
        return;
    }
    let path = dir.join(FILE);
    match serde_json::to_vec_pretty(&json!({"written": written, "paths": paths})) {
        Ok(bytes) => match crate::write_prefs(&path, &bytes) {
            Ok(()) => log::info!("reopen after update: {} document(s) recorded", paths.len()),
            Err(e) => log::warn!("reopen after update: cannot save {}: {e}", path.display()),
        },
        Err(e) => log::warn!("reopen after update: cannot serialize: {e}"),
    }
}

fn take_from(dir: &Path, now: u64) -> Vec<String> {
    let path = dir.join(FILE);
    let Ok(meta) = std::fs::metadata(&path) else { return Vec::new() };
    let bytes = if meta.is_file() && meta.len() <= MAX_BYTES { std::fs::read(&path).ok() } else { None };
    if let Err(e) = std::fs::remove_file(&path)
        && path.exists()
    {
        // A list that can't be removed would reopen the same documents at every start.
        log::warn!("reopen after update: cannot remove {}: {e}", path.display());
        return Vec::new();
    }
    let Some(list) = bytes.and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return Vec::new() };
    let written = list.get("written").and_then(Value::as_u64).unwrap_or(0);
    // From the future (a clock set back) or too old: not this update's.
    if written > now || now.saturating_sub(written) > MAX_AGE_SECS {
        return Vec::new();
    }
    let Some(paths) = list.get("paths").and_then(Value::as_array) else { return Vec::new() };
    paths.iter().filter_map(Value::as_str).filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file())).take(MAX_PATHS).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-reopen-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_saved_documents_come_back_once_in_order() {
        let dir = temp("roundtrip");
        let (a, b) = (dir.join("Bericht 1.docx"), dir.join("O'Brien.txt"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        save_in(&dir, &[a.clone(), b.clone()], 1000);
        let back = take_from(&dir, 1060);
        assert_eq!(back, [a.to_str().unwrap(), b.to_str().unwrap()]);
        assert!(!dir.join(FILE).exists(), "the list is used once");
        assert!(take_from(&dir, 1060).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_list_or_one_from_the_future_is_dropped() {
        let dir = temp("age");
        let a = dir.join("a.docx");
        std::fs::write(&a, b"a").unwrap();
        save_in(&dir, std::slice::from_ref(&a), 1000);
        assert!(take_from(&dir, 1000 + MAX_AGE_SECS + 1).is_empty());
        assert!(!dir.join(FILE).exists(), "a stale list is removed too");
        save_in(&dir, std::slice::from_ref(&a), 5000);
        assert!(take_from(&dir, 4000).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_files_folders_and_junk_are_skipped_without_panicking() {
        let dir = temp("junk");
        let kept = dir.join("kept.docx");
        std::fs::write(&kept, b"k").unwrap();
        save_in(&dir, &[dir.join("gone.docx"), dir.clone(), kept.clone()], 10);
        assert_eq!(take_from(&dir, 10), [kept.to_str().unwrap()]);
        for junk in [&b"not json"[..], b"{}", b"[1,2]", br#"{"written":"x","paths":"y"}"#, br#"{"written":10,"paths":[1,null,{}]}"#] {
            std::fs::write(dir.join(FILE), junk).unwrap();
            assert!(take_from(&dir, 10).is_empty(), "{junk:?}");
            assert!(!dir.join(FILE).exists());
        }
        std::fs::write(dir.join(FILE), vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(take_from(&dir, 10).is_empty());
        assert!(!dir.join(FILE).exists(), "an oversized list is removed unread");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn at_most_max_paths_come_back_and_none_writes_nothing() {
        let dir = temp("cap");
        let file = dir.join("same.docx");
        std::fs::write(&file, b"x").unwrap();
        save_in(&dir, &vec![file.clone(); MAX_PATHS + 10], 10);
        assert_eq!(take_from(&dir, 10).len(), MAX_PATHS);
        save_in(&dir, &[], 10);
        assert!(!dir.join(FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
