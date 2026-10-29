//! End-to-end storage test through the public trait only: write a document's bytes atomically,
//! read them back, exercise conditional writes, listing, removal, and crash recovery.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use wordcraft_storage::local::LocalBackend;
use wordcraft_storage::{StorageBackend, StorageError, WriteOptions};

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn create() -> Scratch {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("wordcraft-storage-it-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Scratch { path }
    }

    fn backend(&self) -> LocalBackend {
        LocalBackend::open(&self.path).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn docx_bytes_survive_an_atomic_save_and_reopen() {
    let scratch = Scratch::create();
    let store = scratch.backend();
    // Minimal ZIP signature stand-in for a .docx package: storage never parses, only carries.
    let bytes = b"PK\x03\x04fake-docx-bytes".to_vec();
    let meta = store.write("reports/memo.docx", &bytes, &WriteOptions { atomic: true, ..Default::default() }).unwrap();
    assert_eq!(store.read("reports/memo.docx").unwrap(), bytes);

    // Reopening through a second handle sees the same version (shared root, ETag sidecar).
    let store2 = scratch.backend();
    let st2 = store2.stat("reports/memo.docx").unwrap();
    assert_eq!((st2.size, st2.etag), (meta.size, meta.etag.clone()));

    // Optimistic concurrency: a writer holding the old tag wins once, then loses.
    let tag = meta.etag.unwrap();
    store2.write("reports/memo.docx", b"PK\x03\x04v2", &WriteOptions { if_match: Some(tag.clone()), atomic: true }).unwrap();
    let stale = store.write("reports/memo.docx", b"PK\x03\x04stale", &WriteOptions { if_match: Some(tag), atomic: true });
    assert!(matches!(stale, Err(StorageError::Conflict(_))));
    assert_eq!(store.read("reports/memo.docx").unwrap(), b"PK\x03\x04v2");
}

#[test]
fn startup_recover_leaves_a_clean_store() {
    let scratch = Scratch::create();
    let store = scratch.backend();
    store.write("a.docx", b"one", &WriteOptions { atomic: true, ..Default::default() }).unwrap();
    // An orphan staging file from a killed save is swept on the next startup.
    fs::write(scratch.path.join(".wordcraft-save-9-a.docx.tmp"), b"half").unwrap();
    let report = store.recover().unwrap();
    assert_eq!((report.completed, report.dropped), (1, 0));
    assert_eq!(report.removed_temp, 1);
    assert_eq!(store.read("a.docx").unwrap(), b"one");
    let names: Vec<String> = store.list("").unwrap().into_iter().map(|e| e.path).collect();
    assert_eq!(names, vec!["a.docx".to_string()]);
}

#[test]
fn paths_cannot_escape_the_root() {
    let scratch = Scratch::create();
    let store = scratch.backend();
    for bad in ["", "/abs.docx", "../up.docx", "a/../../b.docx"] {
        assert!(store.write(bad, b"x", &WriteOptions::default()).is_err(), "{bad}");
        assert!(store.read(bad).is_err(), "{bad}");
    }
}
