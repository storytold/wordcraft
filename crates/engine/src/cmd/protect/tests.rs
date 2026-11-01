use super::*;
use crate::Selection;

fn select(s: &mut Session, text: &str) {
    s.run("select.text", &json!({"text": text})).unwrap();
}

#[test]
fn read_only_with_an_exception_allows_edits_only_inside_it() {
    let mut s = Session::new(Document::from_text("Locked intro.\nFill in here.\nLocked end."));
    select(&mut s, "Fill in here.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    // Inside the exception: typing works.
    select(&mut s, "here");
    s.run("text.insert", &json!({"text": "there"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Fill in there."));
    // Outside it: refused, and the document is as it was.
    let before = s.doc.plain_text(StoryRef::Body);
    select(&mut s, "intro");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert!(s.run("format.bold", &json!({})).is_err());
    // Backspace at the start of the exception would delete outside it.
    let start = s.doc.perm_ranges()[0].start.clone();
    s.sel = Selection::caret(start);
    assert!(s.run("text.backspace", &json!({})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    // Undo can't lift the protection.
    for _ in 0..5 {
        let _ = s.run("edit.undo", &json!({}));
    }
    assert_eq!(s.doc.settings.protection.as_deref(), Some("readOnly"));
}

/// An exception that starts a line: Home, Left and Down put the caret before its start
/// marker, and Enter and typing there go inside the exception. A caret after the end marker at
/// the end of the line goes back inside it too.
#[test]
fn typing_at_the_start_or_end_of_an_exception_line_goes_inside_it() {
    let visible = |s: &Session| s.doc.plain_text(StoryRef::Body).replace(OBJ, "");
    let mut s = Session::new(Document::from_text("Title\nBody text\nEnd."));
    select(&mut s, "Body text");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    s.run("caret.home", &json!({})).unwrap();
    assert_eq!(s.sel.focus, Pos::body(1, 0), "the caret is before the start marker");
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "New "})).unwrap();
    assert_eq!(visible(&s), "Title\n\nNew Body text\nEnd.");
    s.run("caret.end", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "!"})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    assert_eq!(visible(&s), "Title\n\nNew Body text!\n\nEnd.");
    // Outside the exception stays refused: the start of "Title" and the end of "End.".
    for at in [Pos::body(0, 0), s.doc.end_of(StoryRef::Body)] {
        s.sel = Selection::caret(at);
        assert!(s.run("text.newParagraph", &json!({})).is_err());
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    }

    // An empty editable line right after a protected short line, with Built-in Heading styles
    // on: two Enters break the paragraph, and the protected line stays Normal text.
    let mut s = Session::new(Document::from_text("Title\n\nEnd."));
    s.run("tools.autocorrect", &json!({"headingStyles": true})).unwrap();
    s.sel = Selection { anchor: Pos::body(1, 0), focus: Pos::body(2, 0) };
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    s.sel = Selection::caret(Pos::body(0, 2));
    s.run("caret.down", &json!({})).unwrap();
    assert_eq!(s.sel.focus, Pos::body(1, 0), "the caret is before the start marker");
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Notes"})).unwrap();
    assert_eq!(visible(&s), "Title\n\n\nNotes\nEnd.");
    assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.props.style.is_none()));
}

/// Table, row and cell formatting reaches protected text, so an exception inside one cell
/// doesn't let table commands run there change the table, the row or the cell (#1327).
#[test]
fn read_only_exception_in_a_cell_does_not_open_the_table() {
    let mut s = Session::new(Document::from_text("Intro."));
    s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    for (i, t) in ["Alpha", "Bravo", "Charlie", "Delta"].iter().enumerate() {
        if i > 0 {
            s.run("text.tab", &json!({})).unwrap();
        }
        s.run("text.insert", &json!({"text": t})).unwrap();
    }
    select(&mut s, "Alpha");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    // Typing inside the exception still works.
    select(&mut s, "lph");
    s.run("text.insert", &json!({"text": "LPH"})).unwrap();
    select(&mut s, "LPH");
    let before = s.doc.clone();
    for (id, v) in [
        ("table.properties", json!({"align": "center"})),
        ("table.rowHeight", json!({"height": 40})),
        ("table.shading", json!({"color": "FF0000"})),
        ("table.cellAlign", json!({"value": "bottomRight"})),
        ("table.borders", json!({"kind": "none"})),
    ] {
        let e = s.run(id, &v).unwrap_err().to_string();
        assert!(e.contains("only the editable regions"), "{id}: {e}");
        assert_eq!(s.doc, before, "{id}");
    }
}

/// A table wholly inside an exception stays formattable: its table, rows and cells hold no
/// protected content.
#[test]
fn read_only_exception_around_a_table_lets_it_be_formatted() {
    let mut s = Session::new(Document::from_text("Locked.\nStart."));
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    s.run("text.insert", &json!({"text": "Alpha"})).unwrap();
    select(&mut s, "Start.");
    let anchor = s.sel.ordered().0;
    s.sel = Selection { anchor, focus: s.doc.end_of(StoryRef::Body) };
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    select(&mut s, "Alpha");
    s.run("table.properties", &json!({"align": "center"})).unwrap();
    s.run("table.rowHeight", &json!({"height": 40})).unwrap();
    s.run("table.shading", &json!({"color": "FF0000"})).unwrap();
    // The text before the exception is still protected.
    select(&mut s, "Locked");
    assert!(s.run("format.bold", &json!({})).is_err());
}

/// Formatting limited to styles: attaching a template refuses to update the styles from it
/// (it would bypass the Update Styles block), nor can Copy Styles; attaching without
/// updating still works.
#[test]
fn formatting_restriction_refuses_attach_template_update() {
    let mut t = Document::from_text("Template.");
    if let Some(st) = t.styles.styles.iter_mut().find(|st| st.id == "Heading1") {
        st.chr.size = Some(31.0);
    }
    let data = crate::cmd::insert::base64_encode(&crate::io::save_bytes("t.docx", &t).unwrap());
    let mut s = Session::new(Document::from_text("Text."));
    s.run("review.restrict", &json!({"mode": "none", "formatting": true})).unwrap();
    let before = s.doc.clone();
    let e = s.run("tools.attachTemplate", &json!({"path": "Report.dotx", "update": true, "data": data})).unwrap_err().to_string();
    assert!(e.contains("formatting is limited"), "{e}");
    assert_eq!(s.doc, before);
    // Copying styles in replaces their definitions, locked flag included (#3692).
    s.doc.styles.styles.iter_mut().filter(|st| st.id == "Heading1").for_each(|st| st.locked = true);
    let before = s.doc.clone();
    let e = s.run("styles.copyFrom", &json!({"path": "Report.dotx", "styles": ["Heading 1"], "data": data})).unwrap_err().to_string();
    assert!(e.contains("formatting is limited"), "{e}");
    assert_eq!(s.doc, before);
    s.run("tools.attachTemplate", &json!({"path": "Report.dotx"})).unwrap();
    assert_eq!(s.doc.settings.attached_template.as_deref(), Some("Report.dotx"));
}

/// Formatting limited to styles: Paste Formatting and table formatting are direct
/// formatting, and a locked table style can't be applied (#1327).
#[test]
fn formatting_restriction_covers_paste_formatting_and_tables() {
    let mut s = Session::new(Document::from_text("Bold words.\nplain words."));
    s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
    s.run("text.insert", &json!({"text": "Cell"})).unwrap();
    select(&mut s, "Bold");
    s.run("format.bold", &json!({})).unwrap();
    s.run("edit.copyFormat", &json!({})).unwrap();
    s.run("review.restrict", &json!({"formatting": true, "lockedStyles": ["TableGrid"]})).unwrap();
    s.run("review.restrict", &json!({"mode": "none", "formatting": true})).unwrap();
    assert!(s.doc.settings.protect_formatting);
    select(&mut s, "plain");
    let before = s.doc.clone();
    assert!(s.run("edit.pasteFormat", &json!({})).is_err(), "edit.pasteFormat");
    select(&mut s, "Cell");
    for (id, v) in [
        ("table.shading", json!({"color": "FF0000"})),
        ("table.borders", json!({"kind": "none"})),
        ("table.cellAlign", json!({"value": "bottomRight"})),
        ("table.properties", json!({"align": "center"})),
        ("table.rowHeight", json!({"height": 40})),
        ("table.style", json!({"style": "TableGrid"})),
    ] {
        let e = s.run(id, &v).unwrap_err().to_string();
        assert!(e.contains("formatting is limited") || e.contains("is locked"), "{id}: {e}");
    }
    assert_eq!(s.doc, before);
}

/// Page setup and style definitions reach text outside the exceptions too, so an exception
/// doesn't let them change; applying a character style inside one still works.
#[test]
fn read_only_exception_does_not_open_page_setup_or_styles() {
    let mut s = Session::new(Document::from_text("Locked intro.\nFill in here.\nLocked end."));
    select(&mut s, "Fill in here.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    select(&mut s, "here");
    let before = s.doc.clone();
    assert!(s.run("layout.orientation", &json!({"value": "landscape"})).is_err());
    assert!(s.run("layout.margins", &json!({"preset": "narrow"})).is_err());
    assert!(s.run("styles.modify", &json!({"style": "Normal", "chr": {"bold": true}})).is_err());
    assert_eq!(s.doc, before);
    s.run("format.charStyle", &json!({"style": "Strong"})).unwrap();
}

/// Protected for tracked changes: every edit is tracked, and the changes can't be accepted,
/// rejected or untracked (Word disables them too).
#[test]
fn tracked_changes_protection_keeps_revisions() {
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("review.restrict", &json!({"mode": "trackedChanges"})).unwrap();
    select(&mut s, "text");
    s.run("text.insert", &json!({"text": "words"})).unwrap();
    let before = s.doc.clone();
    for id in ["review.acceptAll", "review.rejectAll", "review.accept", "review.reject", "review.trackChanges"] {
        assert!(s.run(id, &json!({})).is_err(), "{id}");
    }
    assert_eq!(s.doc, before);
}

/// A version restore replaces the whole document, including the content the protection kept
/// read-only, so it needs the protection stopped first — and the password with it.
#[test]
fn a_version_restore_needs_the_protection_stopped_first() {
    for mode in ["readOnly", "forms", "comments", "trackedChanges"] {
        let mut s = Session::new(Document::from_text("Original text."));
        s.run("file.versions", &json!({"save": "before"})).unwrap();
        s.run("review.restrict", &json!({"mode": mode, "password": "pw"})).unwrap();
        let before = s.doc.clone();
        let e = s.run("file.versions", &json!({"restore": 0})).unwrap_err().to_string();
        assert!(e.contains("stop protection"), "{mode}: {e}");
        assert_eq!(s.doc, before, "{mode}: the restore changed the document");
        assert_eq!(s.versions.len(), 1, "{mode}: the snapshot is still there");
    }
}

/// A restore never changes the protection: the live document's choice stands, and a
/// snapshot taken while the document was protected doesn't bring that protection back.
#[test]
fn a_version_restore_keeps_the_live_protection() {
    let mut s = Session::new(Document::from_text("Original text."));
    s.run("file.versions", &json!({"save": "before"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly", "password": "pw"})).unwrap();
    // The protection is stopped, with its password, and the version restored after that.
    s.run("review.restrict", &json!({"stop": true, "password": "pw"})).unwrap();
    assert!(s.doc.settings.protection.is_none());
    s.run("file.versions", &json!({"restore": 0})).unwrap();
    assert!(s.doc.settings.protection.is_none(), "the restore brought the protection back");
    assert!(s.doc.settings.protect_hash.is_empty(), "the restore brought the password back");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Original text.");
}

/// A document-wide change that isn't tracked — the properties, the privacy options — is
/// refused while the document is protected: under tracked changes it would change the
/// document with no revision to show for it, and under the other modes the document may not
/// change at all.
#[test]
fn a_document_wide_change_that_is_not_tracked_is_refused_under_protection() {
    for mode in ["readOnly", "forms", "comments", "trackedChanges"] {
        for (id, params) in [("file.properties", json!({"title": "Forged title"})), ("file.privacyOptions", json!({"removePersonalInfo": true}))] {
            let mut s = Session::new(Document::from_text("Some text."));
            s.run("review.restrict", &json!({"mode": mode})).unwrap();
            let before = s.doc.clone();
            let e = s.run(id, &params).unwrap_err().to_string();
            assert!(e.contains("stop protection"), "{mode} {id}: {e}");
            assert_eq!(s.doc, before, "{mode} {id}: the document changed");
        }
    }
    // Without protection they work.
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("file.properties", &json!({"title": "A title"})).unwrap();
    assert_eq!(s.doc.core.title, "A title");
    s.run("file.privacyOptions", &json!({"removePersonalInfo": true})).unwrap();
    assert!(s.doc.settings.remove_personal_info);
}

/// A file command that changes the document is checked like any other: the privacy options,
/// the properties and the inspector's removals are refused in every mode a protected document
/// can be in (#1327 follow-up).
#[test]
fn file_commands_that_change_the_document_are_protected() {
    for mode in ["readOnly", "forms", "comments", "trackedChanges"] {
        for (id, params) in [
            ("file.privacyOptions", json!({"removePersonalInfo": true})),
            ("file.properties", json!({"title": "New title"})),
            ("file.inspect", json!({"remove": ["comments"]})),
        ] {
            let mut s = Session::new(Document::from_text("Some text."));
            s.run("review.restrict", &json!({"mode": mode})).unwrap();
            let before = s.doc.clone();
            assert!(s.run(id, &params).is_err(), "{mode} {id}: a protected document took the change");
            assert_eq!(s.doc, before, "{mode} {id}");
        }
    }
    // Without protection they work.
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("file.inspect", &json!({"remove": ["comments"]})).unwrap();
}

/// The user name is the identity a named exception range is checked against, and the author
/// the new revisions and comments are recorded under: a protected document refuses to change
/// it (#1327 follow-up).
#[test]
fn the_user_name_cannot_change_under_protection() {
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let author = s.author.clone();
    let e = s.run("file.setAuthor", &json!({"name": "Alice"})).unwrap_err().to_string();
    assert!(e.contains("stop protection"), "{e}");
    assert_eq!(s.author, author, "the user name did not change");
    // Without protection it works again.
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("file.setAuthor", &json!({"name": "Alice"})).unwrap();
    assert_eq!(s.author, "Alice");
}

/// The protection of a protected document doesn't change: Word asks for Stop Protection first,
/// and only the explicit way stops it — a command that merely names a mode can't strip it
/// (#1327 follow-up).
#[test]
fn a_protected_documents_protection_needs_an_explicit_stop() {
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    // The "none" shorthand stops the protection: refused, and the protection stays.
    let e = s.run("file.protect", &json!({"mode": "none"})).unwrap_err().to_string();
    assert!(e.contains("stop protection"), "{e}");
    assert_eq!(s.doc.settings.protection.as_deref(), Some("readOnly"));
    // Changing the mode of a protected document is refused too.
    let e = s.run("review.restrict", &json!({"mode": "comments"})).unwrap_err().to_string();
    assert!(e.contains("stop protection"), "{e}");
    assert_eq!(s.doc.settings.protection.as_deref(), Some("readOnly"));
    // Stopping it explicitly works.
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    assert!(s.doc.settings.protection.is_none());
}

/// A password-protected document still needs its password to stop, and the formatting-only
/// protection (the mode "none") is still how the styles are limited.
#[test]
fn a_password_guards_the_protection_and_the_formatting_only_path_still_works() {
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("review.restrict", &json!({"mode": "readOnly", "password": "pw"})).unwrap();
    assert!(s.run("review.restrict", &json!({"stop": true, "password": "wrong"})).is_err());
    assert!(s.run("review.restrict", &json!({"stop": true})).is_err());
    assert_eq!(s.doc.settings.protection.as_deref(), Some("readOnly"));
    // The formatting-only protection: the mode "none" with the formatting restriction.
    let mut t = Session::new(Document::from_text("Some text."));
    t.run("review.restrict", &json!({"mode": "none", "formatting": true})).unwrap();
    assert_eq!(t.doc.settings.protection.as_deref(), Some("none"));
    assert!(t.doc.settings.protect_formatting);
}

/// A password guards the protection whatever its mode: a formatting-only protection with a
/// password can't have that password replaced (and then removed) without knowing it.
#[test]
fn a_password_guards_the_formatting_only_protection_too() {
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("review.restrict", &json!({"mode": "none", "formatting": true, "password": "pw"})).unwrap();
    assert_eq!(s.doc.settings.protection.as_deref(), Some("none"));
    let hash = s.doc.settings.protect_hash.clone();
    assert!(!hash.is_empty(), "the password is set");
    // Replacing the protection would replace the password with one of the caller's choosing.
    assert!(s.run("review.restrict", &json!({"mode": "readOnly", "password": "other"})).is_err());
    assert_eq!(s.doc.settings.protect_hash, hash, "the password was replaced");
    assert_eq!(s.doc.settings.protection.as_deref(), Some("none"), "the protection was replaced");
    // And stopping it needs the password.
    assert!(s.run("review.restrict", &json!({"stop": true})).is_err());
    assert!(s.run("review.restrict", &json!({"stop": true, "password": "wrong"})).is_err());
    assert_eq!(s.doc.settings.protection.as_deref(), Some("none"), "the protection was stopped");
    s.run("review.restrict", &json!({"stop": true, "password": "pw"})).unwrap();
    assert!(s.doc.settings.protection.is_none());
}

/// The settings and the metadata of a protected document are part of what stays as it was:
/// a command that changes them is refused, or rolled back, like any other change outside
/// the editable regions (#1327 follow-up).
#[test]
fn a_protected_document_keeps_its_settings_and_metadata() {
    let mut s = Session::new(Document::from_text("Locked intro.\nFill in here."));
    select(&mut s, "Fill in here.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let before = s.doc.clone();
    // The properties: the change is refused, and the command says why.
    let e = s.run("file.properties", &json!({"title": "Forged title"})).unwrap_err().to_string();
    assert!(e.contains("stop protection"), "{e}");
    assert_eq!(s.doc, before);
    assert_ne!(s.doc.core.title, "Forged title");
    // A custom property too.
    assert!(s.run("file.properties", &json!({"custom": {"Owner": "Alice"}})).is_err());
    assert_eq!(s.doc, before);
    // An edit inside the exception still works, and isn't rolled back.
    select(&mut s, "here");
    s.run("text.insert", &json!({"text": "there"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Fill in there."));
    // Without protection the same command works.
    let mut t = Session::new(Document::from_text("Some text."));
    t.run("file.properties", &json!({"title": "A title"})).unwrap();
    assert_eq!(t.doc.core.title, "A title");
}

/// Reading the document stays allowed under protection: the inspector without removals, the
/// privacy settings, the info, and saving a version snapshot.
#[test]
fn file_commands_that_only_read_stay_allowed() {
    let mut s = Session::new(Document::from_text("Some text."));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    for (id, params) in
        [("file.inspect", json!({})), ("file.privacy", json!({})), ("file.info", json!({})), ("file.versions", json!({"save": "snapshot"}))]
    {
        assert!(s.run(id, &params).is_ok(), "{id}: a protected document refused a read");
    }
}

/// The post-check compares the document's settings and its metadata, so a command that
/// changed either is rolled back even where the command itself isn't refused — the
/// backstop behind the refusal the properties and the privacy options get.
#[test]
fn the_post_check_compares_the_settings_and_the_metadata() {
    let before = Document::from_text("Some text.");
    let mut after = before.clone();
    after.core.title = "Forged".into();
    assert!(!super::outside_unchanged(&before, &after, "someone"), "the metadata isn't compared");
    let mut after = before.clone();
    after.settings.remove_personal_info = true;
    assert!(!super::outside_unchanged(&before, &after, "someone"), "the settings aren't compared");
    assert!(super::outside_unchanged(&before, &before.clone(), "someone"), "an unchanged document isn't unchanged");
}

/// Read only, no exceptions: going to a header the document doesn't have doesn't create
/// one, and loading or filtering recipients doesn't rewrite the merge fields' shown results
/// (both used to change the document from a non-mutating command, #1327).
#[test]
fn read_only_navigation_and_recipients_leave_the_document_alone() {
    let mut s = Session::new(Document::from_text("Dear name,"));
    select(&mut s, "name");
    s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Ada"}, {"Name": "Bob"}]})).unwrap();
    s.run("mailings.preview", &json!({"value": true})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Ada"));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let (before, depth) = (s.doc.clone(), s.undo_depth());
    assert!(s.run("insert.editHeader", &json!({})).is_err());
    assert!(s.run("insert.editFooter", &json!({})).is_err());
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Cyd"}]})).unwrap();
    s.run("mailings.filterRecipients", &json!({"rules": [{"field": "Name", "op": "equal", "value": "Cyd"}]})).unwrap();
    s.run("mailings.sortRecipients", &json!({"field": "Name", "descending": true})).unwrap();
    s.run("mailings.editRecipients", &json!({"rows": [{"Name": "Dee"}]})).unwrap();
    assert!(s.run("mailings.next", &json!({})).is_err());
    assert_eq!(s.doc, before);
    assert_eq!(s.undo_depth(), depth);
}

#[test]
fn forms_mode_allows_edits_only_inside_fillable_controls() {
    let mut s = Session::new(Document::from_text("Locked intro.\nPick size.\nFill name.\nLocked end."));
    select(&mut s, "Pick size.");
    s.run("developer.dropDown", &json!({"tag": "size", "items": ["S", "M", "L"]})).unwrap();
    // A control that takes typing (a drop-down list doesn't: only its entries can be chosen).
    select(&mut s, "Fill name.");
    s.run("developer.plainText", &json!({"tag": "name"})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    assert_eq!(s.doc.settings.protection.as_deref(), Some("forms"));
    // Filling the drop-down by tag (the caret is outside the control): allowed.
    s.run("developer.chooseItem", &json!({"control": "size", "value": "M"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("\nM\n"));
    // Typing inside the control's content: allowed (text.insert replaces the selection).
    select(&mut s, "Fill");
    s.run("text.insert", &json!({"text": "Filled"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Filled name."));
    // Typing and formatting outside: refused, document unchanged.
    let before = s.doc.plain_text(StoryRef::Body);
    select(&mut s, "intro");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert!(s.run("format.bold", &json!({})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
}

/// While Filling in forms is enforced, Word offers no Remove Content Control and greys out the
/// Developer tab's control buttons and Properties. WordCraft removed a fillable control (its
/// content left as plain text), inserted new controls inside one, and renamed one.
#[test]
fn forms_mode_keeps_its_controls() {
    let mut s = Session::new(Document::from_text("Name: Fill name."));
    select(&mut s, "Fill name.");
    s.run("developer.richText", &json!({"tag": "name"})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    let before = s.doc.clone();
    assert!(s.run("developer.removeControl", &json!({"control": "name"})).is_err(), "by tag");
    select(&mut s, "Fill");
    let e = s.run("developer.removeControl", &json!({})).unwrap_err().to_string();
    assert!(e.contains("stop protection"), "at the caret: {e}");
    assert_eq!(s.doc, before);
    // No new control inside the field, and no property changes.
    for insert in CONTROL_INSERTS {
        assert!(s.run(insert, &json!({})).is_err(), "{insert}");
    }
    assert!(s.run("developer.properties", &json!({"control": "name", "tag": "other"})).is_err(), "tag");
    assert_eq!(s.doc, before);
    // Filling it is still allowed.
    s.run("text.insert", &json!({"text": "Ada"})).unwrap();
    assert_eq!(s.doc.control_ranges(StoryRef::Body).len(), 1);
    // Once protection stops, the control can go.
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("developer.removeControl", &json!({"control": "name"})).unwrap();
    assert!(s.doc.control_ranges(StoryRef::Body).is_empty());
}

/// Read only and comments-only protection grey out every edit when the document has no
/// exception the author may edit, and none once it has one (the selection decides then).
/// Comments stay available in comments mode (not in read only); file commands, Restrict Editing
/// and moving the caret always do.
#[test]
fn read_only_and_comments_grey_out_edits_without_an_exception() {
    let mut s = Session::new(Document::from_text("Locked intro.\nFill in here."));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    for id in ["text.insert", "format.bold", "para.alignCenter", "review.newComment"] {
        assert_eq!(s.why_disabled(id), Some("the document is protected (read only)"), "{id}");
    }
    for id in ["review.restrict", "file.save", "caret.right"] {
        assert_eq!(s.why_disabled(id), None, "{id}");
    }
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("review.restrict", &json!({"mode": "comments"})).unwrap();
    assert_eq!(s.why_disabled("text.insert"), Some("only comments are allowed in this document"));
    assert_eq!(s.why_disabled("review.newComment"), None);
    // Tracked changes: typing stays live, accepting or rejecting the changes doesn't.
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("review.restrict", &json!({"mode": "trackedChanges"})).unwrap();
    assert_eq!(s.why_disabled("text.insert"), None);
    assert_eq!(s.why_disabled("review.acceptAll"), Some("changes to this document are tracked"));
    // An exception for everyone: edits are no longer refused outright (the cached facts follow
    // the edit that added it).
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    select(&mut s, "Fill in here.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert_eq!(s.why_disabled("text.insert"), None);
    assert_eq!(s.why_disabled("format.bold"), None);
}

/// Track Changes Options stays available (its options are the user's own), but tracking
/// moves is the document's setting: protection refuses changing it, even with an exception.
#[test]
fn protection_refuses_changing_track_moves_but_not_the_view_options() {
    let mut s = Session::new(Document::from_text("Locked intro.\nFill in here."));
    select(&mut s, "Fill in here.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    for mode in ["readOnly", "comments", "forms"] {
        s.run("review.restrict", &json!({"mode": mode})).unwrap();
        assert_eq!(s.why_disabled("review.trackingOptions"), None, "{mode}");
        assert!(s.run("review.trackingOptions", &json!({"trackMoves": false})).is_err(), "{mode}");
        assert!(s.doc.settings.track_moves, "{mode}");
        s.run("review.trackingOptions", &json!({"insertColor": "00AA00"})).unwrap();
        s.run("review.restrict", &json!({"stop": true})).unwrap();
    }
    s.run("review.restrict", &json!({"mode": "trackedChanges"})).unwrap();
    s.run("review.trackingOptions", &json!({"trackMoves": false})).unwrap();
    assert!(!s.doc.settings.track_moves);
}

/// The ribbon greys out what forms mode refuses outright (as Word for Mac does), and only that:
/// typing into the field and the rest of the ribbon stay live.
#[test]
fn forms_mode_greys_out_the_control_commands() {
    let mut s = Session::new(Document::from_text("Name: Fill name."));
    select(&mut s, "Fill name.");
    s.run("developer.richText", &json!({"tag": "name"})).unwrap();
    let greyed = ["developer.removeControl", "developer.properties"].into_iter().chain(CONTROL_INSERTS);
    assert!(greyed.clone().all(|id| s.why_disabled(id).is_none()), "unprotected");
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    for id in greyed {
        let why = s.why_disabled(id).unwrap_or_default();
        assert!(why.contains("stop protection"), "{id}: {why:?}");
    }
    for id in ["text.insert", "format.bold", "review.restrict", "view.zoom"] {
        assert_eq!(s.why_disabled(id), None, "{id}");
    }
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    assert_eq!(s.why_disabled("developer.removeControl"), None);
}

#[test]
fn forms_mode_without_fillable_controls_stays_locked() {
    let mut s = Session::new(Document::from_text("No controls here."));
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    select(&mut s, "controls");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
}

/// A content-locked control stays frozen in forms mode, next to a fillable one: its content
/// takes no entries or deletion, while the fillable control's interior takes typing. (The
/// locks are set before Restrict Editing: once no control is fillable, protection refuses
/// every command outright, `developer.properties` included.)
#[test]
fn content_locked_control_stays_frozen_in_forms_mode() {
    let mut s = Session::new(Document::from_text("Fill me [combo here] now."));
    select(&mut s, "combo here");
    s.run("developer.comboBox", &json!({"tag": "deg", "items": ["BSc", "MSc"]})).unwrap();
    select(&mut s, "now");
    s.run("developer.richText", &json!({"tag": "free"})).unwrap();
    s.run("developer.properties", &json!({"control": "deg", "lockEdit": true, "lockDelete": true})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    // The content-locked control's own content is frozen: refused.
    assert!(s.run("developer.chooseItem", &json!({"control": "deg", "value": "MSc"})).is_err());
    // Deleting a lockDelete control is still refused under forms mode.
    assert!(s.run("developer.removeControl", &json!({"control": "deg"})).is_err());
    // Typing in the fillable control next to it: allowed.
    select(&mut s, "now");
    s.run("text.insert", &json!({"text": "today"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("today"));
    // Text right next to the controls, inside no fillable control: refused, unchanged.
    let before = s.doc.plain_text(StoryRef::Body);
    select(&mut s, "Fill me");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
}

/// Word blocks content-control property edits until Stop Protection: while any restriction
/// is enforced, `developer.properties` refuses to change a control's lock flags — the
/// presence of `lockEdit`/`lockDelete` is the trigger, whatever their value (absent, they
/// keep the current lock). Under Filling in forms every property is refused (Word greys
/// Properties out); under the other modes the title and tag stay editable.
#[test]
fn lock_property_changes_are_refused_under_protection() {
    let mut s = Session::new(Document::from_text("Fill [pick] now."));
    select(&mut s, "pick");
    s.run("developer.richText", &json!({"tag": "free"})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    // A lock flag on a fillable control: refused, whatever the value.
    assert!(s.run("developer.properties", &json!({"control": "free", "lockEdit": true})).is_err(), "lockEdit: true");
    assert!(s.run("developer.properties", &json!({"control": "free", "lockDelete": true})).is_err(), "lockDelete: true");
    assert!(s.run("developer.properties", &json!({"control": "free", "lockEdit": false})).is_err(), "lockEdit: false");
    // Under forms protection the other properties are refused too.
    assert!(s.run("developer.properties", &json!({"control": "free", "title": "Renamed"})).is_err(), "title");
    assert_eq!(s.doc.fillable_controls(), 1);
    // Any enforced mode refuses it too: read only, even with an exception that allows edits.
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    select(&mut s, "now");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert!(s.run("developer.properties", &json!({"control": "free", "lockEdit": true})).is_err());
    // Without protection, the lock flag works again (and makes the control unfillable).
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("developer.properties", &json!({"control": "free", "lockEdit": true})).unwrap();
    assert_eq!(s.doc.fillable_controls(), 0);
}

/// The review's bypass: with forms protection enforced, a recipient could clear a control's
/// locks through `developer.properties` — the walk never compares the marker payload the
/// command changes, and an empty control has no content to flip either. Refused now.
#[test]
fn forms_mode_cannot_clear_a_controls_lock_by_properties() {
    use wordcraft_doc::control::{ContentControl, ControlKind, ControlLock, ListItem};
    let mut s = Session::new(Document::from_text("Fill now."));
    select(&mut s, "now");
    s.run("developer.richText", &json!({"tag": "free"})).unwrap();
    // An empty content-locked combo box (adjacent markers, nothing between): only the
    // payload `developer.properties` changes differs — the bypass's shape.
    let at = s.doc.end_of(StoryRef::Body);
    let props = CharProps::default();
    let mut ctl = ContentControl::new(ControlKind::ComboBox {
        items: vec![ListItem { display: "BSc".into(), value: "BSc".into() }],
        last_value: String::new(),
    });
    ctl.tag = "deg".into();
    ctl.lock = ControlLock::ContentLocked;
    let start = s.doc.insert_object(&at, InlineObject::ControlStart { control: Box::new(ctl) }, &props).unwrap();
    s.doc.insert_object(&start, InlineObject::ControlEnd, &props).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    assert_eq!(s.doc.fillable_controls(), 1, "only the rich-text control is fillable");
    // Clearing "cannot be edited" (or "cannot be deleted") by properties: refused.
    assert!(s.run("developer.properties", &json!({"control": "deg", "lockEdit": false})).is_err(), "clearing lockEdit");
    assert!(s.run("developer.properties", &json!({"control": "deg", "lockDelete": false})).is_err(), "clearing lockDelete");
    // The control is still content-locked: its content takes no entries.
    assert!(s.run("developer.chooseItem", &json!({"control": "deg", "value": "BSc"})).is_err());
    // After Stop Protection, the lock can be changed again.
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("developer.properties", &json!({"control": "deg", "lockEdit": false})).unwrap();
    assert_eq!(s.doc.fillable_controls(), 2);
}

/// Locks nest: innermost wins. A locked control inside a fillable one keeps its own content
/// frozen while the control around it stays fillable. (Typing into the locked control is
/// already refused by the control rules; the protection walk is what refuses formatting.)
#[test]
fn forms_mode_nested_locks_innermost_wins() {
    let mut s = Session::new(Document::from_text("Alpha Beta."));
    select(&mut s, "Alpha Beta.");
    s.run("developer.richText", &json!({"tag": "outer"})).unwrap();
    select(&mut s, "Beta");
    s.run("developer.plainText", &json!({"tag": "inner"})).unwrap();
    s.run("developer.properties", &json!({"control": "inner", "lockEdit": true})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    // The walk: the fillable control's content is a hole, the locked one's is protected.
    let texts: Vec<String> = outside(&s.doc, &HoleMode::Controls)
        .into_iter()
        .map(|p| match p {
            Piece::Text(t, _) => t,
            _ => String::new(),
        })
        .collect();
    assert!(texts.iter().any(|t| t.contains("Beta")), "the locked control's content is protected");
    assert!(!texts.iter().any(|t| t.contains("Alpha")), "the fillable control's content is a hole");
    // Formatting the fillable control's content: allowed.
    select(&mut s, "Alpha");
    s.run("format.bold", &json!({})).unwrap();
    // Formatting and typing the inner locked control's content: refused, unchanged.
    let before = s.doc.plain_text(StoryRef::Body);
    select(&mut s, "Beta");
    assert!(s.run("format.bold", &json!({})).is_err());
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
}

/// Filling in forms: typing into an empty control's interior is allowed, typing outside any
/// control is refused and leaves the document as it was.
#[test]
fn forms_mode_fills_an_empty_control() {
    let mut s = Session::new(Document::from_text("Intro.\nMiddle.\nTail."));
    select(&mut s, "Middle.");
    s.run("developer.richText", &json!({"tag": "free"})).unwrap();
    // An empty plain-text control at the caret (it shows its placeholder).
    let end = s.doc.end_of(StoryRef::Body);
    s.sel = Selection::caret(end);
    s.run("developer.plainText", &json!({"tag": "name"})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    // Typing into the empty control's interior: allowed.
    let at = s.doc.control_ranges(StoryRef::Body).iter().find(|r| r.control.tag == "name").map(|r| r.content_start()).unwrap();
    s.sel = Selection::caret(at);
    s.run("text.insert", &json!({"text": "Ada"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Ada"));
    // Typing in the rich-text control's content: allowed; outside: refused, unchanged.
    select(&mut s, "Middle");
    s.run("text.insert", &json!({"text": "!"})).unwrap();
    let before = s.doc.plain_text(StoryRef::Body);
    select(&mut s, "Intro");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
}

/// A control inside a table cell is a hole: filling it by tag works, and the cell next to
/// it — outside any control — stays locked. (Controls wrapped around whole rows or cells,
/// whose markers aren't the cell paragraph's text, are not holes; a limit for now.)
#[test]
fn forms_mode_fills_a_control_in_a_table_cell() {
    let mut s = Session::new(Document::from_text("Form."));
    s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
    // The caret is in the first cell: give both cells text, then a drop-down in the second.
    s.run("text.insert", &json!({"text": "Size"})).unwrap();
    s.run("text.tab", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Choice"})).unwrap();
    select(&mut s, "Choice");
    s.run("developer.dropDown", &json!({"tag": "size", "items": ["S", "M"]})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    // Filling the cell's drop-down by tag: allowed.
    s.run("developer.chooseItem", &json!({"control": "size", "value": "M"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("M"));
    // Typing in the neighbouring cell, outside the control: refused, unchanged.
    let before = s.doc.plain_text(StoryRef::Body);
    select(&mut s, "Size");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
}

/// Discoverability in forms mode: the state counts the fillable controls (`formFields`),
/// and Find Next Region I Can Edit walks their contents in order, wrapping round. Other
/// modes keep the perm-range behaviour.
#[test]
fn form_fields_state_and_find_next_region() {
    let mut s = Session::new(Document::from_text("Alpha.\nBeta.\n"));
    select(&mut s, "Alpha.");
    s.run("developer.richText", &json!({"tag": "first"})).unwrap();
    select(&mut s, "Beta.");
    s.run("developer.richText", &json!({"tag": "second"})).unwrap();
    assert_eq!(state(&s)["formFields"], 2, "the count is reported in every mode");
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    assert_eq!(state(&s)["formFields"], 2);
    let ranges = s.doc.control_ranges(StoryRef::Body);
    let first = ranges.iter().find(|r| r.control.tag == "first").unwrap();
    let second = ranges.iter().find(|r| r.control.tag == "second").unwrap();
    let (start, end) = next_region(&s, &s.doc.start_of(StoryRef::Body)).unwrap();
    assert_eq!(start, first.content_start());
    assert_eq!(end, first.end);
    let (start, end) = next_region(&s, &first.end).unwrap();
    assert_eq!(start, second.content_start());
    assert_eq!(end, second.end);
    let (start, end) = next_region(&s, &second.end).unwrap();
    assert_eq!(start, first.content_start(), "wraps round to the first control");
    assert_eq!(end, first.end);
    // Without forms mode, Find Next Region is about perm ranges: none here, nothing to find.
    // (The protection is stopped first: the mode of a protected document doesn't change.)
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert_eq!(next_region(&s, &s.doc.start_of(StoryRef::Body)), None);
}

/// Malformed control markers — an orphan end, a hostile pile of nested starts that never
/// close — keep forms mode conservative: no panic or loop, locked text stays locked (the
/// pairing comes from the doc crate's per-block-list scan, so unclosed markers are orphans
/// whatever their depth, and markers never pair across a table-cell boundary).
#[test]
fn forms_mode_with_malformed_markers_stays_conservative() {
    use wordcraft_doc::control::{ContentControl, ControlKind};
    let mut s = Session::new(Document::from_text("Locked.\nPick.\nEnd."));
    select(&mut s, "Pick.");
    s.run("developer.dropDown", &json!({"tag": "size", "items": ["S", "M"]})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    // An orphan end marker at the end of the first paragraph…
    let props = CharProps::default();
    let paths = s.doc.para_paths(StoryRef::Body);
    let at = Pos { story: StoryRef::Body, path: paths[0].clone(), off: s.doc.para(StoryRef::Body, &paths[0]).map(|p| p.len()).unwrap_or(0) };
    s.doc.insert_object(&at, InlineObject::ControlEnd, &props).unwrap();
    // …and a hostile pile of nested starts, none of them closed (deeper than any cap).
    let mut at = Pos { story: StoryRef::Body, path: paths[2].clone(), off: 0 };
    for _ in 0..24 {
        at = s.doc.insert_object(&at, InlineObject::ControlStart { control: Box::new(ContentControl::new(ControlKind::RichText)) }, &props).unwrap();
    }
    // Only the drop-down is a whole control; the unpaired markers don't count as fillable.
    assert_eq!(s.doc.fillable_controls(), 1);
    s.run("developer.chooseItem", &json!({"control": "size", "value": "M"})).unwrap();
    let before = s.doc.plain_text(StoryRef::Body);
    select(&mut s, "Locked");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    // Text after the unclosed fillable starts is still locked: they are orphans, not a hole.
    select(&mut s, "End");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    // An orphan end mid-paragraph: the text after it, in the same paragraph, stays locked.
    let mid = Pos { story: StoryRef::Body, path: paths[0].clone(), off: 4 };
    s.doc.insert_object(&mid, InlineObject::ControlEnd, &props).unwrap();
    select(&mut s, "ed.");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    // The walk compares the changed character: the pieces of the malformed document and of a
    // clone with one character added outside any control differ.
    let mut changed = s.doc.clone();
    let at = Pos { story: StoryRef::Body, path: paths[0].clone(), off: 0 };
    changed.insert_text(&at, "x", &CharProps::default()).unwrap();
    assert_ne!(outside(&s.doc, &HoleMode::Controls), outside(&changed, &HoleMode::Controls));
}

#[test]
fn password_guards_stop_protection_and_survives_docx() {
    let mut s = Session::new(Document::from_text("Secret plan.\nEditable part."));
    select(&mut s, "Editable part.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly", "password": "pw1", "formatting": true, "lockedStyles": ["Heading1"]})).unwrap();
    let bytes = crate::io::save_bytes("x.docx", &s.doc).unwrap();
    let doc = crate::io::open_bytes("x.docx", &bytes).unwrap();
    let st = &doc.settings;
    assert_eq!(st.protection.as_deref(), Some("readOnly"));
    assert!(st.protect_formatting);
    assert!(st.protect_hash.iter().any(|(k, v)| k == "w:algorithmName" && v == "SHA-512"));
    assert!(doc.styles.get("Heading1").is_some_and(|x| x.locked));
    let r = doc.perm_ranges();
    assert_eq!(r.len(), 1);
    assert!(r[0].everyone());
    let mut t = Session::new(doc);
    assert!(t.run("review.restrict", &json!({"stop": true, "password": "wrong"})).is_err());
    assert!(t.run("review.restrict", &json!({"mode": "none"})).is_err());
    assert!(t.run("para.heading1", &json!({})).is_err(), "locked style");
    t.run("review.restrict", &json!({"stop": true, "password": "pw1"})).unwrap();
    assert!(t.doc.settings.protection.is_none());
    // The password isn't kept for Repeat or macros.
    assert!(t.last_command.as_ref().is_none_or(|(_, p)| p.get("password").is_none()));
}

#[test]
fn forms_protection_survives_save_and_password_guards_stop() {
    let mut s = Session::new(Document::from_text("Form: [pick]."));
    select(&mut s, "pick");
    s.run("developer.dropDown", &json!({"tag": "opt", "items": ["A", "B"]})).unwrap();
    s.run("review.restrict", &json!({"mode": "forms", "password": "pw"})).unwrap();
    s.run("developer.chooseItem", &json!({"control": "opt", "value": "B"})).unwrap();
    let bytes = crate::io::save_bytes("f.docx", &s.doc).unwrap();
    let mut t = Session::new(crate::io::open_bytes("f.docx", &bytes).unwrap());
    assert_eq!(t.doc.settings.protection.as_deref(), Some("forms"));
    assert!(t.doc.plain_text(StoryRef::Body).contains('B'));
    // Still enforced after reopen: outside edit refused, wrong password refused.
    select(&mut t, "Form");
    assert!(t.run("text.insert", &json!({"text": "x"})).is_err());
    assert!(t.run("review.restrict", &json!({"stop": true, "password": "no"})).is_err());
    // Right password lifts it; then outside edits work. Undo never lifts it.
    t.run("review.restrict", &json!({"stop": true, "password": "pw"})).unwrap();
    select(&mut t, "Form");
    t.run("text.insert", &json!({"text": "x"})).unwrap();
}

/// A Word file with the legacy hash (`w:cryptAlgorithmSid`): a wrong password is rejected, the
/// right one unlocks; a hash WordCraft can't check never unlocks.
#[test]
fn legacy_password_is_verified_and_unknown_forms_refused() {
    let protected = |sid: &str| {
        let mut doc = Document::from_text("Locked.");
        doc.settings.protection = Some("readOnly".into());
        doc.settings.protect_hash = [
            ("w:cryptProviderType", "rsaFull"),
            ("w:cryptAlgorithmClass", "hash"),
            ("w:cryptAlgorithmType", "typeAny"),
            ("w:cryptAlgorithmSid", sid),
            ("w:cryptSpinCount", "100000"),
            // SHA-1 of salt 00…0F + Word's legacy key for "Example" (ECMA-376 Part 4 §14.8.1).
            ("w:hash", "pEWelKoH6SVpvMmyPAJ8bHA/g/s="),
            ("w:salt", "AAECAwQFBgcICQoLDA0ODw=="),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        Session::new(doc)
    };
    let mut s = protected("4");
    assert!(s.run("review.restrict", &json!({"stop": true, "password": "wrong"})).is_err());
    assert!(s.run("review.restrict", &json!({"stop": true, "password": ""})).is_err());
    assert!(super::enforced(&s.doc));
    s.run("review.restrict", &json!({"stop": true, "password": "Example"})).unwrap();
    assert!(!super::enforced(&s.doc));
    // MD2 isn't supported: refused whatever the password.
    let mut s = protected("1");
    let e = s.run("review.restrict", &json!({"stop": true, "password": "anything"})).unwrap_err();
    assert!(e.to_string().contains(super::UNSUPPORTED_PASSWORD));
    assert!(super::enforced(&s.doc));
}

// ---- #5075: permitted regions (`w:permStart`/`w:permEnd`) in files ----

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// A DOCX package with `body`, a header `header` (when not empty) referenced from the body's
/// section, and `w:documentProtection` set to `mode`, enforced.
fn protected_docx(body: &str, header: &str, mode: &str) -> Vec<u8> {
    use std::io::Write;
    const RELS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;
    const TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let sect =
        if header.is_empty() { String::new() } else { r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#.to_string() };
    let mut parts = vec![
        ("word/document.xml", format!(r#"<w:document {W_NS}><w:body>{body}{sect}</w:body></w:document>"#)),
        ("word/settings.xml", format!(r#"<w:settings {W_NS}><w:documentProtection w:edit="{mode}" w:enforcement="1"/></w:settings>"#)),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships {RELS}><Relationship Id="rIdS" Type="{TYPE}/settings" Target="settings.xml"/><Relationship Id="rIdH" Type="{TYPE}/header" Target="header1.xml"/></Relationships>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(r#"<Relationships {RELS}><Relationship Id="main" Type="{TYPE}/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ),
    ];
    if !header.is_empty() {
        parts.push(("word/header1.xml", format!(r#"<w:hdr {W_NS}>{header}</w:hdr>"#)));
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in parts {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The text of part `name` in a DOCX package.
fn part_text(bytes: &[u8], name: &str) -> String {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut out).unwrap();
    out
}

fn para(inner: &str) -> String {
    format!("<w:p>{inner}</w:p>")
}

fn run(text: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

/// A read-only file with an exception for everyone, one for a named editor, one limited to the
/// second column of a table (its markers between the table's cells, as the schema allows), and
/// one in the header.
fn regions_file() -> Vec<u8> {
    let cell = |t: &str| format!("<w:tc>{}</w:tc>", para(&run(t)));
    let body = [
        para(&run("Locked intro.")),
        para(&format!(r#"<w:permStart w:id="907" w:edGrp="everyone"/>{}<w:permEnd w:id="907"/>"#, run("Open text."))),
        para(&format!(r#"<w:permStart w:id="12" w:ed="alice@example.com"/>{}<w:permEnd w:id="12"/>"#, run("Alice only."))),
        format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:permStart w:id="5" w:edGrp="everyone" w:colFirst="1" w:colLast="1"/>{}{}</w:tr><w:tr>{}{}<w:permEnd w:id="5"/></w:tr></w:tbl>"#,
            cell("Aone"),
            cell("Bone"),
            cell("Atwo"),
            cell("Btwo")
        ),
        para(&run("Locked end.")),
    ]
    .concat();
    let header = para(&format!(r#"<w:permStart w:id="40" w:edGrp="everyone"/>{}<w:permEnd w:id="40"/>"#, run("Header open.")));
    protected_docx(&body, &header, "readOnly")
}

/// What a range says about itself, for comparing before and after a save.
fn describe(doc: &Document) -> Vec<(String, String, Option<[u32; 2]>, String)> {
    doc.perm_ranges()
        .iter()
        .map(|r| {
            let text = range_text(doc, &r.start, &r.end);
            (r.group.clone(), r.editor.clone(), r.cols, text)
        })
        .collect()
}

#[test]
fn permitted_regions_and_protection_round_trip_through_docx() {
    let doc = crate::io::open_bytes("regions.docx", &regions_file()).unwrap();
    assert_eq!(doc.settings.protection.as_deref(), Some("readOnly"));
    let ranges = describe(&doc);
    assert_eq!(ranges.len(), 4, "{ranges:?}");
    assert!(ranges.contains(&("everyone".into(), String::new(), None, "Open text.".into())), "{ranges:?}");
    assert!(ranges.contains(&(String::new(), "alice@example.com".into(), None, "Alice only.".into())), "{ranges:?}");
    assert!(ranges.contains(&("everyone".into(), String::new(), None, "Header open.".into())), "{ranges:?}");
    assert!(ranges.iter().any(|r| r.2 == Some([1, 1])), "the column range keeps its columns: {ranges:?}");
    // Saved and opened again: the same protection and the same ranges.
    let bytes = crate::io::save_bytes("regions.docx", &doc).unwrap();
    let again = crate::io::open_bytes("regions.docx", &bytes).unwrap();
    assert_eq!(again.settings.protection.as_deref(), Some("readOnly"));
    assert_eq!(describe(&again), ranges);
    let xml = part_text(&bytes, "word/document.xml");
    assert!(xml.contains(r#"w:ed="alice@example.com""#), "{xml}");
    assert!(xml.contains(r#"w:colFirst="1""#) && xml.contains(r#"w:colLast="1""#), "{xml}");
    assert!(part_text(&bytes, "word/settings.xml").contains(r#"w:edit="readOnly""#));
    // And still enforced after the round trip.
    let mut s = Session::new(again);
    select(&mut s, "Locked intro");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    select(&mut s, "Open");
    s.run("text.insert", &json!({"text": "Opened"})).unwrap();
}

/// Under read-only protection the file's regions decide what can change: the everyone range,
/// a named range only for that user name, and a column range only in its own column's cells —
/// the other column's cells between its ends stay protected.
#[test]
fn a_protected_file_lets_only_its_permitted_regions_change() {
    let doc = crate::io::open_bytes("regions.docx", &regions_file()).unwrap();
    let mut s = Session::new(doc.clone());
    s.author = "Bob".into();
    for (text, editable) in [
        ("Locked intro", false),
        ("Open", true),
        ("Alice", false),
        ("Aone", false),
        ("Bone", true),
        ("Atwo", false),
        ("Btwo", true),
        ("Locked end", false),
    ] {
        select(&mut s, text);
        let before = s.doc.clone();
        let r = s.run("text.insert", &json!({"text": "Z"}));
        assert_eq!(r.is_ok(), editable, "{text}: {r:?}");
        if !editable {
            assert_eq!(s.doc, before, "{text}");
        }
    }
    // The named editor may edit their range (the user name, ignoring case).
    let mut s = Session::new(doc);
    s.author = "Alice@Example.com".into();
    select(&mut s, "Alice");
    s.run("text.insert", &json!({"text": "Alicia"})).unwrap();
    select(&mut s, "Locked end");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
}

/// Backspace, Delete, paste and selections at and across a range's edges.
#[test]
fn edits_at_and_across_an_exceptions_edges() {
    let mut s = Session::new(Document::from_text("Locked intro.\nFill in here.\nLocked end."));
    select(&mut s, "Fill in here.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let range = s.doc.perm_ranges()[0].clone();
    let before = s.doc.clone();
    // Backspace at the start and Delete at the end would take something outside.
    s.sel = Selection::caret(range.start.clone());
    assert!(s.run("text.backspace", &json!({})).is_err());
    s.sel = Selection::caret(range.end.clone());
    assert!(s.run("text.delete", &json!({})).is_err());
    assert_eq!(s.doc, before);
    // A selection reaching outside: deleting, typing over or pasting over it is refused.
    s.sel = Selection { anchor: Pos { off: range.end.off - 2, ..range.end.clone() }, focus: Pos::body(2, 3) };
    for (id, v) in [("text.delete", json!({})), ("text.insert", json!({"text": "x"})), ("edit.paste", json!({"text": "pasted"}))] {
        assert!(s.run(id, &v).is_err(), "{id}");
        assert_eq!(s.doc, before, "{id}");
    }
    // Inside: Delete before the end, Backspace after the start, and paste all work.
    s.sel = Selection::caret(Pos { off: range.start.off + 1, ..range.start.clone() });
    s.run("text.backspace", &json!({})).unwrap();
    let end = s.doc.perm_ranges()[0].end.clone();
    s.sel = Selection::caret(Pos { off: end.off - 1, ..end });
    s.run("text.delete", &json!({})).unwrap();
    select(&mut s, "here");
    s.run("edit.paste", &json!({"text": "there"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).replace(OBJ, "").contains("ill in there\n"), "{:?}", s.doc.plain_text(StoryRef::Body));
    // Pasting outside is refused.
    select(&mut s, "intro");
    assert!(s.run("edit.paste", &json!({"text": "pasted"})).is_err());
}

/// Damaged ranges in a file open without a panic and make nothing editable: a start that never
/// ends (it must not open the rest of the document), an end nothing started, an end before its
/// start, a huge id, a missing id, a group WordCraft can't place the user in, no group at all,
/// a pair split between the body and a header, column numbers that aren't numbers, and a pile
/// of nested starts with one end (only the innermost pair is a range).
#[test]
fn damaged_ranges_in_a_file_open_nothing_to_editing() {
    let nested: String = (0..500).map(|_| r#"<w:permStart w:id="10" w:edGrp="everyone"/>"#).collect();
    let body = [
        para(&format!(r#"{}<w:permStart w:id="1" w:edGrp="everyone"/>{}"#, run("Before."), run("Orphan start."))),
        para(&format!(r#"<w:permEnd w:id="2"/>{}"#, run("Orphan end."))),
        para(&format!(r#"<w:permEnd w:id="3"/><w:permStart w:id="3" w:edGrp="everyone"/>{}"#, run("End first."))),
        para(&format!(r#"<w:permStart w:id="99999999999999999999999999999999" w:edGrp="everyone"/>{}"#, run("Huge id."))),
        para(&format!(r#"<w:permStart w:edGrp="everyone"/><w:permStart w:id="7" w:edGrp="editors"/>{}<w:permEnd w:id="7"/>"#, run("Group."))),
        para(&format!(r#"<w:permStart w:id="8"/>{}<w:permEnd w:id="8"/>"#, run("No group."))),
        para(&format!(r#"<w:permStart w:id="9" w:edGrp="everyone"/>{}"#, run("To header."))),
        para(&format!(r#"{nested}{}<w:permEnd w:id="10"/>"#, run("Nested."))),
        para(&format!(r#"<w:permStart w:id="11" w:edGrp="everyone" w:colFirst="abc" w:colLast="-4"/>{}<w:permEnd w:id="11"/>"#, run("Bad columns."))),
        para(&run("Locked end.")),
    ]
    .concat();
    let header = para(&format!(r#"<w:permEnd w:id="9"/>{}"#, run("Header.")));
    let doc = crate::io::open_bytes("damaged.docx", &protected_docx(&body, &header, "readOnly")).unwrap();
    let mut s = Session::new(doc);
    let editable: Vec<String> = describe(&s.doc).into_iter().filter(|r| r.0 == "everyone" || !r.1.is_empty()).map(|(_, _, _, text)| text).collect();
    assert_eq!(editable, ["Nested.", "Bad columns."], "only the matched pairs for everyone");
    assert_eq!(state(&s)["exceptions"], 2);
    for (text, ok) in [
        ("Before", false),
        ("Orphan start", false),
        ("Orphan end", false),
        ("End first", false),
        ("Huge id", false),
        ("Group", false),
        ("No group", false),
        ("To header", false),
        ("Nested", true),
        ("Bad columns", true),
        ("Locked end", false),
    ] {
        select(&mut s, text);
        let before = s.doc.clone();
        let r = s.run("text.insert", &json!({"text": "Z"}));
        assert_eq!(r.is_ok(), ok, "{text}: {r:?}");
        if !ok {
            assert_eq!(s.doc, before, "{text}");
        }
    }
    // The damaged markers are kept (the file says what it says) and still open nothing.
    let bytes = crate::io::save_bytes("damaged.docx", &s.doc).unwrap();
    // The paragraph of `xml` holding `text`.
    fn para_with<'a>(xml: &'a str, text: &str) -> &'a str {
        let at = xml.find(text).unwrap_or_else(|| panic!("{text} in {xml}"));
        let start = xml[..at].rfind("<w:p>").into_iter().chain(xml[..at].rfind("<w:p ")).max().unwrap();
        &xml[start..at + xml[at..].find("</w:p>").unwrap()]
    }
    let xml = part_text(&bytes, "word/document.xml");
    assert!(para_with(&xml, "Orphan end.").contains("<w:permEnd "), "an end nothing started is saved again: {xml}");
    let first = para_with(&xml, "End first.");
    let (end, start) = (first.find("<w:permEnd ").unwrap(), first.find("<w:permStart ").unwrap());
    assert!(end < start, "an end before its start stays before it: {first}");
    let names: Vec<String> = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap().file_names().map(str::to_string).collect();
    let header = names.iter().find(|n| n.starts_with("word/header")).map(|n| part_text(&bytes, n)).unwrap();
    assert!(para_with(&header, "Header.").contains("<w:permEnd "), "the header's end is saved again: {header}");
    let again = crate::io::open_bytes("damaged.docx", &bytes).unwrap();
    assert_eq!(describe(&again), describe(&s.doc), "the same matched pairs, no more");
    let mut t = Session::new(again);
    for text in ["Orphan start", "Orphan end", "End first", "Locked end", "To header"] {
        select(&mut t, text);
        let before = t.doc.clone();
        assert!(t.run("text.insert", &json!({"text": "Z"})).is_err(), "{text}");
        assert_eq!(t.doc, before, "{text}");
    }
}

/// An unmatched start made in the model (a damaged document, a stray marker) leaves the
/// document as locked as it was, greyed out like one without exceptions; Remove All clears it,
/// and a new exception's id is one no marker uses yet.
#[test]
fn an_unmatched_start_keeps_the_document_locked() {
    let mut s = Session::new(Document::from_text("Alpha beta.\nGamma."));
    let at = Pos::body(0, 6);
    s.doc
        .insert_object(&at, InlineObject::PermStart { id: 3, group: "everyone".into(), editor: String::new(), cols: None }, &CharProps::default())
        .unwrap();
    assert!(s.doc.perm_ranges().is_empty());
    assert_eq!(s.doc.next_perm_id(), 4);
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert!(s.why_disabled("text.insert").is_some(), "no exception: edits grey out");
    assert!(s.why_disabled("review.nextEditableRegion").is_some());
    select(&mut s, "Gamma");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    select(&mut s, "beta");
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    s.run("review.restrict", &json!({"exception": "removeAll"})).unwrap();
    assert!(!s.doc.plain_text(StoryRef::Body).contains(OBJ), "the stray marker is gone");
}

/// Find Next Region I Can Edit is a command: it selects each region in turn, wrapping round,
/// and is unavailable when there is none.
#[test]
fn find_next_region_is_a_command() {
    let mut s = Session::new(Document::from_text("One.\nTwo.\nThree.\nFour."));
    assert!(s.run("review.nextEditableRegion", &json!({})).is_err());
    select(&mut s, "Two.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    select(&mut s, "Four.");
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert!(s.why_disabled("review.nextEditableRegion").is_none());
    s.sel = Selection::caret(Pos::body(0, 0));
    let mut found = Vec::new();
    for _ in 0..3 {
        s.run("review.nextEditableRegion", &json!({})).unwrap();
        let (a, b) = s.sel.ordered();
        found.push(range_text(&s.doc, &a, &b));
    }
    assert_eq!(found, ["Two.", "Four.", "Two."]);
    assert!(s.registry.get("review.nextEditableRegion").is_some_and(|c| !c.mutates));
}

/// The visible text between `a` and `b` in one paragraph; across paragraphs, the paths.
fn range_text(doc: &Document, a: &Pos, b: &Pos) -> String {
    if a.story != b.story || a.path != b.path {
        return format!("{:?}..{:?}", a.path, b.path);
    }
    doc.para(a.story, &a.path).and_then(|p| p.text.get(a.off..b.off)).unwrap_or_default().replace(OBJ, "")
}
