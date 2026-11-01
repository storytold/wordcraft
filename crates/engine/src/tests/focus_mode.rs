//! Focus (View › Immersive › Focus, `view.focus`, #1678) integration tests at the engine layer:
//! the command's contract, what it does to the other screen states, and the per-document view
//! state it lives in. What the window shows in Focus is tested in `wordcraft-ui-egui`.

use super::*;

/// Focus is a plain view toggle reported under its camelCase name, so control, MCP and the CLI
/// can read it back from `view.state`.
#[test]
fn focus_is_reported_by_view_state() {
    let mut s = s();
    assert_eq!(run(&mut s, "view.state", json!({}))["focusMode"], false, "a new session starts in the usual window");
    assert_eq!(run(&mut s, "view.focus", json!({"value": true}))["value"], true);
    assert_eq!(run(&mut s, "view.state", json!({}))["focusMode"], true);
    assert_eq!(run(&mut s, "view.focus", json!({"value": false}))["value"], false);
    assert_eq!(run(&mut s, "view.state", json!({}))["focusMode"], false);
}

/// `value` is the only way to say which way: without it the command toggles, with a bool it sets.
#[test]
fn focus_toggles_without_a_value_and_sets_with_one() {
    let mut s = s();
    assert_eq!(run(&mut s, "view.focus", json!({}))["value"], true);
    assert_eq!(run(&mut s, "view.focus", json!({}))["value"], false);
    assert_eq!(run(&mut s, "view.focus", json!({"value": true}))["value"], true);
    assert_eq!(run(&mut s, "view.focus", json!({"value": true}))["value"], true, "true is idempotent");
    assert_eq!(run(&mut s, "view.focus", json!({"value": false}))["value"], false);
    assert_eq!(run(&mut s, "view.focus", json!({"value": false}))["value"], false, "false is idempotent");
}

/// A `value` that is present but is not a bool is refused, not read as "toggle": a loosely typed
/// client sending `{"value": "true"}` used to turn Focus *off* when it was on. Every view switch
/// behaves the same way, so the fix is in one place (`p::toggle`/`p::switch`).
#[test]
fn a_non_boolean_value_is_refused_and_leaves_the_view_alone() {
    let mut s = s();
    run(&mut s, "view.focus", json!({"value": true}));
    for junk in [
        json!({"value": "true"}),
        json!({"value": "false"}),
        json!({"value": 1}),
        json!({"value": 0}),
        json!({"value": null}),
        json!({"value": [true]}),
        json!({"value": {}}),
    ] {
        let r = s.run("view.focus", &junk);
        assert!(matches!(r, Err(crate::CmdError::Params(_))), "{junk} is refused: {r:?}");
        assert!(s.view.focus_mode, "{junk} leaves Focus on");
    }
    // The whole switch family, not just Focus: the view switches, the character and paragraph
    // toggles, the panes, the document and table settings, and the commands that branch on
    // `value` before they read it (`draw.inkToShape` converts the lasso's strokes on that path,
    // and `layout.hyphenation` toggles).
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    // Ink lasso-selected, so `draw.inkToShape`'s junk-value refusal must come before the branch
    // that would convert the strokes: the loop's document check would see the conversion.
    run(&mut s, "draw.stroke", json!({"page": 0, "points": [[40.0, 660.0], [100.0, 660.0]]}));
    run(&mut s, "draw.lasso", json!({"page": 0, "points": [[20.0, 640.0], [120.0, 640.0], [120.0, 690.0], [20.0, 690.0]]}));
    assert!(s.view.draw.lasso.as_ref().is_some_and(|l| !l.items.is_empty()), "the lasso picked the stroke up");
    let converted = run(&mut s, "draw.inkToShape", json!({}));
    assert_eq!(converted["count"], 1, "and without a value it converts it: {converted}");
    let switches = [
        "view.focus",
        "view.fullScreenView",
        "view.fullScreen",
        "view.marks",
        "view.ruler",
        "view.gridlines",
        "table.viewGridlines",
        "view.navigationPane",
        "view.stylesPane",
        "view.commentsPane",
        "view.accessibilityPane",
        "view.altTextPane",
        "view.hideWhiteSpace",
        "view.darkMode",
        "format.bold",
        "format.italic",
        "format.underline",
        "para.keepNext",
        "para.keepLines",
        "layout.hyphenation",
        "layout.differentFirstPage",
        "layout.sectionRtl",
        "draw.inkToShape",
        "table.repeatHeader",
        "table.rtl",
        "review.trackChanges",
        "review.proofing",
        "text.overtype",
        "edit.clipboardPane",
        "styles.inspector",
    ];
    for id in switches {
        let before = (run(&mut s, "view.state", json!({})), s.run("document.inspect", &json!({})).ok(), s.undo_labels());
        let r = s.run(id, &json!({"value": "yes"}));
        assert!(matches!(r, Err(crate::CmdError::Params(_))), "{id} refuses a non-boolean value: {r:?}");
        let after = (run(&mut s, "view.state", json!({})), s.run("document.inspect", &json!({})).ok(), s.undo_labels());
        assert!(after == before, "{id} changed nothing when it refused");
    }
    // Absent still toggles, and a real bool still sets.
    assert_eq!(run(&mut s, "view.focus", json!({}))["value"], false, "no value toggles");
    assert_eq!(run(&mut s, "view.focus", json!({"value": true}))["value"], true);
    // The branch that used to swallow a junk value still runs for a real one.
    run(&mut s, "format.bold", json!({"value": true}));
    assert_eq!(run(&mut s, "format.state", json!({}))["bold"], true, "a real value still sets Bold");
    assert_eq!(run(&mut s, "draw.inkToShape", json!({"value": true}))["value"], true, "and Ink to Shape");
}

/// Focus is presentation only: no undo checkpoint, no document change, no dirtying, and it
/// survives relayouts and every view mode.
#[test]
fn focus_leaves_the_document_and_the_undo_history_alone() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let undo = s.undo_labels();
    s.dirty = false;
    for mode in ["view.draft", "view.outline", "view.webLayout", "view.printLayout"] {
        run(&mut s, mode, json!({}));
        run(&mut s, "view.focus", json!({"value": true}));
        assert!(s.view.focus_mode, "{mode} keeps Focus on");
        assert_eq!(text(&s), "Hello", "{mode} in Focus leaves the text alone");
        assert_eq!(s.undo_labels(), undo, "{mode} in Focus adds no undo step");
        assert!(!s.dirty, "{mode} in Focus doesn't dirty the document");
    }
    run(&mut s, "view.focus", json!({"value": false}));
    assert_eq!(text(&s), "Hello");
    assert_eq!(s.undo_labels(), undo);
}

/// Focus, Full Screen View and Full Screen are three separate switches: Focus and Full Screen
/// View replace each other, Full Screen (the window alone, F11) can be on with Focus and is left
/// alone by it.
#[test]
fn focus_and_the_other_full_screen_states() {
    let mut s = s();
    run(&mut s, "view.focus", json!({"value": true}));
    run(&mut s, "view.fullScreen", json!({"value": true}));
    assert!(s.view.focus_mode && s.view.full_screen);
    run(&mut s, "view.fullScreen", json!({"value": false}));
    assert!(s.view.focus_mode && !s.view.full_screen, "leaving Full Screen keeps Focus");
    run(&mut s, "view.fullScreenView", json!({"value": true}));
    assert!(!s.view.focus_mode && s.view.full_screen_view, "Full Screen View replaces Focus");
    run(&mut s, "view.focus", json!({"value": true}));
    assert!(s.view.focus_mode && !s.view.full_screen_view, "and Focus replaces it back");
}

/// Read Mode and Immersive Reader are reading views, not screen states: turning either on from
/// Focus leaves Focus on, so the window is in Focus *and* Read Mode at once (the status bar and
/// the ribbon are hidden in Focus, so only a keybinding or control can get there).
#[test]
fn read_mode_and_focus_can_both_be_on() {
    let mut s = s();
    run(&mut s, "view.focus", json!({"value": true}));
    run(&mut s, "view.readMode", json!({}));
    assert!(s.view.read_mode && s.view.focus_mode, "Read Mode leaves Focus on");
    run(&mut s, "view.focus", json!({"value": false}));
    assert!(s.view.read_mode && !s.view.focus_mode, "leaving Focus stays in Read Mode");
    run(&mut s, "view.focus", json!({"value": true}));
    run(&mut s, "view.immersive", json!({}));
    assert!(!s.view.read_mode && s.view.focus_mode, "Immersive Reader off leaves Focus on");
    run(&mut s, "view.immersive", json!({}));
    assert!(s.view.read_mode && s.view.focus_mode, "and on leaves it on");
}

/// The view is per document, as zoom and the layout are: Focus in one document doesn't put
/// another in Focus, and a document opened beside it starts with the view the active one had.
#[test]
fn focus_is_per_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "One"}));
    run(&mut s, "view.focus", json!({"value": true}));
    let second = s.add_document(wordcraft_doc::Document::from_text("Two"));
    assert!(s.view.focus_mode, "the new document opens with the view the active one had");
    s.switch_document(second).unwrap();
    assert!(s.view.focus_mode, "and keeps it");
    run(&mut s, "view.focus", json!({"value": false}));
    assert!(!s.view.focus_mode);
    let first = s.documents().first().map(|d| d.id).unwrap_or(second);
    s.switch_document(first).unwrap();
    assert!(s.view.focus_mode, "the first document is still in Focus");
}

/// A saved and reopened document starts in the usual window: Focus is screen state, never part
/// of the file (Word's `.docx` has no place for it either).
#[test]
fn focus_is_not_saved_in_the_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "view.focus", json!({"value": true}));
    let bytes = crate::io::save_bytes("x.docx", &s.doc).unwrap_or_else(|e| panic!("save: {e}"));
    assert!(!bytes.is_empty(), "the document saves");
    let mut back = Session::new(crate::io::open_bytes("x.docx", &bytes).unwrap_or_else(|e| panic!("reopen: {e}")));
    assert!(!back.view.focus_mode, "a reopened document is not in Focus");
    assert_eq!(run(&mut back, "view.state", json!({}))["focusMode"], false);
}
