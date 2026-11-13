//! A table whose `w:tblGrid` (ECMA-376 Part 1 §17.4.48) lists fewer `w:gridCol` than its widest row
//! has cells is saved with a grid built from the cells' own widths (§17.4.16 gridCol, §17.4.17
//! gridSpan), not with 72 pt filler columns. Fixtures are hand-written from the spec.

use std::io::{Read, Write};

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// A table: `tbl_pr` inside `w:tblPr`, `grid` as gridCol twips, and rows of cells, each `Some(tcW
/// twips)` or `None` (no `w:tcPr`). Cell text is "r<row>c<cell>".
fn table(tbl_pr: &str, grid: &[u32], rows: &[&[Option<u32>]]) -> String {
    let grid: String = grid.iter().map(|w| format!(r#"<w:gridCol w:w="{w}"/>"#)).collect();
    let mut s = format!(r#"<w:tbl><w:tblPr>{tbl_pr}</w:tblPr><w:tblGrid>{grid}</w:tblGrid>"#);
    for (r, row) in rows.iter().enumerate() {
        s.push_str("<w:tr>");
        for (c, w) in row.iter().enumerate() {
            let pr = w.map(|w| format!(r#"<w:tcPr><w:tcW w:w="{w}" w:type="dxa"/></w:tcPr>"#)).unwrap_or_default();
            s.push_str(&format!(r#"<w:tc>{pr}<w:p><w:r><w:t>r{r}c{c}</w:t></w:r></w:p></w:tc>"#));
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl><w:p/>");
    s
}

fn save(body: &str) -> String {
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    document_xml(&wordcraft_docx::write(&d).unwrap())
}

/// The value of attribute `attr` in the tag starting at `tag`.
fn attr(tag: &str, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let end = tag.find('>').unwrap_or(tag.len());
    let i = tag[..end].find(&key)? + key.len();
    Some(tag[i..].split('"').next()?.to_string())
}

/// Saved gridCol widths, twips.
fn saved_grid(xml: &str) -> Vec<u32> {
    let start = xml.find("<w:tblGrid").expect("tblGrid written");
    let end = xml[start..].find("</w:tblGrid>").unwrap() + start;
    xml[start..end].match_indices("<w:gridCol").map(|(i, _)| attr(&xml[start + i..], "w:w").unwrap().parse().unwrap()).collect()
}

/// Per saved row, each cell's gridSpan (1 when none is written).
fn saved_spans(xml: &str) -> Vec<Vec<u32>> {
    xml.split("<w:tr")
        .skip(1)
        .map(|row| {
            let row = row.split("</w:tr>").next().unwrap();
            row.split("<w:tc>")
                .skip(1)
                .map(|tc| tc.find("<w:gridSpan").map(|i| attr(&tc[i..], "w:val").unwrap().parse().unwrap()).unwrap_or(1))
                .collect()
        })
        .collect()
}

fn assert_texts(xml: &str, rows: &[&[Option<u32>]]) {
    let mut at = 0;
    for (r, row) in rows.iter().enumerate() {
        for c in 0..row.len() {
            let t = format!(">r{r}c{c}<");
            let i = xml[at..].find(&t).unwrap_or_else(|| panic!("cell text {t} missing or out of order: {xml}"));
            at += i + t.len();
        }
    }
}

#[test]
fn rows_with_different_edges_are_saved_on_one_grid_of_all_their_edges() {
    let rows: &[&[Option<u32>]] = &[&[Some(4500), Some(4500)], &[Some(3000), Some(3000), Some(3000)]];
    let xml = save(&table("", &[9000], rows));
    assert_eq!(saved_grid(&xml), vec![3000, 1500, 1500, 3000], "{xml}");
    assert_eq!(saved_spans(&xml), vec![vec![2, 2], vec![1, 2, 1]], "{xml}");
    assert_texts(&xml, rows);
}

#[test]
fn a_narrower_row_keeps_its_cells_on_the_union_grid() {
    let rows: &[&[Option<u32>]] = &[&[Some(2000), Some(3000)], &[Some(2000), Some(3000), Some(4000)]];
    let xml = save(&table("", &[2000, 3000], rows));
    assert_eq!(saved_grid(&xml), vec![2000, 3000, 4000], "{xml}");
    assert_eq!(saved_spans(&xml), vec![vec![1, 1], vec![1, 1, 1]], "{xml}");
    assert_texts(&xml, rows);
}

#[test]
fn missing_columns_without_cell_widths_share_the_rest_of_the_table_width() {
    let rows: &[&[Option<u32>]] = &[&[Some(2000), None, None]];
    let xml = save(&table(r#"<w:tblW w:w="8000" w:type="dxa"/>"#, &[2000], rows));
    assert_eq!(saved_grid(&xml), vec![2000, 3000, 3000], "{xml}");
    assert_eq!(saved_spans(&xml), vec![vec![1, 1, 1]], "{xml}");
    assert_texts(&xml, rows);
}

#[test]
fn missing_columns_with_no_width_at_all_take_the_mean_of_the_known_columns() {
    let rows: &[&[Option<u32>]] = &[&[Some(2000), None, None]];
    let xml = save(&table("", &[2000], rows));
    assert_eq!(saved_grid(&xml), vec![2000, 2000, 2000], "{xml}");
    assert!(!xml.contains("w:w=\"1440\""), "no 72 pt filler column: {xml}");
    assert_texts(&xml, rows);
}

#[test]
fn a_complete_grid_is_saved_unchanged() {
    let rows: &[&[Option<u32>]] = &[&[Some(3000), Some(3000)]];
    let xml = save(&table("", &[3000, 3000], rows));
    assert_eq!(saved_grid(&xml), vec![3000, 3000], "{xml}");
    assert_eq!(saved_spans(&xml), vec![vec![1, 1]], "{xml}");
    assert_texts(&xml, rows);
}

#[test]
fn a_lone_two_column_cell_is_saved_and_read_back_as_two_columns() {
    let body = r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>r0c0</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let bytes = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"<w:gridSpan w:val="2"/>"#), "{xml}");
    assert_eq!(saved_grid(&xml).len(), 2, "{xml}");
    let back = wordcraft_docx::read(&bytes).unwrap();
    let t = back.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).expect("table read back");
    assert_eq!(t.cols(), 2);
    assert_eq!(t.rows[0].cells[0].props.span, 2);
}

#[test]
fn saved_grid_columns_add_up_to_the_table_width() {
    let rows: &[&[Option<u32>]] = &[&[None, Some(5000), None]];
    let xml = save(&table(r#"<w:tblW w:w="10000" w:type="dxa"/>"#, &[2000], rows));
    let tbl_w: u32 = attr(&xml[xml.find("<w:tblW").unwrap()..], "w:w").unwrap().parse().unwrap();
    assert_eq!(tbl_w, 10000, "{xml}");
    let grid = saved_grid(&xml);
    assert_eq!(grid.len(), 3, "{xml}");
    let sum: u32 = grid.iter().sum();
    assert!(sum.abs_diff(tbl_w) <= grid.len() as u32, "{grid:?} vs {tbl_w}: {xml}");
    assert_texts(&xml, rows);
}
