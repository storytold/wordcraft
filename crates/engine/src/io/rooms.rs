//! A live session's recovery files (plugin protocol 2): the room's whole state, kept in the
//! app's recovery folder so a crash and rejoin merges instead of losing offline edits. The
//! folder is the host's (the desktop app sets it); commands never pass paths here.

use std::path::Path;

/// Writes `body` as `name` in `dir`, making the folder when needed.
pub fn write(dir: &Path, name: &str, body: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(name), body)).map_err(|e| e.to_string())
}

/// Reads `name` in `dir` (at most 64 MiB), if it's there.
pub fn read(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    let len = std::fs::metadata(&path).ok()?.len();
    if len > 64 << 20 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Removes `name` in `dir` (nothing when it isn't there).
pub fn remove(dir: &Path, name: &str) {
    let _ = std::fs::remove_file(dir.join(name));
}
