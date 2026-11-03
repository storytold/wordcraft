//! Embedded objects WordCraft doesn't edit (ODF 1.3 §10.4.6 `draw:object`: a spreadsheet,
//! drawing or presentation sub-document; §10.4.7 `draw:object-ole`: an OLE object's binary file)
//! are kept whole: shown by their replacement picture (or a placeholder WordCraft draws when it
//! can't show that picture, LibreOffice's own metafile format), and saved back as the same object
//! with its files, so LibreOffice opens them as before.

use crate::model::Inline;
use crate::odt::tests::{NS_T, entry, paras, zip_of};
use crate::odt::{export, import, parse};

const NS_D: &str = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;

/// LibreOffice's own metafile format, which WordCraft can't decode: the replacement picture
/// LibreOffice writes for an embedded spreadsheet.
const METAFILE: &[u8] = b"VCLMTF\x01\x00\x31\x00\x00\x00some drawing records";
const SHEET: &str = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:spreadsheet/></office:body></office:document-content>"#;
const MANIFEST: &str = r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="Object 1/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="Object 1/content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Object 1/styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Object 2" manifest:media-type="application/vnd.sun.star.oleobject"/></manifest:manifest>"#;

/// A document with an embedded spreadsheet (`draw:object`) and an OLE object
/// (`draw:object-ole`), each with a metafile replacement, and text after them.
fn package() -> Vec<u8> {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>Sheet <draw:frame draw:name="Object1" text:anchor-type="as-char" svg:width="3in" svg:height="1in"><draw:object xlink:href="./Object 1" xlink:type="simple"/><draw:image xlink:href="./ObjectReplacements/Object 1"/></draw:frame> and OLE <draw:frame text:anchor-type="as-char" svg:width="2in" svg:height="1.5in"><draw:object-ole xlink:href="./Object 2"/><draw:image xlink:href="./ObjectReplacements/Object 2"/></draw:frame></text:p><text:p>After.</text:p></office:text></office:body></office:document-content>"#
    );
    zip_of(&[
        ("META-INF/manifest.xml", MANIFEST.as_bytes()),
        ("content.xml", content.as_bytes()),
        ("Object 1/content.xml", SHEET.as_bytes()),
        ("Object 1/styles.xml", b"<office:document-styles/>"),
        ("Object 2", b"\xd0\xcf\x11\xe0 an OLE compound file"),
        ("ObjectReplacements/Object 1", METAFILE),
        ("ObjectReplacements/Object 2", METAFILE),
    ])
}

#[test]
fn an_embedded_spreadsheet_is_kept_with_a_placeholder_picture() {
    let flow = parse(&package()).unwrap();
    let inlines: Vec<&Inline> = paras(&flow).into_iter().flat_map(|p| &p.inlines).collect();
    let imgs: Vec<&crate::model::Img> = inlines.iter().filter_map(|i| if let Inline::Image(img) = i { Some(img) } else { None }).collect();
    assert_eq!(imgs.len(), 2, "both objects are shown");
    // A placeholder WordCraft can draw, at the frame's size.
    assert!(imgs.iter().all(|i| crate::model::sniff_image(&i.data).is_some()));
    assert_eq!((imgs[0].w, imgs[0].h), (216.0, 72.0));
    let sheet = imgs[0].object.as_deref().expect("the spreadsheet is kept");
    assert_eq!((sheet.mime.as_str(), sheet.ole), ("application/vnd.oasis.opendocument.spreadsheet", false));
    let mut names: Vec<&str> = sheet.files.iter().map(|(n, _)| n.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["content.xml", "styles.xml"]);
    assert_eq!(sheet.replacement.as_deref().map(Vec::as_slice), Some(METAFILE));
    let ole = imgs[1].object.as_deref().expect("the OLE object is kept");
    assert_eq!((ole.mime.as_str(), ole.ole, ole.files.len()), ("application/vnd.sun.star.oleobject", true, 1));
    // The text around them is all there.
    assert!(paras(&flow).iter().any(|p| matches!(p.inlines.first(), Some(Inline::Text(t, _)) if t == "After.")));
}

#[test]
fn embedded_objects_are_saved_back_whole() {
    let doc = import(&package()).unwrap();
    let saved = crate::odt::tests::export_model(&doc).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    let names: Vec<&str> = z.file_names().collect();
    for n in ["Object 1/content.xml", "Object 1/styles.xml", "Object 2", "ObjectReplacements/Object 1", "ObjectReplacements/Object 2"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    assert_eq!(entry(&saved, "Object 1/content.xml"), SHEET);
    let manifest = entry(&saved, "META-INF/manifest.xml");
    assert!(
        manifest.contains(
            r#"manifest:full-path="Object 1/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet""#
        ),
        "{manifest}"
    );
    assert!(manifest.contains(r#"manifest:full-path="Object 2" manifest:media-type="application/vnd.sun.star.oleobject""#), "{manifest}");
    let content = entry(&saved, "content.xml");
    assert!(content.contains(r#"<draw:object xlink:href="./Object 1""#), "{content}");
    assert!(content.contains(r#"<draw:object-ole xlink:href="./Object 2""#), "{content}");
    assert!(content.contains(r#"<draw:image xlink:href="./ObjectReplacements/Object 1""#), "{content}");
    // And it opens again the same way.
    let again = parse(&saved).unwrap();
    let kept = paras(&again).into_iter().flat_map(|p| &p.inlines).filter(|i| matches!(i, Inline::Image(img) if img.object.is_some())).count();
    assert_eq!(kept, 2);
}

#[test]
fn a_docx_save_writes_the_objects_pictures_only() {
    let doc = import(&package()).unwrap();
    let flow = crate::model::from_doc(&doc);
    // Formats without ODF objects (and so DOCX) get the picture alone.
    assert!(paras(&flow).into_iter().flat_map(|p| &p.inlines).all(|i| !matches!(i, Inline::Image(img) if img.object.is_some())));
    assert!(doc.passthrough.keys().any(|k| k.contains("Object") || k.starts_with("odf-object")));
}

#[test]
fn hostile_objects_are_bounded() {
    // An object pointing out of its folder, at the package itself or at nothing, and a folder of
    // many files: never a panic, nothing outside the object taken into it.
    let mut files: Vec<(String, Vec<u8>)> = (0..600).map(|i| (format!("Object 9/f{i}.xml"), vec![b'x'; 10])).collect();
    files.push(("content.xml".into(), Vec::new()));
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:frame svg:width="1in" svg:height="1in"><draw:object xlink:href="../../etc"/></draw:frame><draw:frame svg:width="1in" svg:height="1in"><draw:object xlink:href="./"/></draw:frame><draw:frame svg:width="1in" svg:height="1in"><draw:object-ole xlink:href="./Nothing"/></draw:frame><draw:frame svg:width="1in" svg:height="1in"><draw:object xlink:href="./Object 9"/></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    if let Some(f) = files.last_mut() {
        f.1 = content.into_bytes();
    }
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    let flow = parse(&zip_of(&refs)).unwrap();
    let objects: Vec<&crate::model::OdfObject> = paras(&flow)
        .into_iter()
        .flat_map(|p| &p.inlines)
        .filter_map(|i| if let Inline::Image(img) = i { img.object.as_deref() } else { None })
        .collect();
    assert_eq!(objects.len(), 1, "only the real folder is an object");
    assert!(objects[0].files.len() <= 256);
    assert!(objects[0].files.iter().all(|(n, _)| !n.contains("..") && !n.starts_with('/')));
}

#[test]
fn a_flat_document_holds_its_objects_and_their_pictures_inline() {
    let doc = import(&package()).unwrap();
    let flat = crate::odt::export_flat(&doc).unwrap();
    let text = String::from_utf8(flat.clone()).unwrap();
    // No link is left pointing at a file a flat document doesn't have.
    assert!(!text.contains("xlink:href=\"./ObjectReplacements") && !text.contains("xlink:href=\"./Object"), "{text}");
    assert!(text.contains("<draw:object-ole><office:binary-data>"), "{text}");
    // Opened again: the OLE object's file is back, and both objects are still kept.
    let again = crate::odt::import_flat(&flat).unwrap();
    let saved = export(&again).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    let names: Vec<&str> = z.file_names().collect();
    let ole = crate::odt::tests::entry(&saved, "content.xml");
    assert!(ole.contains("<draw:object-ole"), "{ole}");
    // Both objects with their pictures, each link to a file (or folder) the package has.
    assert_eq!(ole.matches("<draw:image ").count(), 2, "{ole}");
    for link in ole.split("xlink:href=\"").skip(1).filter_map(|r| r.split('"').next()) {
        let path = link.trim_start_matches("./");
        assert!(names.iter().any(|n| *n == path || n.starts_with(&format!("{path}/"))), "{link}: {names:?}");
    }
}

#[test]
fn an_object_in_a_header_is_kept_too() {
    let styles = format!(
        r#"<office:document-styles {NS_T} {NS_D}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1.5cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Logo <draw:frame svg:width="1in" svg:height="0.5in"><draw:object xlink:href="./Object 1"/><draw:image xlink:href="./ObjectReplacements/Object 1"/></draw:frame></text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Body.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = zip_of(&[
        ("META-INF/manifest.xml", MANIFEST.as_bytes()),
        ("content.xml", content.as_bytes()),
        ("styles.xml", styles.as_bytes()),
        ("Object 1/content.xml", SHEET.as_bytes()),
        ("ObjectReplacements/Object 1", METAFILE),
    ]);
    // Unedited: the header as read, and the object's files it names with it.
    let d = import(&pkg).unwrap();
    let saved = export(&d).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    let names: Vec<&str> = z.file_names().collect();
    assert!(names.contains(&"Object 1/content.xml") && names.contains(&"ObjectReplacements/Object 1"), "{names:?}");
    assert!(entry(&saved, "styles.xml").contains("<draw:object xlink:href=\"./Object 1\""));
    assert!(entry(&saved, "META-INF/manifest.xml").contains(r#"manifest:full-path="Object 1/""#));
    // Written from the model: WordCraft's own object, named its way.
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    assert!(z.file_names().any(|n| n.ends_with("/content.xml") && n.starts_with("Object M")), "{:?}", z.file_names().collect::<Vec<_>>());
    assert!(entry(&saved, "styles.xml").contains("<draw:object xlink:href=\"./Object M"));
}
