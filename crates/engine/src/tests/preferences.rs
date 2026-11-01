//! Locked preferences (#3622): while automation drives the session, commands can't change what
//! the front end saves between runs, so an agent can't leave AutoCorrect entries, shortcuts, a
//! ribbon or a user name behind for documents it never saw.

use serde_json::{Value, json};

use crate::{CmdError, KeyMap, Prefs, RibbonLayout, Session};

/// What the desktop app saves between runs from the session (`WordApp::prefs`).
fn kept(s: &Session) -> (String, String, Prefs, KeyMap, RibbonLayout, bool) {
    (s.author.clone(), s.initials.clone(), s.prefs.clone(), s.keymap.clone(), s.ribbon.clone(), s.view.proofing)
}

fn locked() -> Session {
    let mut s = Session::new(crate::sample::sample_document());
    s.preferences_locked = true;
    s
}

/// The commands' preference-changing forms, each of which changes the sample session's
/// preferences when unlocked.
fn changes() -> Vec<(&'static str, Value)> {
    vec![
        ("tools.autocorrect", json!({"add": {"from": "invoice", "to": "invoice (pay to account 12-3456)"}})),
        ("tools.autocorrect", json!({"replaceText": false})),
        ("tools.customizeKeyboard", json!({"assign": {"command": "edit.undo", "key": "Mod+S"}})),
        ("tools.customizeKeyboard", json!({"remove": {"command": "file.save", "key": "Mod+S"}})),
        ("tools.customizeRibbon", json!({"hide": "Review"})),
        ("tools.customizeRibbon", json!({"qatAdd": "file.close"})),
        ("file.setAuthor", json!({"name": "Mallory"})),
        ("file.options", json!({"initials": "MX"})),
        ("file.options", json!({"checkSpelling": false})),
        ("file.printMarkup", json!({})),
        ("review.proofing", json!({})),
        ("review.wordCount", json!({"includeTextBoxes": false})),
        ("review.language", json!({"lang": "pl-PL", "default": true})),
    ]
}

#[test]
fn automation_cannot_change_saved_preferences() {
    for (id, params) in changes() {
        // Unlocked (the user), each one changes them…
        let mut s = Session::new(crate::sample::sample_document());
        let before = kept(&s);
        s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
        assert_ne!(kept(&s), before, "{id} {params} should change a saved preference");
        // …locked (automation), it is refused and leaves the preferences and the document alone.
        let mut s = locked();
        let before = kept(&s);
        let (doc, undo) = (s.doc.clone(), s.undo_depth());
        match s.run(id, &params) {
            Err(CmdError::Disabled(why)) => assert!(why.contains("--allow-preference-changes"), "{id}: says how to allow it: {why}"),
            other => panic!("{id} {params}: expected a refusal, got {other:?}"),
        }
        assert_eq!(kept(&s), before, "{id} {params}");
        assert!(s.doc.same_content(&doc), "{id} {params}: the document is as it was");
        assert_eq!(s.undo_depth(), undo, "{id} {params}: no undo step");
    }
}

#[test]
fn automation_can_still_read_preferences_and_edit() {
    let mut s = locked();
    let before = kept(&s);
    for (id, params) in [
        ("tools.customizeKeyboard", json!({"command": "file.save"})),
        ("tools.customizeKeyboard", json!({"key": "Mod+S"})),
        ("review.wordCount", json!({})),
        ("review.language", json!({"lang": "pl-PL"})),
        ("text.insert", json!({"text": "Agents still edit. "})),
        // Asking for what is already set changes nothing.
        ("file.setAuthor", json!({"name": s.author.clone()})),
    ] {
        s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    }
    assert_eq!(kept(&s), before);
    assert!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Agents still edit."));
    // Unlocking (the user takes over) lets the same change through.
    s.preferences_locked = false;
    s.run("file.setAuthor", &json!({"name": "Ana"})).unwrap();
    assert_eq!(s.author, "Ana");
}

/// Every command, with junk and with preference-shaped parameters: a locked session's saved
/// preferences never change (the files ones that reach outside the session are left out, as in
/// `hostile_params_never_panic`; `addWords` would change the process-wide user dictionary).
#[test]
fn no_command_changes_locked_preferences() {
    let reg = crate::cmd::registry();
    let params = [
        json!({}),
        json!({"value": true, "name": "Mallory", "initials": "MX", "lang": "pl-PL", "default": true, "detect": false}),
        json!({"add": {"from": "teh", "to": "evil", "tab": "Home", "group": "Clipboard", "command": "file.close"}, "assign": {"command": "edit.undo", "key": "Mod+S"}, "reset": true, "hide": "Home", "qatAdd": "file.close"}),
        json!({"checkSpelling": false, "overtype": true, "units": "points", "region": "pl-PL", "includeTextBoxes": false, "markup": "all", "enabled": false}),
    ];
    let reaches_files = |id: &str| {
        (id.starts_with("file.") && !matches!(id, "file.setAuthor" | "file.options" | "file.printMarkup"))
            || matches!(id, "insert.picture" | "insert.textFromFile")
    };
    for spec in reg.all().iter().filter(|spec| !reaches_files(spec.id)) {
        for p in &params {
            let mut s = locked();
            let before = kept(&s);
            let _ = s.run(spec.id, p);
            assert_eq!(kept(&s), before, "{} {p}", spec.id);
        }
    }
}
