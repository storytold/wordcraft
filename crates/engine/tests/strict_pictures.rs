//! Pictures a layout draws that the exporters can't decode (#6005): a DOCX opened, laid out, and
//! asked which of its pictures would be grey boxes, and in which story.
use std::io::Write;
use wordcraft_doc::{Document, MissingPictureReason};
use wordcraft_layout::display::DisplayOptions;
use wordcraft_layout::{LayoutCache, LayoutOptions, StrictOutput, layout, media_check, undecodable_pictures};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn pic(rid: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline><wp:extent cx="508000" cy="508000"/><wp:docPr id="1" name="P"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="p"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="508000" cy="508000"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

/// A document with one header picture (`header_media`) and one body picture (`body_media`), the
/// two parts' bytes as given.
fn docx(header_media: &[u8], body_media: &[u8]) -> Vec<u8> {
    let doc = format!(
        r#"<w:document {NS}><w:body><w:p>{}<w:r><w:t>text</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr></w:body></w:document>"#,
        pic("rIdB")
    );
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let drels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdH" Type="{REL}/header" Target="header2.xml"/><Relationship Id="rIdB" Type="{REL}/image" Target="media/body.png"/></Relationships>"#
    );
    let hrels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdI" Type="{REL}/image" Target="media/image2.bin"/></Relationships>"#
    );
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="bin" ContentType="application/octet-stream"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let hdr = format!(r#"<w:hdr {NS}><w:p>{}</w:p></w:hdr>"#, pic("rIdI"));
    for (name, bytes) in [
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", drels.as_bytes()),
        ("word/header2.xml", hdr.as_bytes()),
        ("word/_rels/header2.xml.rels", hrels.as_bytes()),
        ("word/media/image2.bin", header_media),
        ("word/media/body.png", body_media),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn report(d: &Document, pages: Option<std::ops::RangeInclusive<usize>>) -> Vec<(String, String, MissingPictureReason)> {
    let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
    let (list, truncated, total) = undecodable_pictures(d, &l, pages, &DisplayOptions::default(), StrictOutput::Pdf);
    assert!(!truncated);
    assert_eq!(total, list.len() as u64);
    list.into_iter().map(|m| (m.part, m.story, m.reason)).collect()
}

#[test]
fn a_header_picture_of_corrupt_bytes_is_reported_with_its_part_and_story() {
    let d = wordcraft_docx::read(&docx(b"corrupt bytes, not a picture", &png())).unwrap();
    assert_eq!(report(&d, None), [("word/media/image2.bin".into(), "word/header2.xml".into(), MissingPictureReason::UnsupportedFormat)]);
}

#[test]
fn a_truncated_picture_is_undecodable_not_unsupported() {
    let mut cut = png();
    cut.truncate(cut.len() - 20);
    let d = wordcraft_docx::read(&docx(&png(), &cut)).unwrap();
    assert_eq!(report(&d, None), [("word/media/body.png".into(), "word/document.xml".into(), MissingPictureReason::Undecodable)]);
}

#[test]
fn a_real_png_in_a_header_is_not_reported() {
    let d = wordcraft_docx::read(&docx(&png(), &png())).unwrap();
    assert_eq!(report(&d, None), []);
}

#[test]
fn only_the_selected_pages_are_checked() {
    let d = wordcraft_docx::read(&docx(b"corrupt", &png())).unwrap();
    assert_eq!(report(&d, Some(0..=0)).len(), 1);
    assert_eq!(report(&d, Some(5..=9)), []);
}

#[test]
fn the_check_names_what_the_exporters_draw() {
    assert_eq!(media_check(&png(), StrictOutput::Pdf), Ok(()));
    assert!(media_check(b"", StrictOutput::Pdf).is_err());
    assert!(media_check(&[1, 0, 0, 0, 88, 0, 0, 0], StrictOutput::Pdf).is_err(), "a metafile that doesn't parse is a grey box");
}
