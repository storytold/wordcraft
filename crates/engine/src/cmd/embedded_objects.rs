//! Embedded objects an output shows only as their pictures (#5393).
//!
//! An OLE object (`w:object`: a Word document, a PDF or a workbook put in with Insert › Object) or
//! an OpenDocument object is drawn as the picture the file keeps for it, as Word draws it, and a
//! save to the object's own kind of file writes the object back. A PDF, plain text, Markdown,
//! HTML or LaTeX holds only that picture, or nothing: what is inside the object is not
//! there. The save and export commands to those formats name such objects in their result
//! (`notShown`), and `wordcraft-cli convert` and `text` warn about them, so the loss is never
//! silent.

use wordcraft_doc::graphic::Embedded;
use wordcraft_doc::{Document, InlineObject, StoryRef};

/// Groups nested deeper than this aren't looked into (hostile files).
const MAX_GROUP_DEPTH: usize = 64;

/// The longest program id kept; a longer one is reported as `object`.
const MAX_PROG_ID: usize = 128;

/// Output formats (by file extension) of the whole document that hold an embedded object's
/// picture at most, never what is inside it. (A page image shows one page, so it isn't listed.)
pub const PICTURE_ONLY: &[&str] = &["pdf", "txt", "md", "html", "tex"];

/// Every embedded object in `doc` (body, headers and footers, notes, text boxes), in document
/// order, by what it is: its program id (`Word.Document.12`, `Acrobat.Document.DC`),
/// `OpenDocument object`, or `object` when it names none. A signature line, kept the same way,
/// holds no other document and isn't listed.
pub fn shown_as_pictures(doc: &Document) -> Vec<String> {
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

/// The objects a save to `ext` shows only as pictures: [`shown_as_pictures`] for a
/// [`PICTURE_ONLY`] format, none for a format that keeps the objects.
pub fn not_shown_in(doc: &Document, ext: &str) -> Vec<String> {
    if PICTURE_ONLY.contains(&ext.to_ascii_lowercase().as_str()) { shown_as_pictures(doc) } else { Vec::new() }
}

/// A sentence for people about `names` (from [`shown_as_pictures`]) missing from `output`:
/// `1 embedded object (Word.Document.12) is shown as its picture; its content is not in out.pdf`.
/// `None` when there are none.
pub fn warning(names: &[String], output: &str) -> Option<String> {
    let first = names.first()?;
    if names.len() == 1 {
        return Some(format!("1 embedded object ({first}) is shown as its picture; its content is not in {output}"));
    }
    // Each kind once, with how many of it, in the order first seen.
    let mut kinds: Vec<(&str, usize)> = Vec::new();
    for n in names {
        match kinds.iter_mut().find(|(k, _)| *k == n.as_str()) {
            Some((_, count)) => *count = count.saturating_add(1),
            None => kinds.push((n.as_str(), 1)),
        }
    }
    let listed: Vec<String> = kinds.iter().map(|(k, n)| if *n == 1 { (*k).to_string() } else { format!("{k} ×{n}") }).collect();
    Some(format!("{} embedded objects ({}) are shown as their pictures; their content is not in {output}", names.len(), listed.join(", ")))
}

fn collect(o: &InlineObject, depth: usize, out: &mut Vec<String>) {
    match o {
        InlineObject::Image { ole: Some(e), .. } => {
            if let Some(name) = object_name(e) {
                out.push(name);
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

/// The `ProgID` attribute of the markup's `o:OLEObject` (ECMA-376 Part 4, §19.2.2.20).
fn prog_id(xml: &str) -> Option<&str> {
    let at = xml.find(" ProgID=")?;
    let rest = xml.get(at.saturating_add(" ProgID=".len())..)?;
    let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let rest = rest.get(1..)?;
    let end = rest.find(quote)?;
    rest.get(..end).filter(|id| !id.is_empty() && id.len() <= MAX_PROG_ID && !id.contains(['&', '<']))
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
        assert_eq!(warning(&[], "out.pdf"), None);
        assert_eq!(
            warning(&["Word.Document.12".into()], "out.pdf").as_deref(),
            Some("1 embedded object (Word.Document.12) is shown as its picture; its content is not in out.pdf")
        );
        let names: Vec<String> = ["Acrobat.Document.DC", "Word.Document.12", "Acrobat.Document.DC"].map(String::from).to_vec();
        assert_eq!(
            warning(&names, "report.txt").as_deref(),
            Some("3 embedded objects (Acrobat.Document.DC ×2, Word.Document.12) are shown as their pictures; their content is not in report.txt")
        );
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
        assert_eq!(shown_as_pictures(&doc), ["Word.Document.12"], "the object in the table cell is found");
        let dir = std::env::temp_dir().join(format!("wc-embedded-objects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = crate::Session::new(doc);
        for (name, cmd, named) in
            [("out.pdf", "file.exportPdf", true), ("copy.pdf", "file.save", true), ("out.txt", "file.save", true), ("out.docx", "file.save", false)]
        {
            let r = s.run(cmd, &serde_json::json!({"path": dir.join(name)})).unwrap();
            let want = if named { serde_json::json!(["Word.Document.12"]) } else { serde_json::Value::Null };
            assert_eq!(r["notShown"], want, "{name}: {r}");
        }
        let back = crate::io::open_bytes("out.docx", &std::fs::read(dir.join("out.docx")).unwrap()).unwrap();
        assert_eq!(shown_as_pictures(&back), ["Word.Document.12"], "the .docx keeps the object");
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
    }
}
