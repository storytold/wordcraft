//! OpenType ligatures (`w14:ligatures`, [MS-DOCX]) in document defaults, styles and runs.
//! Synthetic packages, written here; no external documents are needed.

use std::io::Write;
use wordcraft_doc::props::Ligatures;
use wordcraft_doc::{Document, Paragraph};

fn package(defaults: &str, style: &str, run: &str) -> Vec<u8> {
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w14""#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let document = format!(
        r#"<w:document {ns}><w:body><w:p><w:r><w:rPr><w:rStyle w:val="Fancy"/>{run}</w:rPr><w:t>office affinity</w:t></w:r></w:p></w:body></w:document>"#
    );
    let styles = format!(
        r#"<w:styles {ns}><w:docDefaults><w:rPrDefault><w:rPr>{defaults}</w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="character" w:styleId="Fancy"><w:name w:val="Fancy"/><w:rPr>{style}</w:rPr></w:style></w:styles>"#
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [
        ("_rels/.rels", root),
        ("word/_rels/document.xml.rels", rels),
        ("word/document.xml", document.as_str()),
        ("word/styles.xml", styles.as_str()),
    ] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn paragraph(d: &Document) -> &Paragraph {
    d.body.iter().find_map(|b| b.as_para()).unwrap()
}

fn lig(name: &str) -> Option<Ligatures> {
    Some(Ligatures::from_ooxml(name).unwrap())
}

/// Word stores the Font dialog's Ligatures choice as `w14:ligatures`; each level keeps its own
/// value through reading and saving, and an explicit `none` is not the same as no value.
#[test]
fn ligatures_in_defaults_styles_and_runs_survive_saving() {
    let el = |v: &str| format!(r#"<w14:ligatures w14:val="{v}"/>"#);
    let bytes = package(&el("standardContextual"), &el("none"), &el("historicalDiscretional"));
    let mut d = wordcraft_docx::read(&bytes).unwrap();
    for generation in 0..3 {
        assert_eq!(d.styles.default_chr.ligatures, lig("standardContextual"), "defaults, generation {generation}");
        assert_eq!(d.styles.get("Fancy").and_then(|s| s.chr.ligatures), lig("none"), "style, generation {generation}");
        let run = &paragraph(&d).runs[0].props;
        assert_eq!(run.ligatures, lig("historicalDiscretional"), "run, generation {generation}");
        assert_eq!(run.text_effects, None, "not read as a WordArt effect");
        d = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    }
}

/// A value [MS-DOCX] doesn't define is ignored, the run keeps inheriting.
#[test]
fn unknown_ligature_values_are_ignored() {
    let d = wordcraft_docx::read(&package("", "", r#"<w14:ligatures w14:val="fancy"/>"#)).unwrap();
    assert_eq!(paragraph(&d).runs[0].props.ligatures, None);
    let d = wordcraft_docx::read(&package("", "", "<w14:ligatures/>")).unwrap();
    assert_eq!(paragraph(&d).runs[0].props.ligatures, None);
}

/// Without a value nothing is written, so documents that never asked for ligatures stay as they
/// were.
#[test]
fn no_ligatures_element_without_a_value() {
    let d = wordcraft_docx::read(&package("", "", "")).unwrap();
    let out = wordcraft_docx::write(&d).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(out)).unwrap();
    for name in ["word/document.xml", "word/styles.xml"] {
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut z.by_name(name).unwrap(), &mut xml).unwrap();
        assert!(!xml.contains("ligatures"), "{name}");
    }
}
