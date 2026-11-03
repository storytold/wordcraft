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

#[test]
fn a_table_styles_look_is_written_into_its_cells() {
    // A WordCraft table in a built-in style (blue header row, banded rows, bold header text):
    // ODF has no table styles of that kind, so each cell gets what the style gives it, and
    // LibreOffice (and WordCraft, reopening it) draws the same table.
    let mut d = Document::new();
    let mut t = wordcraft_doc::Table::new(3, 2, 288.0);
    t.props.style = Some("GridTable4AccentBlue".into());
    t.props.look.header_row = true;
    t.props.look.banded_rows = true;
    for (ri, row) in t.rows.iter_mut().enumerate() {
        for (ci, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("r{ri}c{ci}"), Default::default()))];
        }
    }
    d.body = vec![std::sync::Arc::new(Block::Table(t)), wordcraft_doc::para_block(wordcraft_doc::Paragraph::default())];
    let st = d.styles.table_style("GridTable4AccentBlue").unwrap().parts;
    let back = import(&export(&d).unwrap()).unwrap();
    let c = cells(&back);
    assert_eq!(c[0].shading, st.header_fill, "the header row's fill");
    assert_eq!(c[2].shading, st.band_fill, "the first band's fill");
    assert_eq!(c[4].shading, None, "the second band is unfilled");
    assert!(side(c[2].borders, |b| b.left).is_some(), "the style's cell borders");
    let t = back.body.iter().find_map(|b| if let Block::Table(t) = &**b { Some(t.clone()) } else { None }).unwrap();
    let first = t.rows[0].cells[0].blocks[0].as_para().unwrap();
    assert_eq!(first.runs[0].props.bold, Some(true), "the header row's bold text");
    let body = t.rows[1].cells[1].blocks[0].as_para().unwrap();
    assert_ne!(body.runs[0].props.bold, Some(true));
}

#[test]
fn a_cells_background_picture_is_read_drawn_and_written() {
    // LibreOffice's elegant agenda: a small gradient at the top left of a cell (§17.3
    // `style:background-image`, no-repeat, top left), under its text.
    let png = {
        let img = image::RgbImage::from_pixel(40, 8, image::Rgb([40, 60, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let styles = r#"<style:style style:name="C1" style:family="table-cell"><style:table-cell-properties fo:border="none"><style:background-image xlink:href="Pictures/g.png" xlink:type="simple" xlink:actuate="onLoad" style:position="top left" style:repeat="no-repeat"/></style:table-cell-properties></style:style>"#;
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text><table:table><table:table-column/><table:table-row><table:table-cell table:style-name="C1"><text:p>Title</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"#
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", o).unwrap();
    std::io::Write::write_all(&mut z, b"application/vnd.oasis.opendocument.text").unwrap();
    z.start_file("content.xml", o).unwrap();
    std::io::Write::write_all(&mut z, content.as_bytes()).unwrap();
    z.start_file("Pictures/g.png", o).unwrap();
    std::io::Write::write_all(&mut z, &png).unwrap();
    let d = import(&z.finish().unwrap().into_inner()).unwrap();
    let pic = cells(&d)[0].picture.clone().expect("the cell's picture");
    assert_eq!((pic.repeat, pic.align), (wordcraft_doc::props::PictureRepeat::None, [0.0, 0.0]));
    assert_eq!(pic.size, [30.0, 6.0], "its own size: 40 × 8 pixels at 96 per inch");
    assert!(d.media.contains_key(&pic.media));
    // Written back with the picture in the package.
    let saved = export(&d).unwrap();
    let back = import(&saved).unwrap();
    let again = cells(&back)[0].picture.clone().expect("kept");
    assert_eq!((again.repeat, again.align, again.size), (pic.repeat, pic.align, pic.size));
}

#[test]
fn what_a_covered_cell_holds_stays_out_of_the_document() {
    // ONLYOFFICE writes paragraphs into covered cells (§9.1.5: not shown); they went before the
    // table as empty paragraphs, pushing it down the page.
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell table:number-columns-spanned="2"><text:p>wide</text:p></table:table-cell><table:covered-table-cell><text:p>hidden</text:p><text:list><text:list-item><text:p/></text:list-item></text:list></table:covered-table-cell></table:table-row><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row></table:table><text:p>after</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    assert!(matches!(&*d.body[0], Block::Table(_)), "the table comes first");
    assert_eq!(d.body.len(), 2);
    assert_eq!(d.body[1].as_para().unwrap().text, "after");
    let Block::Table(t) = &*d.body[0] else { panic!() };
    assert_eq!(t.rows[0].cells[0].props.span, 2);
}
