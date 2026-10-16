//! Markdown source view (`view.markdownSource`, beyond Word): a Markdown document edited as the
//! text of its file, the way a code editor shows it, and back.
//!
//! In source view the document's body is the file's text, one paragraph per line in a
//! monospaced face, and [`Settings::markdown_source`](wordcraft_doc::Settings) is set. The flag
//! is part of the document, so Undo steps back across the switch like any edit. Typing there is
//! literal: no AutoFormat or AutoCorrect, and Paste keeps the text only. Saving as `.md` (or
//! `.txt`) writes the text exactly as it stands; other formats save the document the text
//! describes ([`rendered`]). Commands that format, style or insert objects are unavailable
//! ([`refuses`]): only the text is kept.
//!
//! Switching to source view shows the text the file was opened with (or last edited as) while
//! the document is unchanged since; after edits in the formatted view it's the document written
//! as Markdown, as saving would write it.

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{Document, StoryRef};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// The face source view sets text in: a monospaced one each platform has (others fall back to
/// Courier New's substitutes, `wordcraft_fonts::word`).
const SOURCE_FONT: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(windows) {
    "Consolas"
} else {
    "DejaVu Sans Mono"
};
/// Points.
const SOURCE_SIZE: f32 = 10.5;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("view.markdownSource", "Markdown Source", "View › Views", toggle)
            .params(r#"{"value"?: bool (true: show the Markdown text, false: the formatted document; default: switch)}"#)
            .when(available),
    ]
}

/// The text a formatted Markdown document was read from, while it is unchanged since.
#[derive(Clone, Debug)]
pub struct Origin {
    /// The document as it was when it was read from `text`.
    doc: Document,
    text: Arc<str>,
}

fn is_markdown_name(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

/// Is the open document a Markdown file (by its path, or the name given to it before saving)?
fn is_markdown(s: &Session) -> bool {
    match &s.path {
        Some(path) => is_markdown_name(&path.to_string_lossy()),
        None => s.file_name.as_deref().is_some_and(is_markdown_name),
    }
}

fn available(s: &Session) -> Option<&'static str> {
    (!s.doc.settings.markdown_source && !is_markdown(s)).then_some("only Markdown documents (.md) have a source view")
}

fn toggle(s: &mut Session, v: &Value) -> CmdResult {
    let on = s.doc.settings.markdown_source;
    let want = p::bool(v, "value").unwrap_or(!on);
    if want && !on {
        let text = source_of(s);
        s.doc = source_document(&text, &s.doc);
    } else if on && !want {
        let text = source_text(&s.doc);
        let doc = from_text(&text, &s.doc);
        s.markdown_origin = Some(Origin { doc: doc.clone(), text: text.into() });
        s.doc = doc;
    }
    if want != on {
        // Lines and blocks don't correspond: the caret goes to the start, as on open.
        s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
        s.pending = None;
        s.relayout();
    }
    let mut r = sel_result(s)?;
    if let Some(m) = r.as_object_mut() {
        m.insert("value".into(), json!(want));
    }
    Ok(r)
}

/// The Markdown the formatted document is: the text it was read from while it's unchanged
/// since, else the document written as Markdown (what saving it writes).
fn source_of(s: &Session) -> String {
    match &s.markdown_origin {
        Some(o) if same_content(&o.doc, &s.doc) => o.text.to_string(),
        _ => wordcraft_formats::markdown::export(&s.doc),
    }
}

/// Would the two documents write the same Markdown? Their content is the same (save metadata
/// and the saved zoom may differ).
fn same_content(a: &Document, b: &Document) -> bool {
    a.body == b.body && a.parts == b.parts && a.styles == b.styles && a.numbering == b.numbering && a.media == b.media && a.comments == b.comments
}

/// Characters the document model gives a meaning of its own (an object without its object) or
/// that a text file can't hold, left out of source text; tabs, line feeds and form feeds stay.
fn clean(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars().filter(|c| *c != OBJ && (!c.is_control() || matches!(c, '\n' | '\t' | '\u{000C}'))).collect()
}

/// The source view of `text` (Markdown), on `from`'s page, with its document properties and
/// language.
pub(crate) fn source_document(text: &str, from: &Document) -> Document {
    let mut doc = Document::from_text(&clean(text));
    doc.last_section = from.last_section.clone();
    doc.core = from.core.clone();
    doc.styles.default_chr.lang = from.styles.default_chr.lang.clone();
    if let Some(normal) = doc.styles.get_mut("Normal") {
        normal.chr.font = Some(SOURCE_FONT.into());
        normal.chr.size = Some(SOURCE_SIZE);
        normal.para.space_before = Some(0.0);
        normal.para.space_after = Some(0.0);
        normal.para.line_spacing = Some(LineSpacing::Multiple(1.0));
    }
    doc.settings.markdown_source = true;
    doc
}

/// A source-view document's text as it is saved: its lines (tracked deletions left out) joined
/// by line feeds.
pub fn source_text(doc: &Document) -> String {
    let mut text = doc.final_text(StoryRef::Body);
    // Every paragraph, the last one too, ends with a line feed; the last one's isn't in the file.
    if text.ends_with('\n') {
        text.pop();
    }
    text
}

/// The formatted document Markdown `text` describes, on `from`'s page, with its document
/// properties and language.
fn from_text(text: &str, from: &Document) -> Document {
    let mut doc = wordcraft_formats::markdown::import_on(text, &from.last_section);
    crate::io::fit_pictures(&mut doc);
    doc.core = from.core.clone();
    doc.styles.default_chr.lang = from.styles.default_chr.lang.clone();
    doc.ensure_nonempty();
    doc
}

/// The formatted document a source-view document describes: what saving it in a format other
/// than Markdown or plain text writes.
pub fn rendered(doc: &Document) -> Document {
    from_text(&source_text(doc), doc)
}

/// After `file.open` read a Markdown file into the session: keep the file's text, so source view
/// shows it as it is (`data`: the file's bytes in base64, when they came that way).
pub(crate) fn remember_opened(s: &mut Session, path: &str, data: Option<&str>) {
    let bytes = match data {
        Some(d) => super::insert::base64_decode(d),
        None => read(path),
    };
    s.markdown_origin = bytes.map(|b| Origin { doc: s.doc.clone(), text: clean(&wordcraft_formats::txt::decode(&b)).into() });
}

/// The opened file's bytes, through `io` (which keeps automation inside its read root); `None`
/// for a file over 64 MB, which shows the document written as Markdown.
#[cfg(not(target_arch = "wasm32"))]
fn read(path: &str) -> Option<Vec<u8>> {
    crate::io::read_file_capped(path, 64 << 20, "Markdown file").ok()
}

/// On the web files come through the browser (`data`), never by path.
#[cfg(target_arch = "wasm32")]
fn read(_path: &str) -> Option<Vec<u8>> {
    None
}

/// Paste in source view: text only, the system clipboard's own text when it came with some.
pub(crate) fn paste(s: &mut Session, v: &Value) -> CmdResult {
    let text = match p::str(v, "text") {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            let mut t = super::paste::paste_fragment(s, v)?.plain_text();
            // The fragment's last paragraph ends with a line feed whether or not its mark was copied.
            if t.ends_with('\n') {
                t.pop();
            }
            t
        }
    };
    let text = clean(&text);
    if text.is_empty() {
        return Err(CmdError::Failed("the clipboard is empty".into()));
    }
    super::paste_text(s, &text.replace('\n', "\r"))?;
    sel_result(s)
}

/// Text commands that stay unavailable in source view: they insert breaks, formatting or
/// objects rather than text.
const REFUSED: &[&str] = &[
    "text.pageBreak",
    "text.columnBreak",
    "edit.formatPainter",
    "edit.copyFormat",
    "edit.pasteFormat",
    "edit.pasteSpecial",
    "edit.pasteClipboardItem",
    "edit.pasteAllClipboard",
    "tools.templates",
    "tools.attachTemplate",
    "tools.updateStyles",
    "file.encrypt",
];

/// Commands outside the text groups that work on text alone (proofing, page setup) or only
/// report.
const ALLOWED: &[&str] = &[
    "review.spelling",
    "review.issues",
    "review.suggestions",
    "review.applySuggestion",
    "review.thesaurus",
    "review.wordCount",
    "review.readAloud",
    "review.language",
    "review.editor",
    "layout.margins",
    "layout.size",
    "layout.orientation",
    "layout.pageSetup",
    "format.state",
];

/// Why `id` is unavailable in Markdown source view, if it is. Typing, editing, finding, proofing,
/// viewing and saving work there; formatting, styles, tables, objects, comments and the like
/// would be lost when the text is read back as Markdown.
pub(crate) fn refuses(s: &Session, id: &str) -> Option<&'static str> {
    if !s.doc.settings.markdown_source {
        return None;
    }
    let group = id.split('.').next().unwrap_or("");
    let text = matches!(group, "text" | "caret" | "select" | "edit" | "file" | "view" | "tools" | "readAloud" | "document") && !REFUSED.contains(&id);
    (!text && !ALLOWED.contains(&id)).then_some("not available in Markdown source view")
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wordcraft_doc::{Document, StoryRef};

    use crate::{CmdError, Session};

    /// Markdown written the way people do, not the way the exporter would: a setext heading,
    /// `*` bullets, a padded table, a trailing space and no final line feed.
    const FILE: &str = "Kiln notes\n==========\n\n* Load the kiln \n* Fire it\n\n| Cone | Temp |\n|------|------|\n| 6    | hot  |";

    fn run(s: &mut Session, id: &str, v: Value) -> Value {
        s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    /// A session with `text` opened as `notes.md` (the bytes given, as the web app gives them).
    fn opened(text: &str) -> Session {
        let mut s = Session::new(Document::new());
        let data = super::super::insert::base64_encode(text.as_bytes());
        run(&mut s, "file.open", json!({"path": "notes.md", "data": data}));
        s
    }

    fn source(s: &Session) -> String {
        super::source_text(&s.doc)
    }

    fn style_of(s: &Session, i: usize) -> Option<String> {
        s.doc.body.get(i).and_then(|b| b.as_para()).and_then(|p| p.props.style.clone())
    }

    #[test]
    fn source_view_shows_the_file_as_written_and_saves_it_unchanged() {
        let mut s = opened(FILE);
        assert_eq!(style_of(&s, 0).as_deref(), Some("Heading1"), "opens formatted");
        let r = run(&mut s, "view.markdownSource", json!({}));
        assert_eq!(r["value"], json!(true));
        assert!(s.doc.settings.markdown_source);
        assert_eq!(source(&s), FILE, "the file's own text, not the exporter's");
        assert_eq!(s.doc.body.len(), FILE.lines().count(), "one paragraph per line");
        assert_eq!(crate::io::save_bytes("notes.md", &s.doc).unwrap(), FILE.as_bytes());
        assert_eq!(crate::io::save_bytes("notes.txt", &s.doc).unwrap(), FILE.as_bytes());
        // Back and forth without edits: still the file's text.
        run(&mut s, "view.markdownSource", json!({"value": false}));
        assert_eq!(style_of(&s, 0).as_deref(), Some("Heading1"));
        run(&mut s, "view.markdownSource", json!({"value": true}));
        assert_eq!(source(&s), FILE);
        // Asking for the view already shown changes nothing.
        run(&mut s, "view.markdownSource", json!({"value": true}));
        assert_eq!(source(&s), FILE);
    }

    #[test]
    fn source_view_is_set_in_a_monospaced_face_without_paragraph_spacing() {
        let mut s = opened(FILE);
        run(&mut s, "view.markdownSource", json!({}));
        let normal = s.doc.styles.get("Normal").unwrap();
        assert_eq!(normal.chr.font.as_deref(), Some(super::SOURCE_FONT));
        assert_eq!((normal.para.space_before, normal.para.space_after), (Some(0.0), Some(0.0)));
    }

    #[test]
    fn edits_in_either_view_carry_over() {
        let mut s = opened(FILE);
        // Edited formatted: the source is the document written as Markdown.
        run(&mut s, "caret.docEnd", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "Cool slowly."}));
        run(&mut s, "view.markdownSource", json!({}));
        let text = source(&s);
        assert!(text.starts_with("# Kiln notes\n"), "written by the exporter: {text}");
        assert!(text.contains("Cool slowly."), "{text}");
        // Edited as source: the formatted view reads the text.
        run(&mut s, "caret.docEnd", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "## Glazes"}));
        run(&mut s, "view.markdownSource", json!({}));
        assert!(!s.doc.settings.markdown_source);
        let last = s.doc.body.iter().filter_map(|b| b.as_para()).rfind(|p| !p.text.is_empty()).unwrap();
        assert_eq!((last.text.as_str(), last.props.style.as_deref()), ("Glazes", Some("Heading2")));
    }

    #[test]
    fn undo_steps_back_across_the_switch() {
        let mut s = opened(FILE);
        run(&mut s, "view.markdownSource", json!({}));
        run(&mut s, "text.insert", json!({"text": "X"}));
        assert!(source(&s).starts_with("XKiln"));
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(source(&s), FILE);
        run(&mut s, "edit.undo", json!({}));
        assert!(!s.doc.settings.markdown_source, "the formatted document is back");
        assert_eq!(style_of(&s, 0).as_deref(), Some("Heading1"));
        run(&mut s, "edit.redo", json!({}));
        assert!(s.doc.settings.markdown_source);
        assert_eq!(source(&s), FILE);
    }

    #[test]
    fn switching_views_leaves_no_unsaved_changes() {
        let mut s = opened(FILE);
        assert!(!s.dirty);
        run(&mut s, "view.markdownSource", json!({}));
        assert!(!s.dirty, "only the view changed");
        run(&mut s, "text.insert", json!({"text": "X"}));
        assert!(s.dirty);
        run(&mut s, "view.markdownSource", json!({}));
        assert!(s.dirty, "an edit is still unsaved after switching back");
    }

    #[test]
    fn only_markdown_documents_have_a_source_view() {
        let mut s = Session::new(Document::new());
        assert!(matches!(s.run("view.markdownSource", &json!({})), Err(CmdError::Disabled(_))), "a new document isn't Markdown");
        s.path = Some("report.docx".into());
        assert!(matches!(s.run("view.markdownSource", &json!({})), Err(CmdError::Disabled(_))));
        s.path = None;
        s.file_name = Some("README.MD".into());
        run(&mut s, "view.markdownSource", json!({}));
        assert!(s.doc.settings.markdown_source, "a Markdown name given before saving counts");
    }

    #[test]
    fn source_view_offers_only_text_commands() {
        let mut s = opened(FILE);
        run(&mut s, "view.markdownSource", json!({}));
        run(&mut s, "select.all", json!({}));
        for id in [
            "format.bold",
            "para.heading1",
            "styles.apply",
            "insert.table",
            "insert.picture",
            "table.insertRowBelow",
            "review.newComment",
            "edit.formatPainter",
            "text.pageBreak",
        ] {
            let r = s.run(id, &json!({}));
            assert!(matches!(r, Err(CmdError::Disabled(_)) | Err(CmdError::Unknown(_))), "{id}: {r:?}");
        }
        assert_eq!(source(&s), FILE, "nothing was formatted");
        let spec = s.registry.get("format.bold").unwrap();
        assert!(s.unavailable(spec).is_some(), "front ends grey it out");
        for id in ["edit.copy", "review.wordCount", "view.zoom100", "format.state"] {
            run(&mut s, id, json!({}));
        }
        run(&mut s, "edit.replaceAll", json!({"text": "Fire", "with": "Unload"}));
        assert!(source(&s).contains("* Unload it"));
        // Back in the formatted view everything works again.
        run(&mut s, "view.markdownSource", json!({}));
        run(&mut s, "select.all", json!({}));
        run(&mut s, "format.bold", json!({}));
    }

    #[test]
    fn paste_in_source_view_keeps_text_only() {
        let mut s = opened("");
        run(&mut s, "view.markdownSource", json!({}));
        run(&mut s, "edit.paste", json!({"text": "**kept** as typed\nsecond line", "html": "<h1>Big</h1><p>rich</p>"}));
        assert_eq!(source(&s), "**kept** as typed\nsecond line");
        assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.props.style.is_none() && p.objects.is_empty()));
        run(&mut s, "edit.pasteMerge", json!({"html": "<p><b>only</b> html</p>"}));
        assert_eq!(source(&s), "**kept** as typed\nsecond lineonly html");
        assert!(s.run("edit.paste", &json!({"html": "<p> </p>"})).is_err(), "nothing to paste");
    }

    #[test]
    fn saving_in_another_format_saves_the_document_the_text_describes() {
        let mut s = opened(FILE);
        run(&mut s, "view.markdownSource", json!({}));
        let html = String::from_utf8(crate::io::save_bytes("notes.html", &s.doc).unwrap()).unwrap();
        assert!(html.contains("<h1") && html.contains("<table"), "{html}");
        assert!(!html.contains("=========="), "not the source text");
        let docx = crate::io::save_bytes("notes.docx", &s.doc).unwrap();
        let back = crate::io::open_bytes("notes.docx", &docx).unwrap();
        assert!(!back.settings.markdown_source, "Word files never carry the flag");
        assert_eq!(back.body.first().and_then(|b| b.as_para()).and_then(|p| p.props.style.as_deref()), Some("Heading1"));
        // Native JSON (recovery) keeps the view as it is.
        let json = crate::io::save_bytes("notes.json", &s.doc).unwrap();
        let back = crate::io::open_bytes("notes.json", &json).unwrap();
        assert!(back.settings.markdown_source);
        assert_eq!(super::source_text(&back), FILE);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_file_opened_by_path_saves_back_byte_for_byte() {
        let dir = std::env::temp_dir().join(format!("wordcraft-md-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("README.md");
        std::fs::write(&path, FILE).unwrap();
        let mut s = Session::new(Document::new());
        run(&mut s, "file.open", json!({"path": path.to_string_lossy()}));
        run(&mut s, "view.markdownSource", json!({}));
        assert_eq!(source(&s), FILE);
        run(&mut s, "caret.docEnd", json!({}));
        run(&mut s, "text.insert", json!({"text": "!"}));
        run(&mut s, "file.save", json!({}));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{FILE}!"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_file_text_never_breaks_the_source_view() {
        // An object character with no object, stray controls, CR line ends, a BOM.
        let text = "\u{FEFF}# A\u{FFFC}b\r\nc\u{0}d\u{7}\re\u{000C}f\t\u{2028}";
        let mut s = opened(text);
        run(&mut s, "view.markdownSource", json!({}));
        let src = source(&s);
        assert!(!src.contains('\u{FFFC}') && !src.contains('\u{0}') && !src.contains('\u{FEFF}'), "{src:?}");
        assert!(src.starts_with("# Ab\ncd\ne\u{000C}f\t"), "{src:?}");
        for v in [json!({"value": "yes"}), json!({"value": null}), json!([1, 2]), json!(null), json!({"value": 7})] {
            let _ = s.run("view.markdownSource", &v);
            let _ = s.layout();
        }
        let _ = s.doc.plain_text(StoryRef::Body);
    }
}
