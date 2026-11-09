//! How text flows across pages and tables sit on the page: a page break after a paragraph
//! (`fo:break-after`), lines kept together (`fo:keep-together`), widow and orphan control
//! (`fo:widows`, `fo:orphans`), hyphenation (`fo:hyphenate`), a table's alignment and width
//! (`table:align`, `style:rel-width`) and its rows' heights (`style:row-height`,
//! `style:min-row-height`), read into WordCraft's document and written back.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Align;
use wordcraft_doc::props::HeightRule;

fn doc(styles: &str, body: &str) -> wordcraft_doc::Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

fn para_style(name: &str, para: &str, text: &str) -> String {
    format!(
        r#"<style:style style:name="{name}" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties {para}/><style:text-properties {text}/></style:style>"#
    )
}

fn paras(d: &wordcraft_doc::Document) -> Vec<&wordcraft_doc::props::ParaProps> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| &p.props).collect()
}

#[test]
fn a_page_break_after_starts_the_next_paragraph_on_a_page() {
    let d = doc(
        &para_style("P1", r#"fo:break-after="page""#, ""),
        r#"<text:p text:style-name="P1">One.</text:p><text:p>Two.</text:p><text:p>Three.</text:p>"#,
    );
    let breaks: Vec<bool> = paras(&d).iter().map(|p| p.page_break_before == Some(true)).collect();
    assert_eq!(breaks, [false, true, false]);
    // Written back as a page break before the second paragraph.
    let c = content_of(&export(&d).unwrap());
    assert_eq!(c.matches(r#"fo:break-before="page""#).count(), 1, "{c}");
}

#[test]
fn kept_lines_and_widow_control_are_read_and_written() {
    let styles = format!(
        "{}{}",
        para_style("P1", r#"fo:keep-together="always" fo:widows="3" fo:orphans="3""#, ""),
        para_style("P2", r#"fo:widows="0" fo:orphans="0""#, "")
    );
    let d = doc(&styles, r#"<text:p text:style-name="P1">Kept.</text:p><text:p text:style-name="P2">Loose.</text:p>"#);
    let p = paras(&d);
    assert_eq!((p[0].keep_lines, p[0].widow_control), (Some(true), None));
    assert_eq!((p[1].keep_lines, p[1].widow_control), (None, Some(false)));
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains(r#"fo:keep-together="always""#) && c.contains(r#"fo:widows="0" fo:orphans="0""#), "{c}");
}

#[test]
fn a_hyphenated_paragraph_makes_the_document_hyphenate() {
    // Hyphenation is a paragraph's in ODF, the document's in WordCraft: the document hyphenates
    // and the other paragraphs opt out.
    let d = doc(&para_style("P1", "", r#"fo:hyphenate="true""#), r#"<text:p text:style-name="P1">Hyphenated.</text:p><text:p>Not.</text:p>"#);
    assert!(d.settings.auto_hyphenation);
    let p = paras(&d);
    assert_eq!((p[0].suppress_hyphens, p[1].suppress_hyphens), (None, Some(true)));
    let c = content_of(&export(&d).unwrap());
    assert_eq!(c.matches(r#"fo:hyphenate="true""#).count(), 1, "{c}");
    // No hyphenated paragraph: nothing changes.
    let plain = doc("", "<text:p>Plain.</text:p>");
    assert!(!plain.settings.auto_hyphenation && paras(&plain)[0].suppress_hyphens.is_none());
}

#[test]
fn a_tables_alignment_width_and_row_heights_are_kept() {
    let styles = r#"<style:style style:name="T1" style:family="table"><style:table-properties style:width="3in" style:rel-width="50%" table:align="center"/></style:style><style:style style:name="R1" style:family="table-row"><style:table-row-properties style:row-height="0.5in"/></style:style><style:style style:name="R2" style:family="table-row"><style:table-row-properties style:min-row-height="0.25in"/></style:style>"#;
    let body = r#"<table:table table:style-name="T1"><table:table-column table:number-columns-repeated="2"/><table:table-row table:style-name="R1"><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row><table:table-row table:style-name="R2"><table:table-cell><text:p>c</text:p></table:table-cell><table:table-cell><text:p>d</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell><text:p>e</text:p></table:table-cell><table:table-cell><text:p>f</text:p></table:table-cell></table:table-row></table:table>"#;
    let d = doc(styles, body);
    let t = d.body.iter().find_map(|b| b.as_table()).expect("a table");
    assert_eq!((t.props.align, t.props.width_pct), (Some(Align::Center), Some(50.0)));
    let heights: Vec<(Option<f32>, HeightRule)> = t.rows.iter().map(|r| (r.props.height, r.props.height_rule)).collect();
    assert_eq!(heights, [(Some(36.0), HeightRule::Exact), (Some(18.0), HeightRule::AtLeast), (None, HeightRule::Auto)]);
    let c = content_of(&crate::odt::tests::export_model(&d).unwrap());
    for want in [r#"style:rel-width="50%" table:align="center""#, r#"style:row-height="36pt""#, r#"style:min-row-height="18pt""#] {
        assert!(c.contains(want), "{want} in {c}");
    }
    // A table filling the margins stays so: the text width.
    let plain = doc("", r#"<table:table><table:table-row><table:table-cell><text:p>x</text:p></table:table-cell></table:table-row></table:table>"#);
    let pt = plain.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!((pt.props.align, pt.props.width_pct), (None, Some(100.0)));
    assert!(content_of(&crate::odt::tests::export_model(&plain).unwrap()).contains(r#"table:align="margins""#));
}

#[test]
fn columns_are_written_with_their_share_of_the_table() {
    // A table filling the margins of a page narrower than it was made for keeps its columns'
    // proportions only when they are given as shares too (LibreOffice scales by them).
    let styles = r#"<style:style style:name="A" style:family="table-column"><style:table-column-properties style:column-width="1in"/></style:style><style:style style:name="B" style:family="table-column"><style:table-column-properties style:column-width="3in"/></style:style>"#;
    let d = doc(
        styles,
        r#"<table:table><table:table-column table:style-name="A"/><table:table-column table:style-name="B"/><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row></table:table>"#,
    );
    let c = content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"style:column-width="72pt" style:rel-column-width="7200*""#) && c.contains(r#"style:rel-column-width="21600*""#), "{c}");
    // An aligned table keeps its widths: LibreOffice lays one out oddly with shares.
    let left = r#"<style:style style:name="T" style:family="table"><style:table-properties style:width="4in" table:align="left"/></style:style>"#;
    let d = doc(
        &format!("{styles}{left}"),
        r#"<table:table table:style-name="T"><table:table-column table:style-name="A"/><table:table-column table:style-name="B"/><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row></table:table>"#,
    );
    let c = content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"table:align="left""#) && !c.contains("rel-column-width"), "{c}");
}

#[test]
fn a_tables_columns_keep_their_widths() {
    // LibreOffice never fits a table's columns to their content (Word's fixed layout); a table
    // filling the margins is fitted to them, its columns in their proportions.
    let styles = r#"<style:style style:name="T" style:family="table"><style:table-properties style:width="8in" table:align="margins"/></style:style><style:style style:name="A" style:family="table-column"><style:table-column-properties style:column-width="4in"/></style:style>"#;
    let d = doc(
        styles,
        r#"<table:table table:style-name="T"><table:table-column table:style-name="A" table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>a much longer text in the second column</text:p></table:table-cell></table:table-row></table:table>"#,
    );
    let t = d.body.iter().find_map(|b| b.as_table()).expect("a table");
    assert!(t.props.fixed);
    assert_eq!(t.grid, [288.0, 288.0]);
    assert_eq!(t.props.width_pct, Some(100.0));
}

#[test]
fn a_tables_own_space_above_and_below_is_kept() {
    // LibreOffice's report templates: 0.3 cm below each table (`fo:margin-bottom` on the table).
    let styles = r#"<style:style style:name="T" style:family="table"><style:table-properties style:width="6in" fo:margin-top="0.1in" fo:margin-bottom="0.3cm" table:align="margins"/></style:style>"#;
    let d = doc(
        styles,
        r#"<table:table table:style-name="T"><table:table-row><table:table-cell><text:p>Cell</text:p></table:table-cell></table:table-row></table:table><text:p>After.</text:p>"#,
    );
    let space = |d: &wordcraft_doc::Document| {
        let t = d.body.iter().find_map(|b| b.as_table()).expect("a table");
        (t.props.space_before.map(|v| (v * 100.0).round() / 100.0), t.props.space_after.map(|v| (v * 100.0).round() / 100.0))
    };
    assert_eq!(space(&d), (Some(7.2), Some(8.5)));
    let out = content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(out.contains(r#"fo:margin-top="7.2pt" fo:margin-bottom="8.5"#), "{out}");
    assert_eq!(space(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap()), (Some(7.2), Some(8.5)));
}

#[test]
fn a_table_starting_a_page_starts_it() {
    // LibreOffice's agenda: the minutes table starts a new page (`fo:break-before` on the table).
    let styles = r#"<style:style style:name="T" style:family="table"><style:table-properties style:width="6in" fo:break-before="page" table:align="margins"/></style:style>"#;
    let d = doc(
        styles,
        r#"<text:p>Before.</text:p><table:table table:style-name="T"><table:table-row><table:table-cell><text:p>First cell</text:p></table:table-cell></table:table-row></table:table><text:p>After.</text:p>"#,
    );
    let t = d.body.iter().find_map(|b| b.as_table()).expect("a table");
    let first = t.rows[0].cells[0].blocks[0].as_para().unwrap();
    assert_eq!(first.props.page_break_before, Some(true));
    // Nothing else starts a page.
    assert!(paras(&d).iter().all(|p| p.page_break_before != Some(true)));
    let c = content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert_eq!(c.matches(r#"fo:break-before="page""#).count(), 2, "{c}");
}

#[test]
fn columns_stating_only_their_shares_keep_their_proportions() {
    // LibreOffice's CV template: a table filling the margins whose columns give only
    // `style:rel-column-width`. They were read as equal.
    let column = |name: &str, share: &str| {
        format!(
            r#"<style:style style:name="{name}" style:family="table-column"><style:table-column-properties style:rel-column-width="{share}"/></style:style>"#
        )
    };
    let table = r#"<table:table><table:table-column table:style-name="A"/><table:table-column table:style-name="B"/><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row></table:table>"#;
    let grid = |d: &wordcraft_doc::Document| d.body.iter().find_map(|b| b.as_table()).map(|t| t.grid.clone()).unwrap_or_default();
    let d = doc(&format!("{}{}", column("A", "1000*"), column("B", "3000*")), table);
    let g = grid(&d);
    assert_eq!(g.len(), 2, "{g:?}");
    assert!((g[1] / g[0] - 3.0).abs() < 0.01, "{g:?}");
    // Rewritten from the model, the proportions stay.
    let back = import(&crate::odt::tests::export_model(&d).unwrap()).unwrap();
    let g = grid(&back);
    assert!((g[1] / g[0] - 3.0).abs() < 0.01, "{g:?}");
    // A column without a share leaves the equal split, and junk shares are none.
    for junk in ["0*", "-1*", "NaN*", "1e39*", "abc"] {
        let d = doc(&format!("{}{}", column("A", "1000*"), column("B", junk)), table);
        let g = grid(&d);
        assert_eq!(g.len(), 2, "{junk}: {g:?}");
        assert!((g[0] - g[1]).abs() < 0.01, "{junk}: {g:?}");
    }
    // A table filling the margins with a width stated beside each share keeps the widths: files
    // give them in proportion, and the table is fitted to the margins.
    let stated = r#"<style:style style:name="A" style:family="table-column"><style:table-column-properties style:column-width="2in" style:rel-column-width="2*"/></style:style><style:style style:name="B" style:family="table-column"><style:table-column-properties style:column-width="1in" style:rel-column-width="1*"/></style:style>"#;
    let g = grid(&doc(stated, table));
    assert!((g[0] / g[1] - 2.0).abs() < 0.01, "{g:?}");
}

/// A share-only table wider than the text keeps its stated width, as in LibreOffice; a column with
/// only a width takes it first, and the shares divide the rest; a column with no share of its own
/// takes the default style's.
#[test]
fn columns_by_shares_divide_the_right_width() {
    let column = |name: &str, props: &str| {
        format!(r#"<style:style style:name="{name}" style:family="table-column"><style:table-column-properties {props}/></style:style>"#)
    };
    let table_style = |props: &str| format!(r#"<style:style style:name="T" style:family="table"><style:table-properties {props}/></style:style>"#);
    let table = |names: &[&str]| {
        let columns: String = names
            .iter()
            .map(|n| if n.is_empty() { "<table:table-column/>".to_string() } else { format!(r#"<table:table-column table:style-name="{n}"/>"#) })
            .collect();
        let cells: String = names.iter().map(|_| "<table:table-cell><text:p>x</text:p></table:table-cell>").collect();
        format!(r#"<table:table table:style-name="T">{columns}<table:table-row>{cells}</table:table-row></table:table>"#)
    };
    let grid = |d: &wordcraft_doc::Document| d.body.iter().find_map(|b| b.as_table()).map(|t| t.grid.clone()).unwrap_or_default();
    let near = |got: &[f32], want: &[f32]| {
        assert!(got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.5), "{got:?}, not {want:?}")
    };
    // 576 pt, aligned, in a narrower text area.
    let d = doc(
        &format!(
            "{}{}{}",
            table_style(r#"style:width="8in" table:align="left""#),
            column("A", r#"style:rel-column-width="1000*""#),
            column("B", r#"style:rel-column-width="3000*""#)
        ),
        &table(&["A", "B"]),
    );
    near(&grid(&d), &[144.0, 432.0]);
    near(&grid(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap()), &[144.0, 432.0]);
    // 72 pt stated, the other 300 by shares.
    let d = doc(
        &format!(
            "{}{}{}{}",
            table_style(r#"style:width="372pt" table:align="left""#),
            column("A", r#"style:column-width="72pt""#),
            column("B", r#"style:rel-column-width="1000*""#),
            column("C", r#"style:rel-column-width="3000*""#)
        ),
        &table(&["A", "B", "C"]),
    );
    near(&grid(&d), &[72.0, 75.0, 225.0]);
    // A share on every column goes before a stated width, as in LibreOffice.
    let d = doc(
        &format!(
            "{}{}{}",
            table_style(r#"style:width="400pt" table:align="left""#),
            column("A", r#"style:column-width="72pt" style:rel-column-width="1000*""#),
            column("B", r#"style:rel-column-width="3000*""#)
        ),
        &table(&["A", "B"]),
    );
    near(&grid(&d), &[100.0, 300.0]);
    // An aligned table whose every column states both: the shares (measured in LibreOffice 26.8).
    let d = doc(
        &format!(
            "{}{}{}",
            table_style(r#"style:width="400pt" table:align="left""#),
            column("A", r#"style:column-width="300pt" style:rel-column-width="1*""#),
            column("B", r#"style:column-width="100pt" style:rel-column-width="1*""#)
        ),
        &table(&["A", "B"]),
    );
    near(&grid(&d), &[200.0, 200.0]);
    // A share too small for a column is the narrowest one, not the grid lost to the equal split.
    let d = doc(
        &format!(
            "{}{}{}",
            table_style(r#"style:width="400pt" table:align="left""#),
            column("A", r#"style:rel-column-width="1*""#),
            column("B", r#"style:rel-column-width="65535*""#)
        ),
        &table(&["A", "B"]),
    );
    near(&grid(&d), &[6.0, 400.0]);
    // The default style's share, for a column with none and one whose style says none.
    let styles = r#"<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"><office:styles><style:default-style style:family="table-column"><style:table-column-properties style:rel-column-width="1000*"/></style:default-style></office:styles></office:document-styles>"#;
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{}{}{}</office:automatic-styles><office:body><office:text>{}</office:text></office:body></office:document-content>"#,
        table_style(r#"style:width="400pt" table:align="left""#),
        column("B", r#"style:rel-column-width="3000*""#),
        column("E", r#"style:use-optimal-column-width="false""#),
        table(&["", "B"])
    );
    near(&grid(&import(&package(&content, Some(styles))).unwrap()), &[100.0, 300.0]);
}

/// A table inside a cell doesn't know its width from the page: with only shares and no width of
/// its own to divide, or with some columns stating widths, its columns split it equally, as before.
#[test]
fn a_table_in_a_cell_shares_out_no_page_width() {
    let styles = r#"<style:style style:name="W" style:family="table-column"><style:table-column-properties style:column-width="100pt"/></style:style><style:style style:name="S" style:family="table-column"><style:table-column-properties style:rel-column-width="1*"/></style:style><style:style style:name="L" style:family="table"><style:table-properties table:align="left"/></style:style>"#;
    for (style, columns) in [
        (r#" table:style-name="L""#, r#"<table:table-column table:style-name="S"/><table:table-column table:style-name="S"/>"#),
        ("", r#"<table:table-column table:style-name="W"/><table:table-column table:style-name="S"/>"#),
    ] {
        let inner = format!(
            r#"<table:table{style}>{columns}<table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row></table:table>"#
        );
        let body = format!(
            r#"<table:table><table:table-column table:style-name="W"/><table:table-column table:style-name="W"/><table:table-row><table:table-cell>{inner}</table:table-cell><table:table-cell><text:p>c</text:p></table:table-cell></table:table-row></table:table>"#
        );
        let d = doc(styles, &body);
        let outer = d.body.iter().find_map(|b| b.as_table()).unwrap();
        let inner = outer.rows[0].cells[0].blocks.iter().find_map(|b| b.as_table()).unwrap();
        // The equal split main gives, which layout fits to the cell (not two page-wide columns).
        let equal = inner.grid.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01);
        assert!(equal && inner.grid.first().is_some_and(|w| *w < 300.0), "{style}: {:?}", inner.grid);
    }
}

#[test]
fn a_tables_and_rows_background_show_behind_their_cells() {
    // Table Properties › Background: it was dropped, on screen and in LibreOffice after a save.
    let styles = r##"<style:style style:name="T" style:family="table"><style:table-properties fo:background-color="#ddffdd"/></style:style><style:style style:name="R" style:family="table-row"><style:table-row-properties fo:background-color="#ffdddd"/></style:style><style:style style:name="C" style:family="table-cell"><style:table-cell-properties fo:background-color="#ddddff"/></style:style>"##;
    let table = r#"<table:table table:style-name="T"><table:table-row><table:table-cell><text:p>table</text:p></table:table-cell><table:table-cell table:style-name="C"><text:p>cell</text:p></table:table-cell></table:table-row><table:table-row table:style-name="R"><table:table-cell><text:p>row</text:p></table:table-cell><table:table-cell table:style-name="C"><text:p>cell</text:p></table:table-cell></table:table-row></table:table>"#;
    let colours = |d: &wordcraft_doc::Document| {
        d.body
            .iter()
            .find_map(|b| b.as_table())
            .map(|t| t.rows.iter().flat_map(|r| r.cells.iter().map(|c| c.props.shading.map(|s| s.hex().to_ascii_lowercase()))).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let want = |t: &str, r: &str, c: &str| vec![Some(t.to_string()), Some(c.to_string()), Some(r.to_string()), Some(c.to_string())];
    let d = doc(styles, table);
    assert_eq!(colours(&d), want("ddffdd", "ffdddd", "ddddff"));
    assert_eq!(colours(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap()), want("ddffdd", "ffdddd", "ddddff"));
}

/// As LibreOffice shows them: a cell's own style, else its column's default cell style (a row's
/// isn't applied), else the row's background, else the table's. A row or table clearing its
/// background (`transparent`) doesn't take its parent's; the table's shows behind that row.
#[test]
fn a_cells_background_comes_from_its_column_then_row_then_table() {
    let cell = |n: &str, c: &str| {
        format!(
            r##"<style:style style:name="{n}" style:family="table-cell"><style:table-cell-properties fo:background-color="#{c}"/></style:style>"##
        )
    };
    let styles = format!(
        r##"{}{}<style:style style:name="T" style:family="table"><style:table-properties fo:background-color="#ffff00"/></style:style><style:style style:name="R" style:family="table-row"><style:table-row-properties fo:background-color="#0000ff"/></style:style><style:style style:name="Clear" style:family="table-row" style:parent-style-name="R"><style:table-row-properties fo:background-color="transparent"/></style:style>"##,
        cell("InRow", "ff0000"),
        cell("InColumn", "00ff00"),
    );
    let c = "<table:table-cell><text:p>x</text:p></table:table-cell>";
    let table = format!(
        r#"<table:table table:style-name="T"><table:table-column table:default-cell-style-name="InColumn"/><table:table-column/><table:table-row table:style-name="R" table:default-cell-style-name="InRow">{c}{c}</table:table-row><table:table-row>{c}{c}</table:table-row><table:table-row table:style-name="Clear">{c}{c}</table:table-row></table:table>"#
    );
    let colours = |d: &wordcraft_doc::Document| {
        d.body
            .iter()
            .find_map(|b| b.as_table())
            .map(|t| t.rows.iter().flat_map(|r| r.cells.iter().map(|c| c.props.shading.map(|s| s.hex().to_ascii_lowercase()))).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let s = |v: &str| Some(v.to_string());
    let want = vec![s("00ff00"), s("0000ff"), s("00ff00"), s("ffff00"), s("00ff00"), s("ffff00")];
    let d = doc(&styles, &table);
    assert_eq!(colours(&d), want);
    assert_eq!(colours(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap()), want);
}

/// A column default after a spanning cell belongs to the column the cell is in.
#[test]
fn a_column_default_counts_spanned_columns() {
    let styles =
        r##"<style:style style:name="C" style:family="table-cell"><style:table-cell-properties fo:background-color="#00ff00"/></style:style>"##;
    let table = r#"<table:table><table:table-column/><table:table-column/><table:table-column table:default-cell-style-name="C"/><table:table-row><table:table-cell table:number-columns-spanned="2"><text:p>a</text:p></table:table-cell><table:covered-table-cell/><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row></table:table>"#;
    let d = doc(styles, table);
    let t = d.body.iter().find_map(|b| b.as_table()).unwrap();
    let last = t.rows[0].cells.iter().rev().find(|c| !c.blocks.is_empty()).unwrap();
    assert_eq!(last.props.shading.map(|s| s.hex().to_ascii_lowercase()).as_deref(), Some("00ff00"));
}

/// A column's default cell style that names nothing there is no background, however long its name.
#[test]
fn a_column_default_naming_no_style_is_no_background() {
    let long = "x".repeat(100_000);
    let table = format!(
        r#"<table:table><table:table-column table:number-columns-repeated="63" table:default-cell-style-name="{long}"/><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell></table:table-row></table:table>"#
    );
    let d = doc("", &table);
    let t = d.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows[0].cells[0].props.shading, None);
}

/// A cell style saying `transparent` over a coloured parent has no background of its own: what's
/// behind it shows, in LibreOffice too, from a column's default cell style or the cell's own.
#[test]
fn a_transparent_cell_style_stops_its_parents_colour() {
    let styles = r##"<style:style style:name="PC" style:family="table-cell"><style:table-cell-properties fo:background-color="#ff0000"/></style:style><style:style style:name="KC" style:family="table-cell" style:parent-style-name="PC"><style:table-cell-properties fo:background-color="transparent"/></style:style><style:style style:name="R" style:family="table-row"><style:table-row-properties fo:background-color="#0000ff"/></style:style>"##;
    for (column, cell) in [(r#" table:default-cell-style-name="KC""#, ""), ("", r#" table:style-name="KC""#)] {
        let table = format!(
            r#"<table:table><table:table-column{column}/><table:table-row table:style-name="R"><table:table-cell{cell}><text:p>a</text:p></table:table-cell></table:table-row></table:table>"#
        );
        let d = doc(styles, &table);
        let t = d.body.iter().find_map(|b| b.as_table()).unwrap();
        assert_eq!(t.rows[0].cells[0].props.shading, crate::model::parse_color("#0000ff"), "{column}{cell}");
    }
}

#[test]
fn a_row_that_keeps_together_says_so() {
    // Table › Text Flow: a row not to break across pages. It came back from a save free to break.
    let styles = r#"<style:style style:name="R" style:family="table-row"><style:table-row-properties fo:keep-together="always"/></style:style><style:style style:name="H" style:family="table-row"><style:table-row-properties style:min-row-height="1in" fo:keep-together="always"/></style:style>"#;
    let row = |s: &str, t: &str| format!(r#"<table:table-row{s}><table:table-cell><text:p>{t}</text:p></table:table-cell></table:table-row>"#);
    let table = format!(
        "<table:table>{}{}{}</table:table>",
        row(r#" table:style-name="R""#, "kept"),
        row("", "free"),
        row(r#" table:style-name="H""#, "kept and tall")
    );
    let shape = |d: &wordcraft_doc::Document| {
        d.body
            .iter()
            .find_map(|b| b.as_table())
            .map(|t| t.rows.iter().map(|r| (r.props.cant_split, r.props.height)).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let d = doc(styles, &table);
    assert_eq!(shape(&d), [(true, None), (false, None), (true, Some(72.0))]);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    assert!(content_of(&saved).contains(r#"fo:keep-together="always""#));
    assert_eq!(shape(&import(&saved).unwrap()), [(true, None), (false, None), (true, Some(72.0))]);
}

/// A row whose style says nothing of it, or a row with no style, follows the default style; one
/// that says otherwise doesn't.
#[test]
fn a_rows_keep_together_defaults_to_the_default_style() {
    let styles = r#"<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"><office:styles><style:default-style style:family="table-row"><style:table-row-properties fo:keep-together="always"/></style:default-style></office:styles></office:document-styles>"#;
    let row = |s: &str| format!(r#"<table:table-row{s}><table:table-cell><text:p>x</text:p></table:table-cell></table:table-row>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="H" style:family="table-row"><style:table-row-properties style:min-row-height="1in"/></style:style><style:style style:name="F" style:family="table-row"><style:table-row-properties fo:keep-together="auto"/></style:style></office:automatic-styles><office:body><office:text><table:table>{}{}{}</table:table></office:text></office:body></office:document-content>"#,
        row(""),
        row(r#" table:style-name="H""#),
        row(r#" table:style-name="F""#)
    );
    let keeps = |d: &wordcraft_doc::Document| {
        d.body.iter().find_map(|b| b.as_table()).map(|t| t.rows.iter().map(|r| r.props.cant_split).collect::<Vec<_>>()).unwrap_or_default()
    };
    let d = import(&package(&content, Some(styles))).unwrap();
    assert_eq!(keeps(&d), [true, true, false]);
    assert_eq!(keeps(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap()), [true, true, false]);
}

/// A row free to break under a default row style that keeps rows together says so when written from
/// the model, or a save takes that away: the file's default style stays.
#[test]
fn a_row_free_to_break_stays_so_over_the_default_style() {
    let styles = r#"<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"><office:styles><style:default-style style:family="table-row"><style:table-row-properties fo:keep-together="always"/></style:default-style></office:styles></office:document-styles>"#;
    let row = |s: &str| format!(r#"<table:table-row{s}><table:table-cell><text:p>x</text:p></table:table-cell></table:table-row>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="F" style:family="table-row"><style:table-row-properties fo:keep-together="auto"/></style:style></office:automatic-styles><office:body><office:text><table:table><table:table-column/>{}{}</table:table></office:text></office:body></office:document-content>"#,
        row(r#" table:style-name="F""#),
        row("")
    );
    let mut d = import(&package(&content, Some(styles))).unwrap();
    let keeps = |d: &wordcraft_doc::Document| {
        d.body.iter().find_map(|b| b.as_table()).map(|t| t.rows.iter().map(|r| r.props.cant_split).collect::<Vec<_>>()).unwrap_or_default()
    };
    assert_eq!(keeps(&d), [false, true]);
    if let Some(wordcraft_doc::Block::Table(t)) = d.body.get_mut(0).map(std::sync::Arc::make_mut)
        && let Some(wordcraft_doc::Block::Para(p)) = t.rows[0].cells[0].blocks.get_mut(0).map(std::sync::Arc::make_mut)
    {
        p.text.push('!');
        if let Some(r) = p.runs.last_mut() {
            r.len += 1;
        }
    }
    assert_eq!(keeps(&import(&export(&d).unwrap()).unwrap()), [false, true], "after an edit and a save");
}
