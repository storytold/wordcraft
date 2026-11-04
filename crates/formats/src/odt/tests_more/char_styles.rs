//! Custom character styles (ODF 1.3 §16.2 common `style:style` of family text): kept as document
//! character styles, the text in them keeping the style; written back as common text styles.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;

fn open() -> Document {
    let styles = format!(
        r##"<office:document-styles {NS_T}><office:styles><style:style style:name="Emphasis" style:family="text"><style:text-properties fo:font-style="italic"/></style:style><style:style style:name="Loud" style:display-name="Loud words" style:family="text" style:parent-style-name="Emphasis"><style:text-properties fo:font-weight="bold" fo:color="#aa0000"/></style:style></office:styles></office:document-styles>"##
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text" style:parent-style-name="Loud"><style:text-properties fo:font-size="18pt"/></style:style></office:automatic-styles><office:body><office:text><text:p>a <text:span text:style-name="Emphasis">em</text:span> b <text:span text:style-name="Loud">loud</text:span> c <text:span text:style-name="T1">big</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

/// Each run of the first paragraph: (its text, its character style's name, resolved bold, italic, size).
fn runs(d: &Document) -> Vec<(String, Option<String>, bool, bool, f32)> {
    let p = d.body[0].as_para().unwrap();
    p.run_ranges()
        .map(|(r, props)| {
            let c = d.styles.resolve_char(p.props.style.as_deref(), props);
            let name = props.style.as_ref().and_then(|id| d.styles.get(id)).map(|s| s.name.clone());
            (p.text.get(r).unwrap_or("").to_string(), name, c.bold, c.italic, c.size)
        })
        .filter(|r| r.0.trim().len() > 1)
        .collect()
}

#[test]
fn text_keeps_its_character_style() {
    let d = open();
    let r = runs(&d);
    assert_eq!(
        r.iter().map(|x| (x.0.as_str(), x.1.as_deref())).collect::<Vec<_>>(),
        [("em", Some("Emphasis")), ("loud", Some("Loud words")), ("big", Some("Loud words"))]
    );
    // As shown: its style's formatting, and the parent's.
    assert!(r[0].3 && !r[0].2);
    assert!(r[1].2 && r[1].3);
    assert!(r[2].2 && r[2].3 && r[2].4 == 18.0, "{:?}", r[2]);
    // The style's own formatting stays in it.
    let p = d.body[0].as_para().unwrap();
    let loud = p.run_ranges().find(|(r, _)| p.text.get(r.clone()) == Some("loud")).map(|(_, c)| c.clone()).unwrap();
    assert!(loud.bold.is_none() && loud.italic.is_none() && loud.color.is_none(), "{loud:?}");
}

#[test]
fn character_styles_round_trip() {
    let d = open();
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(runs(&back), runs(&d));
}

#[test]
fn what_a_character_style_turns_off_stays_off() {
    // "Plain" is based on the bold, italic "Loud" and turns both off.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Loud" style:family="text"><style:text-properties fo:font-weight="bold" fo:font-style="italic"/></style:style><style:style style:name="Plain" style:family="text" style:parent-style-name="Loud"><style:text-properties fo:font-weight="normal" fo:font-style="normal"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p><text:span text:style-name="Loud">a</text:span><text:span text:style-name="Plain">b</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let p = d.body[0].as_para().unwrap();
    let shown: Vec<(bool, bool)> =
        p.runs.iter().map(|r| d.styles.resolve_char(p.props.style.as_deref(), &r.props)).map(|c| (c.bold, c.italic)).collect();
    assert_eq!(shown, [(true, true), (false, false)]);
    let s = crate::odt::tests::entry(&export(&d).unwrap(), "styles.xml");
    let plain = s.split(r#"style:name="Plain""#).nth(1).and_then(|r| r.split("</style:style>").next()).unwrap_or("missing");
    assert!(plain.contains(r#"fo:font-weight="normal""#) && plain.contains(r#"fo:font-style="normal""#), "{plain}");
}

#[test]
fn a_character_style_based_on_a_missing_one_keeps_its_own_look() {
    // "Orphan" names a parent the file doesn't define; "Tilted" comes after it.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Orphan" style:family="text" style:parent-style-name="Nowhere"><style:text-properties fo:font-weight="bold"/></style:style><style:style style:name="Tilted" style:family="text"><style:text-properties fo:font-style="italic"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p><text:span text:style-name="Orphan">aa</text:span> <text:span text:style-name="Tilted">bb</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let r = runs(&d);
    assert_eq!(
        r.iter().map(|x| (x.0.as_str(), x.1.as_deref(), x.2, x.3)).collect::<Vec<_>>(),
        [("aa", Some("Orphan"), true, false), ("bb", Some("Tilted"), false, true)]
    );
    assert!(d.styles.get("Nowhere").is_none(), "no style for the missing parent");
}
