//! Original geometry controls for hanging-indent intrinsic table measurement (#4203).
use super::*;
use wordcraft_doc::{CharProps, Paragraph};

fn fixture(text: &str, left: f32, first: f32, width: f32, bold: bool) -> (Document, Table) {
    let mut table = Table::new(1, 2, 180.0);
    table.props.style = None;
    table.props.width = Some(180.0);
    table.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    table.grid = vec![width, 180.0 - width];
    for (cell, width) in table.rows[0].cells.iter_mut().zip(&table.grid) {
        cell.props.width = Some(*width);
    }
    let mut p = Paragraph::with_text(text, CharProps { font: Some(WRAP_TEST_FONT.into()), size: Some(10.0), bold: Some(bold), ..Default::default() });
    p.props = ParaProps { indent_left: Some(left), indent_first: Some(first), ..Default::default() };
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(p)];
    table.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text(
        "Witness text keeps its original column width and wrapping. ".repeat(3).as_str(),
        CharProps { font: Some(WRAP_TEST_FONT.into()), size: Some(10.0), ..Default::default() },
    ))];
    let mut doc = Document::new();
    doc.body = vec![Arc::new(Block::Table(table.clone()))];
    (doc, table)
}

fn cells(layout: &DocLayout) -> Vec<Rect> {
    layout.pages.iter().flat_map(|p| &p.items).filter_map(|it| if let Placed::Cell { rect, .. } = it { Some(*rect) } else { None }).collect()
}

#[test]
fn hanging_autofit_single_line_keeps_the_grid_and_cached_geometry() {
    for (width, left, first) in [(30.0, 30.0, -30.0), (30.0, -27.95, 27.95), (30.0, -4.3, 0.0)] {
        for text in ["R:", "R: "] {
            let (plain, _) = fixture(text, 0.0, 0.0, width, false);
            let (doc, table) = fixture(text, left, first, width, false);
            let expected = cells(&lay(&plain));
            let mut cache = LayoutCache::new();
            let first_layout = layout(&doc, &mut cache, &LayoutOptions::default());
            assert_eq!(cells(&first_layout), expected, "{text:?}, left {left}, first {first}: column geometry");
            assert!(!cache.measures.is_empty(), "ordinary layout must measure the cell content");
            let second_layout = layout(&doc, &mut cache, &LayoutOptions::default());
            assert!(cache.hits > 0, "the second pass must exercise cached measurements");
            assert_eq!(cells(&second_layout), expected);
            assert_eq!(first_layout.pages.len(), second_layout.pages.len());
            let plain_widths = measure_table_columns(&plain, &table_for(&plain));
            let widths = measure_table_columns(&doc, &table);
            assert!((widths[0].0 - plain_widths[0].0).abs() < 0.01, "AutoFit Contents minimum: {widths:?} versus {plain_widths:?}");
            assert!((widths[0].1 - plain_widths[0].1).abs() < 0.01, "AutoFit Contents maximum: {widths:?} versus {plain_widths:?}");
        }
    }
}

fn table_for(doc: &Document) -> Table {
    match &*doc.body[0] {
        Block::Table(t) => t.clone(),
        _ => panic!("fixture must contain a table"),
    }
}

#[test]
fn hanging_autofit_narrow_labels_and_trailing_spaces_fit_on_the_first_line() {
    for left in [255.0 / 20.0, 264.0 / 20.0, 282.0 / 20.0] {
        for bold in [false, true] {
            for text in ["S :", "S : "] {
                let (plain, _) = fixture(text, 0.0, 0.0, 481.0 / 20.0, bold);
                let (doc, _) = fixture(text, left, -left, 481.0 / 20.0, bold);
                assert_eq!(cells(&lay(&doc)), cells(&lay(&plain)), "{text:?}, bold {bold}, indent {left}");
            }
        }
    }
}

#[test]
fn hanging_autofit_retains_room_for_subsequent_lines_and_real_positive_indents() {
    let (plain, plain_table) = fixture("R:\nSubstantial", 0.0, 0.0, 30.0, false);
    let (hanging, hanging_table) = fixture("R:\nSubstantial", 30.0, -30.0, 30.0, false);
    let a = measure_table_columns(&plain, &plain_table)[0];
    let b = measure_table_columns(&hanging, &hanging_table)[0];
    assert!((b.0 - a.0 - 30.0).abs() < 0.01, "the second line really needs the left indent: {a:?}, {b:?}");
    let (positive, _) = fixture("R:", 30.0, 0.0, 30.0, false);
    assert!(cells(&lay(&positive))[0].w > 40.0, "a genuine positive left indent still needs room");
    let (mut fixed, _) = fixture("R:", 30.0, -30.0, 30.0, false);
    if let Block::Table(table) = Arc::make_mut(&mut fixed.body[0]) {
        table.props.fixed = true;
    }
    let (plain, _) = fixture("R:", 0.0, 0.0, 30.0, false);
    assert_eq!(cells(&lay(&fixed)), cells(&lay(&plain)), "fixed widths keep their grid");
}

#[test]
fn hanging_autofit_first_long_word_does_not_pay_a_later_lines_indent() {
    let text = "Substantial W W W W W W W W W W W W W W W W W W W W";
    let (plain, plain_table) = fixture(text, 0.0, 0.0, 30.0, false);
    let (hanging, hanging_table) = fixture(text, 30.0, -30.0, 30.0, false);
    let a = measure_table_columns(&plain, &plain_table)[0].0;
    let b = measure_table_columns(&hanging, &hanging_table)[0].0;
    assert!((a - b).abs() < 0.01, "the first word fits at zero; later short words fit at 30pt: {a} versus {b}");
    assert_eq!(cells(&lay(&hanging))[0].w, cells(&lay(&plain))[0].w);
}

#[test]
fn hanging_autofit_records_open_font_metrics() {
    for (family, size) in [("Arimo", 10.0), (WRAP_TEST_FONT, 8.5)] {
        for bold in [false, true] {
            let resolved = wordcraft_fonts::word::resolve(family, bold, false);
            let face = &resolved.face;
            if family == WRAP_TEST_FONT {
                let bytes = if bold {
                    include_bytes!("../../../../assets/fonts/SourceSans3-Bold.ttf").as_slice()
                } else {
                    include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf").as_slice()
                };
                assert_eq!(face.family, family);
                assert!(face.data() == bytes, "narrow controls use the exact bundled OFL face");
            }
            for label in ["R:", "S :", "S : ", "R\u{a0}:"] {
                let width =
                    wordcraft_fonts::shape(face, label, &[], |c| c).iter().map(|g| g.x_advance as f64).sum::<f64>() * size / face.units_per_em();
                assert!(width.is_finite() && width > 0.0);
                eprintln!(
                    "{family}, requested bold {bold}: {} {}, UPEM {}, {size}pt {label:?}: {width}pt",
                    face.family,
                    face.style,
                    face.units_per_em()
                );
            }
        }
    }
}

#[test]
fn hanging_autofit_trailing_tab_keeps_its_stop_but_trailing_spaces_do_not_add_width() {
    for text in ["R:\t", "R:\t  "] {
        let (mut doc, mut table) = fixture(text, 30.0, -30.0, 30.0, false);
        if let Block::Para(p) = Arc::make_mut(&mut table.rows[0].cells[0].blocks[0]) {
            p.props.tabs = Some(vec![wordcraft_doc::props::TabStop { pos: 42.0, align: Default::default(), leader: Default::default() }]);
        }
        table.props.width = None;
        for cell in &mut table.rows[0].cells {
            cell.props.width = None;
        }
        table.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(Paragraph::new())];
        doc.body[0] = Arc::new(Block::Table(table.clone()));
        let (min, max) = measure_table_columns(&doc, &table)[0];
        assert!((min - 40.8).abs() < 0.01, "the minimum prefix must also retain the tab endpoint: {min}");
        assert!((max - 40.8).abs() < 0.01, "30pt implicit hanging stop precedes the 42pt stop, plus 10.8pt cell margins: {max}");
        assert!((cells(&lay(&doc))[0].w - 40.8).abs() < 0.01, "cached maximum retains the tab stop");
    }
}

#[test]
fn hanging_autofit_nbsp_stays_inside_the_unbreakable_later_word() {
    let (joined, joined_table) = fixture("R:\nA\u{a0}B", 30.0, -30.0, 30.0, false);
    let (breakable, breakable_table) = fixture("R:\nA B", 30.0, -30.0, 30.0, false);
    let joined_min = measure_table_columns(&joined, &joined_table)[0].0;
    let breakable_min = measure_table_columns(&breakable, &breakable_table)[0].0;
    assert!(joined_min > breakable_min + 4.0, "the NBSP joins both letters: {joined_min} versus {breakable_min}");
    assert!(cells(&lay(&joined))[0].w > cells(&lay(&breakable))[0].w + 4.0, "cached layout also retains the joined word");
}

#[test]
fn hanging_autofit_review_numbered_direct_indents_keep_their_measurement() {
    let (mut doc, mut table) = fixture("R:", 30.0, -30.0, 30.0, false);
    let num = doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    if let Block::Para(p) = Arc::make_mut(&mut table.rows[0].cells[0].blocks[0]) {
        p.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
    }
    let hanging = measure_table_columns(&doc, &table)[0];
    if let Block::Para(p) = Arc::make_mut(&mut table.rows[0].cells[0].blocks[0]) {
        p.props.indent_first = Some(0.0);
    }
    let control = measure_table_columns(&doc, &table)[0];
    assert!((hanging.0 - control.0).abs() < 0.01, "numbered minimum keeps the existing positive indent: {hanging:?} versus {control:?}");
    assert!((hanging.1 - control.1).abs() < 0.01, "numbered maximum keeps the existing positive indent: {hanging:?} versus {control:?}");
}

#[test]
fn hanging_autofit_review_disabled_numbering_still_uses_the_first_line_origin() {
    let (doc, mut table) = fixture("R:", 30.0, -30.0, 30.0, false);
    let control = measure_table_columns(&doc, &table)[0];
    if let Block::Para(p) = Arc::make_mut(&mut table.rows[0].cells[0].blocks[0]) {
        p.props.numbering = Some(wordcraft_doc::props::NumRef { num: 0, level: 0 });
    }
    let cleared = measure_table_columns(&doc, &table)[0];
    assert_eq!(cleared, control, "numId 0 explicitly switches numbering off");
}

#[test]
fn hanging_autofit_review_later_tabs_retain_the_endpoint_in_the_minimum() {
    for text in ["R:\n\t", "R:\n\t  "] {
        let (mut doc, mut table) = fixture(text, 30.0, -30.0, 30.0, false);
        if let Block::Para(p) = Arc::make_mut(&mut table.rows[0].cells[0].blocks[0]) {
            p.props.tabs = Some(vec![wordcraft_doc::props::TabStop { pos: 42.0, align: Default::default(), leader: Default::default() }]);
        }
        table.props.width = None;
        for cell in &mut table.rows[0].cells {
            cell.props.width = None;
        }
        table.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(Paragraph::new())];
        doc.body[0] = Arc::new(Block::Table(table.clone()));
        let (min, max) = measure_table_columns(&doc, &table)[0];
        assert!((min - 52.8).abs() < 0.01, "42pt later-line tab endpoint plus 10.8pt cell margins: {min}");
        assert!((max - 52.8).abs() < 0.01, "the maximum already retained the same endpoint: {max}");
        assert!((cells(&lay(&doc))[0].w - 52.8).abs() < 0.01, "cached automatic width retains the tab stop");
    }
}

#[test]
fn hanging_autofit_first_line_tabs_keep_endpoints_across_prefix_breaks() {
    let mut mismatches = Vec::new();
    for (text, endpoint) in [
        ("R: \t", 30.0),
        ("\t", 30.0),
        ("R:\t\t", 42.0),
        (" \t\t  ", 42.0),
        ("R R R R\t", 30.0),
        ("R R R R \t", 42.0),
        ("R R R R R\t", 42.0),
        ("R R R R R \t", 42.0),
    ] {
        let (mut doc, mut table) = fixture(text, 30.0, -30.0, 30.0, false);
        if let Block::Para(p) = Arc::make_mut(&mut table.rows[0].cells[0].blocks[0]) {
            p.props.tabs = Some(vec![wordcraft_doc::props::TabStop { pos: 42.0, align: Default::default(), leader: Default::default() }]);
        }
        table.props.width = None;
        for cell in &mut table.rows[0].cells {
            cell.props.width = None;
        }
        table.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(Paragraph::new())];
        doc.body[0] = Arc::new(Block::Table(table.clone()));
        let expected = endpoint + 10.8;
        let (min, max) = measure_table_columns(&doc, &table)[0];
        let cached = cells(&lay(&doc))[0].w;
        eprintln!("{text:?}: minimum {min}, maximum {max}, cached width {cached}, expected {expected}");
        if (min - expected).abs() >= 0.01 || (max - expected).abs() >= 0.01 || (cached - expected).abs() >= 0.01 {
            mismatches.push((text, min, max, cached, expected));
        }
    }
    assert!(mismatches.is_empty(), "all first-line tab endpoints must remain in both width bounds: {mismatches:?}");
}

#[test]
fn hanging_autofit_positional_tabs_do_not_make_the_measurement_width_a_minimum() {
    use wordcraft_doc::para::{InlineObject, PTabAlign, PTabBase};
    for (prefix, suffix, trailing_tab) in [
        ("R R R R R ", "", false),
        ("R R R R R ", "", true),
        ("", "Witness", false),
        ("", "Witness", true),
        ("R:\n", "Witness", false),
        ("R:\n", "Witness", true),
    ] {
        let (mut doc, mut table) = fixture(prefix, 30.0, -30.0, 30.0, false);
        if let Block::Para(p) = Arc::make_mut(&mut table.rows[0].cells[0].blocks[0]) {
            p.insert_object(
                p.text.len(),
                InlineObject::PositionalTab { align: PTabAlign::Right, relative_to: PTabBase::Margin, leader: Default::default() },
                &CharProps::default(),
            )
            .unwrap();
            if trailing_tab {
                p.insert_text(p.text.len(), "\t", &CharProps::default()).unwrap();
            }
            p.insert_text(p.text.len(), suffix, &CharProps::default()).unwrap();
        }
        doc.body[0] = Arc::new(Block::Table(table.clone()));
        let (min, max) = measure_table_columns(&doc, &table)[0];
        assert!(min < 200.0, "a width-relative tab cannot require the unbounded measurement width: {min}");
        assert!(max > 1_000.0, "the control must really place a right-margin alignment tab, capped by column measurement: {max}");
    }
}
