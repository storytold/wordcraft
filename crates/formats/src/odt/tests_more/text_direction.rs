//! Turned text (ODF 1.3 §20.404 `style:writing-mode`, and LibreOffice's own
//! `loext:writing-mode="bt-lr"` for text running upwards): a table cell's and a text box's text
//! turned down (`tb-rl`) or up opens as WordCraft's cell text direction and text box direction,
//! and is written back the same way.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::props::{TextDirection, TextVert};

const NS_D: &str = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#;

fn open(styles: &str, body: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

fn cell_directions(d: &Document) -> Vec<TextDirection> {
    let t = d.body.iter().find_map(|b| b.as_table()).expect("a table");
    t.rows.iter().flat_map(|r| r.cells.iter()).map(|c| c.props.text_direction).collect()
}

fn box_directions(d: &Document) -> Vec<TextVert> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { kind: ShapeKind::TextBox, story: Some(id), .. } => d.parts.get(id).map(|p| p.body.vert),
            _ => None,
        })
        .collect()
}

#[test]
fn turned_cells_open_and_save_turned() {
    let cell_style =
        |n: &str, a: &str| format!(r#"<style:style style:name="{n}" style:family="table-cell"><style:table-cell-properties {a}/></style:style>"#);
    let styles = [
        cell_style("A", r#"style:writing-mode="lr-tb""#),
        cell_style("B", r#"style:writing-mode="tb-rl""#),
        cell_style("C", r#"loext:writing-mode="bt-lr""#),
    ]
    .concat();
    let cell = |s: &str| format!(r#"<table:table-cell table:style-name="{s}"><text:p>{s}</text:p></table:table-cell>"#);
    let body = format!(
        r#"<table:table><table:table-row>{}{}{}<table:table-cell><text:p>D</text:p></table:table-cell></table:table-row></table:table>"#,
        cell("A"),
        cell("B"),
        cell("C")
    );
    let d = open(&styles, &body);
    let want = [TextDirection::Horizontal, TextDirection::Down, TextDirection::Up, TextDirection::Horizontal];
    assert_eq!(cell_directions(&d), want);
    let saved = export(&d).unwrap();
    let c = content_of(&saved);
    assert!(c.contains(r#"style:writing-mode="tb-rl""#) && c.contains(r#"loext:writing-mode="bt-lr""#), "{c}");
    assert_eq!(cell_directions(&import(&saved).unwrap()), want);
}

#[test]
fn turned_text_boxes_open_and_save_turned() {
    let frame_style =
        |n: &str, a: &str| format!(r#"<style:style style:name="{n}" style:family="graphic"><style:graphic-properties {a}/></style:style>"#);
    let styles =
        [frame_style("down", r#"style:writing-mode="tb-rl""#), frame_style("up", r#"loext:writing-mode="bt-lr""#), frame_style("across", "")]
            .concat();
    let frame = |s: &str| {
        format!(
            r#"<draw:frame draw:style-name="{s}" text:anchor-type="as-char" svg:width="1in" svg:height="2in"><draw:text-box><text:p>{s}</text:p></draw:text-box></draw:frame>"#
        )
    };
    let d = open(&styles, &format!("<text:p>{}{}{}</text:p>", frame("down"), frame("up"), frame("across")));
    let want = [TextVert::Vert, TextVert::Vert270, TextVert::Horz];
    assert_eq!(box_directions(&d), want);
    let saved = export(&d).unwrap();
    assert_eq!(box_directions(&import(&saved).unwrap()), want);
}

#[test]
fn unknown_writing_modes_run_across() {
    let styles =
        r#"<style:style style:name="X" style:family="table-cell"><style:table-cell-properties style:writing-mode="sideways-??"/></style:style>"#;
    let d = open(
        styles,
        r#"<table:table><table:table-row><table:table-cell table:style-name="X"><text:p>x</text:p></table:table-cell></table:table-row></table:table>"#,
    );
    assert_eq!(cell_directions(&d), [TextDirection::Horizontal]);
}
