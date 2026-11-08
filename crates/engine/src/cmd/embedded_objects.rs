//! Embedded objects an output doesn't keep (#5393).
//!
//! An OLE object (`w:object`: a Word document, a PDF or a workbook put in with Insert › Object) or
//! an OpenDocument object is drawn as the picture the file keeps for it, as Word draws it, and a
//! save to the object's own kind of file writes the object back. A PDF, HTML or RTF copy holds
//! only that picture; plain text, Markdown, LaTeX and Word 2003 XML not even that, a placeholder
//! at most. A Word file keeps OLE objects but not OpenDocument ones, an ODT file the other way
//! round. The save and export commands name the objects their output doesn't keep in their
//! result (`notShown`), and `wordcraft-cli convert`, `run --save` and `text` warn about them, so
//! the loss is never silent.

use wordcraft_doc::graphic::Embedded;
use wordcraft_doc::{Document, InlineObject, StoryRef};

/// Groups nested deeper than this aren't looked into (hostile files).
const MAX_GROUP_DEPTH: usize = 64;

/// The longest program id kept; a longer one is reported as `object`.
const MAX_PROG_ID: usize = 128;

/// What an output format keeps of a document's embedded objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// Every object: JSON, or not a copy of the whole document (a page image shows one page).
    All,
    /// OLE objects, which Word itself makes (a Word package or Word XML Document). An
    /// OpenDocument object is kept as its picture.
    Ole,
    /// OpenDocument objects (ODT); an OLE object is kept as its picture.
    OpenDocument,
    /// Each object's picture and nothing inside it (PDF, HTML, RTF).
    Pictures,
    /// Not even the picture, a placeholder at most (plain text, Markdown, LaTeX, Word 2003 XML).
    Nothing,
}

impl Kept {
    /// What a save to a file with extension `ext` keeps (`word2003`: an `.xml` file written as a
    /// Word 2003 XML Document). Extensions are matched as the savers match them, aliases
    /// included (`htm`, `markdown`, `latex`…).
    pub fn by(ext: &str, word2003: bool) -> Kept {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        if wordcraft_docx::Flavor::from_ext(&ext).is_some() {
            return Kept::Ole;
        }
        match ext.as_str() {
            "xml" if word2003 => Kept::Nothing,
            "xml" => Kept::Ole,
            "pdf" => Kept::Pictures,
            "png" | "json" => Kept::All,
            other => match wordcraft_formats::export_format(other) {
                Some("html" | "rtf") => Kept::Pictures,
                Some("odt" | "ott" | "fodt") => Kept::OpenDocument,
                // Plain text, Markdown, LaTeX, and any format added later until it says otherwise.
                Some(_) => Kept::Nothing,
                // Nothing is written in a format no saver knows.
                None => Kept::All,
            },
        }
    }

    /// How a save result names what is left of the objects it doesn't keep (`notShownAs`).
    pub fn shown_as(self) -> &'static str {
        if self == Kept::Nothing { "nothing" } else { "picture" }
    }
}

/// An embedded object: what it is called and whether it is an OpenDocument object.
struct Object {
    name: String,
    odf: bool,
}

/// Every embedded object in `doc` (body, headers and footers, notes, comments, text boxes,
/// groups), in document order. A signature line, kept the same way, holds no other document and
/// isn't listed.
fn objects(doc: &Document) -> Vec<Object> {
    let stories = std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k)));
    let mut out = Vec::new();
    for story in stories {
        for path in doc.para_paths(story) {
            if let Some(p) = doc.para(story, &path) {
                for o in &p.objects {
                    collect(o, 0, &mut out);
                }
            }
        }
    }
    out
}

/// The embedded objects in `doc` that an output keeping `kept` doesn't keep, in document order,
/// by what each is: its program id (`Word.Document.12`, `Acrobat.Document.DC`), `OpenDocument
/// object`, or `object` when it names none.
pub fn not_shown_in(doc: &Document, kept: Kept) -> Vec<String> {
    let lost = |o: &Object| match kept {
        Kept::All => false,
        Kept::Ole => o.odf,
        Kept::OpenDocument => !o.odf,
        Kept::Pictures | Kept::Nothing => true,
    };
    objects(doc).into_iter().filter(lost).map(|o| o.name).collect()
}

/// A sentence for people about `names` (from [`not_shown_in`]) missing from `output`, where
/// `pictures` says whether the output shows their pictures:
/// `1 embedded object (Word.Document.12) is shown as its picture; its content is not in out.pdf`,
/// or `… is left out; its content is not in out.txt`. Control and bidirectional formatting
/// characters in a name are shown as escapes, so a name read from a file can't drive the terminal
/// or reorder the line. `None` when there are none.
pub fn warning(names: &[String], output: &str, pictures: bool) -> Option<String> {
    let first = names.first()?;
    if names.len() == 1 {
        let what = if pictures { "is shown as its picture" } else { "is left out" };
        return Some(format!("1 embedded object ({}) {what}; its content is not in {output}", printable(first)));
    }
    // Each kind once, with how many of it, in the order first seen.
    let mut kinds: Vec<(&str, usize)> = Vec::new();
    for n in names {
        match kinds.iter_mut().find(|(k, _)| *k == n.as_str()) {
            Some((_, count)) => *count = count.saturating_add(1),
            None => kinds.push((n.as_str(), 1)),
        }
    }
    let listed: Vec<String> = kinds.iter().map(|(k, n)| if *n == 1 { printable(k) } else { format!("{} ×{n}", printable(k)) }).collect();
    let what = if pictures { "are shown as their pictures" } else { "are left out" };
    Some(format!("{} embedded objects ({}) {what}; their content is not in {output}", names.len(), listed.join(", ")))
}

/// `name` with control characters (newline, escape, C1…), line and paragraph separators and
/// bidirectional formatting characters shown as escapes (`\u{1b}`).
fn printable(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_control() || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}') {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

fn collect(o: &InlineObject, depth: usize, out: &mut Vec<Object>) {
    match o {
        InlineObject::Image { ole: Some(e), .. } => {
            if let Some(name) = object_name(e) {
                out.push(Object { name, odf: e.is_odf_object() });
            }
        }
        InlineObject::Group { children, .. } if depth < MAX_GROUP_DEPTH => {
            for c in children {
                collect(&c.obj, depth.saturating_add(1), out);
            }
        }
        _ => {}
    }
}

/// What `e` is called, or `None` for kept markup that is no embedded object (a signature line).
fn object_name(e: &Embedded) -> Option<String> {
    if e.is_odf_object() {
        return Some("OpenDocument object".into());
    }
    if !e.xml.contains("OLEObject") {
        return None;
    }
    Some(prog_id(&e.xml).unwrap_or("object").to_string())
}

/// The `ProgID` attribute of the markup's `o:OLEObject` (ECMA-376 Part 4, §19.2.2.20). XML allows
/// white space around the `=` (§3.1 `Eq`), which markup given as JSON may keep.
fn prog_id(xml: &str) -> Option<&str> {
    let space = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    xml.match_indices("ProgID").find_map(|(at, name)| {
        // The attribute's name, not the end of another one (`xProgID`).
        if !xml.get(..at)?.ends_with(space) {
            return None;
        }
        let rest = xml.get(at.saturating_add(name.len())..)?.trim_start_matches(space);
        let rest = rest.strip_prefix('=')?.trim_start_matches(space);
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let rest = rest.get(1..)?;
        let end = rest.find(quote)?;
        rest.get(..end).filter(|id| !id.is_empty() && id.len() <= MAX_PROG_ID && !id.contains(['&', '<']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ole(xml: &str) -> InlineObject {
        InlineObject::Image {
            media: "image1.png".into(),
            w: 40.0,
            h: 30.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: Some(Arc::new(Embedded { xml: xml.into(), refs: Vec::new() })),
        }
    }

    #[test]
    fn program_ids_are_read_from_the_ole_markup() {
        assert_eq!(prog_id(r#"<w:object><o:OLEObject Type="Embed" ProgID="Word.Document.12" ShapeID="x"/></w:object>"#), Some("Word.Document.12"));
        assert_eq!(prog_id("<o:OLEObject ProgID='Acrobat.Document.DC'/>"), Some("Acrobat.Document.DC"));
        // XML allows white space around `=` (markup given as JSON keeps it as written).
        assert_eq!(prog_id(r#"<o:OLEObject ProgID = "Word.Document.12"/>"#), Some("Word.Document.12"));
        assert_eq!(prog_id("<o:OLEObject\n\tProgID\r\n=\t'Excel.Sheet.12'/>"), Some("Excel.Sheet.12"));
        // Another attribute ending in the name isn't it.
        assert_eq!(prog_id(r#"<o:OLEObject xProgID="Wrong" ProgID="Right"/>"#), Some("Right"));
        assert_eq!(prog_id(r#"<o:OLEObject ProgIDs="Wrong"/>"#), None);
        for bad in [
            "<o:OLEObject/>",
            "<o:OLEObject ProgID=\"\"/>",
            "<o:OLEObject ProgID=\"unterminated",
            "<o:OLEObject ProgID=x/>",
            "<o:OLEObject ProgID=\"a&amp;b\"/>",
        ] {
            assert_eq!(prog_id(bad), None, "{bad}");
        }
        let long = format!("<o:OLEObject ProgID=\"{}\"/>", "x".repeat(MAX_PROG_ID + 1));
        assert_eq!(prog_id(&long), None);
    }

    #[test]
    fn objects_are_named_and_signature_lines_left_out() {
        assert_eq!(
            object_name(&Embedded { xml: "<w:object><o:OLEObject ProgID=\"Excel.Sheet.12\"/></w:object>".into(), refs: Vec::new() }).as_deref(),
            Some("Excel.Sheet.12")
        );
        assert_eq!(
            object_name(&Embedded { xml: "<w:object><o:OLEObject Type=\"Link\"/></w:object>".into(), refs: Vec::new() }).as_deref(),
            Some("object")
        );
        assert_eq!(object_name(&Embedded { xml: "<w:pict><v:shape><o:signatureline/></v:shape></w:pict>".into(), refs: Vec::new() }), None);
        let odf = format!("{} mime=\"application/vnd.oasis.opendocument.spreadsheet\" ole=\"false\"/>", Embedded::ODF_OBJECT);
        assert_eq!(object_name(&Embedded { xml: odf, refs: Vec::new() }).as_deref(), Some("OpenDocument object"));
    }

    #[test]
    fn warnings_count_each_kind() {
        assert_eq!(warning(&[], "out.pdf", true), None);
        assert_eq!(
            warning(&["Word.Document.12".into()], "out.pdf", true).as_deref(),
            Some("1 embedded object (Word.Document.12) is shown as its picture; its content is not in out.pdf")
        );
        let names: Vec<String> = ["Acrobat.Document.DC", "Word.Document.12", "Acrobat.Document.DC"].map(String::from).to_vec();
        assert_eq!(
            warning(&names, "report.html", true).as_deref(),
            Some("3 embedded objects (Acrobat.Document.DC ×2, Word.Document.12) are shown as their pictures; their content is not in report.html")
        );
    }

    /// Plain text has no pictures, so its warning doesn't claim one.
    #[test]
    fn a_warning_about_text_claims_no_picture() {
        assert_eq!(
            warning(&["Word.Document.12".into()], "out.txt", false).as_deref(),
            Some("1 embedded object (Word.Document.12) is left out; its content is not in out.txt")
        );
        let names: Vec<String> = ["Package", "Package"].map(String::from).to_vec();
        assert_eq!(
            warning(&names, "the text", false).as_deref(),
            Some("2 embedded objects (Package ×2) are left out; their content is not in the text")
        );
    }

    /// A program id comes from the file (or JSON markup): a newline, a terminal escape or a
    /// right-to-left override in it is shown as an escape, never written to the terminal.
    #[test]
    fn control_and_bidi_characters_in_a_program_id_are_escaped() {
        for (id, shown) in [
            ("Word.Document.12\nwordcraft-cli: wrote out.pdf", "Word.Document.12\\u{a}wordcraft-cli: wrote out.pdf"),
            ("\u{1b}[31mRed\u{1b}[0m", "\\u{1b}[31mRed\\u{1b}[0m"),
            ("Word.\u{202e}21.tnemucoD", "Word.\\u{202e}21.tnemucoD"),
            ("A\u{2066}B\u{200f}C\u{9b}D", "A\\u{2066}B\\u{200f}C\\u{9b}D"),
        ] {
            let w = warning(&[id.to_string()], "out.pdf", true).unwrap();
            assert!(w.contains(&format!("({shown})")), "{w:?}");
            assert!(!w.chars().any(|c| c.is_control() || matches!(c, '\u{202e}' | '\u{2066}' | '\u{200f}')), "{w:?}");
            let w = warning(&[id.to_string(), id.to_string()], "out.pdf", true).unwrap();
            assert!(w.contains(&format!("({shown} ×2)")), "{w:?}");
        }
        // The name itself is kept as read: `notShown` is JSON, which escapes what it must.
        let xml = "<o:OLEObject ProgID=\"Evil\u{202e}Name\"/>";
        assert_eq!(object_name(&Embedded { xml: xml.into(), refs: Vec::new() }).as_deref(), Some("Evil\u{202e}Name"));
    }

    /// The package #5393's script makes: a table cell holding an embedded Word document shown as
    /// an icon (`w:object` with a VML picture and an `o:OLEObject`), as Word writes one.
    fn embedded_docx() -> Vec<u8> {
        use std::io::Write;
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([200, 60, 60, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;
        let para = |t: &str| format!("<w:p><w:r><w:t>{t}</w:t></w:r></w:p>");
        let object = r##"<w:p><w:r><w:object w:dxaOrig="1541" w:dyaOrig="995"><v:shape id="_x0000_i1025" type="#_x0000_t75" style="width:77pt;height:50pt" o:ole=""><v:imagedata r:id="rId5" o:title=""/></v:shape><o:OLEObject Type="Embed" ProgID="Word.Document.12" ShapeID="_x0000_i1025" DrawAspect="Icon" ObjectID="_1" r:id="rId6"/></w:object></w:r></w:p>"##;
        let cell = |c: &str| format!(r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>{c}</w:tc>"#);
        let body = format!(
            r#"{}<w:tbl><w:tblPr><w:tblW w:w="9000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid><w:tr>{}{}</w:tr></w:tbl>{}"#,
            para("Outer document before the table."),
            cell(&para("Attachment")),
            cell(object),
            para("Outer document after the table.")
        );
        let files: Vec<(&str, Vec<u8>)> = vec![
            ("[Content_Types].xml", br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="docx" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec()),
            ("_rels/.rels", br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec()),
            ("word/document.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {ns}><w:body>{body}<w:sectPr/></w:body></w:document>"#).into_bytes()),
            ("word/_rels/document.xml.rels", br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="embeddings/Microsoft_Word_Document.docx"/></Relationships>"#.to_vec()),
            ("word/media/image1.png", png),
            ("word/embeddings/Microsoft_Word_Document.docx", b"PK inner document".to_vec()),
        ];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// #5393: saving to PDF or plain text names the embedded object those files show only as its
    /// picture (`notShown`); a .docx, which writes the object back, names nothing.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_pdf_or_text_save_names_the_objects_it_shows_as_pictures() {
        let doc = crate::io::open_bytes("embedded.docx", &embedded_docx()).unwrap();
        assert_eq!(not_shown_in(&doc, Kept::Pictures), ["Word.Document.12"], "the object in the table cell is found");
        let dir = std::env::temp_dir().join(format!("wc-embedded-objects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = crate::Session::new(doc);
        for (name, cmd, named) in
            [("out.pdf", "file.exportPdf", true), ("copy.pdf", "file.save", true), ("out.txt", "file.save", true), ("out.docx", "file.save", false)]
        {
            let r = s.run(cmd, &serde_json::json!({"path": dir.join(name)})).unwrap();
            let want = if named { serde_json::json!(["Word.Document.12"]) } else { serde_json::Value::Null };
            assert_eq!(r["notShown"], want, "{name}: {r}");
            let shown = if !named {
                None
            } else if name.ends_with(".txt") {
                Some("nothing")
            } else {
                Some("picture")
            };
            assert_eq!(r["notShownAs"].as_str(), shown, "{name}: {r}");
        }
        let back = crate::io::open_bytes("out.docx", &std::fs::read(dir.join("out.docx")).unwrap()).unwrap();
        assert_eq!(not_shown_in(&back, Kept::Pictures), ["Word.Document.12"], "the .docx keeps the object");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deep_groups_are_bounded() {
        let mut o = ole("<o:OLEObject ProgID=\"Word.Document.12\"/>");
        for _ in 0..(MAX_GROUP_DEPTH + 10) {
            o = InlineObject::Group {
                w: 10.0,
                h: 10.0,
                float: Default::default(),
                ch_w: 10.0,
                ch_h: 10.0,
                children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: o }],
                canvas: None,
            };
        }
        let mut out = Vec::new();
        collect(&o, 0, &mut out);
        assert!(out.is_empty(), "past the depth limit nothing is looked into");

        // Beside it, an object as deep as the limit allows (in the group at depth
        // `MAX_GROUP_DEPTH - 1`, counting the outer group as 0) is still found.
        let mut kept = ole("<o:OLEObject ProgID=\"Excel.Sheet.12\"/>");
        for _ in 0..MAX_GROUP_DEPTH - 1 {
            kept = group(vec![kept]);
        }
        let both = group(vec![o, kept]);
        let mut out = Vec::new();
        collect(&both, 0, &mut out);
        assert_eq!(out.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["Excel.Sheet.12"]);
    }

    fn group(children: Vec<InlineObject>) -> InlineObject {
        InlineObject::Group {
            w: 10.0,
            h: 10.0,
            float: Default::default(),
            ch_w: 10.0,
            ch_h: 10.0,
            children: children.into_iter().map(|obj| wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj }).collect(),
            canvas: None,
        }
    }

    fn prog(id: &str) -> InlineObject {
        ole(&format!("<w:object><o:OLEObject Type=\"Embed\" ProgID=\"{id}\"/></w:object>"))
    }

    fn odf() -> InlineObject {
        ole(&format!("{} mime=\"application/vnd.oasis.opendocument.spreadsheet\" ole=\"false\"/>", Embedded::ODF_OBJECT))
    }

    fn para_with(objects: Vec<InlineObject>) -> Arc<wordcraft_doc::Block> {
        let mut p = wordcraft_doc::Paragraph::new();
        for o in objects {
            p.insert_object(0, o, &Default::default()).unwrap();
        }
        Arc::new(wordcraft_doc::Block::Para(p))
    }

    /// A document with an embedded object in every kind of story: the body (one in a group, and
    /// an OpenDocument object), a header, a footer, a footnote, an endnote, a comment and a text
    /// box.
    fn every_story() -> Document {
        use wordcraft_doc::{Part, PartKind};
        let mut doc = Document::new();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([60, 60, 200, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        doc.media.insert("image1.png".into(), Arc::new(png));
        doc.body = vec![para_with(vec![prog("Word.Document.12")]), para_with(vec![group(vec![prog("Acrobat.Document.DC")])]), para_with(vec![odf()])];
        let parts = [
            (PartKind::Header, "Excel.Sheet.12"),
            (PartKind::Footer, "Package"),
            (PartKind::Footnote, "Equation.3"),
            (PartKind::Endnote, "Paint.Picture"),
            (PartKind::Comment, "PowerPoint.Show.12"),
            (PartKind::TextBox, "MSGraph.Chart.8"),
        ];
        for (id, (kind, name)) in (1..).zip(parts) {
            doc.parts.insert(id, Part { kind, blocks: vec![para_with(vec![prog(name)])], body: Default::default() });
        }
        doc
    }

    const EVERY_STORY: [&str; 9] = [
        "Word.Document.12",
        "Acrobat.Document.DC",
        "OpenDocument object",
        "Excel.Sheet.12",
        "Package",
        "Equation.3",
        "Paint.Picture",
        "PowerPoint.Show.12",
        "MSGraph.Chart.8",
    ];

    /// Objects in groups, headers and footers, notes, comments and text boxes are found, in
    /// document order.
    #[test]
    fn objects_in_every_story_are_found() {
        assert_eq!(not_shown_in(&every_story(), Kept::Pictures), EVERY_STORY);
        assert_eq!(not_shown_in(&every_story(), Kept::Nothing), EVERY_STORY);
        assert!(not_shown_in(&every_story(), Kept::All).is_empty());
        assert_eq!(not_shown_in(&every_story(), Kept::Ole), ["OpenDocument object"]);
        let ole: Vec<&str> = EVERY_STORY.into_iter().filter(|n| *n != "OpenDocument object").collect();
        assert_eq!(not_shown_in(&every_story(), Kept::OpenDocument), ole);
    }

    /// Each saver, by every extension it answers to, reports what its file doesn't keep (#5393):
    /// every object in a PDF, HTML or RTF copy (shown as pictures) and in a text, Markdown, LaTeX
    /// or Word 2003 XML copy (not even that); OpenDocument objects in a Word file; Word's own in
    /// an ODT; nothing in JSON or a page image. `file.saveAs` and `file.exportPdf` report the
    /// same way.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_format_and_alias_reports_what_it_does_not_keep() {
        use serde_json::{Value, json};
        let dir = std::env::temp_dir().join(format!("wc-embedded-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ole: Vec<&str> = EVERY_STORY.into_iter().filter(|n| *n != "OpenDocument object").collect();
        let pictures = ["pdf", "html", "htm", "xhtml", "rtf", "PDF", "Htm"];
        let nothing = ["txt", "text", "md", "markdown", "tex", "latex", "ltx", "TXT"];
        let word = ["docx", "docm", "dotx", "dotm", "xml"];
        let odt = ["odt", "ott", "fodt"];
        let mut cases: Vec<(String, Value, Option<&str>, Vec<&str>)> = Vec::new();
        for ext in pictures {
            cases.push((format!("out.{ext}"), json!({}), Some("picture"), EVERY_STORY.to_vec()));
        }
        for ext in nothing {
            cases.push((format!("out.{ext}"), json!({}), Some("nothing"), EVERY_STORY.to_vec()));
        }
        for ext in word {
            cases.push((format!("out.{ext}"), json!({}), Some("picture"), vec!["OpenDocument object"]));
        }
        cases.push(("w2003.xml".into(), json!({"format": "wordml2003"}), Some("nothing"), EVERY_STORY.to_vec()));
        for ext in odt {
            cases.push((format!("out.{ext}"), json!({}), Some("picture"), ole.clone()));
        }
        cases.push(("out.json".into(), json!({}), None, Vec::new()));
        for (name, extra, shown, want) in cases {
            for cmd in ["file.save", "file.saveAs"] {
                let mut s = crate::Session::new(every_story());
                let mut params = json!({"path": dir.join(&name)});
                if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
                    p.extend(e.clone());
                }
                let r = s.run(cmd, &params).unwrap_or_else(|e| panic!("{cmd} {name}: {e}"));
                let got: Vec<&str> = r["notShown"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                assert_eq!(got, want, "{cmd} {name}: {r}");
                assert_eq!(r["notShownAs"].as_str(), shown, "{cmd} {name}: {r}");
            }
        }
        let mut s = crate::Session::new(every_story());
        let r = s.run("file.exportPdf", &json!({"path": dir.join("export.pdf")})).unwrap();
        assert_eq!(r["notShown"], json!(EVERY_STORY), "{r}");
        assert_eq!(r["notShownAs"], "picture");
        let r = s.run("file.exportPng", &json!({"path": dir.join("page.png")})).unwrap();
        assert!(r.get("notShown").is_none(), "a page image shows one page: {r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The savers' own aliases are the ones reported: none is missed, and none is added.
    #[test]
    fn kept_follows_the_savers_extensions() {
        for ext in ["text", "markdown", "htm", "xhtml", "latex", "ltx", ".pdf", "HTML"] {
            assert_ne!(Kept::by(ext, false), Kept::All, "{ext}");
        }
        assert_eq!(Kept::by("xml", false), Kept::Ole);
        assert_eq!(Kept::by("xml", true), Kept::Nothing);
        assert_eq!(Kept::by("Docx", false), Kept::Ole);
        assert_eq!(Kept::by("fodt", false), Kept::OpenDocument);
        assert_eq!(Kept::by("rtf", false), Kept::Pictures);
        for ext in ["png", "json", "zip", ""] {
            assert_eq!(Kept::by(ext, false), Kept::All, "{ext}");
        }
    }
}
