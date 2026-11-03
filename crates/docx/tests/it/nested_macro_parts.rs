//! A VBA project kept with a part WordCraft carries unchanged (an unmodelled part, the glossary, and
//! the parts they reach) follows the main document's own: kept in a package that can hold macros,
//! left out of one that can't (#3629). A hand-written synthetic package, no files produced by Word.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use wordcraft_docx::Flavor;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const OFFICE_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const GLOSSARY: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument";
const REL_VBA: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
const REL_VBA_DATA: &str = "http://schemas.microsoft.com/office/2006/relationships/wordVbaData";
const REL_EXTENSION: &str = "urn:example:extension";
const REL_NOTE: &str = "urn:example:note";
const REL_ODD: &str = "urn:example:odd";

const CT: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/octet-stream"/><Override PartName="/word/document.xml" ContentType="application/vnd.ms-word.document.macroEnabled.main+xml"/><Override PartName="/word/extra.xml" ContentType="application/vnd.example.extra+xml"/><Override PartName="/word/note.xml" ContentType="application/vnd.example.note+xml"/><Override PartName="/word/nested/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/word/nested/vbaData.xml" ContentType="application/vnd.ms-word.vbaData+xml"/><Override PartName="/word/odd.bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/word/glossary/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml"/><Override PartName="/word/glossary/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/word/glossary/vbaData.xml" ContentType="application/vnd.ms-word.vbaData+xml"/></Types>"#;

const EXTRA: &str = r#"<x:extra xmlns:x="urn:example:extension"/>"#;
const NOTE: &str = r#"<x:note xmlns:x="urn:example:extension"/>"#;
const VBA_DATA: &str = r#"<wne:vbaSuppData xmlns:wne="http://schemas.microsoft.com/office/word/2006/wordml"/>"#;
const ENTRY: &str = r#"<w:docPart><w:docPartPr><w:name w:val="Signature"/><w:category><w:name w:val="General"/><w:gallery w:val="docParts"/></w:category><w:types><w:type w:val="normal"/></w:types><w:guid w:val="{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}"/></w:docPartPr><w:docPartBody><w:p><w:r><w:t>Kind regards</w:t></w:r></w:p></w:docPartBody></w:docPart>"#;

fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        assert!(out.insert(f.name().to_string(), b).is_none(), "duplicate part {}", f.name());
    }
    out
}

fn text(files: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(files.get(name).unwrap_or_else(|| panic!("no {name} in {:?}", files.keys())).clone()).unwrap()
}

fn rels(list: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s =
        String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

/// A recognisable stand-in for a compiled VBA project (an OLE compound file in real life).
fn fake_vba(seed: u32) -> Vec<u8> {
    let mut v = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    v.extend((0..2000u32).map(|i| (i.wrapping_add(seed).wrapping_mul(2_654_435_761) >> 11) as u8));
    v
}

/// A package whose main document relates an unmodelled part, `word/extra.xml`, which relates a
/// VBA project (with its VBA data) and a harmless part; a part of an unknown relationship type
/// that has a VBA project's content type; and a glossary that relates a VBA project of its own.
/// None of them is named by id in the body, so the reader keeps them as parts it doesn't model.
fn package() -> Vec<u8> {
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#
    );
    let glossary = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:glossaryDocument xmlns:w="{W}"><w:docParts>{ENTRY}</w:docParts></w:glossaryDocument>"#
    );
    zip(&[
        ("[Content_Types].xml", CT.as_bytes().to_vec()),
        ("_rels/.rels", rels(&[("rId1", OFFICE_DOC, "word/document.xml")])),
        ("word/document.xml", body.into_bytes()),
        (
            "word/_rels/document.xml.rels",
            rels(&[("rId1", REL_EXTENSION, "extra.xml"), ("rId2", REL_ODD, "odd.bin"), ("rId3", GLOSSARY, "glossary/document.xml")]),
        ),
        ("word/extra.xml", EXTRA.as_bytes().to_vec()),
        ("word/_rels/extra.xml.rels", rels(&[("rId1", REL_VBA, "nested/vbaProject.bin"), ("rId2", REL_NOTE, "note.xml")])),
        ("word/note.xml", NOTE.as_bytes().to_vec()),
        ("word/nested/vbaProject.bin", fake_vba(1)),
        ("word/nested/_rels/vbaProject.bin.rels", rels(&[("rId1", REL_VBA_DATA, "vbaData.xml")])),
        ("word/nested/vbaData.xml", VBA_DATA.as_bytes().to_vec()),
        ("word/odd.bin", fake_vba(2)),
        ("word/glossary/document.xml", glossary.into_bytes()),
        ("word/glossary/_rels/document.xml.rels", rels(&[("rId1", REL_VBA, "vbaProject.bin")])),
        ("word/glossary/vbaProject.bin", fake_vba(3)),
        ("word/glossary/_rels/vbaProject.bin.rels", rels(&[("rId1", REL_VBA_DATA, "vbaData.xml")])),
        ("word/glossary/vbaData.xml", VBA_DATA.as_bytes().to_vec()),
    ])
}

fn save(flavor: Flavor) -> BTreeMap<String, Vec<u8>> {
    let doc = wordcraft_docx::read(&package()).expect("read");
    unzip(&wordcraft_docx::write_as(&doc, flavor).expect("write"))
}

/// The relationship parts of `files`, joined.
fn all_rels(files: &BTreeMap<String, Vec<u8>>) -> String {
    files.iter().filter(|(n, _)| n.ends_with(".rels")).map(|(n, b)| format!("{n}: {}\n", String::from_utf8_lossy(b))).collect()
}

#[test]
fn a_vba_project_kept_with_a_carried_part_is_left_out_of_a_macro_free_save() {
    for flavor in [Flavor::Document, Flavor::Template] {
        let files = save(flavor);
        let input = unzip(&package());
        // The unmodelled part, the harmless part it relates and the glossary are carried as before.
        for part in ["word/extra.xml", "word/note.xml", "word/glossary/document.xml"] {
            assert_eq!(files.get(part), input.get(part), "{flavor:?}: {part} not carried verbatim");
        }
        let extra_rels = text(&files, "word/_rels/extra.xml.rels");
        assert!(extra_rels.contains(REL_NOTE) && extra_rels.contains(r#"Target="note.xml""#), "{flavor:?}: {extra_rels}");
        // No VBA project, VBA data or relationship to one, under any name.
        let vba = [fake_vba(1), fake_vba(2), fake_vba(3)];
        for (name, bytes) in &files {
            assert!(!vba.contains(bytes) && !name.to_ascii_lowercase().ends_with(".bin"), "{flavor:?}: {name} written");
            assert!(!name.ends_with("vbaData.xml"), "{flavor:?}: {name} written");
        }
        let rels = all_rels(&files);
        for ty in [REL_VBA, REL_VBA_DATA, REL_ODD] {
            assert!(!rels.contains(&format!(r#"Type="{ty}""#)), "{flavor:?}: {ty} in {rels}");
        }
        let ct = text(&files, "[Content_Types].xml");
        assert!(!ct.contains("vbaProject") && !ct.contains("vbaData"), "{flavor:?}: {ct}");
    }
}

#[test]
fn a_vba_project_kept_with_a_carried_part_stays_in_a_macro_enabled_save() {
    for flavor in [Flavor::MacroDocument, Flavor::MacroTemplate] {
        let files = save(flavor);
        let input = unzip(&package());
        for part in [
            "word/extra.xml",
            "word/note.xml",
            "word/nested/vbaProject.bin",
            "word/nested/vbaData.xml",
            "word/odd.bin",
            "word/glossary/document.xml",
            "word/glossary/vbaProject.bin",
            "word/glossary/vbaData.xml",
        ] {
            assert_eq!(files.get(part), input.get(part), "{flavor:?}: {part} not carried verbatim");
        }
        let extra_rels = text(&files, "word/_rels/extra.xml.rels");
        assert!(extra_rels.contains(&format!(r#"Type="{REL_VBA}""#)) && extra_rels.contains(REL_NOTE), "{flavor:?}: {extra_rels}");
        assert!(text(&files, "word/nested/_rels/vbaProject.bin.rels").contains(REL_VBA_DATA), "{flavor:?}");
        assert!(text(&files, "word/_rels/document.xml.rels").contains(REL_ODD), "{flavor:?}");
        assert!(text(&files, "word/glossary/_rels/document.xml.rels").contains(&format!(r#"Type="{REL_VBA}""#)), "{flavor:?}");
        assert!(text(&files, "word/glossary/_rels/vbaProject.bin.rels").contains(REL_VBA_DATA), "{flavor:?}");
    }
}
