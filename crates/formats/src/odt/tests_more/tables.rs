//! Table cell borders and padding (ODF 1.3 §20.183 `fo:border*`, §20.217–20.221 `fo:padding*`).
//! LibreOffice gives each shared edge to one cell only (left and bottom, the first row's top, the
//! last column's right); WordCraft's cells each have all four sides.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::props::{Border, BorderStyle, Borders, CellProps};
use wordcraft_doc::{Block, Document, Rgb};

const LO_TABLE: &str = r##"<style:style style:name="A1" style:family="table-cell"><style:table-cell-properties fo:padding="0.1cm" fo:border-left="0.5pt solid #000000" fo:border-right="none" fo:border-top="0.5pt solid #000000" fo:border-bottom="0.5pt solid #000000"/></style:style><style:style style:name="B1" style:family="table-cell"><style:table-cell-properties fo:padding="0.1cm" fo:border="0.5pt solid #000000"/></style:style><style:style style:name="A2" style:family="table-cell"><style:table-cell-properties fo:padding-left="2pt" fo:padding-right="3pt" fo:padding-top="0pt" fo:padding-bottom="1pt" fo:border-left="0.5pt solid #000000" fo:border-right="none" fo:border-top="none" fo:border-bottom="2pt double #ff0000"/></style:style><style:style style:name="B2" style:family="table-cell"><style:table-cell-properties fo:border-left="0.5pt solid #000000" fo:border-right="0.5pt solid #000000" fo:border-top="none" fo:border-bottom="0.5pt dashed #000000"/></style:style>"##;

fn table_doc(styles: &str, cells: [&str; 4]) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell table:style-name="{}"><text:p>a</text:p></table:table-cell><table:table-cell table:style-name="{}"><text:p>b</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell table:style-name="{}"><text:p>c</text:p></table:table-cell><table:table-cell table:style-name="{}"><text:p>d</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"#,
        cells[0], cells[1], cells[2], cells[3]
    );
    import(&package(&content, None)).unwrap()
}

fn cells(d: &Document) -> Vec<CellProps> {
    let t = d.body.iter().find_map(|b| if let Block::Table(t) = &**b { Some(t.clone()) } else { None }).expect("a table");
    t.rows.iter().flat_map(|r| r.cells.iter().map(|c| c.props.clone())).collect()
}

fn side(b: Option<Borders>, f: fn(&Borders) -> Option<Border>) -> Option<(BorderStyle, f32)> {
    b.as_ref().and_then(f).filter(|b| b.is_visible()).map(|b| (b.style, b.width))
}

#[test]
fn a_shared_edge_belongs_to_both_cells() {
    let d = table_doc(LO_TABLE, ["A1", "B1", "A2", "B2"]);
    let c = cells(&d);
    // A1's right is "none" but B1's left draws the edge: both cells have it.
    assert_eq!(side(c[0].borders, |b| b.right), Some((BorderStyle::Single, 0.5)));
    // A2's top is "none" but A1's bottom draws it.
    assert_eq!(side(c[2].borders, |b| b.top), Some((BorderStyle::Single, 0.5)));
    // Each cell's own lines.
    assert_eq!(side(c[2].borders, |b| b.bottom), Some((BorderStyle::Double, 2.0)));
    assert_eq!(c[2].borders.and_then(|b| b.bottom).and_then(|b| b.color), Some(Rgb(255, 0, 0)));
    assert_eq!(side(c[3].borders, |b| b.bottom), Some((BorderStyle::Dashed, 0.5)));
}

#[test]
fn cell_padding_is_read() {
    let d = table_doc(LO_TABLE, ["A1", "B1", "A2", "B2"]);
    let c = cells(&d);
    let m = c[0].margins.expect("padding");
    assert!(m.iter().all(|v| (v - 2.835).abs() < 0.01), "{m:?}");
    assert_eq!(c[2].margins, Some([0.0, 2.0, 1.0, 3.0]));
    assert_eq!(c[3].margins, None);
}

#[test]
fn a_table_without_lines_stays_without() {
    let none = r#"<style:style style:name="N" style:family="table-cell"><style:table-cell-properties fo:border="none"/></style:style>"#;
    let d = table_doc(none, ["N", "N", "N", "N"]);
    assert!(cells(&d).iter().all(|c| !c.borders.is_some_and(|b| b.any_visible())));
    let back = import(&export(&d).unwrap()).unwrap();
    assert!(cells(&back).iter().all(|c| !c.borders.is_some_and(|b| b.any_visible())), "{:?}", cells(&back));
}

#[test]
fn borders_and_padding_round_trip() {
    let d = table_doc(LO_TABLE, ["A1", "B1", "A2", "B2"]);
    let back = import(&export(&d).unwrap()).unwrap();
    let (a, b) = (cells(&d), cells(&back));
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        for f in [|b: &Borders| b.top, |b: &Borders| b.left, |b: &Borders| b.bottom, |b: &Borders| b.right] {
            assert_eq!(side(x.borders, f), side(y.borders, f), "cell {i}");
        }
        // A cell without padding of its own has the table's (Word's default), written out.
        let shown = |c: &CellProps| c.resolved_margins([0.0, 5.4, 0.0, 5.4]).map(|v| (v * 100.0).round());
        assert_eq!(shown(x), shown(y), "cell {i}");
    }
}

#[test]
fn hostile_borders_do_not_panic() {
    let bad = r#"<style:style style:name="X" style:family="table-cell"><style:table-cell-properties fo:border="1e30pt solid #zzzzzz" fo:padding="-5pt" fo:border-top="solid" fo:border-left="NaNpt dotted" fo:padding-left="1e30pt"/></style:style>"#;
    let d = table_doc(bad, ["X", "X", "X", "missing"]);
    for c in cells(&d) {
        if let Some(b) = c.borders {
            for s in [b.top, b.left, b.bottom, b.right].into_iter().flatten() {
                assert!(s.width.is_finite() && (0.0..=12.0).contains(&s.width), "{s:?}");
            }
        }
        if let Some(m) = c.margins {
            assert!(m.iter().all(|v| (0.0..=1584.0).contains(v)), "{m:?}");
        }
    }
    let _ = export(&d).unwrap();
}

#[test]
fn only_rows_set_to_repeat_are_written_as_header_rows() {
    // LibreOffice's agendas: a table without repeated rows came back with its first row
    // repeating on the next page, because a table style's header row (on by default in Word's
    // table look) was taken for Repeat as header row.
    let mut d = wordcraft_doc::Document::from_text("");
    let mut t = wordcraft_doc::Table::new(3, 2, 400.0);
    assert!(t.props.look.header_row);
    d.body.push(std::sync::Arc::new(wordcraft_doc::Block::Table(t.clone())));
    let out = crate::odt::tests::content_of(&crate::odt::export(&d).unwrap());
    assert!(!out.contains("table-header-rows"), "{out}");
    // A row set to repeat is one.
    t.rows[0].props.header = true;
    d.body.pop();
    d.body.push(std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
    let out = crate::odt::tests::content_of(&crate::odt::export(&d).unwrap());
    assert_eq!(out.matches("<table:table-header-rows>").count(), 1, "{out}");
    let back = crate::odt::import(&crate::odt::export(&d).unwrap()).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.iter().map(|r| r.props.header).collect::<Vec<_>>(), [true, false, false]);
}
