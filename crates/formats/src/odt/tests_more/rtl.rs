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
