use super::*;

/// Entering Viewing changes no document revision; edits and direct history replay must refuse before mutation.
#[test]
fn viewing_blocks_document_edits_and_history_but_keeps_reading_commands() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original"));
    s.run("text.insert", &json!({"text":"A"})).unwrap();
    let before = s.doc.clone();
    let revision = s.rev();
    s.run("view.viewing", &json!({"value":true})).unwrap();
    assert_eq!(s.rev(), revision);
    assert!(s.doc.same_content(&before));
    for (id, params) in [
        ("text.insert", json!({"text":"bad"})),
        ("edit.paste", json!({"text":"bad"})),
        ("text.backspace", json!({})),
        ("format.bold", json!({})),
        ("review.newComment", json!({"text":"bad"})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("file.encrypt", json!({"password":"test-only"})),
        ("insert.editHeader", json!({})),
        ("edit.formatPainter", json!({})),
        ("draw.pen", json!({})),
    ] {
        assert!(s.registry.get(id).is_some(), "unknown test command {id}");
        assert!(s.run(id, &params).is_err(), "{id} changed a viewed document");
        assert!(s.doc.same_content(&before), "{id} changed document data");
        assert_eq!(s.rev(), revision, "{id} changed revision");
    }
    assert!(!s.undo(), "the direct history API must also respect Viewing");
    assert!(s.doc.same_content(&before));
    for (id, params) in [("select.all", json!({})), ("edit.copy", json!({})), ("view.zoom", json!({"value":125}))] {
        s.run(id, &params).unwrap();
    }
    assert!(s.doc.same_content(&before));
    s.run("view.viewing", &json!({"value":false})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    s.run("text.insert", &json!({"text":"Allowed"})).unwrap();
}

/// The interaction lock is neither serialized nor a replacement for the document's tracking setting.
#[test]
fn viewing_is_manual_transient_and_keeps_track_changes_unchanged() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original"));
    assert!(s.run("text.insert", &json!({"text":"Editable"})).is_ok());
    s.run("review.trackChanges", &json!({"value":true})).unwrap();
    let before = s.doc.clone();
    s.run("view.viewing", &json!({"value":true})).unwrap();
    assert!(s.doc.settings.track_changes);
    assert!(s.doc.same_content(&before));
    let saved = serde_json::to_value(&s.view).unwrap();
    assert!(saved.get("viewing").is_none(), "interaction mode must not persist");
    s.run("view.viewing", &json!({"value":false})).unwrap();
    assert!(s.doc.settings.track_changes);
}

/// Switching to a different document must not transfer or clear the original document's Viewing choice.
#[test]
fn viewing_stays_with_its_document_when_switching_documents() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("First"));
    let first = s.document_id();
    s.run("view.viewing", &json!({"value":true})).unwrap();
    let second = s.add_document(wordcraft_doc::Document::from_text("Second"));
    s.activate(second).unwrap();
    s.run("text.insert", &json!({"text":"Editable"})).unwrap();
    s.activate(first).unwrap();
    assert!(s.run("text.insert", &json!({"text":"Blocked"})).is_err());
}

/// Every registered mutating command is refused before malformed or missing parameters are interpreted.
#[test]
fn viewing_disables_every_undoable_command_before_parameter_handling() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original"));
    s.run("view.viewing", &json!({"value":true})).unwrap();
    let ids: Vec<_> = s.registry.all().iter().filter(|spec| spec.mutates).map(|spec| spec.id).collect();
    for id in ids {
        assert!(s.why_disabled(id).is_some(), "enabled edit: {id}");
        assert!(matches!(s.run(id, &json!({})), Err(crate::CmdError::Disabled(_))), "edit escaped guard: {id}");
    }
}

/// Mixed query/write commands keep their read path but reject writing parameters at the central guard.
#[test]
fn viewing_keeps_queries_but_refuses_polymorphic_write_parameters() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original"));
    s.run("view.viewing", &json!({"value":true})).unwrap();
    for id in ["file.versions", "tools.macros", "mailings.matchFields"] {
        assert!(s.run(id, &json!({})).is_ok(), "query refused: {id}");
    }
    for (id, params) in [
        ("file.versions", json!({"restore":0})),
        ("tools.macros", json!({"run":"example"})),
        ("tools.macros", json!({"define":"example","steps":[]})),
        ("mailings.recipients", json!({"rows":[{"Name":"Example"}]})),
        ("review.trackingOptions", json!({"trackMoves":false})),
        ("file.save", json!({"password":"test-only"})),
        ("styles.apply", json!({"style":"Normal"})),
    ] {
        assert!(matches!(s.run(id, &params), Err(crate::CmdError::Disabled(_))), "write was not refused before parameters: {id}");
    }
}

/// Leaving a voluntary interaction lock must not remove an independently enforced document restriction.
#[test]
fn leaving_viewing_never_removes_document_protection() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Protected"));
    s.run("review.restrict", &json!({"mode":"readOnly"})).unwrap();
    s.run("view.viewing", &json!({"value":true})).unwrap();
    s.run("view.viewing", &json!({"value":false})).unwrap();
    assert_eq!(s.doc.settings.protection.as_deref(), Some("readOnly"));
    assert!(s.run("text.insert", &json!({"text":"Blocked"})).is_err());
}

/// Installing stale window state or temporarily taking a view out cannot unlock the shared document.
#[test]
fn viewing_cannot_be_cleared_by_an_older_window_view() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original"));
    let older = s.copy_view();
    s.run("view.viewing", &json!({"value":true})).unwrap();
    s.put_view(older);
    assert!(s.run("text.insert", &json!({"text":"Blocked"})).is_err());
    let saved = s.take_view();
    assert!(s.run("text.insert", &json!({"text":"Blocked"})).is_err());
    s.put_view(saved);
    s.run("view.viewing", &json!({"value":false})).unwrap();
    assert!(s.run("text.insert", &json!({"text":"Allowed"})).is_ok());
}

/// Invalid toggle values preserve the lock, and live inspection exposes it without creating an edit.
#[test]
fn viewing_rejects_malformed_values_without_unlocking_and_reports_live_state() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original"));
    s.run("view.viewing", &json!({"value":true})).unwrap();
    let depth = s.undo_depth();
    for value in [json!(null), json!("false"), json!(0), json!([]), json!({})] {
        assert!(matches!(s.run("view.viewing", &json!({"value":value})), Err(crate::CmdError::Params(_))));
        assert!(s.view.viewing);
    }
    assert_eq!(s.run("view.state", &json!({})).unwrap()["viewing"], true);
    assert_eq!(s.undo_depth(), depth);
    assert!(!s.dirty);
}

/// Replacing a document starts an independent editable document rather than inheriting the old interaction lock.
#[test]
fn viewing_does_not_carry_over_when_the_document_is_replaced() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Original"));
    s.run("view.viewing", &json!({"value":true})).unwrap();
    s.set_document(wordcraft_doc::Document::from_text("Replacement"));
    assert!(!s.view.viewing);
    s.run("text.insert", &json!({"text":"Editable"})).unwrap();
}

/// Reading existing header/footer stories preserves their content, part structure and document revision.
#[test]
fn viewing_keeps_existing_header_footer_navigation_and_copy_without_creating_parts() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Body"));
    s.run("insert.header", &json!({"text":"Header text"})).unwrap();
    s.run("insert.footer", &json!({"text":"Footer text"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    let before = s.doc.clone();
    let revision = s.rev();
    s.run("view.viewing", &json!({"value":true})).unwrap();
    for (id, text) in [("insert.editHeader", "Header text"), ("insert.editFooter", "Footer text")] {
        s.run(id, &json!({})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        s.run("edit.copy", &json!({})).unwrap();
        assert_eq!(s.clipboard_text, text);
        assert!(s.run("text.insert", &json!({"text":"Blocked"})).is_err());
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(s.sel.focus.story, StoryRef::Body);
        assert!(s.doc.same_content(&before));
        assert_eq!(s.rev(), revision);
    }
}

/// Closing header navigation while Viewing must not perform the usual cleanup that removes an empty part.
#[test]
fn viewing_can_leave_an_empty_header_without_stripping_it() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Body"));
    s.run("insert.createHeader", &json!({})).unwrap();
    let before = s.doc.clone();
    let revision = s.rev();
    s.run("view.viewing", &json!({"value":true})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(s.sel.focus.story, StoryRef::Body);
    assert!(s.doc.same_content(&before));
    assert_eq!(s.rev(), revision);
}

/// Language preferences may affect future documents but cannot rewrite even a blank document being viewed.
#[test]
fn viewing_language_preferences_do_not_rewrite_the_current_blank_document() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.set_editing_language(Some("en-US"));
    s.run("view.viewing", &json!({"value":true})).unwrap();
    let before = s.doc.clone();
    let revision = s.rev();
    s.set_editing_language(Some("ro-RO"));
    assert!(s.doc.same_content(&before));
    assert_eq!(s.rev(), revision);
    s.run("file.new", &json!({"template":"blank"})).unwrap();
    assert!(!s.view.viewing);
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("ro-RO"));
}

/// A stale equation-editing snapshot cannot capture arrows after another window enters Viewing.
#[test]
fn viewing_does_not_restore_equation_editing_from_a_window_snapshot() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.equation", &json!({"linear": "x+y"})).unwrap();
    assert!(s.math.is_some());
    let ordinary = s.copy_view();
    s.put_view(ordinary);
    assert!(s.math.is_some(), "ordinary Editing retains its equation session");
    let older = s.copy_view();
    s.run("view.viewing", &json!({"value": true})).unwrap();
    let before = s.doc.clone();
    let depth = s.undo_depth();
    s.put_view(older);
    assert!(s.math.is_none());
    assert!(s.view.viewing);
    s.run("caret.docStart", &json!({})).unwrap();
    assert!(s.doc.same_content(&before));
    assert_eq!(s.undo_depth(), depth);
}

/// A stale window cannot restore drawing tools or a lasso in Viewing; Editing retains both.
#[test]
fn viewing_does_not_restore_drawing_state_from_a_window_snapshot() {
    use crate::cmd::draw::{DrawMode, LassoSelection};
    use wordcraft_doc::freeform::InkTool;
    for viewing in [true, false] {
        for mode in [DrawMode::Select, DrawMode::Lasso, DrawMode::Eraser, DrawMode::Pen(InkTool::Pen)] {
            let mut s = Session::new(wordcraft_doc::Document::from_text("Hello"));
            s.view.draw.mode = mode;
            s.view.draw.lasso = Some(LassoSelection::default());
            let older = s.copy_view();
            s.view.draw.mode = DrawMode::Select;
            s.view.draw.lasso = None;
            s.run("view.viewing", &json!({"value": viewing})).unwrap();
            let before = s.doc.clone();
            let revision = s.rev();
            let depth = s.undo_depth();
            s.put_view(older);
            assert_eq!(s.view.viewing, viewing);
            assert_eq!(s.view.draw.mode, if viewing { DrawMode::Select } else { mode });
            assert_eq!(s.view.draw.lasso, if viewing { None } else { Some(LassoSelection::default()) });
            s.run("caret.docEnd", &json!({})).unwrap();
            assert!(s.doc.same_content(&before));
            assert_eq!(s.rev(), revision);
            assert_eq!(s.undo_depth(), depth);
        }
    }
}
