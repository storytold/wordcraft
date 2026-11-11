//! Boundary and failure-path coverage through the command interface.
use super::*;

#[test]
fn edge_case_rename_rejects_bad_parameter_types_without_changing_the_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Keep this draft"}));
    run(&mut s, "file.rename", json!({"name": "Draft"}));
    let before = serde_json::to_value(&s.doc).unwrap();
    for value in [
        json!(null),
        json!({}),
        json!({"name": null}),
        json!({"name": 42}),
        json!({"name": true}),
        json!({"name": []}),
        json!({"name": {}}),
        json!({"name": "a\u{0000}b"}),
        json!({"name": "a\nb"}),
        json!({"name": "a|b"}),
    ] {
        assert!(s.run("file.rename", &value).is_err(), "{value}");
        assert_eq!(s.file_name.as_deref(), Some("Draft"));
        assert!(s.path.is_none() && s.dirty);
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(text(&s).is_empty(), "refusals did not consume the edit's undo step");
}

#[test]
fn edge_case_rename_counts_unicode_characters_at_the_length_boundary() {
    let mut s = s();
    for unit in ["a", "é", "界", "📝"] {
        let accepted = unit.repeat(200);
        run(&mut s, "file.rename", json!({"name": accepted}));
        assert_eq!(s.file_name.as_deref(), Some(accepted.as_str()));
        assert!(s.run("file.rename", &json!({"name": unit.repeat(201)})).is_err());
        assert_eq!(s.file_name.as_deref(), Some(accepted.as_str()));
        assert!(!s.dirty && !s.can_undo());
    }
}

#[test]
fn edge_case_failed_open_preserves_the_draft_name_metadata_and_undo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Work that must survive"}));
    run(&mut s, "file.rename", json!({"name": "My draft"}));
    let before = serde_json::to_value(&s.doc).unwrap();
    let document = s.document_id();
    for (path, data) in [("broken.docx", "eA=="), ("broken.docx", "UEsDBA=="), ("broken.doc", "bm90IGEgZG9j"), ("broken.odt", "bm90IGFuIG9kdA==")] {
        assert!(s.run("file.open", &json!({"path": path, "data": data})).is_err(), "{path}: {data}");
        assert_eq!(s.document_id(), document);
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
        assert_eq!(s.file_name.as_deref(), Some("My draft"));
        assert!(s.path.is_none() && s.dirty);
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(text(&s).is_empty());
}

#[test]
fn edge_case_account_normalizes_unicode_and_control_characters_without_editing() {
    let mut s = s();
    for name in ["", " ", "\u{0000}\t\n"] {
        let previous = s.author.clone();
        assert!(s.run("file.setAuthor", &json!({"name": name})).is_err());
        assert_eq!(s.author, previous);
    }
    run(&mut s, "file.setAuthor", json!({"name": format!("  {}\u{0000}  ", "界".repeat(300))}));
    assert_eq!(s.author, "界".repeat(255));
    run(&mut s, "file.options", json!({"initials": "  A\nB📝界CDEFGHIJ  "}));
    assert_eq!(s.initials, "AB📝界CDEFG");
    assert!(!s.dirty && !s.can_undo());
}

#[test]
fn edge_case_failed_save_as_and_rename_preserve_unsaved_work_and_original_bytes() {
    let dir = scratch_dir("edge-save-failures");
    let original = dir.join("original.docx");
    let collision = dir.join("occupied.docx");
    std::fs::create_dir(&collision).unwrap();
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Saved text"}));
    run(&mut s, "file.save", json!({"path": original}));
    let original_bytes = std::fs::read(&original).unwrap();
    run(&mut s, "text.insert", json!({"text": " plus unsaved work"}));
    let pending = text(&s);
    assert!(s.run("file.saveAs", &json!({"path": collision})).is_err());
    assert_eq!(s.path.as_ref(), Some(&original));
    assert!(s.dirty);
    assert_eq!(text(&s), pending);
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
    assert!(s.run("file.rename", &json!({"name": "occupied"})).is_err());
    assert_eq!(s.path.as_ref(), Some(&original));
    assert!(s.dirty && collision.is_dir());
    assert_eq!(text(&s), pending);
    run(&mut s, "file.save", json!({}));
    let reopened = crate::io::open_path(&original).unwrap();
    assert_eq!(reopened.plain_text(StoryRef::Body), pending);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn edge_case_missing_rename_source_keeps_the_document_recoverable() {
    let dir = scratch_dir("edge-missing-rename");
    let original = dir.join("original.docx");
    let recovered = dir.join("recovered.docx");
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Recoverable work"}));
    run(&mut s, "file.save", json!({"path": original}));
    std::fs::remove_file(&original).unwrap();
    run(&mut s, "text.insert", json!({"text": " after external deletion"}));
    let pending = text(&s);
    assert!(s.run("file.rename", &json!({"name": "renamed"})).is_err());
    assert_eq!(s.path.as_ref(), Some(&original));
    assert!(s.dirty && !dir.join("renamed.docx").exists());
    assert_eq!(text(&s), pending);
    run(&mut s, "file.saveAs", json!({"path": recovered}));
    assert_eq!(crate::io::open_path(&recovered).unwrap().plain_text(StoryRef::Body), pending);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn edge_case_rename_refuses_live_and_dangling_symlink_destinations() {
    let dir = scratch_dir("edge-rename-links");
    let original = dir.join("original.docx");
    let other = dir.join("other.docx");
    std::fs::write(&original, b"original bytes").unwrap();
    std::fs::write(&other, b"other bytes").unwrap();
    std::os::unix::fs::symlink(&other, dir.join("alias.docx")).unwrap();
    std::os::unix::fs::symlink(dir.join("missing.docx"), dir.join("dangling.docx")).unwrap();
    let mut s = s();
    s.path = Some(original.clone());
    for name in ["alias", "dangling"] {
        assert!(s.run("file.rename", &json!({"name": name})).is_err(), "{name}");
        assert_eq!(s.path.as_ref(), Some(&original));
        assert!(std::fs::symlink_metadata(dir.join(format!("{name}.docx"))).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&original).unwrap(), b"original bytes");
        assert_eq!(std::fs::read(&other).unwrap(), b"other bytes");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn edge_case_opening_an_empty_file_keeps_the_unsaved_draft_available() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Unsaved original"}));
    run(&mut s, "file.rename", json!({"name": "Original name"}));
    let original = s.document_id();
    run(&mut s, "file.open", json!({"path": "empty.docx", "data": ""}));
    assert_ne!(s.document_id(), original);
    assert!(text(&s).is_empty() && !s.dirty);
    assert_eq!(s.path.as_deref(), Some(std::path::Path::new("empty.docx")));
    run(&mut s, "view.switchWindows", json!({"id": original}));
    assert_eq!(s.document_id(), original);
    assert_eq!(text(&s), "Unsaved original");
    assert_eq!(s.file_name.as_deref(), Some("Original name"));
    assert!(s.dirty && s.path.is_none());
    run(&mut s, "edit.undo", json!({}));
    assert!(text(&s).is_empty());
}
