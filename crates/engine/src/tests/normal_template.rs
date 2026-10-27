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
