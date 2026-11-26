//! A value chosen in a content control in place of its placeholder (a list entry, a date, a check
//! box's mark) takes the control's own run properties (`w:sdtPr/w:rPr`, ECMA-376 Part 1
//! §17.5.2.27), as typed text does (#3116). Packages are hand-written: the controls WordCraft
//! inserts carry no run properties.

use serde_json::json;
use wordcraft_doc::StoryRef;
use wordcraft_doc::props::CharProps;
use wordcraft_engine::Session;
use wordcraft_engine::io::{open_bytes, save_bytes};

const W: &str =
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
const RELS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;
const TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Opens a package with `body` and a grey Placeholder Text character style.
fn open(body: &str) -> wordcraft_doc::Document {
    let styles = r#"<w:style w:type="character" w:styleId="PlaceholderText"><w:name w:val="Placeholder Text"/><w:rPr><w:color w:val="808080"/></w:rPr></w:style>"#;
    let mut zip = craft_zip::Writer::new();
    for (name, value) in [
        (
            "_rels/.rels",
            format!(r#"<Relationships {RELS}><Relationship Id="rId1" Type="{TYPE}/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(r#"<Relationships {RELS}><Relationship Id="rId1" Type="{TYPE}/styles" Target="styles.xml"/></Relationships>"#),
        ),
        ("word/document.xml", format!(r#"<w:document {W}><w:body>{body}</w:body></w:document>"#)),
        ("word/styles.xml", format!(r#"<w:styles {W}>{styles}</w:styles>"#)),
    ] {
        zip.add(name, value.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    }
    open_bytes("form.docx", &zip.finish().unwrap()).unwrap()
}

/// A paragraph holding a control of `kind` (its `w:sdtPr` type element) tagged `tag`. Its
/// placeholder is grey and boxed; its run properties ask for blue Source Serif 4 at 14 pt.
fn control(tag: &str, kind: &str) -> String {
    let own = r#"<w:rPr><w:rFonts w:ascii="Source Serif 4" w:hAnsi="Source Serif 4"/><w:color w:val="1F4E79"/><w:sz w:val="28"/></w:rPr>"#;
    let shown =
        r#"<w:rPr><w:rStyle w:val="PlaceholderText"/><w:color w:val="808080"/><w:bdr w:val="single" w:sz="4" w:space="0" w:color="808080"/></w:rPr>"#;
    format!(
        r#"<w:p><w:sdt><w:sdtPr>{own}<w:tag w:val="{tag}"/><w:showingPlcHdr/>{kind}</w:sdtPr><w:sdtContent><w:r>{shown}<w:t>Choose</w:t></w:r></w:sdtContent></w:sdt></w:p>"#
    )
}

/// The formatting of the control's first character, and the control's own run properties.
fn formatting(d: &wordcraft_doc::Document, tag: &str) -> (CharProps, CharProps) {
    let r = d.control_ranges(StoryRef::Body).into_iter().find(|r| r.control.tag == tag).unwrap();
    let at = r.content_start();
    (d.para_at(&at).unwrap().props_of_char(at.off).clone(), r.control.run_props.clone().unwrap())
}

/// What shows of a run's formatting here: colour, font, size and box.
fn look(p: &CharProps) -> impl PartialEq + std::fmt::Debug {
    (p.color, p.font.clone(), p.size, p.border)
}

/// A list entry, a date or a check box's mark chosen in place of the placeholder takes the
/// control's run properties, as typed text does; it kept the placeholder's grey and box. A saved
/// file keeps the value's formatting.
#[test]
fn choosing_over_a_placeholder_takes_the_controls_formatting() {
    let entries = r#"<w:listItem w:displayText="Small" w:value="S"/><w:listItem w:displayText="Large" w:value="L"/>"#;
    let choices = [
        ("size", format!("<w:dropDownList>{entries}</w:dropDownList>"), "developer.chooseItem", json!({"control": "size", "index": 1})),
        (
            "due",
            r#"<w:date><w:dateFormat w:val="d MMMM yyyy"/></w:date>"#.into(),
            "developer.setDate",
            json!({"control": "due", "date": "2026-12-01"}),
        ),
        ("ok", r#"<w14:checkbox><w14:checked w14:val="0"/></w14:checkbox>"#.into(), "developer.toggleCheckBox", json!({"control": "ok"})),
    ];
    let body: String = choices.iter().map(|(tag, kind, ..)| control(tag, kind)).collect();
    let mut s = Session::new(open(&body));
    for (tag, _, command, params) in &choices {
        let (placeholder, own) = formatting(&s.doc, tag);
        assert_ne!(look(&placeholder), look(&own), "{tag}: the placeholder is formatted apart from its control");
        s.run(command, params).unwrap_or_else(|e| panic!("{command}: {e}"));
        assert_eq!(look(&formatting(&s.doc, tag).0), look(&own), "{tag}: {command} gives the control's run properties");
    }
    let saved = open_bytes("form.docx", &save_bytes("form.docx", &s.doc).unwrap()).unwrap();
    for (tag, ..) in &choices {
        let (value, own) = formatting(&saved, tag);
        assert_eq!(look(&value), look(&own), "{tag}: the saved value keeps the control's run properties");
    }
}
