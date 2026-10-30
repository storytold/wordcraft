//! The Normal template (#4237): Set As Default saves formatting that every new blank document
//! starts from, whichever way it is made; Reset and a damaged template give the built-in
//! formatting back; headless sessions never write one.

use std::sync::Arc;

use serde_json::json;
use wordcraft_doc::Document;

use crate::Session;
use crate::normal_template::{MemoryStore, NormalStore};

fn session(store: &Arc<MemoryStore>) -> Session {
    let mut s = Session::new(Document::new());
    s.normal_store = Some(store.clone());
    s
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn body_font(d: &Document) -> (String, f32) {
    let r = d.styles.resolve_char(Some("Normal"), &Default::default());
    (r.font.clone(), r.size)
}

/// Font › Set As Default (all new documents) reaches every blank-document path of a new session
/// ("restart"): File › New, closing the last document, an empty file, and plain text / Markdown.
#[test]
fn font_set_as_default_reaches_every_blank_document_path() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia", "size": 14.0}}));
    assert_eq!(body_font(&s.doc), ("Georgia".into(), 14.0), "the open document changes too");
    assert!(store.get().is_some(), "the template is saved");

    // A new session, as after a restart.
    let mut s = session(&store);
    let d = s.normal_document();
    assert_eq!(body_font(&d), ("Georgia".into(), 14.0), "start-up");
    run(&mut s, "file.new", json!({}));
    assert_eq!(body_font(&s.doc), ("Georgia".into(), 14.0), "File › New › Blank");
    run(&mut s, "file.new", json!({"template": "blank"}));
    assert_eq!(body_font(&s.doc), ("Georgia".into(), 14.0), "File › New › Blank, by name");
    for _ in 0..4 {
        run(&mut s, "file.closeDocument", json!({"force": true}));
    }
    assert_eq!(s.documents().len(), 1);
    assert_eq!(body_font(&s.doc), ("Georgia".into(), 14.0), "closing the last document");
    run(&mut s, "file.open", json!({"path": "Empty.docx", "data": ""}));
    assert_eq!(body_font(&s.doc), ("Georgia".into(), 14.0), "an empty file");
    let txt = crate::cmd::insert::base64_encode(b"Plain words\n");
    run(&mut s, "file.open", json!({"path": "notes.txt", "data": txt}));
    assert_eq!(body_font(&s.doc), ("Georgia".into(), 14.0), "a text file");
    let md = crate::cmd::insert::base64_encode(b"# Title\n\nBody text\n");
    run(&mut s, "file.open", json!({"path": "notes.md", "data": md}));
    assert_eq!(body_font(&s.doc), ("Georgia".into(), 14.0), "a Markdown file");

    // Templates and Word documents keep their own formatting.
    run(&mut s, "file.new", json!({"template": "letter"}));
    assert_ne!(body_font(&s.doc).0, "Georgia");
    let docx = crate::io::save_bytes("x.docx", &Document::from_text("Theirs")).unwrap();
    run(&mut s, "file.open", json!({"path": "theirs.docx", "data": crate::cmd::insert::base64_encode(&docx)}));
    assert_ne!(body_font(&s.doc).0, "Georgia", "a .docx keeps its own formatting");
}

/// "This document only" changes the document's defaults and leaves the template alone; the
/// document change is one undo step either way.
#[test]
fn this_document_only_leaves_the_template_alone() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let before = body_font(&s.doc);
    run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia"}, "scope": "document"}));
    assert_eq!(body_font(&s.doc).0, "Georgia");
    assert!(store.get().is_none(), "nothing saved");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_font(&s.doc), before, "one undo step");

    run(&mut s, "para.setDefault", json!({"props": {"spaceAfter": 20.0}}));
    assert!(store.get().is_some());
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(s.doc.styles.default_para.space_after, Some(20.0), "the document change undoes");
    let mut fresh = session(&store);
    let d = fresh.normal_document();
    assert_eq!(d.styles.default_para.space_after, Some(20.0), "the template write does not");
}

/// Each Set As Default re-reads the template and changes only what it sets: two windows (or two
/// sessions) lose nothing but the same property.
#[test]
fn saves_change_only_what_was_set() {
    let store = MemoryStore::new();
    let mut a = session(&store);
    let mut b = session(&store);
    run(&mut a, "format.setDefault", json!({"props": {"font": "Georgia"}}));
    run(&mut b, "para.setDefault", json!({"props": {"spaceAfter": 3.0}}));
    run(&mut b, "layout.margins", json!({"top": 100.0, "bottom": 100.0, "left": 50.0, "right": 50.0}));
    run(&mut b, "layout.setDefault", json!({}));
    let d = session(&store).normal_document();
    assert_eq!(body_font(&d).0, "Georgia", "a's font survives b's saves");
    assert_eq!(d.styles.default_para.space_after, Some(3.0));
    assert_eq!(d.last_section.margin_left, 50.0);
    assert_eq!(d.last_section.margin_top, 100.0);
}

/// Page Setup › Set As Default: paper, orientation, margins, header distance, vertical
/// alignment, first page, odd and even. Without it, a new document keeps the region's paper.
#[test]
fn page_setup_default() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "file.options", json!({"region": "de-DE"}));
    assert_eq!(s.normal_document().last_section.page_w, 595.28, "A4 from the region");
    run(&mut s, "format.setDefault", json!({"props": {"bold": true}}));
    assert_eq!(s.normal_document().last_section.page_w, 595.28, "still the region's: page setup wasn't saved");
    run(&mut s, "layout.size", json!({"name": "Letter"}));
    run(&mut s, "layout.orientation", json!({"value": "landscape"}));
    run(&mut s, "layout.verticalAlign", json!({"value": "center"}));
    run(&mut s, "layout.differentOddEven", json!({"value": true}));
    s.doc.last_section.header = 30.0;
    run(&mut s, "layout.setDefault", json!({"scope": "new"}));
    let d = session(&store).normal_document();
    let sp = &d.last_section;
    assert!(sp.landscape);
    assert_eq!((sp.page_w, sp.page_h), (s.doc.last_section.page_w, s.doc.last_section.page_h));
    assert_eq!(sp.valign, s.doc.last_section.valign);
    assert_eq!(sp.header, 30.0);
    assert!(d.settings.even_odd_headers);
    assert!(d.styles.default_chr.bold == Some(true), "the font default is kept");
}

/// Design › Set as Default saves the theme, style set and paragraph spacing.
#[test]
fn design_set_as_default() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "design.theme", json!({"name": "Studio"}));
    run(&mut s, "design.styleSet", json!({"name": "centered"}));
    run(&mut s, "design.paragraphSpacing", json!({"value": "double"}));
    run(&mut s, "design.setDefault", json!({}));
    let d = session(&store).normal_document();
    assert_eq!(d.settings.theme_name, "Studio");
    assert_eq!(d.settings.minor_font, "Georgia");
    assert_eq!(d.styles.get("Heading1").and_then(|h| h.para.align), Some(wordcraft_doc::Align::Center));
    assert_eq!(d.styles.default_para.line_spacing, Some(wordcraft_doc::props::LineSpacing::Multiple(2.0)));
    // "document" changes nothing.
    let mut other = session(&MemoryStore::new());
    run(&mut other, "design.setDefault", json!({"scope": "document"}));
}

/// Modify Style › New documents based on this template: that style, its base when missing,
/// and the list it numbers with.
#[test]
fn a_style_saved_for_new_documents() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "styles.create", json!({"name": "Callout", "basedOn": "Normal"}));
    run(&mut s, "styles.modify", json!({"style": "Callout", "chr": {"italic": true, "size": 18.0}}));
    run(&mut s, "styles.setDefault", json!({"style": "Callout"}));
    let d = session(&store).normal_document();
    let callout = d.styles.styles.iter().find(|st| st.name == "Callout").expect("saved");
    assert_eq!((callout.chr.italic, callout.chr.size), (Some(true), Some(18.0)));
    assert!(s.run("styles.setDefault", &json!({"style": "No Such Style"})).is_err());
}

/// Saving the same styles again doesn't grow the template's lists: each save copies a style's
/// list afresh, and the old copy, numbered by nothing, goes.
#[test]
fn saving_again_does_not_grow_the_lists() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "styles.create", json!({"name": "Steps", "basedOn": "Normal"}));
    let num = s.doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let id = s.doc.styles.styles.iter().find(|st| st.name == "Steps").map(|st| st.id.clone()).unwrap();
    s.doc.styles.get_mut(&id).unwrap().para.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
    let lists = |store: &MemoryStore| {
        let t = crate::normal_template::parse(&store.get().unwrap()).unwrap();
        (t.numbering.nums.len(), t.numbering.abstracts.len())
    };
    run(&mut s, "design.setDefault", json!({}));
    let first = lists(&store);
    assert_eq!(first, (1, 1), "the style's list is saved");
    for _ in 0..3 {
        run(&mut s, "design.setDefault", json!({}));
        run(&mut s, "styles.setDefault", json!({"style": "Steps"}));
    }
    assert_eq!(lists(&store), first);
    let d = session(&store).normal_document();
    let steps = d.styles.styles.iter().find(|st| st.name == "Steps").unwrap();
    let n = steps.para.numbering.expect("still numbered");
    assert!(d.numbering.num(n.num).is_some(), "its list is there");
}

/// The language is never stored: Proofing Language › Set As Default owns it.
#[test]
fn language_is_not_stored() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia", "lang": "fr-FR"}}));
    let t = crate::normal_template::parse(&store.get().unwrap()).unwrap();
    assert_eq!(t.styles.default_chr.lang, None);
    assert!(t.styles.styles.iter().all(|st| st.chr.lang.is_none()));
    let mut s = session(&store);
    s.prefs.default_language = Some("de-DE".into());
    assert_eq!(s.normal_document().styles.default_chr.lang.as_deref(), Some("de-DE"));
}

/// A damaged, oversized or unreadable template: built-in formatting and a status message, no
/// crash; the next Set As Default replaces it.
#[test]
fn a_damaged_template_gives_built_in_defaults() {
    let builtin = body_font(&Session::new(Document::new()).normal_document());
    for junk in [b"garbage".to_vec(), b"PK\x03\x04garbage".to_vec(), vec![0u8; crate::normal_template::MAX_TEMPLATE_BYTES + 1], Vec::new()] {
        let store = MemoryStore::new();
        store.put(Some(junk));
        let mut s = session(&store);
        let d = s.normal_document();
        assert_eq!(body_font(&d), builtin);
        assert!(s.status.contains("Normal template"), "{}", s.status);
        run(&mut s, "file.new", json!({}));
        run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia"}}));
        assert_eq!(body_font(&session(&store).normal_document()).0, "Georgia", "replaced");
    }

    /// A store that can't be read at all.
    struct Broken;
    impl NormalStore for Broken {
        fn load(&self) -> Result<Option<Vec<u8>>, String> {
            Err("permission denied".into())
        }
        fn save(&self, _: &[u8]) -> Result<(), String> {
            Err("read-only".into())
        }
        fn reset(&self) -> Result<(), String> {
            Err("read-only".into())
        }
    }
    let mut s = Session::new(Document::new());
    s.normal_store = Some(Arc::new(Broken));
    run(&mut s, "file.new", json!({}));
    assert!(s.status.contains("Normal template"));
    let before = s.doc.styles.default_chr.clone();
    assert!(s.run("format.setDefault", &json!({"props": {"font": "Georgia"}})).is_err(), "a failed save is an error");
    assert_eq!(s.doc.styles.default_chr, before, "and the document is left as it was");
    assert!(s.run("defaults.reset", &json!({})).is_err());
}

/// Reset deletes the template: new documents are built in again; open ones keep their formatting.
#[test]
fn reset_restores_built_in_formatting() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    let builtin = body_font(&s.normal_document());
    run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia"}}));
    run(&mut s, "defaults.reset", json!({}));
    assert!(store.get().is_none());
    assert_eq!(body_font(&s.doc).0, "Georgia", "the open document keeps it");
    run(&mut s, "file.new", json!({}));
    assert_eq!(body_font(&s.doc), builtin);
}

/// Headless sessions have no template: "new" is refused, "document" works, nothing is written.
#[test]
fn headless_sessions_never_write_a_template() {
    let mut s = Session::new(Document::new());
    assert!(s.run("format.setDefault", &json!({"props": {"font": "Georgia"}})).is_err());
    assert!(s.run("design.setDefault", &json!({})).is_err());
    assert!(s.run("defaults.reset", &json!({})).is_err());
    run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia"}, "scope": "document"}));
    assert_eq!(body_font(&s.doc).0, "Georgia");
    assert!(s.run("format.setDefault", &json!({"props": {}, "scope": "everywhere"})).is_err());
    assert!(s.run("format.setDefault", &json!({"props": {"size": 1e30}, "scope": "document"})).is_ok());
    assert!(s.doc.styles.default_chr.size.is_some_and(|x| x <= 1638.0), "sizes are clamped");
}

/// Review P2: a Set As Default refused after its handler ran (here by Restrict Editing's
/// post-check: document defaults are outside the editable region) saves nothing in the template.
#[test]
fn a_refused_set_as_default_saves_nothing() {
    let store = MemoryStore::new();
    let mut s = Session::new(Document::from_text("Locked intro.\nFill in here.\n"));
    s.normal_store = Some(store.clone());
    run(&mut s, "select.text", json!({"text": "Fill in here."}));
    run(&mut s, "review.restrict", json!({"exception": "add"}));
    run(&mut s, "review.restrict", json!({"mode": "readOnly"}));
    run(&mut s, "select.text", json!({"text": "here"}));
    let before = s.doc.styles.clone();
    assert!(s.run("format.setDefault", &json!({"props": {"font": "Georgia"}})).is_err());
    assert!(s.run("para.setDefault", &json!({"props": {"spaceAfter": 30.0}})).is_err());
    assert_eq!(s.doc.styles, before, "the document is rolled back");
    assert!(store.get().is_none(), "and the template was never written");
    assert_ne!(body_font(&session(&store).normal_document()).0, "Georgia");
}

/// Review P3: Repeat and a recorded macro never replay a Set As Default or Reset, which would
/// rewrite the person's defaults unasked.
#[test]
fn repeat_and_macros_skip_template_commands() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "tools.recordMacro", json!({"name": "m"}));
    run(&mut s, "text.insert", json!({"text": "Hi "}));
    run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia"}}));
    run(&mut s, "tools.recordMacro", json!({}));
    store.put(None);
    let _ = s.run("edit.repeat", &json!({}));
    assert!(store.get().is_none(), "Repeat didn't save the default again");
    run(&mut s, "tools.macros", json!({"run": "m"}));
    assert!(store.get().is_none(), "nor did the macro");
    assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hi Hi Hi ", "Repeat and the macro typed again");
}

/// Review P3: Page Setup › Set As Default saves the page setup of the caret's section, also when
/// the caret is in a footnote (whose story paths don't index the body).
#[test]
fn page_default_reads_the_carets_section_from_any_story() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "text.insert", json!({"text": "One"}));
    run(&mut s, "layout.margins", json!({"left": 100.0, "right": 100.0}));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "Two"}));
    run(&mut s, "layout.margins", json!({"left": 50.0, "right": 50.0}));
    assert_eq!(s.doc.sections().len(), 2);
    run(&mut s, "references.footnote", json!({}));
    assert_ne!(s.sel.focus.story, wordcraft_doc::StoryRef::Body, "the caret is in the footnote");
    run(&mut s, "layout.setDefault", json!({}));
    assert_eq!(session(&store).normal_document().last_section.margin_left, 50.0, "the second section's, where the note is");
}

/// Review P3: a revision record in the params never reaches the defaults or the template.
#[test]
fn paragraph_defaults_carry_no_revision() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    let props = json!({"spaceAfter": 7.0, "numChange": {"rev": 3, "original": "1."}, "fmtChange": null});
    run(&mut s, "para.setDefault", json!({"props": props}));
    assert!(s.doc.styles.default_para.num_change.is_none());
    let d = session(&store).normal_document();
    assert_eq!(d.styles.default_para.space_after, Some(7.0));
    assert!(d.styles.default_para.num_change.is_none());
    assert!(d.styles.get("Normal").is_some_and(|n| n.para.num_change.is_none()));
}

/// Review P3: the template commands with junk parameters against a template store: no panic,
/// and whatever they save still reads as a template.
#[test]
fn template_commands_survive_hostile_params() {
    let junk = [
        json!(null),
        json!({}),
        json!({"scope": 5}),
        json!({"scope": "new", "props": 7}),
        json!({"scope": "new", "props": {"size": 1e30, "spacing": -1e30, "kern": 1e30, "font": "\u{0}\u{FFFF}", "color": "zz"}}),
        json!({"scope": "new", "props": {"spaceAfter": 1e30, "indentLeft": -1e30, "lineSpacing": {"rule": "multiple", "value": 1e30}, "tabs": [[1e30]]}}),
        json!({"scope": "document", "style": "\u{0}", "props": {"bold": "x"}}),
        json!({"style": 42}),
    ];
    for id in ["format.setDefault", "para.setDefault", "layout.setDefault", "styles.setDefault", "design.setDefault", "defaults.reset"] {
        for j in &junk {
            let store = MemoryStore::new();
            let mut s = Session::new(crate::sample::sample_document());
            s.normal_store = Some(store.clone());
            let _ = s.run(id, j);
            let _ = s.layout();
            if let Some(bytes) = store.get() {
                crate::normal_template::parse(&bytes).unwrap_or_else(|e| panic!("{id} {j}: {e}"));
                let _ = session(&store).normal_document();
            }
        }
    }
}

/// Re-review P2: writing the template reads the stored file, which a person can replace. A panic
/// while reading or merging it is an error that rolls the command back, never an abort.
#[test]
fn a_panic_while_writing_the_template_is_an_error() {
    struct Panics;
    impl NormalStore for Panics {
        fn load(&self) -> Result<Option<Vec<u8>>, String> {
            panic!("damaged template")
        }
        fn save(&self, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn reset(&self) -> Result<(), String> {
            Ok(())
        }
    }
    let mut s = Session::new(Document::from_text("Text.\n"));
    s.normal_store = Some(Arc::new(Panics));
    let before = s.doc.styles.clone();
    assert!(s.run("format.setDefault", &json!({"props": {"font": "Georgia"}})).is_err());
    assert!(s.run("para.setDefault", &json!({"props": {"spaceAfter": 30.0}})).is_err());
    assert_eq!(s.doc.styles, before, "the document is rolled back");
}

/// Re-review P3: a Set As Default inside a macro reaches the template only if the whole macro
/// succeeds; a later failing step leaves the template as it was.
#[test]
fn a_failing_macro_saves_no_default() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    let steps = json!([
        {"command": "format.setDefault", "params": {"props": {"font": "Georgia"}}},
        {"command": "para.setDefault", "params": {"props": {"spaceAfter": 30.0}}},
        {"command": "no.such.command", "params": {}},
    ]);
    run(&mut s, "tools.macros", json!({"define": {"name": "m", "steps": steps}}));
    assert!(s.run("tools.macros", &json!({"run": "m"})).is_err());
    assert!(store.get().is_none(), "the template was never written");
    // The same macro without the failing step does save.
    let steps = json!([{"command": "format.setDefault", "params": {"props": {"font": "Georgia"}}}]);
    run(&mut s, "tools.macros", json!({"define": {"name": "ok", "steps": steps}}));
    run(&mut s, "tools.macros", json!({"run": "ok"}));
    assert_eq!(body_font(&session(&store).normal_document()).0, "Georgia");
}

/// Review of #4449–#4458: with formatting limited to the document's styles, a Set As Default
/// that changes the open document is refused as Modify Style is, whatever its scope, and saves
/// nothing.
#[test]
fn set_as_default_obeys_limit_formatting() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "review.restrict", json!({"mode": "none", "formatting": true}));
    let before = s.doc.styles.clone();
    assert!(s.run("styles.modify", &json!({"style": "Normal", "chr": {"font": "Georgia"}})).is_err(), "Modify Style is refused");
    for scope in ["document", "new"] {
        assert!(s.run("format.setDefault", &json!({"props": {"font": "Georgia"}, "scope": scope})).is_err(), "font, {scope}");
        assert!(s.run("para.setDefault", &json!({"props": {"spaceAfter": 30.0}, "scope": scope})).is_err(), "paragraph, {scope}");
    }
    assert_eq!(s.doc.styles, before, "the document is unchanged");
    assert!(store.get().is_none(), "and nothing is saved");
}

/// Review of #4449–#4458: a numbered style saved for new documents brings its list, so a new
/// document's headings keep their numbers, and the first list made there is a list of its own.
#[test]
fn numbered_styles_keep_their_list_in_new_documents() {
    for command in ["design.setDefault", "styles.setDefault"] {
        let store = MemoryStore::new();
        let mut s = session(&store);
        let num = s.doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        s.doc.styles.get_mut("Heading1").unwrap().para.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
        run(&mut s, command, json!({"style": "Heading1"}));
        let mut fresh = session(&store);
        run(&mut fresh, "file.new", json!({}));
        let heading = fresh.doc.styles.get("Heading1").and_then(|h| h.para.numbering).expect("still numbered");
        assert!(fresh.doc.numbering.num(heading.num).is_some(), "{command}: its list came too");
        run(&mut fresh, "text.insert", json!({"text": "Item"}));
        run(&mut fresh, "para.numbering", json!({}));
        let item = fresh.doc.para_at(&fresh.sel.focus).and_then(|p| p.props.numbering).expect("a list");
        assert_ne!(item.num, heading.num, "{command}: the new list doesn't take the heading's over");
        assert_eq!(fresh.doc.styles.get("Heading1").and_then(|h| h.para.numbering), Some(heading));
    }
}

/// Review of #4449–#4458: every page setup Page Setup accepts comes back as saved: the
/// template is read like any document, with no checks of its own to disagree with.
#[test]
fn page_defaults_keep_every_page_setup_page_setup_accepts() {
    // (width, height, left and right, top and bottom): the largest page, margins leaving just
    // the room for text, and a page the size of a label.
    for (w, h, side, end) in [(1584.0, 1584.0, 0.0, 0.0), (612.0, 792.0, 288.0, 378.0), (1584.0, 1584.0, 774.0, 774.0), (40.0, 40.0, 2.0, 2.0)] {
        let store = MemoryStore::new();
        let mut s = session(&store);
        run(&mut s, "layout.margins", json!({"left": 0.0, "right": 0.0, "top": 0.0, "bottom": 0.0}));
        run(&mut s, "layout.size", json!({"width": w, "height": h}));
        run(&mut s, "layout.margins", json!({"left": side, "right": side, "top": end, "bottom": end}));
        run(&mut s, "layout.setDefault", json!({}));
        let sp = session(&store).normal_document().last_section;
        assert_eq!((sp.page_w, sp.page_h), (w, h));
        assert_eq!((sp.margin_left, sp.margin_right, sp.margin_top, sp.margin_bottom), (side, side, end, end), "{w}×{h}");
    }
}

/// Review of #4449–#4458: Set As Default saves only what was set, on the Normal template's own
/// styles: the open document's other formatting (here a justified Normal from its .docx) stays
/// out of new documents.
#[test]
fn set_as_default_saves_only_what_was_set() {
    let store = MemoryStore::new();
    let mut theirs = Document::from_text("Theirs");
    if let Some(n) = theirs.styles.get_mut("Normal") {
        n.para.align = Some(wordcraft_doc::Align::Justify);
        n.chr.italic = Some(true);
    }
    theirs.styles.default_para.space_before = Some(40.0);
    let docx = crate::io::save_bytes("x.docx", &theirs).unwrap();
    let mut s = session(&store);
    run(&mut s, "file.open", json!({"path": "theirs.docx", "data": crate::cmd::insert::base64_encode(&docx)}));
    run(&mut s, "format.setDefault", json!({"props": {"font": "Georgia"}}));
    run(&mut s, "para.setDefault", json!({"props": {"spaceAfter": 9.0}}));
    let d = session(&store).normal_document();
    let rp = d.styles.resolve_para(&wordcraft_doc::ParaProps { style: Some("Normal".into()), ..Default::default() });
    assert_eq!(body_font(&d).0, "Georgia");
    assert_eq!(rp.space_after, 9.0);
    assert_ne!(rp.align, wordcraft_doc::Align::Justify, "the open document's Normal isn't saved");
    assert_ne!(rp.space_before, 40.0, "nor its defaults");
    assert!(!d.styles.resolve_char(Some("Normal"), &Default::default()).italic);
}

/// #4237 review (as on #4456): Font › Set As Default sets the colour back to automatic for new
/// documents.
#[test]
fn font_default_colour_goes_back_to_automatic() {
    let store = MemoryStore::new();
    let mut s = session(&store);
    run(&mut s, "format.setDefault", json!({"props": {"color": "FF0000"}}));
    let colour = |store: &Arc<MemoryStore>| session(store).normal_document().styles.resolve_char(Some("Normal"), &Default::default()).color;
    assert_eq!(colour(&store), wordcraft_doc::TextColor::Rgb(wordcraft_doc::Rgb(255, 0, 0)));
    run(&mut s, "format.setDefault", json!({"props": {"color": "Auto"}}));
    assert_eq!(colour(&store), wordcraft_doc::TextColor::Auto);
}
