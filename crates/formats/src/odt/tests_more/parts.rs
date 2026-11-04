//! Package parts WordCraft doesn't read (ODF 1.3 Part 2 §3): macros and dialogs (`Basic/`),
//! LibreOffice's interface configuration, `manifest.rdf` and other applications' files are kept
//! as they are and written back with their manifest entries; the thumbnail and digital
//! signatures, which describe the package as it was, are not.

use std::io::{Cursor, Read, Write};

use crate::odt::tests::NS_T;
use crate::odt::{decrypt, encrypt, export, import};

const MODULE: &str = r#"<?xml version="1.0" encoding="UTF-8"?><script:module xmlns:script="http://openoffice.org/2000/script" script:name="Module1" script:language="StarBasic">Sub Hello
  MsgBox "Hi"
End Sub
</script:module>"#;

/// A package with parts WordCraft doesn't read, `extra` more files, and a manifest listing
/// some of them.
fn odt(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Macro.</text:p></office:text></office:body></office:document-content>"#
    );
    let manifest = r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Basic/Standard/Module1.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Basic/script-lc.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Configurations2/" manifest:media-type="application/vnd.sun.xml.ui.configuration"/><manifest:file-entry manifest:full-path="manifest.rdf" manifest:media-type="application/rdf+xml"/><manifest:file-entry manifest:full-path="Thumbnails/thumbnail.png" manifest:media-type="image/png"/></manifest:manifest>"#;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let files: Vec<(&str, &[u8])> = vec![
        ("mimetype", b"application/vnd.oasis.opendocument.text"),
        ("content.xml", content.as_bytes()),
        ("META-INF/manifest.xml", manifest.as_bytes()),
        ("Basic/Standard/Module1.xml", MODULE.as_bytes()),
        ("Basic/script-lc.xml", b"<library:libraries/>"),
        ("manifest.rdf", b"<rdf:RDF/>"),
        ("Thumbnails/thumbnail.png", b"\x89PNG old"),
        ("META-INF/documentsignatures.xml", b"<document-signatures/>"),
        ("other-app/data.bin", b"\x00\x01\x02"),
    ];
    for (name, data) in files.into_iter().chain(extra.iter().copied()) {
        z.start_file(name, o).unwrap();
        z.write_all(data).unwrap();
    }
    z.add_directory("Configurations2/toolbar/", o).unwrap();
    z.finish().unwrap().into_inner()
}

/// Each file of package `bytes` with its contents.
fn files(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        out.push((f.name().to_string(), b));
    }
    out
}

fn file<'a>(fs: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    fs.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice())
}

#[test]
fn macros_and_other_parts_survive_a_save() {
    let saved = export(&import(&odt(&[])).unwrap()).unwrap();
    let fs = files(&saved);
    assert_eq!(file(&fs, "Basic/Standard/Module1.xml"), Some(MODULE.as_bytes()));
    assert_eq!(file(&fs, "Basic/script-lc.xml"), Some(&b"<library:libraries/>"[..]));
    assert_eq!(file(&fs, "manifest.rdf"), Some(&b"<rdf:RDF/>"[..]));
    assert_eq!(file(&fs, "other-app/data.bin"), Some(&b"\x00\x01\x02"[..]));
    // Listed in the manifest with their media types; a file the manifest didn't list isn't.
    let manifest = String::from_utf8(file(&fs, "META-INF/manifest.xml").unwrap().to_vec()).unwrap();
    for want in [
        r#"manifest:full-path="Basic/Standard/Module1.xml" manifest:media-type="text/xml""#,
        r#"manifest:full-path="manifest.rdf" manifest:media-type="application/rdf+xml""#,
    ] {
        assert!(manifest.contains(want), "{want} in {manifest}");
    }
    // A file the manifest didn't list is listed now, without a media type (ODF 1.3 Part 2 §4.3:
    // every file is); a folder with no file in it isn't.
    assert!(manifest.contains(r#"manifest:full-path="other-app/data.bin" manifest:media-type="""#), "{manifest}");
    assert!(!manifest.contains("Configurations2"), "{manifest}");
    // The old thumbnail and the signatures (of the package as it was) are gone.
    assert!(file(&fs, "Thumbnails/thumbnail.png").is_none() && file(&fs, "META-INF/documentsignatures.xml").is_none());
    // And again after a second save.
    let again = files(&export(&import(&saved).unwrap()).unwrap());
    assert_eq!(file(&again, "Basic/Standard/Module1.xml"), Some(MODULE.as_bytes()));
}

#[test]
fn kept_parts_survive_encryption() {
    let saved = encrypt(&export(&import(&odt(&[])).unwrap()).unwrap(), "pw").unwrap();
    let fs = files(&decrypt(&saved, "pw").unwrap());
    assert_eq!(file(&fs, "Basic/Standard/Module1.xml"), Some(MODULE.as_bytes()));
}

#[test]
fn parts_with_hostile_names_are_not_written() {
    let bad: [(&str, &[u8]); 5] = [("../evil.txt", b"x"), ("a//b.txt", b"x"), ("./c.txt", b"x"), ("d\\e.txt", b"x"), ("f/../../g.txt", b"x")];
    let fs = files(&export(&import(&odt(&bad)).unwrap()).unwrap());
    for (name, _) in bad {
        assert!(file(&fs, name).is_none(), "{name}");
    }
    assert!(fs.iter().all(|(n, _)| !n.contains("..")));
    // Parts named as WordCraft's own are WordCraft's.
    let own: [(&str, &[u8]); 2] = [("Pictures/stale.png", b"x"), ("ObjectReplacements/Object 9", b"x")];
    let fs = files(&export(&import(&odt(&own)).unwrap()).unwrap());
    assert!(file(&fs, "Pictures/stale.png").is_none() && file(&fs, "ObjectReplacements/Object 9").is_none());
}

#[test]
fn who_changed_the_document_last_and_when_round_trip() {
    // meta.xml: the author (`meta:initial-creator`), who changed it last (`dc:creator`), when
    // (`dc:date`) and how often it was saved (`meta:editing-cycles`).
    let mut d = wordcraft_doc::Document::new();
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("x", Default::default()))];
    d.core.creator = "Ada".into();
    d.core.last_modified_by = "Grace".into();
    d.core.created = "2026-01-02T03:04:05Z".into();
    d.core.modified = "2026-11-03T10:20:30Z".into();
    d.core.revision = 7;
    let saved = crate::odt::export(&d).unwrap();
    let meta = crate::odt::tests::entry(&saved, "meta.xml");
    assert!(meta.contains("<meta:initial-creator>Ada</meta:initial-creator>") && meta.contains("<dc:creator>Grace</dc:creator>"), "{meta}");
    assert!(meta.contains("<dc:date>2026-11-03T10:20:30</dc:date>") && meta.contains("<meta:editing-cycles>7</meta:editing-cycles>"), "{meta}");
    let back = crate::odt::import(&saved).unwrap();
    assert_eq!((back.core.last_modified_by.as_str(), back.core.modified.as_str(), back.core.revision), ("Grace", "2026-11-03T10:20:30Z", 7));
}

#[test]
fn every_file_written_has_its_manifest_entry() {
    // ODF 1.3 Part 2 §4.3: every file but `mimetype` and `META-INF/`'s is listed, a kept part
    // with no media type (LibreOffice's empty accelerator configuration) too: LibreOffice won't
    // open a package that lists one less.
    let saved = export(&import(&odt(&[("Configurations2/accelerator/current.xml", b"")])).unwrap()).unwrap();
    let manifest = crate::odt::tests::entry(&saved, "META-INF/manifest.xml");
    for (name, _) in files(&saved) {
        if name == "mimetype" || name.starts_with("META-INF/") || name.ends_with('/') {
            continue;
        }
        assert!(manifest.contains(&format!("manifest:full-path=\"{name}\"")), "{name}: {manifest}");
    }
    assert!(manifest.contains(r#"manifest:full-path="Configurations2/accelerator/current.xml" manifest:media-type="""#), "{manifest}");
}

#[test]
fn a_damaged_media_type_isnt_shown_in_the_error() {
    // A package whose `mimetype` holds a picture: refused, saying so without the bytes.
    let pkg = crate::odt::tests::zip_of(&[("mimetype", b"\x89PNG\r\n\x1a\n\x00\x00binary"), ("content.xml", b"<x/>")]);
    let e = import(&pkg).unwrap_err();
    assert_eq!(e, "ODT: not a text document");
    let pkg = crate::odt::tests::zip_of(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", b"<x/>")]);
    assert_eq!(import(&pkg).unwrap_err(), "ODT: not a text document (application/vnd.oasis.opendocument.spreadsheet)");
}
