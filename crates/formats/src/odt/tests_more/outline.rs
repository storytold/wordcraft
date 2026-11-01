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
    let xml = crate::odt::tests::styles_of(&export(&open(NUMBERED)).unwrap());
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
