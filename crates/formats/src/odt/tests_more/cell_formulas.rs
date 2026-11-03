//! Table cell formulas (ODF 1.3 §19.646 `table:formula`, with the cell's value
//! `office:value`): LibreOffice's (`ooow:sum <A1:A3>`) open as Word formula fields
//! (`=SUM(A1:A3)`) showing the cell's value, so Update Field recalculates them, and formula
//! fields in cells are written back as cell formulas.

use crate::odt::cell_formula::{odf_of, word_of};
use crate::odt::import;
use crate::odt::tests::{NS_T, content_of, package};
use wordcraft_doc::InlineObject;

#[test]
fn libreoffice_formulas_say_what_word_formulas_do() {
    for (odf, word) in [
        ("ooow:sum <A1:A3>", "=SUM(A1:A3)"),
        ("ooow:<B1>*<B2>+<B3>", "=B1*B2+B3"),
        ("ooow:mean <A1:B3>", "=AVERAGE(A1:B3)"),
        ("ooow:sum <a1>|<B2>|3", "=SUM(A1,B2,3)"),
        ("ooow:<A1> add <A2> mul 2", "=A1+A2*2"),
        ("ooow:max (<A1>-1)|<B1>", "=MAX(A1-1,B1)"),
        ("=-(<C2>/4)", "=-(C2/4)"),
        // Brackets round the whole say nothing; others stay.
        ("ooow:((sum <A1:A2>))", "=SUM(A1:A2)"),
        ("ooow:(<A1>)+(<A2>)", "=(A1)+(A2)"),
    ] {
        assert_eq!(word_of(odf).as_deref(), Some(word), "{odf}");
    }
    // What Word can't say: another table's cells, logic written as words, junk.
    for odf in ["ooow:<Table2.A1>+1", "ooow:<A1> and <A2>", "ooow:sum", "ooow:<A1", "ooow:sum <A1>|", "ooow:$x", "", "ooow:)))"] {
        assert_eq!(word_of(odf), None, "{odf}");
    }
}

#[test]
fn word_formulas_are_written_as_libreoffice_says_them() {
    // In the last row (row 3 of 4, from 0) of a 4 × 3 table, column B.
    let at = ((3, 1), (4, 3));
    for (word, odf) in [
        ("=SUM(ABOVE)", "ooow:(sum <B1:B3>)"),
        ("=SUM(LEFT)", "ooow:(sum <A4:A4>)"),
        ("=sum(right) \\# \"0.00\"", "ooow:(sum <C4:C4>)"),
        ("=AVERAGE(A1:B3,4)", "ooow:(mean <A1:B3>|4)"),
        ("=B1*B2+B3", "ooow:<B1> * <B2> + <B3>"),
        ("=(A1-2)/4", "ooow:( <A1> - 2 ) / 4"),
    ] {
        assert_eq!(odf_of(word, at.0, at.1).as_deref(), Some(odf), "{word}");
    }
    // Nothing below the last row; logic Word has that LibreOffice writes otherwise; not a formula.
    for word in ["=SUM(BELOW)", "=IF(A1>2,1,0)", "=SUM(A1", "PAGE", "=", "=SUM(ABOVE,(A1))", "=Sheet!A1"] {
        assert_eq!(odf_of(word, at.0, at.1), None, "{word}");
    }
    // Above the first row and left of the first column there is nothing.
    assert_eq!(odf_of("=SUM(ABOVE)", (0, 0), (2, 2)), None);
    assert_eq!(odf_of("=SUM(LEFT)", (0, 0), (2, 2)), None);
}

const TABLE: &str = r#"<table:table table:name="T"><table:table-column table:number-columns-repeated="2"/>
<table:table-row><table:table-cell office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="2.5"><text:p>2.5</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="float" office:value="3"><text:p>3</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="4"><text:p>4</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell table:formula="ooow:sum &lt;A1:A2&gt;" office:value-type="float" office:value="4"><text:p>4</text:p></table:table-cell><table:table-cell table:formula="ooow:&lt;Table9.B1&gt;" office:value-type="float" office:value="2.5"><text:p>2.5</text:p></table:table-cell></table:table-row>
</table:table>"#;

fn open() -> wordcraft_doc::Document {
    let content =
        format!(r#"<office:document-content {NS_T}><office:body><office:text>{TABLE}</office:text></office:body></office:document-content>"#);
    import(&package(&content, None)).unwrap()
}

/// The fields in the table's cells: (row, column, code, result).
fn fields(d: &wordcraft_doc::Document) -> Vec<(usize, usize, String, String)> {
    let mut out = Vec::new();
    let Some(t) = d.body.iter().find_map(|b| b.as_table()) else { return out };
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            for p in cell.blocks.iter().filter_map(|b| b.as_para()) {
                for o in &p.objects {
                    if let InlineObject::Field { instr, result, .. } = o {
                        out.push((r, c, instr.clone(), result.clone()));
                    }
                }
            }
        }
    }
    out
}

#[test]
fn a_cell_formula_opens_as_a_formula_field_showing_its_value() {
    let d = open();
    assert_eq!(fields(&d), [(2, 0, "=SUM(A1:A2)".to_string(), "4".to_string())]);
    // One Word can't say keeps its value as text.
    let t = d.body.iter().find_map(|b| b.as_table()).unwrap();
    let last = t.rows[2].cells[1].blocks[0].as_para().unwrap();
    assert_eq!(last.text, "2.5");
}

#[test]
fn formula_fields_in_cells_are_written_as_cell_formulas() {
    let d = open();
    let c = content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"table:formula="ooow:(sum &lt;A1:A2&gt;)" office:value-type="float" office:value="4""#), "{c}");
    // Read back, the same field.
    let again = import(&crate::odt::tests::export_model(&d).unwrap()).unwrap();
    assert_eq!(fields(&again), fields(&d));
}

#[test]
fn cells_holding_numbers_are_written_as_numbers() {
    let c = content_of(&crate::odt::tests::export_model(&open()).unwrap());
    assert!(c.contains(r#"office:value-type="float" office:value="2.5"><text:p"#), "{c}");
    // Text, numbers with more than a number in them and empty cells are text.
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><table:table><table:table-row><table:table-cell><text:p>x</text:p></table:table-cell><table:table-cell><text:p>1,5</text:p></table:table-cell><table:table-cell><text:p>-</text:p></table:table-cell><table:table-cell><text:p>1.2.3</text:p></table:table-cell><table:table-cell><text:p/></table:table-cell><table:table-cell><text:p> -7 </text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"#
    );
    let c = content_of(&crate::odt::tests::export_model(&import(&package(&content, None)).unwrap()).unwrap());
    assert_eq!(c.matches(r#"office:value-type="float""#).count(), 1, "{c}");
    assert!(c.contains(r#"office:value="-7""#), "{c}");
}
