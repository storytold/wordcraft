use super::*;
use wordcraft_doc::props::{CharProps, LineSpacing};

fn resolved_spacing(mark_size: f32, lines: Option<f32>, points: f32, automatic: bool, line_spacing: LineSpacing) -> f32 {
    let mut d = Document::from_text("Probe");
    d.styles.default_chr.size = Some(12.0);
    if let Some(normal) = d.styles.get_mut("Normal") {
        normal.chr.size = Some(12.0);
    }
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.mark = CharProps { font: Some("Source Sans 3".into()), size: Some(mark_size), ..Default::default() };
    p.props.space_after = Some(points);
    p.props.space_after_lines = lines;
    p.props.space_after_auto = Some(automatic);
    p.props.line_spacing = Some(line_spacing);
    let l = lay(&d);
    l.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { para, .. } => Some(para.rp.space_after),
            _ => None,
        })
        .unwrap()
}

#[test]
fn paragraph_line_units_ignore_direct_mark_and_exact_line_height() {
    // Word for Mac 16.113.4: with no document grid, direct mark sizes 9/12/24/36 pt
    // all give 6 pt for 50 units, even with exact line spacing.
    assert_eq!(resolved_spacing(12.0, Some(0.5), 10.0, false, LineSpacing::Multiple(1.0)), 6.0);
    for mark in [9.0, 24.0, 36.0] {
        assert_eq!(resolved_spacing(mark, Some(0.5), 10.0, false, LineSpacing::Multiple(1.0)), 6.0);
    }
    assert_eq!(resolved_spacing(12.0, Some(0.5), 10.0, false, LineSpacing::Exactly(24.0)), 6.0);
    assert_eq!(resolved_spacing(12.0, Some(1.0), 10.0, false, LineSpacing::Multiple(1.0)), 12.0);
    assert_eq!(resolved_spacing(12.0, Some(0.0), 10.0, false, LineSpacing::Multiple(1.0)), 10.0);
    assert_eq!(resolved_spacing(12.0, Some(0.5), 10.0, true, LineSpacing::Multiple(1.0)), 14.0);
}

#[test]
fn paragraph_spacing_uses_grid_pitch_when_snapping_is_disabled() {
    let mut d = Document::from_text("Probe");
    d.last_section.doc_grid =
        Some(wordcraft_doc::section::DocGrid { kind: wordcraft_doc::section::DocGridType::Lines, line_pitch: 20.0, char_space: 0 });
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.props.snap_to_grid = Some(false);
    p.props.space_after = Some(0.0);
    p.props.space_after_lines = Some(0.5);
    let l = lay(&d);
    let after = l.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { para, .. } => Some(para.rp.space_after),
            _ => None,
        })
        .unwrap();
    assert_eq!(after, 10.0);
    // A retained pitch on a disabled grid does not change the 12 pt fallback.
    d.last_section.doc_grid =
        Some(wordcraft_doc::section::DocGrid { kind: wordcraft_doc::section::DocGridType::Default, line_pitch: 20.0, char_space: 0 });
    let l = lay(&d);
    let after = l.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { para, .. } => Some(para.rp.space_after),
            _ => None,
        })
        .unwrap();
    assert_eq!(after, 6.0);
}
