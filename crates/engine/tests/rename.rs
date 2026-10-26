//! Original synthetic rename cases. Every test owns its directory and document copies.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::{Session, io};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("wordcraft-rename-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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

const TEXT: &str = "Original synthetic rename case.";

/// A document on disk under `name`, with the session already pointing at it.
fn opened(d: &Scratch, name: &str) -> Session {
    let path = d.path(name);
    std::fs::write(&path, io::save_bytes(name, &Document::from_text(TEXT)).unwrap()).unwrap();
    let mut s = Session::new(Document::from_text(TEXT));
    s.path = Some(path);
    s
}

fn rename(s: &mut Session, name: &str) -> Value {
    s.run("file.rename", &json!({"name": name})).unwrap_or_else(|e| panic!("file.rename {name}: {e}"))
}

#[test]
fn a_name_that_already_ends_in_the_extension_keeps_it() {
    // #2983: an agent or a script passes the whole filename, so the extension is not doubled.
    let d = Scratch::new();
    let mut s = opened(&d, "in.docx");
    let out = rename(&mut s, "renamed.docx");
    assert_eq!(d.names(), ["renamed.docx"], "the file is renamed, not doubled");
    assert_eq!(s.path, Some(d.path("renamed.docx")), "the session follows the file");
    assert!(out["path"].as_str().unwrap().ends_with("renamed.docx"), "{out}");
    assert_eq!(io::open_path(&d.path("renamed.docx")).unwrap().plain_text(StoryRef::Body), TEXT, "the renamed file still opens");
}

#[test]
fn a_stem_still_gains_the_extension() {
    // The File › Info field edits the stem; that path keeps working.
    let d = Scratch::new();
    let mut s = opened(&d, "in.docx");
    rename(&mut s, "plain");
    assert_eq!(d.names(), ["plain.docx"]);
    assert_eq!(s.path, Some(d.path("plain.docx")));
}

#[test]
fn the_extension_is_recognised_in_any_case() {
    let d = Scratch::new();
    let mut s = opened(&d, "in.docx");
    rename(&mut s, "Renamed.DOCX");
    assert_eq!(d.names(), ["Renamed.DOCX"]);
    assert_eq!(s.path, Some(d.path("Renamed.DOCX")));
}

#[test]
fn a_template_keeps_its_own_format() {
    // The format is the file's, not the name's: a template stays one, and a name carrying the
    // template's extension is not doubled either.
    let d = Scratch::new();
    let mut s = opened(&d, "in.dotx");
    rename(&mut s, "template");
    assert_eq!(d.names(), ["template.dotx"]);

    let d = Scratch::new();
    let mut s = opened(&d, "in.dotx");
    rename(&mut s, "template.dotx");
    assert_eq!(d.names(), ["template.dotx"]);
}

#[test]
fn a_stem_that_ends_in_the_extension_letters_still_gains_the_extension() {
    // `Path::extension()` carries no leading dot, so a stem like `mydocx` must not read as a name
    // that already ends in `.docx`.
    let d = Scratch::new();
    let mut s = opened(&d, "in.docx");
    rename(&mut s, "mydocx");
    assert_eq!(d.names(), ["mydocx.docx"]);
    assert_eq!(s.path, Some(d.path("mydocx.docx")));
}

#[test]
fn a_name_with_another_extension_keeps_the_file_format() {
    // `notes.txt` does not carry the document's extension, so the extension is still appended:
    // the file stays a Word document whatever its name says.
    let d = Scratch::new();
    let mut s = opened(&d, "in.docx");
    rename(&mut s, "notes.txt");
    assert_eq!(d.names(), ["notes.txt.docx"]);
}

#[test]
fn an_unsaved_document_takes_the_name_as_given() {
    // No file yet: nothing is written, and no extension is invented.
    let mut s = Session::new(Document::from_text(TEXT));
    rename(&mut s, "draft.docx");
    assert_eq!(s.path, None);
    assert_eq!(s.file_name.as_deref(), Some("draft.docx"));
}
