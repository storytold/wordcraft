//! What saving in a format that can't hold everything would lose (#1316). Saving to RTF or ODT
//! makes that file the document's own (it is "saved", like a .docx), so before the user does it
//! the app warns about the features the writer drops, and scripts get the list in `file.save`'s
//! result.
//!
//! The list follows the writers, not guesses: [`wordcraft_formats::rtf::export`] and
//! [`wordcraft_formats::odt::export`] both write [`wordcraft_formats::model::from_doc`]'s flow,
//! which holds only the body's paragraphs, tables, pictures, bookmarks and links. Both write
//! equations as their linear text (`model::equations_as_text`) and leave charts out
//! (`Inline::Figure`); RTF also leaves out pictures it can't re-encode
//! ([`wordcraft_formats::rtf::can_write_picture`]).

use std::collections::BTreeSet;

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Block, Blocks, Document, Paragraph, PartKind};

/// A feature a format can't keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Loss {
    TrackedChanges,
    Comments,
    Notes,
    HeadersFooters,
    ContentControls,
    Equations,
    Charts,
    Pictures,
    Shapes,
    EmbeddedObjects,
    Fields,
    HiddenText,
    Sections,
    PhoneticGuides,
    Macros,
    Other,
}

impl Loss {
    /// Every loss, in the order they are listed.
    pub const ALL: [Loss; 16] = [
        Loss::TrackedChanges,
        Loss::Comments,
        Loss::Notes,
        Loss::HeadersFooters,
        Loss::ContentControls,
        Loss::Equations,
        Loss::Charts,
        Loss::Pictures,
        Loss::Shapes,
        Loss::EmbeddedObjects,
        Loss::Fields,
        Loss::HiddenText,
        Loss::Sections,
        Loss::PhoneticGuides,
        Loss::Macros,
        Loss::Other,
    ];

    /// The id scripts see in `file.save`'s `losses`.
    pub fn id(self) -> &'static str {
        match self {
            Loss::TrackedChanges => "trackedChanges",
            Loss::Comments => "comments",
            Loss::Notes => "notes",
            Loss::HeadersFooters => "headersFooters",
            Loss::ContentControls => "contentControls",
            Loss::Equations => "equations",
            Loss::Charts => "charts",
            Loss::Pictures => "pictures",
            Loss::Shapes => "shapes",
            Loss::EmbeddedObjects => "embeddedObjects",
            Loss::Fields => "fields",
            Loss::HiddenText => "hiddenText",
            Loss::Sections => "sections",
            Loss::PhoneticGuides => "phoneticGuides",
            Loss::Macros => "macros",
            Loss::Other => "other",
        }
    }

    /// What the user is told, in plain English (the interface translates it).
    pub fn label(self) -> &'static str {
        match self {
            Loss::TrackedChanges => "Tracked changes",
            Loss::Comments => "Comments",
            Loss::Notes => "Footnotes and endnotes",
            Loss::HeadersFooters => "Headers and footers",
            Loss::ContentControls => "Content controls",
            Loss::Equations => "Equations (written as plain text)",
            Loss::Charts => "Charts and SmartArt",
            Loss::Pictures => "Pictures RTF can't hold (such as EMF, WMF and SVG)",
            Loss::Shapes => "Shapes, text boxes and drawings",
            Loss::EmbeddedObjects => "Embedded objects (their pictures are kept)",
            Loss::Fields => "Fields (their current text is kept)",
            Loss::HiddenText => "Hidden text",
            Loss::Sections => "Section breaks and columns",
            Loss::PhoneticGuides => "Phonetic guides",
            Loss::Macros => "Macros",
            Loss::Other => "Other content from the original file",
        }
    }

    /// The loss with this [`Self::id`].
    pub fn from_id(id: &str) -> Option<Loss> {
        Loss::ALL.into_iter().find(|l| l.id() == id)
    }
}

/// What saving `doc` as `name` (a file name or path; the extension picks the format) would lose.
/// Empty for formats that keep everything (.docx and its kin, .xml, .json) and for copies that
/// never become the document's file (.txt, .pdf…); RTF and ODT list what their writers drop.
pub fn format_losses(doc: &Document, name: &str) -> Vec<Loss> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let ext = file.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "rtf" | "odt") {
        return Vec::new();
    }
    let mut found = BTreeSet::new();
    // Neither writer has comments (Print Markup appends them only to HTML, Markdown and text).
    if !doc.comments.is_empty() {
        found.insert(Loss::Comments);
    }
    // Only the body is written: no headers, footers, notes or text box stories.
    if doc.parts.values().any(|p| matches!(p.kind, PartKind::Header | PartKind::Footer) && has_content(&p.blocks, 0)) {
        found.insert(Loss::HeadersFooters);
    }
    // A macro-enabled document's macros live in a package part only Word packages have.
    if doc.passthrough.contains_key(VBA_PROJECT_PART) {
        found.insert(Loss::Macros);
    }
    // One section's page size and margins are written, not its columns or other sections.
    if doc.last_section.columns.count > 1 {
        found.insert(Loss::Sections);
    }
    // A tracked change of the page setup.
    if doc.last_section.fmt_change.is_some() {
        found.insert(Loss::TrackedChanges);
    }
    // Custom XML parts (data bound to content controls, add-in data) live only in Word packages.
    if doc.passthrough.keys().any(|k| k.starts_with("customXml/")) {
        found.insert(Loss::Other);
    }
    let w = Walk { doc, rtf: ext == "rtf" };
    w.blocks(&doc.body, 0, &mut found);
    wordcraft_doc::control::walk_controls(&doc.body, &mut |_| {
        found.insert(Loss::ContentControls);
    });
    found.into_iter().collect()
}

/// The part name Word packages keep macros under (`wordcraft_docx::package::VBA_PROJECT_PART`).
const VBA_PROJECT_PART: &str = "word/vbaProject.bin";

/// Deepest table nesting looked into (hostile documents). The writers write tables nested deeper
/// flattened into their cells' paragraphs, so features that deep go unlisted (rare, odd files).
const MAX_DEPTH: usize = wordcraft_formats::model::MAX_DEPTH;

/// Deepest nesting `Walk` follows. The writers flatten tables nested past `MAX_DEPTH` into their
/// cells' paragraphs but still write the cells' objects, so a chart or shape that deep is still
/// lost and is looked for; only a hostile document goes past this.
const WALK_DEPTH: usize = 64;

/// The document being checked and the format.
struct Walk<'a> {
    doc: &'a Document,
    rtf: bool,
}

impl Walk<'_> {
    fn blocks(&self, bl: &Blocks, depth: usize, found: &mut BTreeSet<Loss>) {
        if depth > WALK_DEPTH {
            return;
        }
        for b in bl {
            match &**b {
                Block::Para(p) => self.para(p, found),
                Block::Table(t) => {
                    let changed = t.props.fmt_change.is_some()
                        || t.rows.iter().any(|r| r.props.fmt_change.is_some() || r.cells.iter().any(|c| c.props.fmt_change.is_some()));
                    if changed {
                        found.insert(Loss::TrackedChanges);
                    }
                    for r in &t.rows {
                        for c in &r.cells {
                            self.blocks(&c.blocks, depth + 1, found);
                        }
                    }
                }
            }
        }
    }

    fn para(&self, p: &Paragraph, found: &mut BTreeSet<Loss>) {
        // A paragraph that ends a section other than the last: a section break.
        if p.section.is_some() {
            found.insert(Loss::Sections);
        }
        // Deleted text is left out and inserted text written as plain text; formatting changes and
        // paragraph mark revisions aren't written.
        if p.props.fmt_change.is_some() || p.props.num_change.is_some() || p.mark.ins.is_some() || p.mark.del.is_some() {
            found.insert(Loss::TrackedChanges);
        }
        for r in &p.runs {
            if r.props.ins.is_some() || r.props.del.is_some() || r.props.fmt_change.is_some() {
                found.insert(Loss::TrackedChanges);
            }
            // Hidden text is left out.
            if r.props.hidden == Some(true) {
                found.insert(Loss::HiddenText);
            }
        }
        for o in &p.objects {
            let loss = match o {
                // The picture is written, the object it shows isn't.
                InlineObject::Image { ole: Some(_), .. } => Loss::EmbeddedObjects,
                // RTF leaves out pictures it can't re-encode as PNG (EMF, WMF, SVG).
                InlineObject::Image { media, .. } if self.rtf => match self.doc.media.get(media) {
                    Some(data) if !wordcraft_formats::rtf::can_write_picture(&wordcraft_formats::model::media_ext(media, data), data) => {
                        Loss::Pictures
                    }
                    _ => continue,
                },
                // Charts and SmartArt become alt text, which both writers leave out.
                InlineObject::Graphic { .. } => Loss::Charts,
                InlineObject::Shape { .. } | InlineObject::Group { .. } => Loss::Shapes,
                // A field's current text is written; its code isn't.
                InlineObject::Field { .. } | InlineObject::FieldStart { .. } => Loss::Fields,
                InlineObject::ControlStart { .. } => Loss::ContentControls,
                InlineObject::NoteRef { .. } => Loss::Notes,
                InlineObject::CommentStart { .. } | InlineObject::CommentEnd { .. } => Loss::Comments,
                // Both writers write an equation's linear text: its layout is lost.
                InlineObject::Equation { .. } => Loss::Equations,
                // The base text is written, the ruby over it isn't.
                InlineObject::Ruby { .. } => Loss::PhoneticGuides,
                // Content kept from the source file only as its text.
                InlineObject::Opaque { .. } => Loss::Other,
                _ => continue,
            };
            found.insert(loss);
        }
    }
}

/// Whether a header or footer shows anything (text or an object such as a page number).
fn has_content(bl: &Blocks, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && bl.iter().any(|b| match &**b {
            Block::Para(p) => !p.text.trim().is_empty(),
            Block::Table(t) => t.rows.iter().any(|r| r.cells.iter().any(|c| has_content(&c.blocks, depth + 1))),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    fn session() -> Session {
        let mut s = Session::new(Document::new());
        s.run("text.insert", &json!({"text": "Plain words "})).unwrap();
        s
    }

    fn with(cmds: &[(&str, serde_json::Value)]) -> Document {
        let mut s = session();
        for (id, v) in cmds {
            s.run(id, v).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        s.doc
    }

    #[test]
    fn a_plain_document_loses_nothing() {
        let doc = session().doc;
        for name in ["a.rtf", "a.odt", "a.docx", "a.json", "a.xml"] {
            assert_eq!(format_losses(&doc, name), Vec::<Loss>::new(), "{name}");
        }
        // Lists, tables, links, bookmarks and pictures go through both writers.
        let doc = with(&[
            ("insert.table", json!({"rows": 2, "cols": 2})),
            ("insert.link", json!({"url": "https://example.com", "text": "site"})),
            ("insert.bookmark", json!({"name": "here"})),
        ]);
        for name in ["a.rtf", "a.odt"] {
            assert_eq!(format_losses(&doc, name), Vec::<Loss>::new(), "{name}");
        }
    }

    #[test]
    fn each_feature_the_writers_drop_is_listed() {
        let cases: Vec<(Loss, Vec<(&str, serde_json::Value)>)> = vec![
            (Loss::TrackedChanges, vec![("review.trackChanges", json!({"value": true})), ("text.insert", json!({"text": "new"}))]),
            (Loss::Comments, vec![("select.all", json!({})), ("review.newComment", json!({"text": "Check this"}))]),
            (Loss::Notes, vec![("references.footnote", json!({"text": "A note"}))]),
            (Loss::Notes, vec![("references.endnote", json!({"text": "A note"}))]),
            (Loss::HeadersFooters, vec![("insert.header", json!({"text": "Running head"}))]),
            (Loss::HeadersFooters, vec![("insert.footer", json!({"text": "Page foot"}))]),
            (Loss::ContentControls, vec![("developer.richText", json!({"title": "Name"}))]),
            (Loss::Equations, vec![("insert.equation", json!({"linear": "a^2+b^2=c^2", "edit": false}))]),
            (Loss::Charts, vec![("insert.chart", json!({"type": "pie"}))]),
            (Loss::Charts, vec![("insert.smartArt", json!({"layout": "process"}))]),
            (Loss::Shapes, vec![("insert.shape", json!({"kind": "rectangle"}))]),
            (Loss::Shapes, vec![("insert.textBox", json!({"text": "Boxed"}))]),
            (Loss::Fields, vec![("insert.field", json!({"instr": "AUTHOR", "result": "Me"}))]),
            (Loss::HiddenText, vec![("select.all", json!({})), ("format.hidden", json!({"value": true}))]),
            (Loss::Sections, vec![("layout.break", json!({"kind": "nextPage"}))]),
            (Loss::Sections, vec![("layout.columns", json!({"count": 2}))]),
            (Loss::PhoneticGuides, vec![("select.all", json!({})), ("format.phonetic", json!({"ruby": "ruby"}))]),
        ];
        for (loss, cmds) in cases {
            let doc = with(&cmds);
            for name in ["x.rtf", "X.ODT", "/some/dir/x.rtf"] {
                assert!(format_losses(&doc, name).contains(&loss), "{name}: {cmds:?} → {loss:?}, got {:?}", format_losses(&doc, name));
            }
            for name in ["x.docx", "x.dotm", "x.xml", "x.json"] {
                assert_eq!(format_losses(&doc, name), Vec::<Loss>::new(), "{name} keeps everything");
            }
        }
    }

    /// A chart in a table nested past the writers' flattening depth is still lost, so still listed.
    #[test]
    fn a_chart_in_deeply_nested_tables_is_listed() {
        let mut doc = with(&[("insert.chart", json!({"type": "pie"}))]);
        let mut inner = doc.body.clone();
        for _ in 0..(MAX_DEPTH + 2) {
            let mut t = wordcraft_doc::table::Table::new(1, 1, 100.0);
            t.rows[0].cells[0].blocks = inner;
            inner = vec![std::sync::Arc::new(Block::Table(t))];
        }
        doc.body = inner;
        for name in ["n.rtf", "n.odt"] {
            assert!(format_losses(&doc, name).contains(&Loss::Charts), "{name}");
        }
    }

    #[test]
    fn macros_and_embedded_objects_are_listed() {
        let mut doc = session().doc;
        doc.passthrough.insert("word/vbaProject.bin".into(), std::sync::Arc::new(vec![1, 2, 3]));
        assert_eq!(format_losses(&doc, "m.rtf"), vec![Loss::Macros]);
        let mut doc = session().doc;
        doc.media.insert("media/p.png".into(), std::sync::Arc::new(vec![0x89, b'P', b'N', b'G']));
        let ole = std::sync::Arc::new(wordcraft_doc::graphic::Embedded::default());
        let img = InlineObject::Image {
            media: "media/p.png".into(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: Some(ole),
        };
        let mut s = Session::new(doc);
        let at = s.sel.focus.clone();
        s.doc.insert_object(&at, img, &Default::default()).unwrap();
        assert_eq!(format_losses(&s.doc, "m.odt"), vec![Loss::EmbeddedObjects]);
    }

    /// A document with one picture of these bytes.
    fn picture(key: &str, bytes: &[u8]) -> Document {
        let mut doc = session().doc;
        doc.media.insert(key.into(), std::sync::Arc::new(bytes.to_vec()));
        let img =
            InlineObject::Image { media: key.into(), w: 10.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
        let mut s = Session::new(doc);
        let at = s.sel.focus.clone();
        s.doc.insert_object(&at, img, &Default::default()).unwrap();
        s.doc
    }

    /// RTF re-encodes pictures other than PNG and JPEG and leaves out those it can't read (EMF,
    /// WMF, SVG); ODT stores the bytes as they are.
    #[test]
    fn pictures_rtf_cannot_hold_are_listed_for_rtf_only() {
        let emf = [
            1u8, 0, 0, 0, 0x6c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b' ', b'E',
            b'M', b'F',
        ];
        let wmf = [0xD7u8, 0xCD, 0xC6, 0x9A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"/>".to_vec();
        for (key, bytes) in [("media/a.emf", emf.to_vec()), ("media/b.wmf", wmf.to_vec()), ("media/c.svg", svg)] {
            let doc = picture(key, &bytes);
            assert_eq!(format_losses(&doc, "p.rtf"), vec![Loss::Pictures], "{key}");
            assert_eq!(format_losses(&doc, "p.odt"), Vec::<Loss>::new(), "{key}: ODT keeps the bytes");
        }
        // PNG goes into RTF as it is.
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        assert_eq!(format_losses(&picture("media/d.png", &png), "p.rtf"), Vec::<Loss>::new());
    }

    /// A tracked change of the page setup and custom XML parts aren't written either.
    #[test]
    fn section_changes_and_custom_xml_are_listed() {
        let mut doc = session().doc;
        let old = doc.last_section.clone();
        doc.last_section.fmt_change = wordcraft_doc::props::PropChange::boxed(0, old);
        assert_eq!(format_losses(&doc, "s.rtf"), vec![Loss::TrackedChanges]);
        let mut doc = session().doc;
        doc.passthrough.insert("customXml/item1.xml".into(), std::sync::Arc::new(b"<data/>".to_vec()));
        assert_eq!(format_losses(&doc, "s.odt"), vec![Loss::Other]);
    }

    #[test]
    fn the_list_has_no_duplicates_and_follows_the_order_of_all() {
        let doc = with(&[
            ("references.footnote", json!({"text": "One"})),
            ("references.footnote", json!({"text": "Two"})),
            ("select.all", json!({})),
            ("review.newComment", json!({"text": "Hm"})),
        ]);
        assert_eq!(format_losses(&doc, "a.rtf"), vec![Loss::Comments, Loss::Notes]);
    }

    /// Scripts never see a dialog: `file.save` to RTF/ODT saves as before (the file becomes the
    /// document's, nothing left unsaved) and says what the format couldn't keep.
    #[test]
    fn file_save_reports_what_rtf_and_odt_lose() {
        let dir = std::env::temp_dir().join(format!("wordcraft-losses-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for ext in ["rtf", "odt"] {
            let path = dir.join(format!("notes.{ext}"));
            let mut s = session();
            s.run("select.all", &json!({})).unwrap();
            s.run("review.newComment", &json!({"text": "Check"})).unwrap();
            let r = s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
            assert_eq!(r["saved"], true, "{ext}");
            assert_eq!(r["losses"], json!(["comments"]), "{ext}");
            assert!(!s.dirty, "{ext}: saved");
            assert_eq!(s.path.as_deref(), Some(path.as_path()), "{ext}");
            assert!(path.exists());

            let mut s = session();
            let r = s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
            assert_eq!(r["losses"], json!([]), "{ext}: a plain document loses nothing");
        }
        let mut s = session();
        let r = s.run("file.save", &json!({"path": dir.join("notes.docx").to_string_lossy()})).unwrap();
        assert!(r.get("losses").is_none(), "a Word document keeps everything");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_and_labels_are_distinct() {
        let ids: std::collections::HashSet<_> = Loss::ALL.iter().map(|l| l.id()).collect();
        let labels: std::collections::HashSet<_> = Loss::ALL.iter().map(|l| l.label()).collect();
        assert_eq!(ids.len(), Loss::ALL.len());
        assert_eq!(labels.len(), Loss::ALL.len());
    }
}
