//! Pictures dropped at open are recorded for strict export (#6005): a missing part, an external
//! target and a link-only picture (ECMA-376 Part 1 §20.1.8.13 `a:blip` `r:embed`/`r:link`).
//! Original synthetic packages; nothing is fetched, the reader has no network code.
use std::io::Write;
use wordcraft_doc::{Document, InlineObject, MissingPictureReason};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:v="urn:schemas-microsoft-com:vml""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn blip(attrs: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline><wp:extent cx="254000" cy="254000"/><wp:docPr id="1" name="P"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="p"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip {attrs}/></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="254000" cy="254000"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

/// A package: `body` in `word/document.xml` (whose relationships are `doc_rels`), an optional
/// header with `header` as its content (relationships `header_rels`) and `extra` parts.
fn package(body: &str, doc_rels: &str, header: Option<(&str, &str)>, extra: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let hdr_rel = if header.is_some() { format!(r#"<Relationship Id="rIdH" Type="{REL}/header" Target="header2.xml"/>"#) } else { String::new() };
    let sect = if header.is_some() { r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"# } else { "<w:sectPr/>" };
    let doc = format!(r#"<w:document {NS}><w:body><w:p>{body}</w:p>{sect}</w:body></w:document>"#);
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let drels = format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{doc_rels}{hdr_rel}</Relationships>"#);
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="bin" ContentType="application/octet-stream"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut put = |name: &str, bytes: &[u8]| {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    };
    put("[Content_Types].xml", types.as_bytes());
    put("_rels/.rels", rels.as_bytes());
    put("word/document.xml", doc.as_bytes());
    put("word/_rels/document.xml.rels", drels.as_bytes());
    if let Some((content, hrels)) = header {
        put("word/header2.xml", format!(r#"<w:hdr {NS}><w:p>{content}</w:p></w:hdr>"#).as_bytes());
        put(
            "word/_rels/header2.xml.rels",
            format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{hrels}</Relationships>"#).as_bytes(),
        );
    }
    for (n, b) in extra {
        put(n, b);
    }
    zip.finish().unwrap().into_inner()
}

fn image_rel(id: &str, target: &str, external: bool) -> String {
    let mode = if external { r#" TargetMode="External""# } else { "" };
    format!(r#"<Relationship Id="{id}" Type="{REL}/image" Target="{target}"{mode}/>"#)
}

fn missing(d: &Document) -> Vec<(String, String, MissingPictureReason)> {
    d.missing_pictures.list.iter().map(|m| (m.part.clone(), m.story.clone(), m.reason)).collect()
}

fn pictures(d: &Document) -> usize {
    d.body[0].as_para().unwrap().objects.iter().filter(|o| matches!(o, InlineObject::Image { .. })).count()
}

#[test]
fn external_missing_and_link_only_pictures_are_recorded_and_still_dropped() {
    let body = [blip(r#"r:embed="rIdExt""#), blip(r#"r:embed="rIdGone""#), blip(r#"r:link="rIdLink""#), blip(r#"r:embed="rIdNone""#)].concat();
    let rels = [
        image_rel("rIdExt", "http://127.0.0.1:9/never-fetched.png", true),
        image_rel("rIdGone", "media/absent.png", false),
        image_rel("rIdLink", "file:///C:/pics/linked.png", true),
    ]
    .concat();
    let d = wordcraft_docx::read(&package(&body, &rels, None, &[])).unwrap();
    assert_eq!(pictures(&d), 0, "nothing is drawn for them, as before");
    assert_eq!(
        missing(&d),
        [
            ("http://127.0.0.1:9/never-fetched.png".into(), "word/document.xml".into(), MissingPictureReason::ExternalTarget),
            ("word/media/absent.png".into(), "word/document.xml".into(), MissingPictureReason::MissingPart),
            ("file:///C:/pics/linked.png".into(), "word/document.xml".into(), MissingPictureReason::LinkOnly),
            ("rIdNone".into(), "word/document.xml".into(), MissingPictureReason::MissingPart),
        ]
    );
    assert!(!d.missing_pictures.truncated);
}

#[test]
fn a_header_pictures_story_is_the_header_part() {
    let rels = [image_rel("rIdHx", "https://example.invalid/logo.png", true), image_rel("rIdOk", "media/ok.png", false)].concat();
    let header = [blip(r#"r:embed="rIdHx""#), blip(r#"r:embed="rIdOk""#)].concat();
    let bytes = package("", "", Some((&header, &rels)), &[("word/media/ok.png", png())]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(missing(&d), [("https://example.invalid/logo.png".into(), "word/header2.xml".into(), MissingPictureReason::ExternalTarget)]);
    assert_eq!(d.missing_pictures.main, "word/document.xml");
    assert!(d.missing_pictures.part_names.values().any(|n| &**n == "word/header2.xml"));
    assert_eq!(d.missing_pictures.media_parts.get("ok.png").map(String::as_str), Some("word/media/ok.png"));
}

#[test]
fn a_picture_that_is_there_is_not_recorded() {
    let body = blip(r#"r:embed="rIdOk""#);
    let d = wordcraft_docx::read(&package(&body, &image_rel("rIdOk", "media/ok.png", false), None, &[("word/media/ok.png", png())])).unwrap();
    assert_eq!(pictures(&d), 1);
    assert!(d.missing_pictures.list.is_empty());
}

#[test]
fn vml_image_data_links_are_recorded() {
    let pict =
        |attrs: &str| format!(r#"<w:r><w:pict><v:shape id="s" style="width:50pt;height:50pt"><v:imagedata {attrs}/></v:shape></w:pict></w:r>"#);
    let body = [pict(r#"r:id="rIdExt""#), pict(r#"r:href="rIdLink""#), pict(r#"r:id="rIdGone""#)].concat();
    let rels = [
        image_rel("rIdExt", "https://example.invalid/a.png", true),
        image_rel("rIdLink", "https://example.invalid/b.png", true),
        image_rel("rIdGone", "media/none.png", false),
    ]
    .concat();
    let d = wordcraft_docx::read(&package(&body, &rels, None, &[])).unwrap();
    let got: Vec<_> = missing(&d).into_iter().map(|(p, _, r)| (p, r)).collect();
    assert_eq!(
        got,
        [
            ("https://example.invalid/a.png".to_string(), MissingPictureReason::ExternalTarget),
            ("https://example.invalid/b.png".to_string(), MissingPictureReason::LinkOnly),
            ("word/media/none.png".to_string(), MissingPictureReason::MissingPart),
        ]
    );
}

#[test]
fn the_list_is_capped_and_the_document_is_still_equal() {
    let n = wordcraft_doc::MAX_MISSING_PICTURES + 5;
    let body: String = (0..n).map(|i| blip(&format!(r#"r:embed="r{i}""#))).collect();
    let rels: String = (0..n).map(|i| image_rel(&format!("r{i}"), &format!("media/gone{i}.png"), false)).collect();
    let d = wordcraft_docx::read(&package(&body, &rels, None, &[])).unwrap();
    assert_eq!(d.missing_pictures.list.len(), wordcraft_doc::MAX_MISSING_PICTURES);
    assert!(d.missing_pictures.truncated);
    // Not part of the document: `==` and the clone ignore it, and a save doesn't carry it.
    let mut e = d.clone();
    e.missing_pictures = Default::default();
    assert_eq!(d, e);
    assert!(!serde_json::to_string(&d).unwrap().contains("gone"));
}

/// Every note of a part shares the one allocation of its source name, so a file with many notes
/// in a part with a long name does not keep a copy of the name per note.
#[test]
fn the_notes_of_a_part_share_one_copy_of_its_name() {
    let notes: String = (1..=3).map(|i| format!(r#"<w:footnote w:id="{i}"><w:p><w:r><w:t>n{i}</w:t></w:r></w:p></w:footnote>"#)).collect();
    let rels = format!(r#"<Relationship Id="rIdF" Type="{REL}/footnotes" Target="footnotes.xml"/>"#);
    let part = format!(r#"<w:footnotes {NS}>{notes}</w:footnotes>"#);
    let d = wordcraft_docx::read(&package("x", &rels, None, &[("word/footnotes.xml", part.into_bytes())])).unwrap();
    let names: Vec<_> = d.missing_pictures.part_names.values().collect();
    assert!(names.len() >= 3, "{names:?}");
    assert!(names.iter().all(|n| &***n == "word/footnotes.xml"));
    assert!(names.windows(2).all(|w| std::sync::Arc::ptr_eq(w[0], w[1])), "each note holds its own copy of the part name");
}
