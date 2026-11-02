//! Legal symbols: commands, shortcuts, spacing and DOCX round-trip.

use crate::MODULE;
use crate::symbols::{PALETTE, fix_sign_spacing_in_str, sign_spacing_space_ranges};
use crate::tests::roundtrip;
use crate::tests::{law, run};
use serde_json::json;
use wordcraft_doc::para::NBSP;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_engine::{CmdError, Selection, Session};

fn press(s: &mut Session, combo: &str) -> Result<serde_json::Value, CmdError> {
    let id = s.registry.by_shortcut(combo).map(|c| c.id).unwrap_or_else(|| panic!("nothing bound to {combo}"));
    s.run(id, &json!({}))
}

#[test]
fn every_palette_command_is_installed() {
    let s = law();
    for entry in PALETTE {
        assert!(s.registry.get(entry.command).is_some(), "missing {}", entry.command);
    }
}

#[test]
fn every_symbol_command_inserts_its_glyph() {
    let mut s = law();
    for entry in PALETTE {
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, entry.command, json!({}));
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert!(text.ends_with(entry.glyph), "{} should insert {}", entry.command, entry.glyph);
    }
}

#[test]
fn law_symbol_shortcuts_do_not_conflict_with_core() {
    let core = wordcraft_engine::cmd::registry();
    let s = law();
    let mut seen = std::collections::HashMap::new();
    for c in s.registry.all() {
        if !c.id.starts_with("law.symbols.") {
            continue;
        }
        for k in c.shortcut.split(" / ").filter(|k| !k.is_empty()) {
            let key = k.to_ascii_lowercase();
            if let Some(other) = core.by_shortcut(k).map(|c| c.id) {
                panic!("{k} is already bound in core to {other}");
            }
            if let Some(other) = seen.insert(key, c.id) {
                panic!("{k} is bound to both {other} and {}", c.id);
            }
        }
    }
    assert_eq!(s.registry.by_shortcut("Mod+Alt+4").map(|c| c.id), Some("law.symbols.section"));
    assert_eq!(s.registry.by_shortcut("Mod+Alt+Shift+4").map(|c| c.id), Some("law.symbols.sections"));
    assert_eq!(s.registry.by_shortcut("Mod+Alt+6").map(|c| c.id), Some("law.symbols.paragraph"));
    assert_eq!(s.registry.by_shortcut("Mod+Alt+Shift+6").map(|c| c.id), Some("law.symbols.paragraphs"));
}

#[test]
fn law_symbol_shortcuts_insert_through_dispatch() {
    let mut s = law();
    press(&mut s, "Mod+Alt+4").unwrap();
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().text, "§");
    press(&mut s, "Mod+Alt+Shift+4").unwrap();
    assert!(s.doc.para_at(&s.sel.focus).unwrap().text.ends_with("§§"));
    press(&mut s, "Mod+Alt+6").unwrap();
    assert!(s.doc.para_at(&s.sel.focus).unwrap().text.ends_with("¶"));
    press(&mut s, "Mod+Alt+Shift+6").unwrap();
    assert!(s.doc.para_at(&s.sel.focus).unwrap().text.ends_with("¶¶"));
}

#[test]
fn section_sign_inserts_with_nonbreaking_space_before_a_digit() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "RSMo "}));
    run(&mut s, "law.symbols.section", json!({}));
    for ch in "1.010".chars() {
        run(&mut s, "text.insert", json!({"text": ch.to_string()}));
    }
    let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert_eq!(text, format!("RSMo §{NBSP}1.010"), "{text}");
}

/// Inserting a sign at the caret when the text already begins with " 1.010" must turn that
/// ordinary space into an NBSP instead of leaving a breakable space between sign and number.
#[test]
fn inserting_signs_before_existing_space_and_number_uses_nbsp() {
    for (command, glyph) in
        [("law.symbols.section", "§"), ("law.symbols.sections", "§§"), ("law.symbols.paragraph", "¶"), ("law.symbols.paragraphs", "¶¶")]
    {
        let mut s = law();
        run(&mut s, "text.insert", json!({"text": " 1.010"}));
        s.sel = Selection::caret(Pos::body(0, 0));
        run(&mut s, command, json!({}));
        let text = s.doc.para_at(&Pos::body(0, 0)).unwrap().text.clone();
        assert_eq!(text, format!("{glyph}{NBSP}1.010"), "{command}: {text}");
        // The caret sits after the wider NBSP, so the next keystroke continues after it.
        run(&mut s, "text.insert", json!({"text": "X"}));
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("{glyph}{NBSP}X1.010"), "{command} caret placement");
        // The whole insertion is one undo step and restores exactly what the user had.
        assert!(s.undo(), "{command} typing is undoable");
        assert!(s.undo(), "{command} is undoable");
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, " 1.010", "{command} undo restores what the user typed");

        let mut s = law();
        run(&mut s, "text.insert", json!({"text": " 1.010"}));
        s.sel = Selection::caret(Pos::body(0, 0));
        run(&mut s, command, json!({}));
        let doc = roundtrip(&s);
        let text = doc.para_at(&Pos::body(0, 0)).unwrap().text.clone();
        assert_eq!(text, format!("{glyph}{NBSP}1.010"), "{command} NBSP survives DOCX: {text}");
    }
}

#[test]
fn inserting_sign_before_space_and_number_keeps_the_space_formatting() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": " 1"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    s.sel = Selection::caret(Pos::body(0, 0));
    run(&mut s, "law.symbols.section", json!({}));
    let para = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(para.text, format!("§{NBSP}1"));
    assert!(para.runs.iter().any(|r| r.props.bold.is_some_and(|b| b)), "the replaced space keeps its formatting");
}

#[test]
fn fix_spacing_command_and_docx_roundtrip() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "See § 1.010 and §§ 2."}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert_eq!(text, format!("See §{NBSP}1.010 and §§{NBSP}2."));
    let doc = roundtrip(&s);
    let text = doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap().text.clone();
    assert_eq!(text, format!("See §{NBSP}1.010 and §§{NBSP}2."), "NBSP survives DOCX: {text}");
}

#[test]
fn fix_spacing_undo_restores_the_ordinary_space() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "See § 1"}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("See §{NBSP}1"));
    assert!(s.undo(), "the spacing fix is undoable");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "See § 1");
    assert!(s.redo());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("See §{NBSP}1"));
}

#[test]
fn fix_spacing_respects_selection_within_one_paragraph() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "A § 1 and B § 2"}));
    let para = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let left = para.text.find("§ 1").unwrap();
    let right = left + "§ 1".len();
    s.sel = Selection { anchor: Pos::body(0, left), focus: Pos::body(0, right) };
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "selection"}));
    let text = s.doc.para_at(&Pos::body(0, 0)).unwrap().text.clone();
    assert_eq!(text, format!("A §{NBSP}1 and B § 2"));
}

#[test]
fn fix_spacing_respects_selection_across_paragraphs() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "§ 2"}));
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(2, 4) };
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "selection"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"));
    assert_eq!(s.doc.para_at(&Pos::body(2, 0)).unwrap().text, format!("§{NBSP}2"));
}

#[test]
fn fix_spacing_preserves_character_formatting() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    let para = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(para.text, format!("§{NBSP}1"));
    assert!(para.runs.iter().any(|r| r.props.bold.is_some_and(|b| b)), "bold run survives spacing fix");
    let doc = roundtrip(&s);
    let para = doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(para.text, format!("§{NBSP}1"));
    assert!(para.runs.iter().any(|r| r.props.bold.is_some_and(|b| b)));
}

#[test]
fn typing_section_uses_nbsp_and_sec_is_unchanged() {
    let mut s = law();
    s.prefs.autocorrect.enabled = true;
    for ch in "§ 1.010".chars() {
        run(&mut s, "text.insert", json!({"text": ch.to_string()}));
    }
    let typed = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert_eq!(typed, format!("§{NBSP}1.010"));
    // `sec.` typed one character at a time must survive the same single-character AutoFormat hook.
    // (Word's own sentence capitalisation is turned off so the test isolates the legal spacing.)
    let mut s2 = law();
    s2.prefs.autocorrect.enabled = true;
    s2.prefs.autocorrect.cap_sentences = false;
    for ch in "sec. 1.010".chars() {
        run(&mut s2, "text.insert", json!({"text": ch.to_string()}));
    }
    let sec = s2.doc.para_at(&s2.sel.focus).unwrap().text.clone();
    assert_eq!(sec, "sec. 1.010");
}

#[test]
fn fix_spacing_maps_the_selection_so_typing_replaces_it() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "selection"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"));
    // The selection still covers the phrase even though the NBSP grew a byte: typing replaces it.
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "X", "typing replaces the whole selection");
    // Undo restores the fixed phrase, redo replaces it again.
    assert!(s.undo());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"));
    assert!(s.redo());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "X");
}

#[test]
fn fix_spacing_document_scope_keeps_the_caret_after_changed_text() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    // A caret at the end of the paragraph must move past the wider NBSP, not slip inside it.
    s.sel = Selection::caret(Pos::body(0, 4));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    run(&mut s, "text.insert", json!({"text": "2"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}12"));
}

#[test]
fn document_spacing_fixes_non_body_stories_too() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    let note = run(&mut s, "references.footnote", json!({"text": "¶ 4"}))["id"].as_u64().expect("note id") as u32;
    let header = run(&mut s, "insert.header", json!({"text": "¶ 2"}))["story"].as_u64().expect("header id") as u32;
    let footer = run(&mut s, "insert.footer", json!({"text": "§ 3"}))["story"].as_u64().expect("footer id") as u32;
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    assert!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text.starts_with(&format!("§{NBSP}1")), "body");
    assert!(s.doc.plain_text(StoryRef::Part(note)).contains(&format!("¶{NBSP}4")), "footnote");
    assert_eq!(s.doc.plain_text(StoryRef::Part(header)), format!("¶{NBSP}2"), "header");
    assert_eq!(s.doc.plain_text(StoryRef::Part(footer)), format!("§{NBSP}3"), "footer");
}

/// Body paragraph 0 and a header paragraph 0 share `Path([0])`, so mapping a replaced space by
/// path alone moves a header caret that this command never edited (a header `§ 1` and a body
/// `§ 1` used to be indistinguishable). The story must match too.
#[test]
fn document_spacing_does_not_move_a_caret_in_another_story_with_the_same_path() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    let header = run(&mut s, "insert.header", json!({"text": "Header text"}))["story"].as_u64().expect("header id") as u32;
    // A caret inside the header, past the body's space offset but at the same path.
    s.sel = Selection::caret(Pos { story: StoryRef::Part(header), path: Path::top(0), off: 4 });
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert_eq!(s.doc.para(StoryRef::Part(header), &Path::top(0)).unwrap().text, "HeadXer text", "the header caret stays put");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"), "the body is still fixed");
}

#[test]
fn legal_autoformat_does_not_run_when_law_is_disabled() {
    crate::symbols::register_text_hook();
    let _enabled = law();
    let mut disabled = Session::new(Document::new());
    disabled.prefs.autocorrect.enabled = true;
    assert!(!disabled.module_enabled(MODULE));
    for ch in "§ 1".chars() {
        disabled.run("text.insert", &json!({"text": ch.to_string()})).unwrap();
    }
    let text = disabled.doc.para_at(&disabled.sel.focus).unwrap().text.clone();
    assert_eq!(text, "§ 1");
}

#[test]
fn fix_sign_spacing_string_cases() {
    let (_, ch) = fix_sign_spacing_in_str("¶ 9");
    assert!(ch);
    assert!(sign_spacing_space_ranges("A § 1 B", 0, 3).is_empty());
    assert_eq!(sign_spacing_space_ranges("A § 1 B", 2, 7).len(), 1);
}

/// A long run of signs with no following space-plus-digit must be skipped in one step. Scanning
/// the tail once per sign was quadratic and could freeze `law.symbols.fixSpacing`; this completes
/// immediately only when the whole run is consumed at once.
#[test]
fn fix_sign_spacing_long_run_does_not_rescan() {
    let long = "§".repeat(100_000);
    let (out, changed) = fix_sign_spacing_in_str(&long);
    assert!(!changed);
    assert_eq!(out.chars().count(), 100_000);
    assert!(sign_spacing_space_ranges(&long, 0, long.len()).is_empty());

    let mixed = format!("{} 1", "¶".repeat(100_000));
    let (out, changed) = fix_sign_spacing_in_str(&mixed);
    assert!(changed);
    assert_eq!(out, format!("{}{NBSP}1", "¶".repeat(100_000)));
    assert_eq!(sign_spacing_space_ranges(&mixed, 0, mixed.len()).len(), 1);
}

#[test]
fn the_law_tab_holds_the_symbols_while_lawcraft_is_on() {
    let mut s = law();
    let tab = s.ribbon.tabs().iter().find(|t| t.name == crate::RIBBON_TAB).cloned().expect("the Law tab");
    assert!(tab.custom && !tab.hidden);
    assert_eq!(tab.groups[0].commands, ["law.pleadings.new"], "New Pleading leads the Law tab");
    let symbols = tab.groups.iter().find(|g| g.name == "Legal Symbols").expect("symbols group");
    assert_eq!(symbols.commands, PALETTE.iter().map(|e| e.command).collect::<Vec<_>>());
    assert!(serde_json::to_value(&s.ribbon).unwrap().get("tabs").is_none(), "the tab isn't saved with the user's layout");
    s.prefs.modules.insert(MODULE.into(), false);
    crate::install(&mut s);
    assert!(s.ribbon.tabs().iter().all(|t| t.name != crate::RIBBON_TAB), "gone once LawCraft is off");
}

/// With Track Changes on, the spacing fix and the typing correction are tracked changes: Reject
/// All puts the ordinary spaces back, Accept All keeps the non-breaking ones.
#[test]
fn spacing_corrections_are_tracked_changes_while_tracking() {
    for (answer, want) in [("review.rejectAll", "See § 1.010 and ¶ 4."), ("review.acceptAll", "See §\u{a0}1.010 and ¶\u{a0}4.")] {
        let mut s = law();
        run(&mut s, "text.insert", json!({"text": "See § 1.010 and ¶ 4."}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
        assert!(!s.doc.revisions.is_empty(), "recorded as revisions");
        run(&mut s, answer, json!({}));
        assert_eq!(s.doc.plain_text(StoryRef::Body), want, "{answer}");
    }
    for (answer, want) in [("review.rejectAll", "§ "), ("review.acceptAll", "§\u{a0}1")] {
        let mut s = law();
        s.prefs.autocorrect.enabled = true;
        run(&mut s, "text.insert", json!({"text": "§ "}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        run(&mut s, "text.insert", json!({"text": "1"}));
        run(&mut s, answer, json!({}));
        assert_eq!(s.doc.plain_text(StoryRef::Body), want, "typing {answer}");
    }
}

/// A sign inserted before an existing space and number while tracking: the space's change to a
/// non-breaking one is tracked like the sign, so Reject All gives back exactly what was there.
#[test]
fn sign_before_existing_space_is_tracked_while_tracking() {
    for (command, glyph) in [("law.symbols.section", "§"), ("law.symbols.paragraphs", "¶¶")] {
        for (answer, want) in [("review.rejectAll", " 1.010".to_string()), ("review.acceptAll", format!("{glyph}{NBSP}X1.010"))] {
            let mut s = law();
            run(&mut s, "text.insert", json!({"text": " 1.010"}));
            run(&mut s, "review.trackChanges", json!({"value": true}));
            s.sel = Selection::caret(Pos::body(0, 0));
            run(&mut s, command, json!({}));
            // The caret sits after the new non-breaking space.
            run(&mut s, "text.insert", json!({"text": "X"}));
            run(&mut s, answer, json!({}));
            assert_eq!(s.doc.plain_text(StoryRef::Body), want, "{command} {answer}");
        }
    }
}
