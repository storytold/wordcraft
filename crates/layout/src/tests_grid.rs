//! Document grid (`w:docGrid`): line pitch and character pitch (#391). Assertions are relations
//! to the pitch, never font metrics (fonts differ between machines; CJK glyphs may be missing).

use super::*;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::section::{DocGrid, DocGridType};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn with_grid(text: &str, kind: DocGridType, char_space: i32) -> Document {
    let mut d = Document::from_text(text);
    for b in d.body.iter_mut() {
        if let Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
            p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        }
    }
    d.last_section.doc_grid = Some(DocGrid { kind, line_pitch: 18.0, char_space });
    d
}

/// (first-page lines, every line height) of a layout.
fn lines(l: &DocLayout) -> (usize, Vec<f32>) {
    let mut first = 0;
    let mut hs = Vec::new();
    for (pi, p) in l.pages.iter().enumerate() {
        for it in &p.items {
            if let Placed::Lines { para, l0, l1, .. } = it {
                if pi == 0 {
                    first += l1 - l0;
                }
                hs.extend(para.lines[*l0..*l1].iter().map(|line| line.height));
            }
        }
    }
    (first, hs)
}

#[test]
fn line_grid_snaps_line_heights_and_fixes_lines_per_page() {
    let text = vec!["Grid line"; 80].join("\n");
    let d = with_grid(&text, DocGridType::Lines, 0);
    let (first, hs) = lines(&lay(&d));
    for h in &hs {
        let cells = h / 18.0;
        assert!((cells - cells.round()).abs() < 1e-3 && cells >= 1.0, "line height {h} is not a multiple of the pitch");
    }
    // Text at the default size fits one pitch: 648 pt of text height holds 36 lines.
    let h = hs[0];
    assert_eq!(first, (648.0 / h + 1e-3).floor() as usize, "heights {h}");
    // Turning the paragraphs' snapToGrid off leaves their natural line heights.
    let mut off = d.clone();
    for b in off.body.iter_mut() {
        if let Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.snap_to_grid = Some(false);
        }
    }
    let plain = with_grid(&text, DocGridType::Default, 0);
    assert_eq!(lines(&lay(&off)).1[0], lines(&lay(&plain)).1[0]);
}

/// Spacing in lines (`w:beforeLines`/`w:afterLines`): a line is 12 pt, whatever the font size,
/// unless the section has a line grid, whose pitch it then is (measured in Word: 100 hundredths
/// before with no grid or a `linePitch` but no grid type is 12 pt; with `type="lines"` and
/// `linePitch="360"` it is 18 pt, as with `linesAndChars` or `snapToChars`, and in a paragraph
/// that doesn't snap to the grid too). Non-zero lines win over the points.
#[test]
fn spacing_in_lines_is_twelve_points_or_the_line_pitch() {
    let gap = |kind: DocGridType, lines: Option<f32>| {
        let mut d = with_grid("A\nB", kind, 0);
        if let Some(Block::Para(p)) = d.body.get_mut(1).map(std::sync::Arc::make_mut) {
            p.props.space_before = Some(5.0);
            p.props.space_before_lines = lines;
        }
        let l = lay(&d);
        let y = |block: usize| {
            l.pages[0]
                .items
                .iter()
                .find_map(|it| match it {
                    Placed::Lines { path, y, .. } if *path == Path::top(block) => Some(*y),
                    _ => None,
                })
                .unwrap()
        };
        y(1) - y(0)
    };
    let kinds = [(DocGridType::Default, 12.0), (DocGridType::Lines, 18.0), (DocGridType::LinesAndChars, 18.0), (DocGridType::SnapToChars, 18.0)];
    for (kind, line) in kinds {
        let points = gap(kind, None);
        let one = gap(kind, Some(1.0)) - points + 5.0;
        assert!((one - line).abs() < 0.01, "{kind:?}: one line of space before is {one}, not {line}");
        let half = gap(kind, Some(0.5)) - points + 5.0;
        assert!((half - line / 2.0).abs() < 0.01, "{kind:?}: half a line is {half}");
        assert!((gap(kind, Some(0.0)) - points).abs() < 0.01, "{kind:?}: zero lines leave the points");
    }
}

#[test]
fn character_grid_places_east_asian_text_on_the_pitch() {
    let text = "日本語の文書abc";
    // The default font size plus 2.5 pt: a pitch unlike any font's advance.
    let lines = wordcraft_doc::SectionProps { doc_grid: Some(DocGrid { kind: DocGridType::Lines, ..Default::default() }), ..Default::default() };
    let size = para::Grid::of(&Document::from_text(""), &lines).font_size;
    let cs = DocGrid::char_space_for(size + 2.5, size);
    let d = with_grid(text, DocGridType::LinesAndChars, cs);
    let pitch = d.last_section.grid_char_pitch(size).unwrap();
    assert!((pitch - (size + 2.5)).abs() < 0.01);
    let l = lay(&d);
    let plain = lay(&with_grid(text, DocGridType::Lines, cs));
    let first =
        |l: &DocLayout| l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap();
    let (g, p) = (first(&l), first(&plain));
    let line = &g.lines[0];
    let latin = text.find('a').unwrap();
    for (k, c) in g.clusters.iter().enumerate() {
        let x = line.xs[k] - line.xs[0];
        if c.start < latin {
            // Ideographs and kana sit on the pitch, one cell each.
            assert!((x - pitch * k as f32).abs() < 0.01, "cluster {k} at {x}, pitch {pitch}");
            assert!((c.adv - pitch).abs() < 0.01);
        } else {
            // Latin text keeps its own advances.
            assert!((c.adv - p.clusters[k].adv).abs() < 0.01, "latin cluster {k}");
        }
    }
}
