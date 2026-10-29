//! SVG pictures (#1910): Word stores an SVG picture as a PNG `a:blip` with the SVG in the
//! [MS-ODRAWXML] `asvg:svgBlip` extension. Fixtures are built in the test from the spec.

use std::io::{Read, Write};

use wordcraft_doc::{Document, InlineObject, Paragraph, para_block};

const SVG_EXT: &str = "{96DAC541-7B7A-43D3-8B79-37D633B846F1}";
const SVG_NS: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="#ff0000"/></svg>"##;

fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 80, y as u8 * 120, 200, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn entries(pkg: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(pkg)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut b = Vec::new();
            f.read_to_end(&mut b).unwrap();
            (f.name().to_string(), b)
        })
        .collect()
}

fn part(pkg: &[u8], name: &str) -> Option<Vec<u8>> {
    entries(pkg).into_iter().find(|(n, _)| n == name).map(|(_, b)| b)
}

fn text(pkg: &[u8], name: &str) -> String {
    String::from_utf8(part(pkg, name).unwrap_or_default()).unwrap()
}

/// A paragraph holding a picture whose `a:blip` is `blip` (its `r:embed` attribute and contents).
fn pic_para(blip: &str) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="508000" cy="254000"/><wp:docPr id="1" name="Graphic 1" descr="logo"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="Graphic 1"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip {blip}</a:blip><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="508000" cy="254000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    )
}

/// Word's SVG picture blip: the PNG (`rId1`) under `r:embed`, the SVG (`rId2`) in the extension.
fn svg_blip() -> String {
    format!(r#"r:embed="rId1"><a:extLst><a:ext uri="{SVG_EXT}"><asvg:svgBlip xmlns:asvg="{SVG_NS}" r:embed="rId2"/></a:ext></a:extLst>"#)
}

/// A package as Word writes an SVG picture: the PNG under `r:embed`, the SVG in the extension.
fn word_svg_package(svg: &[u8]) -> Vec<u8> {
    package(&pic_para(&svg_blip()), svg)
}

/// A package with `body`, `rId1` → `media/image1.png` (a PNG) and `rId2` → `media/image2.svg`
/// (`svg`).
fn package(body: &str, svg: &[u8]) -> Vec<u8> {
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}</w:body></w:document>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image2.svg"/></Relationships>"#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let png = tiny_png();
    zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/image1.png", &png),
        ("word/media/image2.svg", svg),
    ])
}

fn only_picture(d: &Document) -> (String, f32, f32) {
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    p.objects
        .iter()
        .find_map(|o| match o {
            InlineObject::Image { media, w, h, .. } => Some((media.clone(), *w, *h)),
            _ => None,
        })
        .expect("a picture")
}

#[test]
fn word_svg_picture_reads_as_the_svg() {
    let d = wordcraft_docx::read(&word_svg_package(SVG)).unwrap();
    let (media, w, h) = only_picture(&d);
    assert_eq!(d.media.get(&media).map(|m| m.as_slice()), Some(SVG), "the picture is the SVG, not its PNG fallback");
    assert_eq!((w, h), (40.0, 20.0));
    // Word's PNG fallback is kept alongside, to be written back on save.
    assert_eq!(d.media.len(), 2);
}

/// Review 1 #3: Word's own PNG fallback (which has any text the SVG holds) is written back as it
/// was, not drawn again.
#[test]
fn word_png_fallback_is_written_back_unchanged() {
    let d = wordcraft_docx::read(&word_svg_package(SVG)).unwrap();
    let pkg = wordcraft_docx::write(&d).unwrap();
    let pngs: Vec<Vec<u8>> = entries(&pkg).into_iter().filter(|(n, _)| n.starts_with("word/media/") && n.ends_with(".png")).map(|(_, b)| b).collect();
    assert_eq!(pngs, vec![tiny_png()], "one PNG: Word's own fallback");
    let r = wordcraft_docx::read(&pkg).unwrap();
    assert_eq!(r.media.get(&only_picture(&r).0).map(|m| m.as_slice()), Some(SVG));
}

#[test]
fn unreadable_svg_part_falls_back_to_the_png() {
    let d = wordcraft_docx::read(&word_svg_package(b"\x89PNG not an svg")).unwrap();
    let (media, _, _) = only_picture(&d);
    assert_eq!(d.media.get(&media).map(|m| m.as_slice()), Some(tiny_png().as_slice()));
}

/// PR review: an SVG target an ordinary picture loaded first (so not checked as an SVG) is still
/// checked when an `asvg:svgBlip` names it; the SVG picture then shows its PNG fallback.
#[test]
fn unreadable_svg_part_loaded_earlier_still_falls_back_to_the_png() {
    let body = format!("{}{}", pic_para(r#"r:embed="rId2">"#), pic_para(&svg_blip()));
    let d = wordcraft_docx::read(&package(&body, b"not an svg")).unwrap();
    let media: Vec<String> = d
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Image { media, .. } => Some(media.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(media.len(), 2);
    assert_eq!(d.media.get(&media[1]).map(|m| m.as_slice()), Some(tiny_png().as_slice()), "the SVG picture shows its PNG");
    assert!(d.svg_fallbacks.is_empty());
}

/// PR review: SVG media under a raster name (an `asvg:svgBlip` target named `.png`) is still
/// written as an SVG picture: an `.svg` part with its PNG fallback.
#[test]
fn svg_media_with_a_raster_name_is_written_as_an_svg() {
    let mut d = Document::new();
    d.media.insert("image2.png".into(), std::sync::Arc::new(SVG.to_vec()));
    let mut p = Paragraph::new();
    let obj = InlineObject::Image {
        media: "image2.png".into(),
        w: 30.0,
        h: 15.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![para_block(p)];
    let pkg = wordcraft_docx::write(&d).unwrap();
    let xml = text(&pkg, "word/document.xml");
    assert!(xml.contains("<asvg:svgBlip "), "{xml}");
    let svgs: Vec<Vec<u8>> = entries(&pkg).into_iter().filter(|(n, _)| n.starts_with("word/media/") && n.ends_with(".svg")).map(|(_, b)| b).collect();
    assert_eq!(svgs, vec![SVG.to_vec()]);
    let pngs: Vec<Vec<u8>> = entries(&pkg).into_iter().filter(|(n, _)| n.starts_with("word/media/") && n.ends_with(".png")).map(|(_, b)| b).collect();
    assert_eq!(pngs.len(), 1);
    assert!(pngs[0].starts_with(b"\x89PNG"), "the fallback is a PNG");
    let r = wordcraft_docx::read(&pkg).unwrap();
    assert_eq!(r.media.get(&only_picture(&r).0).map(|m| m.as_slice()), Some(SVG));
}

fn svg_doc() -> Document {
    let mut d = Document::new();
    let key = d.add_media(SVG.to_vec(), "svg");
    let mut p = Paragraph::new();
    let obj = InlineObject::Image { media: key, w: 30.0, h: 15.0, alt: "logo".into(), float: Default::default(), crop: [0.0; 4], ole: None };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![para_block(p)];
    d
}

#[test]
fn svg_picture_is_written_as_word_does() {
    let pkg = wordcraft_docx::write(&svg_doc()).unwrap();
    let xml = text(&pkg, "word/document.xml");
    let rels = text(&pkg, "word/_rels/document.xml.rels");
    // a:blip r:embed → a PNG fallback; the extension's svgBlip r:embed → the SVG part.
    let blip = xml.find("<a:blip ").map(|i| &xml[i..]).unwrap();
    let rid = |s: &str, from: &str| -> String {
        let at = s.find(from).unwrap() + from.len();
        s[at..].split('"').next().unwrap().to_string()
    };
    let png_rid = rid(blip, "r:embed=\"");
    assert!(blip.contains(&format!("<a:ext uri=\"{SVG_EXT}\">")), "{blip}");
    assert!(blip.contains(&format!("xmlns:asvg=\"{SVG_NS}\"")), "{blip}");
    let svg_rid = rid(&blip[blip.find("<asvg:svgBlip ").unwrap()..], "r:embed=\"");
    let target = |id: &str| -> String {
        let at = rels.find(&format!("Id=\"{id}\"")).unwrap();
        let rel = &rels[rels[..at].rfind('<').unwrap()..];
        let t = rid(rel, "Target=\"");
        format!("word/{t}")
    };
    let png = part(&pkg, &target(&png_rid)).expect("the PNG fallback part");
    assert!(png.starts_with(b"\x89PNG"), "the fallback is a PNG");
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 80, "rasterised at twice the natural size");
    assert_eq!(part(&pkg, &target(&svg_rid)).as_deref(), Some(SVG), "the SVG part is the original");
    let ct = text(&pkg, "[Content_Types].xml");
    assert!(ct.contains(r#"Extension="svg" ContentType="image/svg+xml""#), "{ct}");
    assert!(ct.contains(r#"Extension="png" ContentType="image/png""#), "{ct}");
}

#[test]
fn svg_picture_round_trips() {
    let d = svg_doc();
    let r = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let (media, w, h) = only_picture(&r);
    assert_eq!(r.media.get(&media).map(|m| m.as_slice()), Some(SVG));
    assert_eq!((w, h), (30.0, 15.0));
    // The SVG and the fallback made on the first save.
    assert_eq!(r.media.len(), 2);
    // And again: the second save writes that same fallback once, not a new one.
    let first = wordcraft_docx::write(&d).unwrap();
    let pkg = wordcraft_docx::write(&r).unwrap();
    let pngs = |pkg: &[u8]| -> Vec<Vec<u8>> {
        entries(pkg).into_iter().filter(|(n, _)| n.starts_with("word/media/") && n.ends_with(".png")).map(|(_, b)| b).collect()
    };
    assert_eq!(pngs(&pkg).len(), 1);
    assert_eq!(pngs(&pkg), pngs(&first));
}

#[test]
fn unreadable_svg_is_written_without_a_broken_fallback() {
    let mut d = svg_doc();
    for m in d.media.values_mut() {
        *m = std::sync::Arc::new(b"<svg xmlns=\"http://www.w3.org/2000/svg\"".to_vec());
    }
    let pkg = wordcraft_docx::write(&d).unwrap();
    // Still a readable package with the picture kept.
    let r = wordcraft_docx::read(&pkg).unwrap();
    assert_eq!(r.media.len(), 1);
}

#[test]
fn word_2003_xml_gets_the_png_fallback() {
    let xml = String::from_utf8(wordcraft_docx::write_wordml2003(&svg_doc()).unwrap()).unwrap();
    assert!(xml.contains(".png\""), "the picture is the PNG fallback");
    assert!(!xml.contains("image/svg"));
}
