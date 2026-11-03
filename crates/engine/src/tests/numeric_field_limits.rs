use super::{Session, column_table, column_texts, formula_result, json, run};

fn assert_formula(s: &Session, source: &str, instr: &str, result: &str) {
    assert_eq!(column_texts(s)[0], source, "formula input: {instr}");
    assert_eq!(formula_result(s, 1, 0).as_deref(), Some(result), "{instr}");
    let table = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    let instructions: Vec<_> = table.rows[1].cells[0]
        .blocks
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| &p.objects)
        .filter_map(|o| match o {
            wordcraft_doc::InlineObject::Field { instr, .. } => Some(instr.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(instructions, [instr], "saving and refresh retain the formula code");
}

fn assert_saved_formula(s: &Session, source: &str, instr: &str, result: &str) {
    for name in ["limits.json", "limits.docx"] {
        let mut reopened = Session::new(s.doc.clone());
        for _ in 0..2 {
            let bytes = crate::io::save_bytes(name, &reopened.doc).unwrap();
            reopened = Session::new(crate::io::open_bytes(name, &bytes).unwrap());
            assert_formula(&reopened, source, instr, result);
        }
    }
}

/// The format limits apply to inserted and refreshed formulas, without changing their inputs.
/// Saving also preserves the cached result before an explicit refresh and after Undo/Redo.
#[test]
fn numeric_field_limits_refresh_history_and_save() {
    let error = "Error! Number cannot be represented in specified format.";
    let roman = format!("{}dcclxvii", "m".repeat(32));
    for (format, limit, over, supported, exceeded) in [
        ("CardText", "999999", "1000000", "nine hundred ninety-nine thousand nine hundred ninety-nine".to_string(), error.to_string()),
        ("OrdText", "999999", "1000000", "nine hundred ninety-nine thousand nine hundred ninety-ninth".to_string(), error.to_string()),
        ("roman", "32767", "32768", roman.clone(), error.to_string()),
        ("ROMAN", "32767", "32768", roman.to_uppercase(), error.to_string()),
        ("CardText \\* Upper", "999999", "1000000", "NINE HUNDRED NINETY-NINE THOUSAND NINE HUNDRED NINETY-NINE".to_string(), error.to_uppercase()),
        ("Arabic", "999999", "1000000", "999999".to_string(), "1000000".to_string()),
    ] {
        let instr = format!("=A1 \\* {format}");
        let mut s = column_table("en-US", &[limit, ""]);
        run(&mut s, "table.formula", json!({"formula": instr}));
        assert_formula(&s, limit, &instr, &supported);
        assert_saved_formula(&s, limit, &instr, &supported);

        run(&mut s, "select.text", json!({"text": limit}));
        run(&mut s, "text.insert", json!({"text": over}));
        assert_formula(&s, over, &instr, &supported);
        assert_saved_formula(&s, over, &instr, &supported);
        let depth = s.undo_depth();
        run(&mut s, "references.updateFields", json!({"all": true}));
        assert_eq!(s.undo_depth(), depth + 1, "one refresh edit: {instr}");
        assert_formula(&s, over, &instr, &exceeded);
        assert_saved_formula(&s, over, &instr, &exceeded);

        run(&mut s, "edit.undo", json!({}));
        assert_formula(&s, over, &instr, &supported);
        run(&mut s, "edit.undo", json!({}));
        assert_formula(&s, limit, &instr, &supported);
        run(&mut s, "edit.redo", json!({}));
        assert_formula(&s, over, &instr, &supported);
        run(&mut s, "edit.redo", json!({}));
        assert_formula(&s, over, &instr, &exceeded);
        assert_saved_formula(&s, over, &instr, &exceeded);

        let mut inserted_over = column_table("en-US", &[over, ""]);
        run(&mut inserted_over, "table.formula", json!({"formula": instr}));
        assert_formula(&inserted_over, over, &instr, &exceeded);
    }
}

/// Field-only range errors must not narrow the shared numbering formatter's existing output.
#[test]
fn numeric_field_limits_leave_numbering_and_decimal_controls_unchanged() {
    use wordcraft_doc::section::NumFormat;
    assert_eq!(NumFormat::UpperRoman.format(32_768), format!("{}DCCLXVIII", "M".repeat(32)));
    assert_eq!(NumFormat::Decimal.format(1_000_000), "1000000");
    assert_eq!(NumFormat::CardinalText.format(21), "Twenty-One");
    assert_eq!(NumFormat::OrdinalText.format(21), "Twenty-First");
    for (format, value, expected) in [("CardText", "0", "zero"), ("OrdText", "1", "first"), ("roman", "1", "i"), ("ROMAN", "1", "I")] {
        let mut s = column_table("en-US", &[value, ""]);
        let instr = format!("=A1 \\* {format}");
        run(&mut s, "table.formula", json!({"formula": instr}));
        assert_formula(&s, value, &instr, expected);
    }
}
