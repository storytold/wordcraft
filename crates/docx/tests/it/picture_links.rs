//! A picture's hyperlink (#5267): Word keeps it on the drawing (`wp:docPr/a:hlinkClick`,
//! ECMA-376 Part 1 §20.4.2.5) rather than in a `w:hyperlink` around the run. Original
//! synthetic packages; the 2×2 picture is generated in code.
use std::io::{Read, Write};
use std::sync::Arc;
use wordcraft_doc::{CharProps, Document, InlineObject, Pos};

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// An inline picture's run; `link` goes into its `wp:docPr`.
fn picture_run(id: u32, link: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="254000" cy="254000"/><wp:docPr id="{id}" name="Picture {id}">{link}</wp:docPr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="square.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdPic"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="254000" cy="254000"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

fn package(body: &str) -> Vec<u8> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}<w:sectPr/></w:body></w:document>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdPic" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/square.png"/><Relationship Id="rIdWeb" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/picture" TargetMode="External"/><Relationship Id="rIdRun" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/run" TargetMode="External"/></Relationships>"#;
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let png = png();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", xml.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/media/square.png", png.as_slice()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The link on each picture of the first paragraph, in order.
fn picture_links(doc: &Document) -> Vec<Option<String>> {
    let p = doc.body[0].as_para().unwrap();
    p.object_offsets()
        .into_iter()
        .filter(|off| matches!(p.object_at(*off), Some(InlineObject::Image { .. })))
        .map(|off| p.props_of_char(off).link.clone())
        .collect()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn a_link_on_a_pictures_drawing_is_read() {
    let body = format!(
        "<w:p>{}{}{}<w:hyperlink r:id=\"rIdRun\">{}</w:hyperlink></w:p>",
        // Word's way: the link on the drawing.
        picture_run(1, r#"<a:hlinkClick r:id="rIdWeb"/>"#),
        // A click naming a relationship that is no hyperlink, and one naming none.
        picture_run(2, r#"<a:hlinkClick r:id="rIdPic"/>"#),
        picture_run(3, r#"<a:hlinkClick r:id="rIdMissing"/>"#),
        // A link around the run wins over the drawing's.
        picture_run(4, r#"<a:hlinkClick r:id="rIdWeb"/>"#),
    );
    let doc = wordcraft_docx::read(&package(&body)).unwrap();
    assert_eq!(picture_links(&doc), [Some("https://example.com/picture".to_string()), None, None, Some("https://example.com/run".to_string())]);
}

#[test]
fn a_linked_picture_is_saved_with_its_link_on_the_drawing_too() {
    let mut doc = Document::new();
    doc.media.insert("square.png".into(), Arc::new(png()));
    let picture = || InlineObject::Image {
        media: "square.png".into(),
        w: 20.0,
        h: 20.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
        fx: Default::default(),
    };
    let linked = |l: &str| CharProps { link: Some(l.to_string()), ..Default::default() };
    // A place in this document stays a `w:hyperlink w:anchor` only: `a:hlinkClick` names a relationship.
    doc.insert_object(&Pos::body(0, 0), picture(), &linked("#Spot")).unwrap();
    doc.insert_object(&Pos::body(0, 0), picture(), &linked("https://example.com/saved")).unwrap();
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let xml = part(&bytes, "word/document.xml");
    let clicks: Vec<&str> = xml.match_indices("<a:hlinkClick ").map(|(i, _)| &xml[i..]).collect();
    assert_eq!(clicks.len(), 1, "{xml}");
    let rid = clicks[0].split('"').nth(1).unwrap();
    let docpr = xml.find("<wp:docPr ").unwrap();
    assert!(docpr < xml.find("<a:hlinkClick ").unwrap() && xml.find("<a:hlinkClick ").unwrap() < xml.find("</wp:docPr>").unwrap(), "{xml}");
    let rels = part(&bytes, "word/_rels/document.xml.rels");
    assert!(rels.contains(&format!(r#"Id="{rid}""#)) && rels.contains(r#"Target="https://example.com/saved""#), "{rels}");
    // The run's own hyperlink names the same relationship, so the address is stored once.
    assert_eq!(rels.matches("https://example.com/saved").count(), 1, "{rels}");
    let back = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(picture_links(&back), [Some("https://example.com/saved".to_string()), Some("#Spot".to_string())]);
    // The drawing's link alone (as Word writes it) reads back the same.
    let without_run_link = xml.replace(&format!(r#"<w:hyperlink r:id="{rid}" w:history="1">"#), "").replacen("</w:hyperlink>", "", 1);
    assert_ne!(without_run_link, xml);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        if name == "word/document.xml" {
            data = without_run_link.clone().into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&data).unwrap();
    }
    let edited = out.finish().unwrap().into_inner();
    assert_eq!(picture_links(&wordcraft_docx::read(&edited).unwrap())[0], Some("https://example.com/saved".to_string()));
}
