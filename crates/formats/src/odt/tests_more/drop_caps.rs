//! Drop caps (ODF 1.3 §17.9 `style:drop-cap`): a paragraph's first letter over several lines,
//! its distance from the text, kept as WordCraft's drop cap and written back.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;

fn open(drop: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties>{drop}</style:paragraph-properties></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">Once upon a time there was a long paragraph.</text:p><text:p>Plain.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

/// Each paragraph's drop cap: (lines, distance).
fn caps(d: &Document) -> Vec<(Option<u8>, Option<f32>)> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| (p.props.drop_cap, p.props.drop_cap_distance.map(|v| (v * 100.0).round() / 100.0))).collect()
}

#[test]
fn a_drop_cap_opens() {
    assert_eq!(caps(&open(r#"<style:drop-cap style:lines="3" style:length="1" style:distance="0.1in"/>"#)), [(Some(3), Some(7.2)), (None, None)]);
    // One line is no drop cap (LibreOffice's "1").
    assert_eq!(caps(&open(r#"<style:drop-cap style:lines="1"/>"#)), [(None, None), (None, None)]);
}

#[test]
fn drop_caps_round_trip() {
    let d = open(r#"<style:drop-cap style:lines="2" style:distance="4pt"/>"#);
    let bytes = crate::odt::tests::export_model(&d).unwrap();
    assert!(content_of(&bytes).contains(r#"<style:drop-cap style:lines="2" style:length="1" style:distance="4pt"/>"#), "{}", content_of(&bytes));
    assert_eq!(caps(&import(&bytes).unwrap()), caps(&d));
}

#[test]
fn hostile_drop_caps_do_not_panic() {
    for drop in
        [r#"<style:drop-cap style:lines="99999999999" style:distance="1e30in"/>"#, r#"<style:drop-cap style:lines="-3" style:length="word"/>"#]
    {
        let d = open(drop);
        assert!(caps(&d).iter().all(|(l, dist)| l.is_none_or(|l| l <= 10) && dist.is_none_or(|v| v.is_finite() && v <= 1584.0)), "{drop}");
        let _ = export(&d).unwrap();
    }
}
