//! Positional tabs (`w:ptab`, ECMA-376 Part 1 §17.3.3.23). Fixtures are hand-written from the spec.

use wordcraft_doc::InlineObject;
use wordcraft_doc::para::{PTabAlign, PTabBase};
use wordcraft_doc::props::TabLeader;

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = craft_zip::Writer::new();
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.add(name, bytes, craft_zip::Compression::Deflated).unwrap();
    }
    zw.finish().unwrap()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();

    String::from_utf8(z.read("word/document.xml", 16 << 20).unwrap().unwrap()).unwrap()
}

fn ptab(relative_to: &str, alignment: &str, leader: &str) -> String {
    format!(r#"<w:r><w:ptab w:relativeTo="{relative_to}" w:alignment="{alignment}" w:leader="{leader}"/></w:r>"#)
}

#[test]
fn positional_tabs_keep_their_alignment_base_and_leader() {
    let body = format!(
        r#"<w:p><w:r><w:t>A</w:t></w:r>{}<w:r><w:t>B</w:t></w:r>{}<w:r><w:t>C</w:t></w:r>{}<w:r><w:t>D</w:t></w:r></w:p>"#,
        ptab("margin", "right", "dot"),
        ptab("indent", "center", "none"),
        ptab("margin", "left", "underscore"),
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    assert_eq!(p.plain_text(), "A\tB\tC\tD");
    assert_eq!(
        p.objects,
        vec![
            InlineObject::PositionalTab { align: PTabAlign::Right, relative_to: PTabBase::Margin, leader: TabLeader::Dot },
            InlineObject::PositionalTab { align: PTabAlign::Center, relative_to: PTabBase::Indent, leader: TabLeader::None },
            InlineObject::PositionalTab { align: PTabAlign::Left, relative_to: PTabBase::Margin, leader: TabLeader::Underscore },
        ]
    );
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    for (rel, align, leader) in [("margin", "right", "dot"), ("indent", "center", "none"), ("margin", "left", "underscore")] {
        let want = format!(r#"<w:ptab w:relativeTo="{rel}" w:alignment="{align}" w:leader="{leader}"/>"#);
        assert!(xml.contains(&want), "{want} missing from {xml}");
    }
    assert!(!xml.contains("<w:tab/>"), "{xml}");
}

/// A positional tab whose alignment or base isn't one the spec defines stays an ordinary tab.
#[test]
fn a_positional_tab_with_an_unknown_alignment_is_a_plain_tab() {
    let body = format!(r#"<w:p><w:r><w:t>A</w:t></w:r>{}<w:r><w:t>B</w:t></w:r></w:p>"#, ptab("margin", "decimal", "none"));
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    assert_eq!(p.plain_text(), "A\tB");
    assert!(p.objects.is_empty(), "{:?}", p.objects);
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains("<w:tab/>") && !xml.contains("w:ptab"), "{xml}");
}
