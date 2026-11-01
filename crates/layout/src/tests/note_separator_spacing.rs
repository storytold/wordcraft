use super::*;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{Paragraph, para_block};

fn separator(cont: bool, after: Option<f32>) -> wordcraft_doc::Blocks {
    let mut p = Paragraph::new();
    p.props.line_spacing = Some(LineSpacing::Exactly(14.0));
    p.props.space_before = Some(0.0);
    p.props.space_after = after;
    p.insert_object(0, InlineObject::note_separator(cont), &Default::default()).unwrap();
    vec![para_block(p)]
}

fn fixture(long: bool, after: Option<f32>, continuation_after: Option<f32>) -> (Document, Vec<u32>) {
    let text =
        (1..=45).map(|i| format!("SYNTHETIC BODY {i:02}: {}", "Sample text for paragraph pagination. ".repeat(8))).collect::<Vec<_>>().join("\n");
    let mut doc = pagination_fixture_document(&text);
    doc.styles.default_para.space_after = Some(6.0);
    let positions: Vec<usize> = if long { vec![4] } else { (4..45).step_by(5).collect() };
    let mut ids = Vec::new();
    for block in positions {
        let text = "SYNTHETIC NOTE: sample note text for pagination. ".repeat(if long { 180 } else { 4 });
        let id = footnote(&mut doc, &Pos::body(block, 0), &text);
        let props = &mut doc.para_mut(StoryRef::Part(id), &Path::top(0)).unwrap().props;
        props.space_after = Some(0.0);
        props.line_spacing = Some(LineSpacing::Exactly(14.0));
        ids.push(id);
    }
    doc.footnote_separators.separator = Some(separator(false, after));
    doc.footnote_separators.continuation_separator = Some(separator(true, continuation_after));
    (doc, ids)
}

/// Compare actual line ranges and page coordinates, not source properties which deliberately differ.
fn signature(layout: &DocLayout, body_paragraphs: usize) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let mut styles = 0;
    for (page, p) in layout.pages.iter().enumerate() {
        for item in &p.items {
            match item {
                Placed::Lines { story, path, l0, l1, x, y, para, .. } => {
                    if *story == StoryRef::Body {
                        seen.insert(path.0[0]);
                    }
                    out.push(format!("{page}:{story:?}:{path:?}:{l0}..{l1}:{x:.3}:{y:.3}"));
                    for style in &para.styles {
                        styles += 1;
                        assert_eq!(style.face.family, WRAP_TEST_FONT);
                        assert_eq!(style.face.style, "Regular");
                        assert!(style.face.data() == include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf").as_slice());
                    }
                }
                Placed::Rule { x0, y0, x1, y1, .. } => out.push(format!("{page}:rule:{x0:.3}:{y0:.3}:{x1:.3}:{y1:.3}")),
                _ => {}
            }
        }
    }
    assert_eq!(seen.len(), body_paragraphs, "every body paragraph must survive");
    assert!(styles > 0, "font checks inspect real lines");
    out
}

/// Word Mac 16.114.3 produces identical positions for separator after-space 0, 6 and 12 pt.
#[test]
fn note_separator_trailing_after_does_not_displace_body_or_notes() {
    let (base, ids) = fixture(false, Some(0.0), Some(0.0));
    let baseline = lay(&base);
    assert!(baseline.pages.len() >= 4);
    for id in ids {
        assert!(!note_pieces(&baseline, id).is_empty());
    }
    let expected = signature(&baseline, base.body.len());
    for after in [None, Some(6.0), Some(12.0), Some(48.0)] {
        let (doc, _) = fixture(false, after, Some(0.0));
        assert_eq!(signature(&lay(&doc), doc.body.len()), expected, "separator after={after:?} must not consume body room");
    }
}

/// Continuation separators use the same container rule; the long note must actually continue.
#[test]
fn continuation_separator_trailing_after_keeps_note_pieces_in_place() {
    let (base, ids) = fixture(true, Some(0.0), Some(0.0));
    let baseline = lay(&base);
    assert!(note_pieces(&baseline, ids[0]).len() >= 2);
    assert!(baseline.pages.iter().any(|p| p.items.iter().any(|i| matches!(i, Placed::Rule { x0, x1, .. } if (*x1 - *x0 - 468.0).abs() < 0.01))));
    let expected = signature(&baseline, base.body.len());
    for after in [None, Some(6.0), Some(12.0)] {
        let (doc, _) = fixture(true, Some(0.0), after);
        assert_eq!(signature(&lay(&doc), doc.body.len()), expected, "continuation after={after:?}");
    }
}

/// The internal gap remains between two paragraphs of a custom separator.
#[test]
fn note_separator_internal_paragraph_spacing_is_retained() {
    for after in [0.0, 12.0] {
        let (mut doc, _) = fixture(false, Some(0.0), Some(0.0));
        let mut first = Paragraph::with_text("SYNTHETIC SEPARATOR TEXT", Default::default());
        first.props =
            ParaProps { line_spacing: Some(LineSpacing::Exactly(14.0)), space_before: Some(0.0), space_after: Some(after), ..Default::default() };
        doc.footnote_separators.separator.as_mut().unwrap().insert(0, para_block(first));
        let layout = lay(&doc);
        signature(&layout, doc.body.len());
        let y: Vec<f32> = layout.pages[0]
            .items
            .iter()
            .filter_map(|i| match i {
                Placed::Lines { story, y, .. } if *story == DECOR_STORY => Some(*y),
                _ => None,
            })
            .collect();
        assert_eq!(y.len(), 2);
        assert!((y[1] - y[0] - 14.0 - after).abs() < 0.01, "internal after-space stays in the separator: {y:?}");
    }
}

/// A continuation notice remains an ordinary box, including its own trailing spacing.
#[test]
fn continuation_notice_retains_its_after_space() {
    let make = |after| {
        let (mut doc, _) = fixture(true, Some(0.0), Some(0.0));
        let mut p = Paragraph::with_text("SYNTHETIC CONTINUATION NOTICE", Default::default());
        p.props = ParaProps { line_spacing: Some(LineSpacing::Exactly(14.0)), space_after: Some(after), ..Default::default() };
        doc.footnote_separators.continuation_notice = Some(vec![para_block(p)]);
        doc
    };
    let a = make(0.0);
    let b = make(12.0);
    assert_ne!(signature(&lay(&a), a.body.len()), signature(&lay(&b), b.body.len()), "notice spacing is not separator spacing");
}

/// The native observation is about footnotes; endnote separator spacing keeps its prior behavior.
#[test]
fn endnote_separator_after_space_is_unchanged() {
    let make = |after| {
        let mut doc = pagination_fixture_document("SYNTHETIC BODY");
        let id = doc.add_part(wordcraft_doc::PartKind::Endnote, vec![para_block(Paragraph::with_text("SYNTHETIC ENDNOTE", Default::default()))]);
        doc.insert_object(
            &Pos::body(0, 0),
            InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Endnote, id, custom: String::new() },
            &Default::default(),
        )
        .unwrap();
        doc.endnote_separators.separator = Some(separator(false, Some(after)));
        (doc, id)
    };
    let (a, id) = make(0.0);
    let (b, _) = make(12.0);
    let at = |doc: &Document| lay(doc).caret(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }).unwrap().top;
    assert!((at(&b) - at(&a) - 12.0).abs() < 0.01, "endnote separator after-space still shifts its note");
}

/// A paragraph before a trailing floating table keeps its internal after-space.
#[test]
fn separator_keeps_after_space_before_a_trailing_floating_table() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::{HeightRule, TableFloat};
    let make = |after| {
        let mut doc = pagination_fixture_document("SYNTHETIC BODY");
        let id = footnote(&mut doc, &Pos::body(0, 0), "SYNTHETIC NOTE");
        let note = doc.para_mut(StoryRef::Part(id), &Path::top(0)).unwrap();
        note.props.line_spacing = Some(LineSpacing::Exactly(14.0));
        note.props.space_after = Some(0.0);
        let mut blocks = separator(false, Some(after));
        let mut table = wordcraft_doc::Table::new(1, 1, 40.0);
        table.props.style = None;
        table.props.fixed = true;
        table.props.cell_margins = Some([0.0; 4]);
        table.props.float = Some(TableFloat { h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 200.0, y: -10.0, ..Default::default() });
        table.rows[0].props.height = Some(8.0);
        table.rows[0].props.height_rule = HeightRule::Exact;
        let mut p = Paragraph::new();
        p.props = ParaProps { line_spacing: Some(LineSpacing::Exactly(6.0)), space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
        table.rows[0].cells[0].blocks = vec![para_block(p)];
        blocks.push(Arc::new(Block::Table(table)));
        doc.footnote_separators.separator = Some(blocks);
        (doc, id)
    };
    let positions = |after| {
        let (doc, id) = make(after);
        let layout = lay(&doc);
        signature(&layout, doc.body.len());
        let rule = layout.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                Placed::Rule { x0, x1, y0, .. } if (x1 - x0 - 144.0).abs() < 0.01 => Some(*y0),
                _ => None,
            })
            .unwrap();
        let note = layout.caret(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }).unwrap().top;
        (rule, note)
    };
    let a = positions(0.0);
    let b = positions(12.0);
    assert!((a.0 - b.0 - 12.0).abs() < 0.01, "the gap before the floating table stays reserved: {a:?}, {b:?}");
    assert!((a.1 - b.1).abs() < 0.01, "the note stays clear of the complete separator area");
}
