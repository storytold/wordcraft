use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_doc::para::PAGE_BREAK;
use wordcraft_doc::props::CharProps;

use crate::Session;

/// Creates a document with `lines` lines of text.
fn create_multiline_doc(lines: usize) -> Session {
    let mut text = String::new();
    for i in 1..=lines {
        text.push_str(&format!("Line {i}: The quick brown fox jumps over the lazy dog.\n"));
    }
    Session::new(Document::from_text(&text))
}

#[test]
fn fit_one_page_fails_on_single_page() {
    let mut s = create_multiline_doc(5);
    let before = s.doc.clone();
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("already fits on one page"), "unexpected error: {err}");
    assert_eq!(s.doc, before, "document must not change on failure");
}

#[test]
fn fit_one_page_fails_on_hard_page_break() {
    let text = format!("Page 1 content\n{PAGE_BREAK}Page 2 content\n");
    let mut s = Session::new(Document::from_text(&text));
    let before = s.doc.clone();
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("explicit page or section breaks"), "unexpected error: {err}");
    assert_eq!(s.doc, before, "document must not change on failure");
}

#[test]
fn fit_one_page_refuses_page_break_before_before_search() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);
    if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut s.doc.body[1]) {
        p.props.page_break_before = Some(true);
    }
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);
    let before = s.doc.clone();

    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("explicit page or section breaks"), "page_break_before must be reported as an explicit break: {err}");
    assert_eq!(s.doc, before, "document must not change on refusal");
}

#[test]
fn fit_one_page_fails_on_next_page_section_break() {
    let mut s = Session::new(Document::from_text("First section body text"));
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Second section text on next page"})).unwrap();
    assert_eq!(s.layout().pages.len(), 2, "break must cause 2 pages");

    let before = s.doc.clone();
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("explicit page or section breaks"), "unexpected error: {err}");
    assert_eq!(s.doc, before, "document must not change on failure");
}

#[test]
fn fit_one_page_condenses_slight_overflow_and_undoes() {
    // Find the boundary where the document just spills onto page 2
    let mut lines = 30;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 2;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2, "must start as a 2-page document with {lines} lines");

    let doc_before = s.doc.clone();
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "fit should succeed on slight overflow: {res:?}");
    assert_eq!(s.layout().pages.len(), 1, "document must now fit on 1 page");
    assert!(s.status.contains("Document fitted to 1 page"), "status was: {}", s.status);

    // Test single atomic undo
    let undo_res = s.run("edit.undo", &json!({}));
    assert!(undo_res.is_ok(), "undo must succeed");
    assert_eq!(s.layout().pages.len(), 2, "undo must restore 2-page layout");
    assert_eq!(s.doc, doc_before, "undo must restore exact pre-fit document state");

    // Test redo
    let redo_res = s.run("edit.redo", &json!({}));
    assert!(redo_res.is_ok(), "redo must succeed");
    assert_eq!(s.layout().pages.len(), 1, "redo must restore 1-page layout");
}

#[test]
fn fit_one_page_respects_max_delta() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2, "must start as 2-page document");

    let doc_backup = s.doc.clone();
    let ok_res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(ok_res.is_ok(), "must succeed when positive font delta 0.5 is permitted: {ok_res:?}");
    assert_eq!(s.layout().pages.len(), 1);

    s.doc = doc_backup;
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.0, "adjustSpacing": false}));
    assert!(res.is_err(), "must fail when font delta is capped at 0.0 and spacing is disabled");
}

#[test]
fn fit_one_page_respects_step_pt() {
    let mut text = String::new();
    for i in 1..=28 {
        text.push_str(&format!("Line {i}: The quick brown fox jumps over the lazy dog.\n"));
    }
    let mut s = Session::new(Document::from_text(&text));
    for b in &mut s.doc.body {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.mark.size = Some(11.4);
            for r in &mut p.runs {
                r.props.size = Some(11.4);
            }
        }
    }
    assert_eq!(s.layout().pages.len(), 2, "must start as 2-page document at 11.4 pt");

    // Both calls hold maxDeltaPt constant at 0.4 and disable spacing adjustments.
    // Default 0.5 pt step generates no candidates <= 0.4 pt, so it must fail.
    let doc_backup = s.doc.clone();
    let default_res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.4, "adjustSpacing": false}));
    assert!(default_res.is_err(), "default step 0.5 must skip all candidates when maxDelta is 0.4");

    // With stepPt = 0.1 under the same maxDeltaPt = 0.4, fine-grained steps (0.1..0.4) succeed (11.4 -> 11.0 pt).
    s.doc = doc_backup;
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.4, "stepPt": 0.1, "adjustSpacing": false}));
    assert!(res.is_ok(), "stepPt=0.1 must generate fine-grained candidates under maxDelta 0.4: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_allows_spacing_only_at_font_floor() {
    let mut lines = 35;
    let mut s = create_multiline_doc(lines);
    let set_font = |s: &mut Session| {
        for b in &mut s.doc.body {
            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
                for r in &mut p.runs {
                    r.props = CharProps { size: Some(8.5), ..Default::default() };
                }
            }
        }
        s.touch();
        s.relayout();
    };
    set_font(&mut s);
    while s.layout().pages.len() < 2 && lines < 120 {
        lines += 1;
        s = create_multiline_doc(lines);
        set_font(&mut s);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Spacing-only candidate must be permitted to run and succeed
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "spacing-only candidate must succeed at font floor: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
    assert!(s.status.contains("spacing: "), "status: {}", s.status);
    assert!(!s.status.contains("font: -"), "font must remain untouched: {}", s.status);
}

#[test]
fn fit_one_page_fails_when_body_font_already_at_minimum_readable_size() {
    let mut lines = 35;
    let mut s = create_multiline_doc(lines);
    let set_font = |s: &mut Session| {
        for b in &mut s.doc.body {
            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
                for r in &mut p.runs {
                    r.props = CharProps { size: Some(8.0), ..Default::default() };
                }
            }
        }
        s.touch();
        s.relayout();
    };
    set_font(&mut s);
    while s.layout().pages.len() < 2 && lines < 120 {
        lines += 1;
        s = create_multiline_doc(lines);
        set_font(&mut s);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // When spacing adjustment is disabled, no candidate can run without reducing font
    let res = s.run("layout.fitOnePage", &json!({"adjustSpacing": false}));
    assert!(res.is_err(), "must refuse to shrink font when already <= 8.5 pt");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Unable to shrink document further without reducing text beyond readability limits."), "unexpected error message: {err}");
}

#[test]
fn fit_one_page_bounds_fine_step_loop() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2, "fixture must reach candidate generation on 2 pages");

    // With stepPt = 0.001 and maxDeltaPt = 2.0 (ratio = 2000), fine-step candidate count
    // is clamped by MAX_FINE_STEPS (20), executing quickly without doing thousands of iterations.
    let res = s.run("layout.fitOnePage", &json!({"stepPt": 0.001, "maxDeltaPt": 2.0, "adjustSpacing": false}));
    assert!(res.is_ok(), "fitOnePage with bounded fine steps succeeds or terminates gracefully: {res:?}");
}

#[test]
fn fit_one_page_supports_max_delta_above_2pt() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    // Add 8 lines of spill (within the 12-line spill limit)
    for i in 1..=8 {
        s.run("text.insert", &json!({"text": format!("Spill paragraph line {i}\n")})).unwrap();
    }
    assert_eq!(s.layout().pages.len(), 2);

    // With maxDeltaPt = 3.0, candidate above 2 pt fits the document
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 3.0}));
    assert!(res.is_ok(), "fitOnePage with maxDeltaPt=3.0 must succeed: {res:?}");
    assert_eq!(s.layout().pages.len(), 1, "must fit on 1 page");
}

#[test]
fn fit_one_page_anti_compound_guard_prevents_repeated_invocations() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    loop {
        for b in &mut s.doc.body {
            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
                p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(0.94));
                for r in &mut p.runs {
                    r.props.size = Some(11.0);
                }
            }
        }
        s.touch();
        s.relayout();
        if s.layout().pages.len() >= 2 || lines >= 80 {
            break;
        }
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Simulate a document that already used the full 1.0 pt font and 0.94 spacing budgets.
    s.doc.fit_state = Some(wordcraft_doc::FitState {
        cum_delta: 1.0,
        cum_space: 0.94,
        original_font_sizes: vec![12.0],
        original_line_spacings: vec![wordcraft_doc::props::LineSpacing::Multiple(1.0)],
        ..Default::default()
    });
    s.touch();
    s.relayout();
    let doc_before = s.doc.clone();

    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err(), "exhausted budgets must be rejected");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Unable to shrink document further without reducing text beyond readability limits."), "unexpected error message: {err}");
    assert_eq!(s.doc, doc_before, "document must not be mutated on rejected invocation");
}

#[test]
fn fit_one_page_already_fits_wins_over_exhausted_budget() {
    let mut s = create_multiline_doc(5);
    assert_eq!(s.layout().pages.len(), 1);
    for b in &mut s.doc.body {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(0.94));
            for r in &mut p.runs {
                r.props.size = Some(11.0);
            }
        }
    }
    s.doc.fit_state = Some(wordcraft_doc::FitState {
        cum_delta: 1.0,
        cum_space: 0.94,
        original_font_sizes: vec![12.0],
        original_line_spacings: vec![wordcraft_doc::props::LineSpacing::Multiple(1.0)],
        ..Default::default()
    });
    s.touch();
    s.relayout();

    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("already fits on one page"), "already-fits must take precedence over exhausted budgets: {err}");
}

#[test]
fn fit_one_page_checks_complex_script_font_floor() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Insert an Arabic run with size 12.0 pt but size_cs 9.0 pt
    let arabic_p = wordcraft_doc::Paragraph::with_text(
        "نص تجريبي باللغة العربية",
        CharProps { size: Some(12.0), size_cs: Some(9.0), cs: Some(true), ..Default::default() },
    );
    s.doc.body.insert(0, wordcraft_doc::para_block(arabic_p));
    assert_eq!(s.layout().pages.len(), 2);

    // A 1.0 pt reduction would reduce size_cs from 9.0 to 8.0 (< 8.5 pt floor).
    // The trial must not commit.
    let _ = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 1.0, "adjustSpacing": false}));
    let p = s.doc.body[0].as_para().unwrap();
    let cs_size = p.runs[0].props.size_cs.unwrap_or(9.0);
    assert!(cs_size >= 8.5, "complex-script size must remain >= 8.5 pt, was {cs_size}");
}

#[test]
fn fit_one_page_checks_editable_text_box_font_floor() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Add a body-anchored text box with 9.0 pt font
    let box_id = s.doc.add_part(
        wordcraft_doc::PartKind::TextBox,
        vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(
            "Text box body text",
            CharProps { size: Some(9.0), ..Default::default() },
        ))],
    );
    // Link it into body
    let mut shape_p = wordcraft_doc::Paragraph::with_text("Before box ", CharProps::default());
    let shape = wordcraft_doc::InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 100.0,
        h: 50.0,
        fill: None,
        stroke: None,
        stroke_width: 0.5,
        float: Default::default(),
        story: Some(box_id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    shape_p.objects.push(shape);
    s.doc.body.insert(0, wordcraft_doc::para_block(shape_p));
    assert_eq!(s.layout().pages.len(), 2);

    // With maxDeltaPt = 1.0 and adjustSpacing = false, 9.0 pt would reduce to 8.0 pt.
    // The trial must not be committed.
    let _ = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 1.0, "adjustSpacing": false}));
    let part = s.doc.parts.get(&box_id).unwrap();
    let box_p = part.blocks[0].as_para().unwrap();
    let sz = box_p.runs[0].props.size.unwrap();
    assert!(sz >= 8.5, "editable text box font must remain >= 8.5 pt, was {sz}");
}

#[test]
fn fit_one_page_fails_when_overflow_exceeds_limits() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    // Add 16 extra lines to page 2: this overflow is too large to fit into page 1 with maxDeltaPt = 0.5
    for i in 1..=16 {
        s.run("text.insert", &json!({"text": format!("Overflow line {i}\n")})).unwrap();
    }
    assert_eq!(s.layout().pages.len(), 2, "must be a 2-page document");

    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5}));
    assert!(res.is_err(), "must refuse when overflow exceeds aesthetic reduction limits");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("exceeds limits for aesthetic page reduction"), "unexpected error message: {err}");
}

#[test]
fn fit_one_page_validates_effective_font_in_tables_before_committing() {
    let mut s = Session::new(Document::new());
    let mut table = wordcraft_doc::Table::new(1, 1, 400.0);
    let cell_p = wordcraft_doc::Paragraph::with_text("Cell text", CharProps { size: Some(9.0), ..Default::default() });
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cell_p)];
    s.doc.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(table))];

    let mut text = String::new();
    for i in 1..=45 {
        text.push_str(&format!("Line {i} following table: The quick brown fox jumps over the lazy dog.\n"));
    }
    s.doc.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, CharProps::default())));
    assert_eq!(s.layout().pages.len(), 2);

    // If maxDeltaPt is 1.0, 9.0 pt would reduce to 8.0 pt (violating 8.5 pt floor).
    // The candidate must not be committed.
    let _ = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 1.0, "adjustSpacing": false}));
    if let Some(tbl) = s.doc.body[0].as_table() {
        let p = tbl.rows[0].cells[0].blocks[0].as_para().unwrap();
        let sz = p.runs[0].props.size.unwrap();
        assert!(sz >= 8.5, "table cell font size must remain >= 8.5 pt, was {sz}");
    }
}

#[test]
fn fit_one_page_validates_effective_font_in_table_styles_before_committing() {
    let mut s = Session::new(Document::new());
    // Create a custom table style with 9.0 pt font
    let st = wordcraft_doc::styles::Style {
        id: "CustomTableStyle".into(),
        name: "CustomTableStyle".into(),
        kind: wordcraft_doc::styles::StyleKind::Table,
        chr: CharProps { size: Some(9.0), ..Default::default() },
        table: Some(wordcraft_doc::styles::TableStyleParts::default()),
        ..Default::default()
    };
    std::sync::Arc::make_mut(&mut s.doc.styles).upsert(st);

    let mut table = wordcraft_doc::Table::new(1, 1, 400.0);
    table.props.style = Some("CustomTableStyle".to_string());
    let cell_p = wordcraft_doc::Paragraph::with_text("Cell text inheriting table style", CharProps::default());
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cell_p)];
    s.doc.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(table))];

    let mut text = String::new();
    for i in 1..=45 {
        text.push_str(&format!("Line {i} following table: The quick brown fox jumps over the lazy dog.\n"));
    }
    s.doc.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, CharProps::default())));
    assert_eq!(s.layout().pages.len(), 2);

    // If maxDeltaPt is 1.0, 9.0 pt in the table style would reduce to 8.0 pt (violating 8.5 pt floor).
    // The candidate must not be committed.
    let _ = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 1.0, "adjustSpacing": false}));
    let style = s.doc.styles.table_style("CustomTableStyle").unwrap();
    let sz = style.chr.size.unwrap();
    assert!(sz >= 8.5, "table style font size must remain >= 8.5 pt, was {sz}");
}

#[test]
fn fit_one_page_measures_only_final_page_placed_lines() {
    let mut s = Session::new(Document::new());
    let words = "The quick brown fox jumps over the lazy dog and runs across the wide open fields in the sunshine. ";
    let mut reps = 20;
    loop {
        let text = words.repeat(reps);
        s.doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, CharProps::default()))];
        s.touch();
        s.relayout();
        if s.layout().pages.len() >= 2 {
            break;
        }
        reps += 1;
    }
    assert_eq!(s.layout().pages.len(), 2);

    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "fitOnePage must succeed when single paragraph slightly spills onto page 2: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_exempts_style_inherited_superscript() {
    let mut s = Session::new(Document::new());
    // Create a Character style with Superscript alignment and 7.0 pt font
    let st = wordcraft_doc::styles::Style {
        id: "SuperStyle".into(),
        name: "SuperStyle".into(),
        kind: wordcraft_doc::styles::StyleKind::Character,
        chr: CharProps { vert_align: Some(wordcraft_doc::props::VertAlign::Superscript), size: Some(7.0), ..Default::default() },
        ..Default::default()
    };
    std::sync::Arc::make_mut(&mut s.doc.styles).upsert(st);

    let words = "Regular readable text that fills up lines in the document body. ";
    let mut reps = 20;
    loop {
        let mut p = wordcraft_doc::Paragraph::with_text(&words.repeat(reps), CharProps { size: Some(11.0), ..Default::default() });
        // Add a run with style-inherited superscript
        p.runs.push(wordcraft_doc::Run { len: 3, props: CharProps { style: Some("SuperStyle".into()), ..Default::default() } });
        p.text.push_str("123");
        s.doc.body = vec![wordcraft_doc::para_block(p)];
        s.touch();
        s.relayout();
        if s.layout().pages.len() >= 2 {
            break;
        }
        reps += 1;
    }
    assert_eq!(s.layout().pages.len(), 2);

    // fitOnePage must not be blocked by the 7.0 pt superscript run because it is exempt
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "style-inherited superscript must be exempt from font floor: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_refused_when_protect_formatting_enabled() {
    let mut s = Session::new(Document::new());
    s.doc.settings.protection = Some("none".into());
    s.doc.settings.protect_formatting = true;
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("formatting is limited to the document's styles"), "unexpected error: {err}");
}

#[test]
fn fit_one_page_zero_max_delta_never_reduces_font() {
    let mut s = Session::new(Document::new());
    let words = "Content line with fractional font size for zero delta testing. ";
    let mut reps = 20;
    loop {
        let text = words.repeat(reps);
        s.doc.body =
            vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, CharProps { size: Some(10.3), ..Default::default() }))];
        s.touch();
        s.relayout();
        if s.layout().pages.len() >= 2 {
            break;
        }
        reps += 1;
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Call fitOnePage with maxDeltaPt = 0.0 and a tiny stepPt (which should not produce any positive candidate)
    let _ = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.0, "stepPt": 0.001}));
    let p = s.doc.body[0].as_para().unwrap();
    assert_eq!(p.runs[0].props.size, Some(10.3), "font size must remain completely unchanged when maxDeltaPt is 0");
}

#[test]
fn fit_one_page_fails_with_failed_cmd_error() {
    let mut s = create_multiline_doc(5);
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(matches!(res, Err(crate::CmdError::Failed(_))), "expected CmdError::Failed, got {res:?}");
}

#[test]
fn fit_one_page_never_populates_custom_props() {
    let mut lines = 30;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 2;
        s = create_multiline_doc(lines);
    }
    assert!(s.doc.custom_props.is_empty());
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok());
    assert!(s.doc.custom_props.is_empty(), "custom_props must not be populated with internal fit state");
}

#[test]
fn fit_one_page_allows_subsequent_fit_after_typography_reset() {
    use std::sync::Arc;

    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // First invocation succeeds
    let res1 = s.run("layout.fitOnePage", &json!({}));
    assert!(res1.is_ok());
    assert_eq!(s.layout().pages.len(), 1);

    // Reset styles back to Normal with 12 pt font and 1.15 line spacing
    if let Some(st) = Arc::make_mut(&mut s.doc.styles).styles.iter_mut().find(|st| st.id == "Normal") {
        st.chr.size = Some(12.0);
        st.para.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.15));
    }
    for b in &mut s.doc.body {
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(b) {
            p.props.line_spacing = None;
            for r in &mut p.runs {
                r.props.size = None;
            }
        }
    }
    s.touch();
    s.relayout();

    // Now add lines to make it 2 pages again
    while s.layout().pages.len() < 2 {
        s.run("text.insert", &json!({"text": "Additional fresh paragraph after style reset.\n"})).unwrap();
    }
    assert_eq!(s.layout().pages.len(), 2);

    // A subsequent fitOnePage run must NOT be permanently blocked and should succeed
    let res2 = s.run("layout.fitOnePage", &json!({}));
    assert!(res2.is_ok(), "fitOnePage must succeed after typography reset: {res2:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_small_paragraph_mark_with_runs_never_blocks_fit() {
    let mut lines = 30;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 2;
        s = create_multiline_doc(lines);
    }
    // Set a 1.0 pt paragraph mark on a body paragraph that has normal 11 pt text runs
    if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut s.doc.body[0]) {
        p.mark.size = Some(1.0);
    }
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "small mark with normal text runs must not block fit: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_small_comment_font_never_blocks_body_fit() {
    let mut lines = 30;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 2;
        s = create_multiline_doc(lines);
    }
    // Add a comment part with 6.0 pt text
    let comment_p = wordcraft_doc::Paragraph::with_text("Small comment text", CharProps { size: Some(6.0), ..Default::default() });
    s.doc.add_part(wordcraft_doc::PartKind::Comment, vec![wordcraft_doc::para_block(comment_p)]);
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "small comment font must not block body fit: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_anti_compound_caps_total_reduction_across_runs() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);

    // Initial fit with -0.5 pt font reduction (spacing disabled so font budget is exercised)
    let res1 = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res1.is_ok());
    assert_eq!(s.layout().pages.len(), 1);
    assert_eq!(s.doc.fit_state.as_ref().map(|f| f.cum_delta), Some(0.5));

    // Add content to spill to 2 pages again
    for i in 1..=12 {
        s.run("text.insert", &json!({"text": format!("Spill paragraph line {i}\n")})).unwrap();
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Candidate requiring > 0.5 pt more (e.g. total > 1.0 pt) must be rejected
    let res2 = s.run("layout.fitOnePage", &json!({}));
    assert!(res2.is_err(), "second run must not exceed cumulative 1.0 pt reduction budget");
}

#[test]
fn fit_one_page_tracks_reduced_text_provenance_without_global_minimum() {
    let mut s = create_multiline_doc(27);
    // Add a 9.0 pt run to the first paragraph alongside 11.0 pt runs
    if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut s.doc.body[0]) {
        p.runs.push(wordcraft_doc::para::Run { len: 5, props: CharProps { size: Some(9.0), ..Default::default() } });
        p.text.push_str("extra");
    }
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    // Initial fit with -0.5 pt: 9.0 pt -> 8.5 pt, 11.0 pt -> 10.5 pt (spacing disabled)
    let res1 = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res1.is_ok());
    assert_eq!(s.layout().pages.len(), 1);
    assert_eq!(s.doc.fit_state.as_ref().map(|f| f.cum_delta), Some(0.5));

    // Reset only the 8.5 pt run back to 12.0 pt (so current global minimum font becomes 10.5 pt)
    if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut s.doc.body[0]) {
        p.runs.last_mut().unwrap().props.size = Some(12.0);
    }
    s.touch();
    s.relayout();

    // Add content to spill to 2 pages again
    for i in 1..=12 {
        s.run("text.insert", &json!({"text": format!("Spill paragraph line {i}\n")})).unwrap();
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Because the 10.5 pt runs are still reduced, fit state must NOT be discarded,
    // and a candidate requiring > 0.5 pt more font reduction must be rejected by the 1.0 pt cumulative cap
    let res2 = s.run("layout.fitOnePage", &json!({}));
    assert!(res2.is_err(), "cumulative budget must still hold even when the original minimum font was changed");
}

#[test]
fn fit_one_page_allows_spill_with_bottom_placed_footnote() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Add a single-line footnote to the document (reference on page 1)
    let note_p = wordcraft_doc::Paragraph::with_text("Short footnote text.", CharProps::default());
    let note_id = s.doc.add_part(wordcraft_doc::PartKind::Footnote, vec![wordcraft_doc::para_block(note_p)]);
    let noteref = wordcraft_doc::para::InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id: note_id, custom: String::new() };
    s.doc.insert_object(&wordcraft_doc::Pos::body(0, 5), noteref, &Default::default()).unwrap();
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    // Even though the footnote is placed at the bottom of the page (large y),
    // fill_ratio must measure only actual content height so fitOnePage is not falsely refused
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "bottom-placed footnote must not drive fill ratio above 30%: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_with_final_page_footnote() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Insert note reference into a paragraph on the final page (page 2)
    let last_para_idx = s.doc.body.len().saturating_sub(1);
    let note_p = wordcraft_doc::Paragraph::with_text("Final page footnote text.", CharProps::default());
    let note_id = s.doc.add_part(wordcraft_doc::PartKind::Footnote, vec![wordcraft_doc::para_block(note_p)]);
    let noteref = wordcraft_doc::para::InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id: note_id, custom: String::new() };
    s.doc.insert_object(&wordcraft_doc::Pos::body(last_para_idx, 0), noteref, &Default::default()).unwrap();
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    // Verify footnote is assigned to the final page (page index 1)
    let page2_has_footnote = s.layout().pages[1]
        .items
        .iter()
        .any(|item| matches!(item, wordcraft_layout::Placed::Lines { story: wordcraft_doc::StoryRef::Part(id), .. } if *id == note_id));
    assert!(page2_has_footnote, "footnote must be assigned to page 2");

    // With modest spill and small footnote within the 30% fill threshold, fitOnePage succeeds
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "final-page footnote within 30% fill limit must allow fitOnePage to succeed: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_skips_deleted_text_when_markup_hidden() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);

    // Insert a deleted text run with small font (e.g. 6.0 pt, below the 8.5 pt floor)
    if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut s.doc.body[0]) {
        let mut del_props = CharProps { size: Some(6.0), ..Default::default() };
        del_props.del = Some(1);
        p.runs.push(wordcraft_doc::para::Run { len: 8, props: del_props });
        p.text.push_str("deleted!");
    }
    s.touch();
    s.relayout();

    // When show_markup is false (the default in print view), deleted run is hidden
    // and skipped during font-floor scan, allowing fitOnePage to succeed
    s.view.show_markup = false;
    let doc_backup = s.doc.clone();
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res.is_ok(), "deleted text below floor must be skipped when markup is hidden: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);

    // When show_markup is true, deleted run is drawn and evaluated in font-floor scan,
    // blocking font reduction because effective font is below 8.5 pt floor
    s.doc = doc_backup;
    s.view.show_markup = true;
    let res_markup = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res_markup.is_err(), "deleted text below floor must block font reduction when markup is shown");
}

#[test]
fn fit_one_page_tracks_reduced_complex_script_font_sizes() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);
    let styles_before = s.doc.styles.clone();

    // Set complex script font size on body run while keeping standard font size default (12 pt)
    if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut s.doc.body[0]) {
        p.runs[0].props.size_cs = Some(14.0);
    }
    s.touch();
    s.relayout();

    // First fit reduces font size by 0.5 pt (size -> 11.5, size_cs -> 13.5)
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res.is_ok(), "first fit must succeed: {res:?}");
    assert_eq!(s.doc.fit_state.as_ref().map(|f| f.cum_delta), Some(0.5));

    // User restores every other size (the styles the fit rescaled), leaving only this run's
    // explicit complex-script size reduced at 13.5 pt.
    s.doc.styles = styles_before;
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    // Because size_cs is still reduced, cumulative tracking retains the 0.5 pt delta provenance
    assert_eq!(s.doc.fit_state.as_ref().map(|f| f.cum_delta), Some(0.5));

    // Second fit within remaining 0.5 pt budget succeeds up to 1.0 pt cap
    let res2 = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res2.is_ok(), "second fit within remaining budget must succeed: {res2:?}");
    assert_eq!(s.doc.fit_state.as_ref().map(|f| f.cum_delta), Some(1.0));

    // Third fit exceeding cumulative 1.0 limit must be refused
    let res3 = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res3.is_err(), "fit exceeding cumulative limit must be refused");
}

#[test]
fn fit_one_page_preserves_budget_across_document_serialization() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);

    // Apply fit
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res.is_ok(), "fit must succeed: {res:?}");
    assert_eq!(s.doc.fit_state.as_ref().map(|f| f.cum_delta), Some(0.5));

    // Serialize and deserialize document (e.g. saving and reopening)
    let json_bytes = serde_json::to_vec(&s.doc).expect("serialization must succeed");
    let reopened_doc: wordcraft_doc::Document = serde_json::from_slice(&json_bytes).expect("deserialization must succeed");

    // Verify fit_state is preserved outside custom properties
    assert_eq!(reopened_doc.fit_state.as_ref().map(|f| f.cum_delta), Some(0.5));
    assert!(reopened_doc.custom_props.is_empty(), "custom_props must remain empty");

    // Reopen into a new session with additional spill
    let mut s2 = Session::new(reopened_doc);
    let extra_p = wordcraft_doc::Paragraph::with_text("Extra overflow paragraph.", CharProps::default());
    s2.doc.body.push(wordcraft_doc::para_block(extra_p));
    s2.touch();
    s2.relayout();

    // Second fit on reopened document preserves previous 0.5 delta, allowing only remaining 0.5 pt up to 1.0 cap
    let res2 = s2.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res2.is_ok(), "second fit within remaining budget must succeed");
    assert_eq!(s2.doc.fit_state.as_ref().map(|f| f.cum_delta), Some(1.0));

    // Third fit exceeding the cumulative 1.0 limit must be refused
    let res3 = s2.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res3.is_err(), "fit exceeding cumulative limit must be refused");
}

#[test]
fn fit_one_page_enforces_durable_limit_when_fit_state_missing() {
    // A document with body font already at 8.5 pt readability floor and fit_state == None
    let set_font = |s: &mut Session| {
        for b in &mut s.doc.body {
            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
                for r in &mut p.runs {
                    r.props.size = Some(8.5);
                }
            }
        }
        s.touch();
        s.relayout();
    };
    let mut lines = 27;
    let mut s = create_multiline_doc(lines);
    set_font(&mut s);
    while s.layout().pages.len() < 2 && lines < 120 {
        lines += 1;
        s = create_multiline_doc(lines);
        set_font(&mut s);
    }
    assert_eq!(s.layout().pages.len(), 2);
    s.doc.fit_state = None;
    s.touch();
    s.relayout();
    let before = s.doc.clone();

    // With adjustSpacing = false, font reduction is bounded by the durable 8.5 pt floor and refused
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res.is_err(), "fit must be refused when typography is at durable readability floor");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Unable to shrink document further without reducing text beyond readability limits."), "unexpected error: {err}");
    assert_eq!(s.doc, before, "a refused fit must leave the document unchanged");
}

#[test]
fn fit_one_page_skips_hidden_text_when_hidden_marks_off() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);

    // Insert a hidden text run with small font (6.0 pt, below the 8.5 pt floor) into an existing paragraph
    if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut s.doc.body[0]) {
        let mut hidden_props = CharProps { size: Some(6.0), ..Default::default() };
        hidden_props.hidden = Some(true);
        p.runs.push(wordcraft_doc::para::Run { len: 7, props: hidden_props });
        p.text.push_str("secret!");
    }
    s.touch();
    s.relayout();

    // When marks are off, hidden text is skipped from layout and floor checks
    s.view.marks = false;
    s.prefs.marks.hidden = false;
    let doc_backup = s.doc.clone();

    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res.is_ok(), "hidden text with small font must be skipped when hidden marks are off: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);

    // Reset document and test with marks turned on
    s.doc = doc_backup;
    s.view.marks = true;
    s.touch();
    s.relayout();

    let res_with_marks = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res_with_marks.is_err(), "hidden text with small font must block fit when marks are visible");
}

#[test]
fn fit_one_page_accounts_for_vertical_alignment_padding() {
    let mut s = create_multiline_doc(26);
    let extra_p = wordcraft_doc::Paragraph::with_text("Spill line on page 2.", CharProps::default());
    s.doc.body.push(wordcraft_doc::para_block(extra_p));

    // Set page vertical alignment to Bottom
    s.doc.last_section.valign = wordcraft_doc::section::PageVAlign::Bottom;
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    // Fit must succeed because the actual spill height (1 line) is well under 30% of page height,
    // even though the line was translated to the bottom of page 2 by valign padding.
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.5, "adjustSpacing": false}));
    assert!(res.is_ok(), "fit must succeed with vertical alignment padding: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_accounts_for_note_separator_decor_story() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Set custom footnote separator story so layout produces DECOR_STORY
    s.doc.footnote_separators.separator = Some(vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("---", CharProps::default()))]);

    // Insert note reference into a paragraph on the final page (page 2)
    let last_para_idx = s.doc.body.len().saturating_sub(1);
    let note_p = wordcraft_doc::Paragraph::with_text("Final page footnote text.", CharProps::default());
    let note_id = s.doc.add_part(wordcraft_doc::PartKind::Footnote, vec![wordcraft_doc::para_block(note_p)]);
    let noteref = wordcraft_doc::para::InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id: note_id, custom: String::new() };
    s.doc.insert_object(&wordcraft_doc::Pos::body(last_para_idx, 0), noteref, &Default::default()).unwrap();
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    // Verify layout produces DECOR_STORY on page 2
    let page2 = &s.layout().pages[1];
    let has_decor =
        page2.items.iter().any(|item| matches!(item, wordcraft_layout::Placed::Lines { story, .. } if *story == wordcraft_layout::DECOR_STORY));
    assert!(has_decor, "footnote on page 2 must generate DECOR_STORY separator");

    // Fit must succeed without DECOR_STORY inflating body_max_y
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_ok(), "fit with final page footnote and DECOR_STORY must succeed: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_tracks_fixed_spacing_independent_of_paragraph_mark_size() {
    let mut lines = 28;
    let mut s = create_multiline_doc(lines);
    let set_spacing = |s: &mut Session| {
        for b in &mut s.doc.body {
            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
                p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Exactly(16.0));
            }
        }
        s.touch();
        s.relayout();
    };
    set_spacing(&mut s);
    while s.layout().pages.len() < 2 && lines < 80 {
        lines += 1;
        s = create_multiline_doc(lines);
        set_spacing(&mut s);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Fit with spacing reduction
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.0, "adjustSpacing": true}));
    assert!(res.is_ok(), "spacing reduction must succeed: {res:?}");
    assert!(s.doc.fit_state.as_ref().is_some_and(|f| f.cum_space < 1.0));

    // Change paragraph mark font size on paragraphs (e.g. from 12 pt to 24 pt)
    for b in &mut s.doc.body {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.mark.size = Some(24.0);
        }
    }
    s.touch();
    s.relayout();

    // Second fit with spacing only must be refused because spacing floor 0.94 was reached
    // and tracked provenance was retained despite paragraph mark size change
    let res2 = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.0, "adjustSpacing": true}));
    assert!(res2.is_err(), "anti-compound guard must detect reduced fixed spacing and reject repeat compression");
}

#[test]
fn fit_one_page_reports_search_exhaustion_when_candidates_cannot_remove_page() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);

    // Calling with zero maxDeltaPt and adjustSpacing: false produces no candidate steps
    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 0.0, "adjustSpacing": false}));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("no permitted typography adjustment could remove a page"),
        "error message must distinguish search exhaustion from overflow limits: {err}"
    );
}

#[test]
fn fit_one_page_excludes_line_number_decorations_from_final_page_limits() {
    let mut lines = 33;
    let enable_line_numbers = |s: &mut Session| {
        s.doc.last_section.line_numbers = Some(wordcraft_doc::section::LineNumbering::default());
        s.touch();
        s.relayout();
    };
    let mut s = create_multiline_doc(lines);
    enable_line_numbers(&mut s);
    while s.layout().pages.len() < 2 && lines < 60 {
        lines += 1;
        s = create_multiline_doc(lines);
        enable_line_numbers(&mut s);
    }
    assert_eq!(s.layout().pages.len(), 2);

    // Every body line on the final page also places a line-number decoration. Counting those as
    // content would double the final-page line count and refuse before any candidate is tried.
    let decorated = s.layout().pages[1]
        .items
        .iter()
        .filter(|item| matches!(item, wordcraft_layout::Placed::Lines { story, .. } if *story == wordcraft_layout::DECOR_STORY))
        .count();
    assert!(decorated > 0, "the final page must carry line-number decorations");

    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 4.0}));
    assert!(res.is_ok(), "line-number decorations must not count as final-page content: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_counts_side_by_side_footnote_cells_once() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);

    // A footnote table whose row's cells sit side by side: their text occupies one vertical
    // interval, so their heights must not be summed as if stacked.
    let mut table = wordcraft_doc::Table::new(1, 3, 420.0);
    let cell_text = "The quick brown fox jumps over the lazy dog again and again.";
    for cell in &mut table.rows[0].cells {
        cell.blocks =
            vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(cell_text, CharProps { size: Some(21.0), ..Default::default() }))];
    }
    let note_blocks: Vec<std::sync::Arc<wordcraft_doc::Block>> = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(table))];
    let note_id = s.doc.add_part(wordcraft_doc::PartKind::Footnote, note_blocks);

    let last_para_idx = s.doc.body.len().saturating_sub(1);
    let noteref = wordcraft_doc::para::InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id: note_id, custom: String::new() };
    s.doc.insert_object(&wordcraft_doc::Pos::body(last_para_idx, 0), noteref, &Default::default()).unwrap();
    s.touch();
    s.relayout();
    assert_eq!(s.layout().pages.len(), 2);

    let note_cells = s.layout().pages[1]
        .items
        .iter()
        .filter(|item| matches!(item, wordcraft_layout::Placed::Cell { story: wordcraft_doc::StoryRef::Part(id), .. } if *id == note_id))
        .count();
    assert!(note_cells >= 2, "the footnote table must place its cells side by side on the final page");

    let res = s.run("layout.fitOnePage", &json!({"maxDeltaPt": 4.0}));
    assert!(res.is_ok(), "side-by-side footnote cells must count once in the fill height: {res:?}");
    assert_eq!(s.layout().pages.len(), 1);
}

#[test]
fn fit_one_page_refuses_when_formatting_changes_cannot_be_tracked() {
    let mut s = create_multiline_doc(27);
    assert_eq!(s.layout().pages.len(), 2);
    s.doc.settings.track_changes = true;
    s.prefs.markup.track_formatting = true;
    let before = s.doc.clone();

    // Style reductions cannot be recorded as formatting revisions, so the command must refuse
    // rather than change typography that rejecting all changes cannot undo.
    let res = s.run("layout.fitOnePage", &json!({}));
    assert!(res.is_err(), "layout.fitOnePage must refuse while formatting is tracked");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("tracks formatting changes"), "unexpected error: {err}");
    assert_eq!(s.doc, before, "a refused fit must leave the document unchanged");
}
