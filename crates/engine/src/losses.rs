//! What saving in a format that can't hold everything would lose (#1316). Saving to RTF or ODT
//! makes that file the document's own (it is "saved", like a .docx), so before the user does it
//! the app warns about the features the writer drops, and scripts get the list in `file.save`'s
//! result.
//!
//! Each format has its own list, because each writer drops different things. The RTF writer
//! ([`wordcraft_formats::rtf::export`]) keeps comments, footnotes and endnotes, tracked changes
//! (insertions, deletions, formatting changes) and sections with their columns; it drops headers
//! and footers, content controls, equations' layout, charts and SmartArt, shapes, fields' codes,
//! hidden text, phonetic guides, the watermark and the pictures it can't encode
//! ([`wordcraft_formats::rtf::can_write_picture`]). The ODT writer
//! ([`wordcraft_formats::odt::export`]) keeps all of those but a paragraph's tracked formatting
//! changes, SmartArt as an editable diagram (it writes a group of shapes), most content controls
//! (only plain text and choice controls become form controls) and embedded objects other than
//! those read from ODF files. Neither writes tracked changes of a table's, row's or cell's look or
//! of the page setup, macros or custom XML parts. The tests save and reopen a document with each
//! feature, so a writer that gains one (or loses one) fails them until this list follows (#3747,
//! where #1316 was re-filed).

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
    Watermark,
    Macros,
    Other,
}

impl Loss {
    /// Every loss, in the order they are listed.
    pub const ALL: [Loss; 17] = [
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
        Loss::Watermark,
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
            Loss::Watermark => "watermark",
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
            Loss::Watermark => "Watermark",
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
/// never become the document's file (.txt, .pdf…); RTF and ODT list what their own writer drops.
pub fn format_losses(doc: &Document, name: &str) -> Vec<Loss> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let ext = file.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    let format = match ext.as_str() {
        "rtf" => Format::Rtf,
        "odt" => Format::Odt,
        _ => return Vec::new(),
    };
    Walk { doc, format: Some(format) }.run().into_iter().collect()
}

/// A format with a list of its own.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Rtf,
    Odt,
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

/// The document being checked and the format. Each check names who drops the feature with
/// [`Self::drops`]; with no format every feature counts (the tests' inventory of what a document
/// has).
struct Walk<'a> {
    doc: &'a Document,
    format: Option<Format>,
}

impl Walk<'_> {
    /// Whether the feature is lost: `rtf` and `odt` say whether each writer drops it. Which one
    /// does follows the writers ([`wordcraft_formats::rtf::export`], [`wordcraft_formats::odt::export`]);
    /// the tests save and reopen a document with each feature, so a writer that gains one fails them.
    fn drops(&self, rtf: bool, odt: bool) -> bool {
        match self.format {
            Some(Format::Rtf) => rtf,
            Some(Format::Odt) => odt,
            None => true,
        }
    }

    fn run(&self) -> BTreeSet<Loss> {
        let (doc, mut found) = (self.doc, BTreeSet::new());
        // RTF has no headers or footers; ODT writes each master page's. A control in any story is its
        // text. Notes and comments are written by both, so what they hold is checked like the body.
        for part in doc.parts.values() {
            let header = matches!(part.kind, PartKind::Header | PartKind::Footer);
            if header && self.drops(true, false) && has_content(&part.blocks, 0) {
                found.insert(Loss::HeadersFooters);
            }
            if matches!(part.kind, PartKind::TextBox) {
                continue;
            }
            self.blocks(&part.blocks, 0, &mut found);
            wordcraft_doc::control::walk_controls(&part.blocks, &mut |_| {
                found.insert(Loss::ContentControls);
            });
        }
        // Both write comments (RTF as annotations) and notes; kept for the tests' inventory.
        if self.drops(false, false) && !doc.comments.is_empty() {
            found.insert(Loss::Comments);
        }
        // RTF writes no watermark (ODT writes one as a shape in the headers, as Word does).
        if self.drops(true, false) && doc.settings.watermark.as_ref().is_some_and(|w| !w.text.trim().is_empty()) {
            found.insert(Loss::Watermark);
        }
        // A macro-enabled document's macros live in a package part only Word packages have.
        if doc.passthrough.contains_key(VBA_PROJECT_PART) {
            found.insert(Loss::Macros);
        }
        // Both write sections with their page setup and columns, so these are never lost; the check
        // is kept for the tests' inventory.
        if self.drops(false, false) && doc.last_section.columns.count > 1 {
            found.insert(Loss::Sections);
        }
        // A tracked change of the page setup is written by neither.
        if doc.last_section.fmt_change.is_some() {
            found.insert(Loss::TrackedChanges);
        }
        // Custom XML parts (data bound to content controls, add-in data) live only in Word packages.
        if doc.passthrough.keys().any(|k| k.starts_with("customXml/")) {
            found.insert(Loss::Other);
        }
        self.blocks(&doc.body, 0, &mut found);
        // A control other than the plain text and choice controls (and any block-level one) is its
        // text in ODT, whose form controls are those; RTF has none.
        wordcraft_doc::control::walk_controls(&doc.body, &mut |c| {
            if self.drops(true, c.block || !wordcraft_formats::model::plain_control(&c.kind)) {
                found.insert(Loss::ContentControls);
            }
        });
        found
    }

    fn blocks(&self, bl: &Blocks, depth: usize, found: &mut BTreeSet<Loss>) {
        if depth > WALK_DEPTH {
            return;
        }
        for b in bl {
            match &**b {
                Block::Para(p) => self.para(p, found),
                Block::Table(t) => {
                    // A tracked change of a table's, row's or cell's look is written by neither.
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
        // A paragraph that ends a section other than the last: a section break (both write it).
        if self.drops(false, false) && p.section.is_some() {
            found.insert(Loss::Sections);
        }
        // Insertions, deletions, paragraph mark revisions and the formatting changes of runs are
        // written by both; RTF also writes a paragraph's formatting changes, ODT doesn't.
        if (p.props.fmt_change.is_some() || p.props.num_change.is_some()) && self.drops(false, true) {
            found.insert(Loss::TrackedChanges);
        }
        if self.drops(false, false) && (p.mark.ins.is_some() || p.mark.del.is_some()) {
            found.insert(Loss::TrackedChanges);
        }
        for r in &p.runs {
            if self.drops(false, false) && (r.props.ins.is_some() || r.props.del.is_some() || r.props.fmt_change.is_some()) {
                found.insert(Loss::TrackedChanges);
            }
            // RTF leaves hidden text out; ODT keeps it hidden.
            if r.props.hidden == Some(true) && self.drops(true, false) {
                found.insert(Loss::HiddenText);
            }
        }
        for o in &p.objects {
            let loss = match o {
                InlineObject::Image { media, ole, .. } => {
                    // The picture is written, the object it shows isn't (ODT writes only the objects
                    // it read from ODF files).
                    if ole.as_ref().is_some_and(|e| self.drops(true, !e.is_odf_object())) {
                        found.insert(Loss::EmbeddedObjects);
                    }
                    // RTF leaves out pictures it can neither write as they are nor re-encode as PNG (SVG).
                    match self.doc.media.get(media) {
                        Some(data)
                            if self.format == Some(Format::Rtf)
                                && !wordcraft_formats::rtf::can_write_picture(&wordcraft_formats::model::media_ext(media, data), data) =>
                        {
                            Loss::Pictures
                        }
                        _ => continue,
                    }
                }
                // ODT writes a chart as a chart; a diagram (SmartArt) it writes as a group of shapes, and
                // RTF as alt text, which it leaves out.
                InlineObject::Graphic { graphic, .. } => {
                    if !self.drops(true, graphic.chart.is_none()) {
                        continue;
                    }
                    Loss::Charts
                }
                // RTF writes drawings as nothing; ODT writes them.
                InlineObject::Shape { .. } | InlineObject::Group { .. } if self.drops(true, false) => Loss::Shapes,
                // A field's current text is written by RTF, its code isn't; ODT writes fields.
                InlineObject::Field { .. } | InlineObject::FieldStart { .. } if self.drops(true, false) => Loss::Fields,
                // RTF writes an equation's linear text: its layout is lost. ODT writes a formula object.
                InlineObject::Equation { .. } if self.drops(true, false) => Loss::Equations,
                // RTF writes the base text, the ruby over it isn't; ODT writes ruby.
                InlineObject::Ruby { .. } if self.drops(true, false) => Loss::PhoneticGuides,
                InlineObject::NoteRef { .. } if self.drops(false, false) => Loss::Notes,
                InlineObject::CommentStart { .. } | InlineObject::CommentEnd { .. } if self.drops(false, false) => Loss::Comments,
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

    /// Every feature of `doc` that some writer drops (the check without a format's own knowledge).
    fn inventory(doc: &Document) -> BTreeSet<Loss> {
        Walk { doc, format: None }.run()
    }

    /// `doc` saved as `ext` and opened again.
    fn round_trip(doc: &Document, ext: &str) -> Document {
        let name = format!("x.{ext}");
        let bytes = crate::io::save_bytes(&name, doc).unwrap();
        crate::io::open_bytes(&name, &bytes).unwrap()
    }

    /// A table, `change` applied to the table inside it.
    fn table_with(change: impl Fn(&mut wordcraft_doc::table::Table)) -> Document {
        let mut doc = with(&[("insert.table", json!({"rows": 2, "cols": 2}))]);
        for b in doc.body.iter_mut() {
            if let Block::Table(t) = std::sync::Arc::make_mut(b) {
                change(t);
            }
        }
        doc
    }

    /// A one-column document with a tracked change of its page setup.
    fn section_change() -> Document {
        let mut doc = session().doc;
        let old = doc.last_section.clone();
        doc.last_section.fmt_change = wordcraft_doc::props::PropChange::boxed(0, old);
        doc
    }

    /// A document with a plain text content control in its header.
    fn control_in_header() -> Document {
        let mut doc = with(&[("insert.header", json!({"text": "Head"}))]);
        let control = Box::new(wordcraft_doc::control::ContentControl {
            kind: wordcraft_doc::control::ControlKind::Text { multi_line: false },
            ..Default::default()
        });
        for part in doc.parts.values_mut().filter(|p| p.kind == PartKind::Header) {
            for b in part.blocks.iter_mut() {
                if let Block::Para(p) = std::sync::Arc::make_mut(b) {
                    p.insert_object(0, InlineObject::ControlStart { control: control.clone() }, &Default::default()).unwrap();
                    let end = p.text.chars().count();
                    p.insert_object(end, InlineObject::ControlEnd, &Default::default()).unwrap();
                }
            }
        }
        doc
    }

    /// A document whose footnote holds `obj`.
    fn in_note(obj: InlineObject) -> Document {
        let mut doc = with(&[("references.footnote", json!({"text": "A note"}))]);
        for part in doc.parts.values_mut().filter(|p| p.kind == PartKind::Footnote) {
            for b in part.blocks.iter_mut() {
                if let Block::Para(p) = std::sync::Arc::make_mut(b) {
                    p.insert_object(0, obj.clone(), &Default::default()).unwrap();
                }
            }
        }
        doc
    }

    /// A document whose header holds a SmartArt diagram (ODT keeps headers, but not this).
    fn smart_art_in_header() -> Document {
        let diagram = with(&[("insert.smartArt", json!({"layout": "process"}))]);
        let obj = diagram.body.iter().find_map(|b| b.as_para()?.objects.iter().find(|o| matches!(o, InlineObject::Graphic { .. })).cloned());
        let mut doc = with(&[("insert.header", json!({"text": "Head"}))]);
        for part in doc.parts.values_mut().filter(|p| p.kind == PartKind::Header) {
            for b in part.blocks.iter_mut() {
                if let (Block::Para(p), Some(o)) = (std::sync::Arc::make_mut(b), obj.clone()) {
                    p.insert_object(0, o, &Default::default()).unwrap();
                }
            }
        }
        doc
    }

    fn with_passthrough(key: &str, bytes: &[u8]) -> Document {
        let mut doc = session().doc;
        doc.passthrough.insert(key.into(), std::sync::Arc::new(bytes.to_vec()));
        doc
    }

    /// One case per feature: the feature's loss, a document with it, and whether the RTF and the ODT
    /// writer drop it. The document is saved and opened again, so the truth comes from the writers:
    /// a writer that starts keeping a feature fails the case until its entry changes.
    fn cases() -> Vec<(&'static str, Loss, Document, bool, bool)> {
        use wordcraft_doc::props::PropChange;
        let track = || ("review.trackChanges", json!({"value": true}));
        let all = || ("select.all", json!({}));
        vec![
            ("inserted text", Loss::TrackedChanges, with(&[track(), ("text.insert", json!({"text": "new"}))]), false, false),
            ("deleted text", Loss::TrackedChanges, with(&[track(), all(), ("text.delete", json!({}))]), false, false),
            ("inserted paragraph mark", Loss::TrackedChanges, with(&[track(), ("text.newParagraph", json!({}))]), false, false),
            ("tracked bold", Loss::TrackedChanges, with(&[track(), all(), ("format.bold", json!({}))]), false, false),
            ("tracked alignment", Loss::TrackedChanges, with(&[track(), ("para.alignCenter", json!({}))]), false, true),
            ("tracked bullets", Loss::TrackedChanges, with(&[track(), ("para.bullets", json!({}))]), false, true),
            ("tracked table look", Loss::TrackedChanges, table_with(|t| t.props.fmt_change = PropChange::boxed(0, t.props.clone())), true, true),
            (
                "tracked row look",
                Loss::TrackedChanges,
                table_with(|t| t.rows[0].props.fmt_change = PropChange::boxed(0, t.rows[0].props.clone())),
                true,
                true,
            ),
            (
                "tracked cell look",
                Loss::TrackedChanges,
                table_with(|t| t.rows[0].cells[0].props.fmt_change = PropChange::boxed(0, t.rows[0].cells[0].props.clone())),
                true,
                true,
            ),
            ("tracked page setup", Loss::TrackedChanges, section_change(), true, true),
            ("comment", Loss::Comments, with(&[all(), ("review.newComment", json!({"text": "Check this"}))]), false, false),
            ("footnote", Loss::Notes, with(&[("references.footnote", json!({"text": "A note"}))]), false, false),
            ("endnote", Loss::Notes, with(&[("references.endnote", json!({"text": "A note"}))]), false, false),
            ("header", Loss::HeadersFooters, with(&[("insert.header", json!({"text": "Running head"}))]), true, false),
            ("footer", Loss::HeadersFooters, with(&[("insert.footer", json!({"text": "Page foot"}))]), true, false),
            ("page number", Loss::HeadersFooters, with(&[("insert.pageNumber", json!({}))]), true, false),
            ("rich text control", Loss::ContentControls, with(&[("developer.richText", json!({"title": "Name"}))]), true, true),
            ("plain text control", Loss::ContentControls, with(&[("developer.plainText", json!({"title": "Name"}))]), true, false),
            ("check box control", Loss::ContentControls, with(&[("developer.checkBox", json!({}))]), true, false),
            ("control in a header", Loss::ContentControls, control_in_header(), true, true),
            ("equation", Loss::Equations, with(&[("insert.equation", json!({"linear": "a^2+b^2=c^2", "edit": false}))]), true, false),
            ("chart", Loss::Charts, with(&[("insert.chart", json!({"type": "pie"}))]), true, false),
            ("smart art", Loss::Charts, with(&[("insert.smartArt", json!({"layout": "process"}))]), true, true),
            ("shape", Loss::Shapes, with(&[("insert.shape", json!({"kind": "rectangle"}))]), true, false),
            ("text box", Loss::Shapes, with(&[("insert.textBox", json!({"text": "Boxed"}))]), true, false),
            ("field", Loss::Fields, with(&[("insert.field", json!({"instr": "AUTHOR", "result": "Me"}))]), true, false),
            (
                "field in a footnote",
                Loss::Fields,
                in_note(InlineObject::Field {
                    instr: "AUTHOR".into(),
                    result: "Me".into(),
                    locked: false,
                    dirty: false,
                    code: false,
                    form: String::new(),
                }),
                true,
                false,
            ),
            ("smart art in a header", Loss::Charts, smart_art_in_header(), true, true),
            ("hidden text", Loss::HiddenText, with(&[all(), ("format.hidden", json!({"value": true}))]), true, false),
            ("section break", Loss::Sections, with(&[("layout.break", json!({"kind": "nextPage"}))]), false, false),
            ("columns", Loss::Sections, with(&[("layout.columns", json!({"count": 2}))]), false, false),
            ("phonetic guide", Loss::PhoneticGuides, with(&[all(), ("format.phonetic", json!({"ruby": "ruby"}))]), true, false),
            ("watermark", Loss::Watermark, with(&[("design.watermark", json!({"text": "DRAFT"}))]), true, false),
            ("macros", Loss::Macros, with_passthrough("word/vbaProject.bin", &[1, 2, 3]), true, true),
            ("custom XML", Loss::Other, with_passthrough("customXml/item1.xml", b"<data/>"), true, true),
            ("embedded object", Loss::EmbeddedObjects, word_object(), true, true),
        ]
    }

    /// A document with a picture standing for a Word embedded object (OLE).
    fn word_object() -> Document {
        let mut doc = session().doc;
        doc.media.insert("media/p.png".into(), std::sync::Arc::new(vec![0x89, b'P', b'N', b'G']));
        let img = InlineObject::Image {
            media: "media/p.png".into(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: Some(std::sync::Arc::new(wordcraft_doc::graphic::Embedded::default())),
            fx: Default::default(),
        };
        let mut s = Session::new(doc);
        let at = s.sel.focus.clone();
        s.doc.insert_object(&at, img, &Default::default()).unwrap();
        s.doc
    }

    /// ODT writes back the objects it read from ODF files (their files travel with the document);
    /// RTF writes only their pictures.
    #[test]
    fn an_odf_object_is_kept_by_odt_only() {
        let mut doc = word_object();
        let odf = std::sync::Arc::new(wordcraft_doc::graphic::Embedded {
            xml: format!("{} />", wordcraft_doc::graphic::Embedded::ODF_OBJECT),
            refs: Vec::new(),
        });
        assert!(odf.is_odf_object());
        for b in doc.body.iter_mut() {
            if let Block::Para(p) = std::sync::Arc::make_mut(b) {
                for o in p.objects.iter_mut() {
                    if let InlineObject::Image { ole, .. } = o {
                        *ole = Some(odf.clone());
                    }
                }
            }
        }
        assert_eq!(format_losses(&doc, "o.rtf"), vec![Loss::EmbeddedObjects]);
        assert_eq!(format_losses(&doc, "o.odt"), Vec::<Loss>::new());
    }

    /// Each feature is listed for the format whose writer drops it and for no other: asserted
    /// against a real export and import, so the list can't drift from the writers (#3747).
    #[test]
    fn each_feature_is_listed_exactly_where_its_writer_drops_it() {
        for (what, loss, doc, rtf_drops, odt_drops) in cases() {
            assert!(inventory(&doc).contains(&loss), "{what}: the document has {loss:?}");
            for (ext, drops) in [("rtf", rtf_drops), ("odt", odt_drops)] {
                let back = round_trip(&doc, ext);
                assert_eq!(!inventory(&back).contains(&loss), drops, "{what} in .{ext}: the writer {} it", if drops { "keeps" } else { "drops" });
                for name in [format!("x.{ext}"), format!("/some/dir/X.{}", ext.to_uppercase())] {
                    assert_eq!(format_losses(&doc, &name).contains(&loss), drops, "{what}: {name}");
                }
            }
            for name in ["x.docx", "x.dotm", "x.xml", "x.json"] {
                assert_eq!(format_losses(&doc, name), Vec::<Loss>::new(), "{name} keeps everything");
            }
        }
    }

    /// A diagram in a table nested past the writers' flattening depth is still lost, so still listed.
    #[test]
    fn a_diagram_in_deeply_nested_tables_is_listed() {
        let mut doc = with(&[("insert.smartArt", json!({"layout": "process"}))]);
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

    /// A document with one picture of these bytes.
    fn picture(key: &str, bytes: &[u8]) -> Document {
        let mut doc = session().doc;
        doc.media.insert(key.into(), std::sync::Arc::new(bytes.to_vec()));
        let img = InlineObject::Image {
            media: key.into(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
            fx: Default::default(),
        };
        let mut s = Session::new(doc);
        let at = s.sel.focus.clone();
        s.doc.insert_object(&at, img, &Default::default()).unwrap();
        s.doc
    }

    /// RTF writes PNG, JPEG and metafiles as they are, re-encodes other pictures it can read and
    /// leaves out the rest (SVG); ODT stores the bytes as they are.
    #[test]
    fn pictures_rtf_cannot_hold_are_listed_for_rtf_only() {
        let emf = [
            1u8, 0, 0, 0, 0x6c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b' ', b'E',
            b'M', b'F',
        ];
        let wmf = [0xD7u8, 0xCD, 0xC6, 0x9A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"/>".to_vec();
        // An EMF, and a WMF with records after its placeable header, are written as metafiles (#4259).
        let mut wmf_body = wmf.to_vec();
        wmf_body.extend([1u8, 0, 9, 0, 0, 3, 0, 0]);
        for (key, bytes) in [("media/a.emf", emf.to_vec()), ("media/e.wmf", wmf_body)] {
            assert_eq!(format_losses(&picture(key, &bytes), "p.rtf"), Vec::<Loss>::new(), "{key}");
        }
        // A placeable header alone and SVG have nothing RTF can hold.
        for (key, bytes) in [("media/b.wmf", wmf.to_vec()), ("media/c.svg", svg)] {
            let doc = picture(key, &bytes);
            assert_eq!(format_losses(&doc, "p.rtf"), vec![Loss::Pictures], "{key}");
            assert_eq!(format_losses(&doc, "p.odt"), Vec::<Loss>::new(), "{key}: ODT keeps the bytes");
        }
        // A picture whose header declares more pixels than the writer will decode (a BMP of 100 000
        // pixels each way) is left out by the writer too, so it is listed. Only the header is read
        // (the check runs after every edit): a GIF cut short after a good header isn't found.
        let mut bmp = b"BM".to_vec();
        bmp.extend([0u8; 12]);
        bmp.extend([40u8, 0, 0, 0]);
        bmp.extend(100_000i32.to_le_bytes());
        bmp.extend(100_000i32.to_le_bytes());
        bmp.extend([1u8, 0, 24, 0]);
        bmp.extend([0u8; 24]);
        let (key, doc) = ("media/h.bmp", picture("media/h.bmp", &bmp));
        assert!(!crate::io::save_bytes("p.rtf", &doc).unwrap().windows(5).any(|w| w == b"\\pict"), "{key}: the writer drops it");
        assert_eq!(format_losses(&doc, "p.rtf"), vec![Loss::Pictures], "{key}");
        assert_eq!(format_losses(&doc, "p.odt"), Vec::<Loss>::new(), "{key}: ODT keeps the bytes");
        // PNG goes into RTF as it is.
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        assert_eq!(format_losses(&picture("media/d.png", &png), "p.rtf"), Vec::<Loss>::new());
    }

    #[test]
    fn the_list_has_no_duplicates_and_follows_the_order_of_all() {
        let mut doc = with(&[("developer.richText", json!({"title": "One"})), ("developer.richText", json!({"title": "Two"}))]);
        doc.passthrough.insert("word/vbaProject.bin".into(), std::sync::Arc::new(vec![1]));
        for name in ["a.rtf", "a.odt"] {
            assert_eq!(format_losses(&doc, name), vec![Loss::ContentControls, Loss::Macros], "{name}");
        }
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
            s.run("developer.richText", &json!({"title": "Name"})).unwrap();
            let r = s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
            assert_eq!(r["saved"], true, "{ext}");
            assert_eq!(r["losses"], json!(["contentControls"]), "{ext}");
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
