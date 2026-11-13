//! Right-to-left paragraphs (ODF 1.3 §20.404 `style:writing-mode` on paragraph properties: `rl-tb`
//! and `rl`), read as WordCraft's bidirectional paragraphs and written back.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;

fn open(mode: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties style:writing-mode="{mode}"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">שלום עולם</text:p><text:p>Left to right</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

fn bidi(d: &Document) -> Vec<Option<bool>> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.bidi).collect()
}

#[test]
fn right_to_left_paragraphs_are_read() {
    assert_eq!(bidi(&open("rl-tb")), [Some(true), None]);
    assert_eq!(bidi(&open("rl")), [Some(true), None]);
    assert_eq!(bidi(&open("lr-tb")), [None, None]);
}

#[test]
fn right_to_left_paragraphs_round_trip() {
    let d = open("rl-tb");
    assert_eq!(bidi(&import(&export(&d).unwrap()).unwrap()), [Some(true), None]);
}

/// A named left-to-right style based on a right-to-left one stays left to right, and so does a
/// paragraph whose own style says left to right over a right-to-left named style: through import
/// and through export and reimport.
#[test]
fn left_to_right_over_a_right_to_left_style_stays_left_to_right() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Rtl" style:family="paragraph"><style:paragraph-properties style:writing-mode="rl-tb"/></style:style><style:style style:name="Ltr" style:family="paragraph" style:parent-style-name="Rtl"><style:paragraph-properties style:writing-mode="lr-tb"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Rtl"><style:paragraph-properties style:writing-mode="lr-tb"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="Rtl">שלום</text:p><text:p text:style-name="Ltr">child</text:p><text:p text:style-name="P1">own</text:p></office:text></office:body></office:document-content>"#
    );
    let dirs = |d: &Document| d.body.iter().filter_map(|b| b.as_para()).map(|p| d.styles.resolve_para(&p.props).bidi).collect::<Vec<_>>();
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(dirs(&d), [true, false, false], "import");
    // The child style and the paragraph each keep their own left to right.
    assert_eq!(d.styles.get("Ltr").and_then(|s| s.para.bidi), Some(false));
    assert_eq!(bidi(&d), [None, None, Some(false)]);
    assert_eq!(dirs(&import(&export(&d).unwrap()).unwrap()), [true, false, false], "export and reimport");
}
