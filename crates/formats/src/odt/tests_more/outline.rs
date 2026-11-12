//! Heading numbering (ODF 1.3 §16.34 `text:outline-style`): the outline style's levels number the
//! headings of those levels; WordCraft numbers them through the heading styles, as Word does.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::section::NumFormat;

fn open(outline: &str) -> Document {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:default-outline-level="1"/><style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph" style:default-outline-level="2"/>{outline}</office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:h text:style-name="Heading_20_1" text:outline-level="1">One</text:h><text:h text:style-name="Heading_20_2" text:outline-level="2">One one</text:h><text:p>Body</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

const NUMBERED: &str = r#"<text:outline-style style:name="Outline"><text:outline-level-style text:level="1" style:num-format="1" style:num-suffix="."/><text:outline-level-style text:level="2" style:num-format="1" style:num-suffix="." text:display-levels="2"/><text:outline-level-style text:level="3" style:num-format=""/></text:outline-style>"#;

/// Each heading style's numbering: (its level's format and text).
fn heading_numbers(d: &Document) -> Vec<Option<(NumFormat, String)>> {
    (1..=3)
        .map(|n| {
            let (pp, _) = d.styles.para_style_props(&format!("Heading{n}"));
            let r = pp.numbering?;
            let lv = d.numbering.level(r.num, r.level)?;
            (lv.format != NumFormat::None).then(|| (lv.format, lv.text.clone()))
        })
        .collect()
}

#[test]
fn an_outline_style_numbers_the_headings() {
    let d = open(NUMBERED);
    assert_eq!(heading_numbers(&d), [Some((NumFormat::Decimal, "%1.".into())), Some((NumFormat::Decimal, "%1.%2.".into())), None]);
    // Both levels are one list, so 1.1 follows 1.
    let num = |n: u32| d.styles.para_style_props(&format!("Heading{n}")).0.numbering.map(|r| (r.num, r.level));
    assert_eq!(num(1).map(|n| n.0), num(2).map(|n| n.0));
    assert_eq!((num(1).map(|n| n.1), num(2).map(|n| n.1)), (Some(0), Some(1)));
}

#[test]
fn an_unnumbered_outline_numbers_nothing() {
    let none = r#"<text:outline-style style:name="Outline"><text:outline-level-style text:level="1" style:num-format=""/></text:outline-style>"#;
    assert_eq!(heading_numbers(&open(none)), [None, None, None]);
    assert_eq!(heading_numbers(&open("")), [None, None, None]);
}

#[test]
fn heading_numbering_round_trips() {
    let d = open(NUMBERED);
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(heading_numbers(&back), heading_numbers(&d));
    // A document without numbered headings stays without.
    let plain = open("");
    assert_eq!(heading_numbers(&import(&export(&plain).unwrap()).unwrap()), [None, None, None]);
}

#[test]
fn exported_heading_numbers_are_followed_by_a_tab() {
    // Without label alignment LibreOffice set the number against the heading's text.
    let xml = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&open(NUMBERED)).unwrap());
    let outline = xml.split("<text:outline-style").nth(1).unwrap_or("");
    assert!(outline.contains(r#"text:label-followed-by="listtab""#), "{outline}");
    assert!(outline.contains(r#"text:list-level-position-and-space-mode="label-alignment""#));
}

#[test]
fn numbered_headings_are_not_written_as_list_items() {
    // The outline style numbers them; wrapped in a list they took that list's indents.
    let xml = crate::odt::tests::content_of(&export(&open(NUMBERED)).unwrap());
    assert!(!xml.contains("<text:list"), "{xml}");
    assert!(xml.contains(r#"<text:h text:style-name="Heading_20_2" text:outline-level="2">One one</text:h>"#), "{xml}");
}

#[test]
fn unnumbered_heading_styles_are_detached_from_the_outline() {
    // LibreOffice gives a heading style the outline's numbering unless it names no list style;
    // an unnumbered level then still indented the heading.
    let xml = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&open("")).unwrap());
    let h1 = xml.split("<style:style ").find(|s| s.starts_with(r#"style:name="Heading_20_1""#)).unwrap_or("");
    assert!(h1.contains(r#"style:list-style-name="""#), "{h1}");
    // A numbered level stays attached.
    let xml = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&open(NUMBERED)).unwrap());
    let h1 = xml.split("<style:style ").find(|s| s.starts_with(r#"style:name="Heading_20_1""#)).unwrap_or("");
    assert!(!h1.contains("style:list-style-name"), "{h1}");
}

#[test]
fn a_heading_looks_as_its_style_and_has_the_level_its_element_gives() {
    // Google Docs' 2011 export: `<text:h>` without `text:outline-level` (level 1, §19.850.4) in a
    // style based on Heading 2. It looks as Heading 2 (as LibreOffice draws it) and is at outline
    // level 1; saved, both stay.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph"><style:text-properties fo:font-size="18pt" fo:font-weight="bold"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P7" style:family="paragraph" style:parent-style-name="Heading_20_2"/></office:automatic-styles><office:body><office:text><text:h text:style-name="P7">Clarifying the beat</text:h><text:h text:style-name="P7" text:outline-level="2">Second</text:h></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let props = |d: &Document, i: usize| d.body[i].as_para().unwrap().props.clone();
    let p = props(&d, 0);
    assert_eq!((p.style.as_deref(), p.outline_level), (Some("Heading2"), Some(0)));
    let p = props(&d, 1);
    assert_eq!(p.style.as_deref(), Some("Heading2"));
    assert!(p.outline_level.is_none() || p.outline_level == Some(1));
    assert_eq!(d.styles.resolve_char(p.style.as_deref(), &Default::default()).size, 18.0);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"text:outline-level="1">Clarifying"#) || c.contains(r#"text:outline-level="1"><"#), "{c}");
    let back = import(&saved).unwrap();
    assert_eq!((props(&back, 0).style.as_deref(), props(&back, 0).outline_level), (Some("Heading2"), Some(0)));
}
