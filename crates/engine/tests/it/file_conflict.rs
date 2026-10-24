//! Synthetic external-writer cases; originals are frozen and only working copies are saved.
#![cfg(not(target_arch = "wasm32"))]
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::{Session, io};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("wordcraft-conflict-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn open(path: &Path) -> Session {
    let mut s = Session::new(Document::new());
    s.run("file.open", &json!({"path": path})).unwrap();
    s.run("document.setText", &json!({"text": "Local edit"})).unwrap();
    s
}
fn original(d: &Scratch) -> PathBuf {
    let p = d.path("working.docx");
    io::save_path(&p, &Document::from_text("Original A")).unwrap();
    p
}

#[test]
fn rename_retains_conflict_protection_and_the_original_content_baseline() {
    for mode in ["unchanged", "replaced", "deleted"] {
        let d = Scratch::new();
        let path = original(&d);
        let frozen = std::fs::read(&path).unwrap();
        let mut s = open(&path);
        s.run("file.rename", &json!({"name": "renamed"})).unwrap();
        let renamed = d.path("renamed.docx");
        assert!(!path.exists());
        assert_eq!(std::fs::read(&renamed).unwrap(), frozen);
        let external = if mode == "replaced" {
            let replacement = d.path("external.docx");
            io::save_path(&replacement, &Document::from_text("External version")).unwrap();
            std::fs::remove_file(&renamed).unwrap();
            std::fs::rename(replacement, &renamed).unwrap();
            Some(std::fs::read(&renamed).unwrap())
        } else {
            if mode == "deleted" {
                std::fs::remove_file(&renamed).unwrap();
            }
            None
        };
        let core = s.doc.core.clone();
        let result = s.run("file.save", &json!({}));
        if mode == "unchanged" {
            result.unwrap();
            assert_eq!(io::open_path(&renamed).unwrap().plain_text(StoryRef::Body), "Local edit");
        } else {
            assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))));
            assert_eq!(s.path.as_deref(), Some(renamed.as_path()));
            assert!(s.dirty);
            assert_eq!(s.doc.core, core);
            assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
            assert_eq!(std::fs::read(&renamed).ok(), external);
            let copy = d.path("copy.docx");
            s.run("file.saveAs", &json!({"path": copy})).unwrap();
            assert_eq!(io::open_path(&copy).unwrap().plain_text(StoryRef::Body), "Local edit");
            assert_eq!(std::fs::read(&renamed).ok(), external);
        }
        assert!(!path.exists(), "saving a renamed document never recreates its old path");
    }
}

#[test]
fn failed_and_noop_renames_keep_the_existing_version() {
    for name in ["occupied", "working"] {
        let d = Scratch::new();
        let path = original(&d);
        let occupied = d.path("occupied.docx");
        io::save_path(&occupied, &Document::from_text("Another document")).unwrap();
        let frozen = std::fs::read(&occupied).unwrap();
        let mut s = open(&path);
        let result = s.run("file.rename", &json!({"name": name}));
        if name == "occupied" {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        assert_eq!(s.path.as_deref(), Some(path.as_path()));
        io::save_path(&path, &Document::from_text("External version")).unwrap();
        let external = std::fs::read(&path).unwrap();
        assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
        assert!(s.dirty);
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
        assert_eq!(std::fs::read(&path).unwrap(), external);
        assert_eq!(std::fs::read(&occupied).unwrap(), frozen);
    }
}

#[test]
fn refreshed_date_fields_keep_the_original_native_file_version() {
    let d = Scratch::new();
    let path = d.path("dated.docx");
    let mut doc = Document::new();
    let para = doc.para_paths(StoryRef::Body).remove(0);
    doc.para_mut(StoryRef::Body, &para)
        .unwrap()
        .insert_object(
            0,
            wordcraft_doc::InlineObject::Field {
                instr: r#"DATE \@ "yyyy-MM-dd""#.into(),
                result: "2020-01-01".into(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &wordcraft_doc::CharProps::default(),
        )
        .unwrap();
    io::save_path(&path, &doc).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut s = Session::new(Document::new());
    s.open_path(&path, None).unwrap();
    let para = s.doc.para_paths(StoryRef::Body).remove(0);
    let wordcraft_doc::InlineObject::Field { result, .. } = &s.doc.para(StoryRef::Body, &para).unwrap().objects[0] else {
        panic!("the date field survives native open");
    };
    assert_eq!(result, &wordcraft_engine::cmd::insert::format_date("yyyy-MM-dd"));
    assert_eq!(std::fs::read(&path).unwrap(), original, "refreshing a field does not rewrite its source");
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": " Local edit"})).unwrap();
    s.run("file.save", &json!({})).unwrap();
    s.run("document.setText", &json!({"text": "Later local edit"})).unwrap();
    io::save_path(&path, &Document::from_text("External version")).unwrap();
    let external = std::fs::read(&path).unwrap();
    assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert!(s.dirty);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Later local edit");
    assert_eq!(std::fs::read(path).unwrap(), external);
}

#[test]
fn word2003_save_keeps_format_and_refuses_external_changes() {
    let d = Scratch::new();
    let path = d.path("working.xml");
    let opts = io::ExportOptions { word2003: true, ..Default::default() };
    let bytes = io::save_bytes_with("working.xml", &Document::from_text("Original XML"), None, &opts).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let mut s = open(&path);
    s.run("file.save", &json!({})).unwrap();
    let saved = std::fs::read(&path).unwrap();
    assert_eq!(wordcraft_docx::sniff_xml(&saved), wordcraft_docx::XmlKind::WordMl2003);
    assert_eq!(io::open_path(&path).unwrap().plain_text(StoryRef::Body), "Local edit");
    s.run("document.setText", &json!({"text": "Second local edit"})).unwrap();
    let external = io::save_bytes_with("working.xml", &Document::from_text("External XML"), None, &opts).unwrap();
    std::fs::write(&path, &external).unwrap();
    let core = s.doc.core.clone();
    let err = s.run("file.save", &json!({"format": "wordml2003"})).unwrap_err();
    assert!(err.to_string().contains("file"), "{err}");
    assert!(s.dirty);
    assert_eq!(s.doc.core, core);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Second local edit");
    assert_eq!(std::fs::read(&path).unwrap(), external);
    let copy = d.path("copy.xml");
    s.run("file.saveAs", &json!({"path": copy, "format": "wordml2003"})).unwrap();
    assert_eq!(wordcraft_docx::sniff_xml(&std::fs::read(&copy).unwrap()), wordcraft_docx::XmlKind::WordMl2003);
    assert_eq!(io::open_path(&copy).unwrap().plain_text(StoryRef::Body), "Second local edit");
    assert_eq!(std::fs::read(path).unwrap(), external);
}

#[test]
fn empty_native_file_keeps_its_version_until_the_first_save() {
    for changed in [false, true] {
        let d = Scratch::new();
        let path = d.path("new.docx");
        std::fs::write(&path, []).unwrap();
        let mut s = open(&path);
        let external = if changed { b"Independent external writer".as_slice() } else { &[] };
        if changed {
            std::fs::write(&path, external).unwrap();
        }
        let result = s.run("file.save", &json!({}));
        if changed {
            assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))));
            assert!(s.dirty);
            assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
            assert_eq!(std::fs::read(&path).unwrap(), external);
        } else {
            result.unwrap();
            assert_eq!(io::open_path(&path).unwrap().plain_text(StoryRef::Body), "Local edit");
        }
    }
}

fn assert_native_path_keeps_region_and_conflict_protection(path: &Path) {
    std::fs::write(path, "Synthetic note").unwrap();
    let mut s = Session::new(Document::new());
    s.system_region = wordcraft_engine::Region::from_tag("en-US");
    s.open_path(path, None).unwrap();
    assert_eq!(s.path.as_deref(), Some(path));
    let sections = s.doc.sections();
    let page = sections[0].1;
    assert_eq!((page.page_w, page.page_h), (612.0, 792.0));
    s.run("document.setText", &json!({"text": "Local edit"})).unwrap();
    std::fs::write(path, "External note").unwrap();
    assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "External note");
    assert!(s.dirty);
}

#[test]
fn native_unicode_path_retains_region_and_conflict_protection() {
    let d = Scratch::new();
    assert_native_path_keeps_region_and_conflict_protection(&d.path("café.txt"));
}

// Match the CLI's existing platform guard: APFS cannot create invalid-UTF-8 fixtures.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn native_non_unicode_path_retains_region_and_conflict_protection() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let d = Scratch::new();
    assert_native_path_keeps_region_and_conflict_protection(&d.0.join(OsString::from_vec(b"caf\xe9.txt".to_vec())));
}

#[test]
fn same_path_text_saves_advance_our_bytes_but_a_copy_keeps_the_original_identity() {
    let d = Scratch::new();
    let path = d.path("working.txt");
    std::fs::write(&path, "Original text").unwrap();
    let mut s = open(&path);
    for _ in 0..2 {
        s.run("text.insert", &json!({"text": " Continued"})).unwrap();
        s.run("file.save", &json!({})).unwrap();
        assert_eq!(s.path.as_ref(), Some(&path));
        assert!(!s.dirty, "saving the open text file completes the save");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), s.doc.plain_text(StoryRef::Body));
    }
    std::fs::write(&path, "External text").unwrap();
    assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "External text");
    let original = original(&d);
    let mut s = open(&original);
    s.run("file.saveAs", &json!({"path": d.path("copy.txt")})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&original));
    assert!(s.dirty);
    let external = io::save_bytes("external.docx", &Document::from_text("External document")).unwrap();
    std::fs::write(&original, &external).unwrap();
    assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert_eq!(std::fs::read(&original).unwrap(), external);
}

#[test]
fn versioned_open_keeps_the_automation_read_boundary() {
    use wordcraft_engine::file_access::{AutomationRoots, confine};
    let d = Scratch::new();
    let path = original(&d);
    let input = d.path("input");
    std::fs::create_dir(&input).unwrap();
    let allowed = input.join("allowed.docx");
    std::fs::copy(&path, &allowed).unwrap();
    let roots = AutomationRoots::new(Some(&input), None).unwrap().unwrap();
    let mut s = Session::new(Document::from_text("Unsaved draft"));
    confine(Some(&roots), || {
        assert!(s.prepare_native_open(&path).is_err_and(|e| e.contains("outside the read root")));
        let result = s.run("file.open", &json!({"path": path}));
        assert!(result.is_err_and(|e| e.to_string().contains("outside the read root")));
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Unsaved draft");
        assert!(s.path.is_none());
        s.run("file.open", &json!({"path": allowed})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Original A");
    });
}

#[test]
fn versioned_save_keeps_the_automation_write_boundary() {
    use wordcraft_engine::file_access::{AutomationRoots, confine};
    let d = Scratch::new();
    let path = original(&d);
    let before = std::fs::read(&path).unwrap();
    let mut s = open(&path);
    let core = s.doc.core.clone();
    let output = d.path("output");
    std::fs::create_dir(&output).unwrap();
    let copy = output.join("copy.docx");
    let roots = AutomationRoots::new(None, Some(&output)).unwrap().unwrap();
    confine(Some(&roots), || {
        let result = s.run("file.save", &json!({}));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(result.is_err_and(|e| e.to_string().contains("outside the write root")));
        assert!(s.dirty);
        assert_eq!(s.path.as_ref(), Some(&path));
        assert_eq!(s.doc.core, core);
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
        s.run("file.saveAs", &json!({"path": copy})).unwrap();
        assert!(!s.dirty);
        assert_eq!(s.path.as_ref(), Some(&copy));
    });
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(io::open_path(&copy).unwrap().plain_text(StoryRef::Body), "Local edit");
}

fn conflict(s: &mut Session, path: &Path, expected: Option<&[u8]>) {
    let core = s.doc.core.clone();
    let password = s.password.clone();
    let err = s.run("file.save", &json!({})).unwrap_err();
    assert!(err.to_string().contains("file"), "{err}");
    assert!(s.dirty);
    assert_eq!(s.path.as_deref(), Some(path));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
    assert_eq!(s.doc.core, core);
    assert_eq!(s.password, password);
    match expected {
        Some(bytes) => assert_eq!(std::fs::read(path).unwrap(), bytes),
        None => assert!(!path.exists()),
    }
}
#[test]
fn refuses_external_rewrite_atomic_replacement_and_identical_metadata() {
    for mode in ["rewrite", "atomic", "metadata"] {
        let d = Scratch::new();
        let p = original(&d);
        let mut original = std::fs::read(&p).unwrap();
        let mut b = io::save_bytes("external.docx", &Document::from_text("External B")).unwrap();
        if mode == "metadata" {
            // Compression does not promise equal sizes for equal-length text. ZIP comments
            // pad the original synthetic packages without changing their document contents.
            let size = original.len().max(b.len());
            for bytes in [&mut original, &mut b] {
                let end = bytes.len();
                assert_eq!(&bytes[end - 22..end - 18], b"PK\x05\x06");
                assert_eq!(&bytes[end - 2..], &[0, 0]);
                let comment = u16::try_from(size - end).unwrap();
                bytes[end - 2..].copy_from_slice(&comment.to_le_bytes());
                bytes.resize(size, b' ');
            }
            std::fs::write(&p, &original).unwrap();
            assert_eq!(io::open_bytes("external.docx", &b).unwrap().plain_text(StoryRef::Body), "External B");
            assert_eq!(io::open_path(&p).unwrap().plain_text(StoryRef::Body), "Original A");
        }
        let meta = std::fs::metadata(&p).unwrap();
        let mut s = open(&p);
        if mode == "atomic" {
            let tmp = d.path("external.docx");
            std::fs::write(&tmp, &b).unwrap();
            std::fs::rename(tmp, &p).unwrap();
        } else {
            std::fs::write(&p, &b).unwrap();
        }
        if mode == "metadata" {
            assert_eq!(b.len(), original.len());
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(meta.modified().unwrap()))
                .unwrap();
            assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), meta.modified().unwrap());
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(std::fs::metadata(&p).unwrap().ino(), meta.ino());
            }
        }
        conflict(&mut s, &p, Some(&b));
        assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    }
}
#[test]
fn deletion_is_a_conflict_but_a_separate_save_as_preserves_external_bytes() {
    let d = Scratch::new();
    let p = original(&d);
    let mut s = open(&p);
    std::fs::remove_file(&p).unwrap();
    conflict(&mut s, &p, None);
    let b = io::save_bytes("external.docx", &Document::from_text("External B")).unwrap();
    std::fs::write(&p, &b).unwrap();
    conflict(&mut s, &p, Some(&b));
    let copy = d.path("copy.docx");
    s.run("file.saveAs", &json!({"path": copy})).unwrap();
    assert!(!s.dirty);
    assert_eq!(s.path.as_ref(), Some(&copy));
    assert_eq!(std::fs::read(&p).unwrap(), b);
    assert_eq!(io::open_path(&copy).unwrap().plain_text(StoryRef::Body), "Local edit");
}

#[test]
fn identical_replacement_is_benign_and_success_advances_expected_content() {
    let d = Scratch::new();
    let p = original(&d);
    let mut s = open(&p);
    let a = std::fs::read(&p).unwrap();
    let replacement = d.path("replacement.docx");
    std::fs::write(&replacement, &a).unwrap();
    std::fs::rename(&replacement, &p).unwrap();
    s.run("file.save", &json!({})).unwrap();
    assert!(!s.dirty);
    s.run("document.setText", &json!({"text": "Second local edit"})).unwrap();
    s.run("file.save", &json!({})).unwrap();
    assert!(!s.dirty);
    assert_eq!(io::open_path(&p).unwrap().plain_text(StoryRef::Body), "Second local edit");
    s.run("document.setText", &json!({"text": "Local edit"})).unwrap();
    std::fs::write(&p, &a).unwrap();
    conflict(&mut s, &p, Some(&a));
    s.run("file.open", &json!({"path": p})).unwrap();
    s.run("document.setText", &json!({"text": "Reloaded edit"})).unwrap();
    s.run("file.save", &json!({})).unwrap();
}

#[test]
fn failure_keeps_expected_version_and_does_not_stamp_or_change_password() {
    let d = Scratch::new();
    let p = original(&d);
    let mut s = open(&p);
    let a = std::fs::read(&p).unwrap();
    let core = s.doc.core.clone();
    let blocked = d.path("blocked.docx");
    std::fs::create_dir(&blocked).unwrap();
    assert!(s.run("file.saveAs", &json!({"path": blocked, "password": "synthetic-password"})).is_err());
    assert_eq!(s.doc.core, core);
    assert!(s.password.is_none());
    assert!(s.dirty);
    assert_eq!(std::fs::read(&p).unwrap(), a);
    let b = io::save_bytes("external.docx", &Document::from_text("External B")).unwrap();
    std::fs::write(&p, &b).unwrap();
    conflict(&mut s, &p, Some(&b));
    std::fs::write(&p, &a).unwrap();
    s.run("file.save", &json!({})).unwrap();
}

#[test]
fn replacing_document_clears_identity_and_each_session_keeps_its_own_version() {
    let d = Scratch::new();
    let p = original(&d);
    let mut first = open(&p);
    let mut second = open(&p);
    first.run("file.save", &json!({})).unwrap();
    let changed = std::fs::read(&p).unwrap();
    conflict(&mut second, &p, Some(&changed));
    second.run("file.new", &json!({})).unwrap();
    assert!(second.path.is_none());
    second.run("document.setText", &json!({"text": "New draft"})).unwrap();
    second.run("file.saveAs", &json!({"path": d.path("draft.docx")})).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), changed);
    // Host-supplied bytes do not claim a native filesystem baseline.
    first.set_document(Document::from_text("Replacement draft"));
    first.path = None;
    first.run("file.saveAs", &json!({"path": d.path("replacement-draft.docx")})).unwrap();
}

#[test]
fn protected_open_and_save_compare_ciphertext_without_plaintext_files() {
    let d = Scratch::new();
    let p = d.path("protected.docx");
    io::save_path_with(&p, &Document::from_text("Protected A"), Some("synthetic-password"), &io::ExportOptions::default()).unwrap();
    let mut s = Session::new(Document::new());
    s.run("file.open", &json!({"path": p, "password": "synthetic-password"})).unwrap();
    s.run("document.setText", &json!({"text": "Local edit"})).unwrap();
    s.run("file.save", &json!({})).unwrap();
    assert!(wordcraft_docx::is_encrypted(&std::fs::read(&p).unwrap()));
    s.run("document.setText", &json!({"text": "Local edit"})).unwrap();
    let b =
        io::save_bytes_with("external.docx", &Document::from_text("Protected B"), Some("synthetic-password"), &io::ExportOptions::default()).unwrap();
    std::fs::write(&p, &b).unwrap();
    conflict(&mut s, &p, Some(&b));
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    let copy = d.path("protected-copy.docx");
    s.run("file.saveAs", &json!({"path": copy})).unwrap();
    assert!(wordcraft_docx::is_encrypted(&std::fs::read(&copy).unwrap()));
    assert!(io::needs_password(&io::open_path(&copy).unwrap_err()));
    assert_eq!(io::open_path_with(&copy, Some("synthetic-password")).unwrap().0.plain_text(StoryRef::Body), "Local edit");
}

#[test]
fn save_as_path_spellings_symlinks_and_hardlinks_cannot_bypass_conflicts() {
    for mode in [
        "dot",
        #[cfg(unix)]
        "symlink",
        "hardlink",
    ] {
        let d = Scratch::new();
        let p = original(&d);
        let mut s = open(&p);
        let alias = d.path("alias.docx");
        let target = match mode {
            "dot" => d.0.join("./working.docx"),
            #[cfg(unix)]
            "symlink" => {
                std::os::unix::fs::symlink(&p, &alias).unwrap();
                alias.clone()
            }
            _ => {
                std::fs::hard_link(&p, &alias).unwrap();
                alias.clone()
            }
        };
        let b = io::save_bytes("external.docx", &Document::from_text("External B with longer synthetic content to exercise changed-size hard links"))
            .unwrap();
        std::fs::write(&p, &b).unwrap();
        let core = s.doc.core.clone();
        assert!(matches!(s.run("file.saveAs", &json!({"path": target})), Err(wordcraft_engine::CmdError::FileConflict(_))));
        assert!(s.dirty);
        assert_eq!(s.path.as_ref(), Some(&p));
        assert_eq!(s.doc.core, core);
        assert_eq!(std::fs::read(&p).unwrap(), b);
        assert_eq!(std::fs::read(&target).unwrap(), b);
    }
}

#[test]
fn unreadable_non_file_destination_refuses_save_without_losing_edits() {
    let d = Scratch::new();
    let p = original(&d);
    let mut s = open(&p);
    std::fs::remove_file(&p).unwrap();
    std::fs::create_dir(&p).unwrap();
    std::fs::write(p.join("keep"), b"External directory sentinel").unwrap();
    assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert!(s.dirty);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
    assert_eq!(std::fs::read(p.join("keep")).unwrap(), b"External directory sentinel");
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
}

#[test]
fn explicit_password_save_conflict_retains_the_original_password_and_metadata() {
    let d = Scratch::new();
    let path = original(&d);
    let mut s = open(&path);
    let external = io::save_bytes("external.docx", &Document::from_text("External B")).unwrap();
    std::fs::write(&path, &external).unwrap();
    let core = s.doc.core.clone();
    assert!(matches!(s.run("file.save", &json!({"password": "synthetic-password"})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert!(s.dirty && s.password.is_none());
    assert_eq!(s.doc.core, core);
    assert_eq!(std::fs::read(path).unwrap(), external);
}

#[cfg(unix)]
#[test]
fn retargeting_a_parent_symlink_keeps_logical_path_ownership() {
    let d = Scratch::new();
    let first = d.path("first");
    let second = d.path("second");
    let link = d.path("alias");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let original = io::save_bytes("original.docx", &Document::from_text("Original A")).unwrap();
    let external = io::save_bytes("external.docx", &Document::from_text("External B")).unwrap();
    std::fs::write(first.join("working.docx"), &original).unwrap();
    std::fs::write(second.join("working.docx"), &external).unwrap();
    std::os::unix::fs::symlink(&first, &link).unwrap();
    let path = link.join("working.docx");
    let mut s = open(&path);
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
    assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert!(s.dirty);
    assert_eq!(std::fs::read(first.join("working.docx")).unwrap(), original);
    assert_eq!(std::fs::read(second.join("working.docx")).unwrap(), external);
}

#[cfg(unix)]
#[test]
fn alias_of_replaced_symlink_preserves_the_external_entry() {
    let d = Scratch::new();
    let path = original(&d);
    let original_bytes = std::fs::read(&path).unwrap();
    let mut s = open(&path);
    let core = s.doc.core.clone();
    let backup = d.path("original-backup.docx");
    std::fs::rename(&path, &backup).unwrap();
    let external = d.path("external.docx");
    io::save_path(&external, &Document::from_text("External version")).unwrap();
    let external_bytes = std::fs::read(&external).unwrap();
    std::os::unix::fs::symlink(&external, &path).unwrap();
    let alias = d.path("directory-alias");
    std::os::unix::fs::symlink(&d.0, &alias).unwrap();
    let result = s.run("file.saveAs", &json!({"path": alias.join("working.docx")}));
    assert!(
        matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))),
        "{result:?}; entry_symlink={}; external_unchanged={}; backup_unchanged={}",
        path.is_symlink(),
        std::fs::read(&external).unwrap() == external_bytes,
        std::fs::read(&backup).unwrap() == original_bytes
    );
    assert_eq!(std::fs::read_link(&path).unwrap(), external);
    assert_eq!(std::fs::read(&external).unwrap(), external_bytes);
    assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
    assert_eq!(s.path.as_ref(), Some(&path));
    assert_eq!(s.doc.core, core);
    assert!(s.dirty);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
}

#[cfg(unix)]
#[test]
fn alias_of_retargeted_logical_path_preserves_external_content() {
    let d = Scratch::new();
    let original = original(&d);
    let original_bytes = std::fs::read(&original).unwrap();
    let logical = d.path("logical.docx");
    std::os::unix::fs::symlink(&original, &logical).unwrap();
    let mut s = open(&logical);
    let core = s.doc.core.clone();
    let external = d.path("external.docx");
    io::save_path(&external, &Document::from_text("External version")).unwrap();
    let external_bytes = std::fs::read(&external).unwrap();
    std::fs::remove_file(&logical).unwrap();
    std::os::unix::fs::symlink(&external, &logical).unwrap();
    let result = s.run("file.saveAs", &json!({"path": external}));
    assert!(
        matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))),
        "{result:?}; external_unchanged={}; original_unchanged={}",
        std::fs::read(&external).unwrap() == external_bytes,
        std::fs::read(&original).unwrap() == original_bytes
    );
    assert_eq!(std::fs::read(&external).unwrap(), external_bytes);
    assert_eq!(std::fs::read_link(&logical).unwrap(), external);
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
    assert_eq!(s.path.as_ref(), Some(&logical));
    assert_eq!(s.doc.core, core);
    assert!(s.dirty);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Local edit");
}

#[test]
fn case_variant_save_obeys_the_destination_filesystem() {
    let d = Scratch::new();
    let path = original(&d);
    let original_bytes = std::fs::read(&path).unwrap();
    let mut s = open(&path);
    let backup = d.path("original-backup.docx");
    std::fs::rename(&path, &backup).unwrap();
    let variant = d.path("WORKING.docx");
    io::save_path(&variant, &Document::from_text("External version")).unwrap();
    let external_bytes = std::fs::read(&variant).unwrap();
    let same_entry = path.exists();
    let result = s.run("file.saveAs", &json!({"path": variant}));
    if same_entry {
        assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))), "{result:?}");
        assert_eq!(std::fs::read(&variant).unwrap(), external_bytes);
        assert!(s.dirty);
        assert_eq!(s.path.as_ref(), Some(&path));
    } else {
        result.unwrap();
        assert_eq!(io::open_path(&variant).unwrap().plain_text(StoryRef::Body), "Local edit");
        assert!(!path.exists(), "a distinct spelling stays a separate Save As destination");
    }
    assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
}

#[test]
fn current_replacement_hard_link_retains_external_bytes() {
    let d = Scratch::new();
    let path = original(&d);
    let original_bytes = std::fs::read(&path).unwrap();
    let mut s = open(&path);
    let backup = d.path("original-backup.docx");
    std::fs::rename(&path, &backup).unwrap();
    io::save_path(&path, &Document::from_text("External replacement")).unwrap();
    let external_bytes = std::fs::read(&path).unwrap();
    let alias = d.path("current-hard-link.docx");
    std::fs::hard_link(&path, &alias).unwrap();
    let result = s.run("file.saveAs", &json!({"path": alias}));
    assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))), "{result:?}");
    assert_eq!(std::fs::read(&path).unwrap(), external_bytes);
    assert_eq!(std::fs::read(&alias).unwrap(), external_bytes);
    assert!(same_file::is_same_file(&path, &alias).unwrap());
    assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
    assert!(s.dirty);
}

#[test]
fn deleted_case_variant_retains_observed_filename_aliases() {
    for (name, variant) in [("working.docx", "WORKING.DOCX"), ("café.docx", "CAFÉ.DOCX")] {
        let d = Scratch::new();
        let path = d.path(name);
        io::save_path(&path, &Document::from_text("Original A")).unwrap();
        let original_bytes = std::fs::read(&path).unwrap();
        let variant = d.path(variant);
        let aliases = same_file::is_same_file(&path, &variant).unwrap_or(false);
        let mut s = open(&path);
        let backup = d.path("original-backup.docx");
        std::fs::rename(&path, &backup).unwrap();
        let result = s.run("file.saveAs", &json!({"path": variant}));
        if aliases {
            assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))), "{result:?}");
            assert!(!variant.exists());
            assert!(s.dirty);
            s.run("file.saveAs", &json!({"path": d.path("separate-copy.docx")})).unwrap();
        } else {
            result.unwrap();
            assert_eq!(io::open_path(&variant).unwrap().plain_text(StoryRef::Body), "Local edit");
            assert!(!path.exists());
        }
        assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
    }
}

#[test]
fn deleted_canonically_equivalent_unicode_name_is_reserved() {
    let d = Scratch::new();
    let path = d.path("café.docx");
    io::save_path(&path, &Document::from_text("Original A")).unwrap();
    let original_bytes = std::fs::read(&path).unwrap();
    let mut s = open(&path);
    let backup = d.path("original-backup.docx");
    std::fs::rename(&path, &backup).unwrap();
    let decomposed = d.path("cafe\u{301}.docx");
    assert!(matches!(s.run("file.saveAs", &json!({"path": decomposed})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert!(!decomposed.exists());
    assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
    assert!(s.dirty);
    s.run("file.saveAs", &json!({"path": d.path("separate-copy.docx")})).unwrap();
}

#[cfg(unix)]
#[test]
fn special_entries_fail_without_blocking_alias_checks() {
    const CHILD: &str = "WORDCRAFT_FIFO_ALIAS_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "file_conflict::special_entries_fail_without_blocking_alias_checks", "--nocapture"])
            .env(CHILD, "1")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "FIFO alias checks failed: {status}");
                return;
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                let _ = child.kill();
                child.wait().unwrap();
                let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("wordcraft-conflict-{}-0", child.id())));
                panic!("FIFO alias checks blocked for five seconds");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    use std::os::unix::fs::FileTypeExt;
    let fifo = |path: &Path| assert!(std::process::Command::new("mkfifo").arg(path).status().unwrap().success());
    let d = Scratch::new();
    let path = original(&d);
    let original_bytes = std::fs::read(&path).unwrap();
    let uppercase = d.path("WORKING.DOCX");
    if !uppercase.exists() {
        fifo(&uppercase);
        let _session = open(&path);
        assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
        std::fs::remove_file(&uppercase).unwrap();
    }
    let mut s = open(&path);
    let core = s.doc.core.clone();
    let backup = d.path("original-backup.docx");
    std::fs::rename(&path, &backup).unwrap();
    fifo(&path);
    let existing = d.path("existing-copy.docx");
    io::save_path(&existing, &Document::from_text("External copy")).unwrap();
    let external_bytes = std::fs::read(&existing).unwrap();
    assert!(matches!(s.run("file.saveAs", &json!({"path": existing})), Err(wordcraft_engine::CmdError::FileConflict(_))));
    assert!(s.dirty);
    assert_eq!(s.doc.core, core);
    assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
    assert_eq!(std::fs::read(&existing).unwrap(), external_bytes);
    assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    let copy = d.path("new-copy.docx");
    s.run("file.saveAs", &json!({"path": copy})).unwrap();
    assert_eq!(io::open_path(&copy).unwrap().plain_text(StoryRef::Body), "Local edit");
}

#[test]
fn relative_deleted_unicode_alias_retains_conflict_protection() {
    const CHILD: &str = "WORDCRAFT_RELATIVE_ALIAS_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let d = Scratch::new();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "file_conflict::relative_deleted_unicode_alias_retains_conflict_protection", "--nocapture"])
            .current_dir(&d.0)
            .env(CHILD, "1")
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "relative alias child failed: {status}");
        return;
    }
    // The child owns its working directory; parallel tests keep their own process cwd.
    let path = std::env::current_dir().unwrap().join("café.docx");
    io::save_path(&path, &Document::from_text("Original A")).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut s = open(&path);
    let core = s.doc.core.clone();
    std::fs::rename(&path, "frozen-original.docx").unwrap();
    let result = s.run("file.saveAs", &json!({"path": "cafe\u{301}.docx"}));
    assert_eq!(std::fs::read("frozen-original.docx").unwrap(), original);
    assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))), "{result:?}");
    assert!(!Path::new("cafe\u{301}.docx").exists());
    assert!(s.dirty);
    assert_eq!(s.doc.core, core);
    s.run("file.saveAs", &json!({"path": "separate-copy.docx"})).unwrap();
    assert_eq!(io::open_path(Path::new("separate-copy.docx")).unwrap().plain_text(StoryRef::Body), "Local edit");
    assert_eq!(std::fs::read("frozen-original.docx").unwrap(), original);
}

#[cfg(unix)]
#[test]
fn lossy_save_through_alias_retains_the_sessions_logical_path() {
    use std::os::unix::fs::symlink;
    let d = Scratch::new();
    let a = d.path("a");
    let b = d.path("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let canonical = a.join("working.txt");
    let external = b.join("working.txt");
    std::fs::write(&canonical, "Original A").unwrap();
    std::fs::write(&external, "External B").unwrap();
    let frozen = std::fs::read(&external).unwrap();
    let alias = d.path("alias");
    symlink(&a, &alias).unwrap();
    let logical = alias.join("working.txt");
    let mut s = open(&logical);
    s.run("file.save", &json!({"path": canonical})).unwrap();
    assert_eq!(s.path.as_ref(), Some(&logical));
    assert!(s.dirty, "lossy text saves keep the richer document open");
    std::fs::remove_file(&alias).unwrap();
    symlink(&b, &alias).unwrap();
    let core = s.doc.core.clone();
    let result = s.run("file.save", &json!({}));
    assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))), "{result:?}");
    assert_eq!(std::fs::read(&external).unwrap(), frozen);
    assert_eq!(s.doc.core, core);
    assert!(s.dirty);
    assert_eq!(s.path.as_ref(), Some(&logical));
    let copy = d.path("separate-copy.docx");
    s.run("file.saveAs", &json!({"path": copy})).unwrap();
    assert_eq!(io::open_path(&copy).unwrap().plain_text(StoryRef::Body), "Local edit");
    assert_eq!(std::fs::read(&external).unwrap(), frozen);
}

#[test]
fn native_byte_open_keeps_the_read_baseline_through_password_retry() {
    for password in [None, Some("synthetic-password")] {
        let d = Scratch::new();
        let path = d.path("native.docx");
        let a = io::save_bytes_with("native.docx", &Document::from_text("Original A"), password, &io::ExportOptions::default()).unwrap();
        std::fs::write(&path, &a).unwrap();
        let mut s = Session::new(Document::from_text("Unsaved draft"));
        s.dirty = true;
        let document = s.document_id();
        let (mut params, _guard) = s.prepare_native_open(&path).unwrap();
        assert_eq!(s.document_id(), document);
        assert!(s.dirty && s.path.is_none());
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Unsaved draft");
        let b = io::save_bytes("external.docx", &Document::from_text("External B")).unwrap();
        std::fs::write(&path, &b).unwrap();
        if password.is_some() {
            assert!(s.run("file.open", &params).is_err());
            assert_eq!(s.document_id(), document);
            assert!(s.dirty && s.path.is_none());
            let malformed = d.path("malformed.docx");
            std::fs::write(&malformed, b"Not a document package").unwrap();
            let (other, guard) = s.prepare_native_open(&malformed).unwrap();
            assert!(s.run("file.open", &other).is_err());
            drop(guard);
        }
        params["password"] = json!(password);
        s.run("file.open", &params).unwrap();
        s.run("text.insert", &json!({"text":"Local edits"})).unwrap();
        assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
        assert_eq!(std::fs::read(&path).unwrap(), b);
        assert!(s.dirty);
        assert!(s.doc.plain_text(StoryRef::Body).contains("Local edits"));
        let copy = d.path("copy.docx");
        s.run("file.saveAs", &json!({"path":copy})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b);
        assert!(io::open_path_with(&copy, password).unwrap().0.plain_text(StoryRef::Body).contains("Local edits"));
    }
}

#[test]
fn canceled_native_request_cannot_reopen_without_its_baseline() {
    let d = Scratch::new();
    let path = original(&d);
    let mut s = Session::new(Document::from_text("Unsaved draft"));
    s.dirty = true;
    let document = s.document_id();
    let (params, guard) = s.prepare_native_open(&path).unwrap();
    for field in ["path", "data"] {
        for value in [None, Some(Value::Null), Some(json!(true)), Some(json!(123)), Some(json!([])), Some(json!({}))] {
            let mut invalid = params.clone();
            if let Some(value) = value {
                invalid[field] = value;
            } else {
                invalid.as_object_mut().unwrap().remove(field);
            }
            assert!(s.run("file.open", &invalid).is_err(), "native requests never fall through to ordinary Open: {invalid}");
            assert_eq!(s.document_id(), document);
            assert!(s.dirty && s.path.is_none());
        }
    }
    drop(guard);
    assert!(s.run("file.open", &params).is_err_and(|e| e.to_string().contains("expired")));
    assert_eq!(s.document_id(), document);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Unsaved draft");
    assert!(s.dirty && s.path.is_none());
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn native_requests_distinguish_identical_bytes_and_lossy_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    for password in [None, Some("synthetic-password")] {
        let d = Scratch::new();
        let a = d.0.join(OsString::from_vec(b"caf\xff.docx".to_vec()));
        let b = d.0.join(OsString::from_vec(b"caf\xfe.docx".to_vec()));
        let bytes = io::save_bytes_with("original.docx", &Document::from_text("Original"), password, &io::ExportOptions::default()).unwrap();
        std::fs::write(&a, &bytes).unwrap();
        std::fs::write(&b, &bytes).unwrap();
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        let mut s = Session::new(Document::new());
        let (first, _first_guard) = s.prepare_native_open(&a).unwrap();
        if password.is_some() {
            assert!(s.run("file.open", &first).is_err());
        }
        let (mut second, _second_guard) = s.prepare_native_open(&b).unwrap();
        second["password"] = json!(password);
        s.run("file.open", &second).unwrap();
        assert_eq!(s.path.as_ref(), Some(&b));
        s.run("text.insert", &json!({"text":"Local edit"})).unwrap();
        s.run("file.save", &json!({})).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), bytes, "the other native original never changes");
        assert!(io::open_path_with(&b, password).unwrap().0.plain_text(StoryRef::Body).contains("Local edit"));
        s.run("text.insert", &json!({"text":"Later edit"})).unwrap();
        std::fs::write(&b, &bytes).unwrap();
        assert!(matches!(s.run("file.save", &json!({})), Err(wordcraft_engine::CmdError::FileConflict(_))));
        assert_eq!(std::fs::read(&b).unwrap(), bytes);
        assert!(s.dirty);
        assert!(s.run("file.open", &first).is_err_and(|e| e.to_string().contains("expired")));
    }
}

/// Rejected document replacements must keep the original external-writer baseline.
#[test]
fn rejected_document_replacements_keep_external_save_protection() {
    use wordcraft_doc::{CharProps, InlineObject};
    for (command, params) in
        [("review.compare", json!({"text": "Different document.", "showIn": "new"})), ("mailings.envelopes", json!({"delivery": "Recipient"}))]
    {
        let d = Scratch::new();
        let path = original(&d);
        let mut s = open(&path);
        let end = s.doc.end_of(StoryRef::Body);
        s.doc.insert_object(&end, InlineObject::PermEnd { id: 1 }, &CharProps::default()).unwrap();
        let start = s.doc.start_of(StoryRef::Body);
        s.doc
            .insert_object(&start, InlineObject::PermStart { id: 1, group: String::new(), editor: s.author.clone() }, &CharProps::default())
            .unwrap();
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        let before = s.doc.clone();
        assert!(s.run(command, &params).is_err(), "{command}");
        assert_eq!(s.doc, before, "{command}");
        assert_eq!(s.path.as_deref(), Some(path.as_path()), "{command}");
        io::save_path(&path, &Document::from_text("External writer after refusal")).unwrap();
        let external = std::fs::read(&path).unwrap();
        let result = s.run("file.save", &json!({}));
        assert!(matches!(result, Err(wordcraft_engine::CmdError::FileConflict(_))), "{command}: {result:?}");
        assert_eq!(std::fs::read(&path).unwrap(), external, "{command}");
        assert!(s.dirty);
        assert_eq!(s.doc, before, "{command}");
    }
}
