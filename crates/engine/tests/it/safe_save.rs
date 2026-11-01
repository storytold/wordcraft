//! Original synthetic save cases. Every test owns its directory and document copies.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::{Session, io};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("wordcraft-safe-save-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(&self.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn original(path: &Path) -> Vec<u8> {
    let bytes = io::save_bytes("original.docx", &Document::from_text("Original synthetic contract.")).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}
fn edited() -> Document {
    Document::from_text("Original synthetic contract. Edited sentinel.")
}
fn saved(path: &Path) {
    assert_eq!(io::open_path(path).unwrap().plain_text(StoryRef::Body), edited().plain_text(StoryRef::Body));
}

#[test]
fn an_existing_temporary_file_is_not_overwritten_or_removed() {
    let d = Scratch::new();
    let path = d.path("contract.docx");
    original(&path);
    let neighbour = d.path("contract.docx.tmp");
    std::fs::write(&neighbour, b"Keep this existing sibling.").unwrap();
    io::save_path(&path, &edited()).unwrap();
    saved(&path);
    assert_eq!(std::fs::read(&neighbour).unwrap(), b"Keep this existing sibling.");
    assert_eq!(d.names(), ["contract.docx", "contract.docx.tmp"]);
}

#[test]
fn an_existing_temporary_directory_does_not_block_a_save() {
    let d = Scratch::new();
    let path = d.path("contract.docx");
    original(&path);
    let neighbour = d.path("contract.docx.tmp");
    std::fs::create_dir(&neighbour).unwrap();
    std::fs::write(neighbour.join("keep"), b"Keep this directory.").unwrap();
    io::save_path(&path, &edited()).unwrap();
    saved(&path);
    assert_eq!(std::fs::read(neighbour.join("keep")).unwrap(), b"Keep this directory.");
}

#[cfg(unix)]
#[test]
fn temporary_links_cannot_redirect_a_save_into_the_original_or_a_neighbour() {
    for link_to_original in [false, true] {
        let d = Scratch::new();
        let path = d.path("contract.docx");
        original(&path);
        let other = d.path("neighbour.txt");
        std::fs::write(&other, b"Keep this neighbour.").unwrap();
        let link = d.path("contract.docx.tmp");
        let target = if link_to_original { &path } else { &other };
        std::os::unix::fs::symlink(target, &link).unwrap();
        io::save_path(&path, &edited()).unwrap();
        assert!(!path.is_symlink());
        saved(&path);
        assert_eq!(std::fs::read_link(&link).unwrap(), *target);
        assert_eq!(std::fs::read(&other).unwrap(), b"Keep this neighbour.");
    }
}

#[test]
fn failed_save_as_keeps_original_dirty_session_and_destination() {
    let d = Scratch::new();
    let path = d.path("original.docx");
    let bytes = original(&path);
    let blocked = d.path("blocked.docx");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("keep"), b"Keep this destination.").unwrap();
    let mut s = Session::new(edited());
    s.path = Some(path.clone());
    s.dirty = true;
    let before = s.doc.plain_text(StoryRef::Body);
    assert!(s.run("file.saveAs", &json!({"path": blocked})).is_err());
    assert!(s.dirty);
    assert_eq!(s.path.as_ref(), Some(&path));
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read(blocked.join("keep")).unwrap(), b"Keep this destination.");
    assert_eq!(d.names(), ["blocked.docx", "original.docx"]);
}

#[test]
fn successful_save_as_keeps_original_and_updates_session_only_after_saving() {
    let d = Scratch::new();
    let path = d.path("original.docx");
    let bytes = original(&path);
    let copy = d.path("copy.docx");
    let mut s = Session::new(edited());
    s.path = Some(path.clone());
    s.dirty = true;
    s.run("file.saveAs", &json!({"path": copy})).unwrap();
    assert!(!s.dirty);
    assert_eq!(s.path.as_ref(), Some(&copy));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    saved(&copy);
    assert_eq!(d.names(), ["copy.docx", "original.docx"]);
}

#[test]
fn missing_parent_keeps_original_and_session_unchanged() {
    let d = Scratch::new();
    let path = d.path("original.docx");
    let bytes = original(&path);
    let mut s = Session::new(edited());
    s.path = Some(path.clone());
    s.dirty = true;
    assert!(s.run("file.saveAs", &json!({"path": d.path("missing/copy.docx")})).is_err());
    assert!(s.dirty);
    assert_eq!(s.path, Some(path.clone()));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(d.names(), ["original.docx"]);
}

#[test]
fn encrypted_save_reopens_only_with_password_and_leaves_neighbours_alone() {
    let d = Scratch::new();
    let path = d.path("contract.docx");
    original(&path);
    let neighbour = d.path("contract.docx.tmp");
    std::fs::write(&neighbour, b"Keep this sibling.").unwrap();
    io::save_path_with(&path, &edited(), Some("synthetic-test-password"), &io::ExportOptions::default()).unwrap();
    assert!(io::needs_password(&io::open_path(&path).unwrap_err()));
    let (doc, encrypted) = io::open_path_with(&path, Some("synthetic-test-password")).unwrap();
    assert!(encrypted);
    assert_eq!(doc.plain_text(StoryRef::Body), edited().plain_text(StoryRef::Body));
    assert_eq!(std::fs::read(&neighbour).unwrap(), b"Keep this sibling.");
    assert_eq!(d.names(), ["contract.docx", "contract.docx.tmp"]);
}

#[cfg(unix)]
#[test]
fn replacement_keeps_existing_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let d = Scratch::new();
    let path = d.path("contract.docx");
    original(&path);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    io::save_path(&path, &edited()).unwrap();
    saved(&path);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}
