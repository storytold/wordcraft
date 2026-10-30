//! Document grid (`w:docGrid`): line pitch and character pitch (#391). Assertions are relations
//! to the pitch, never font metrics (fonts differ between machines; CJK glyphs may be missing).

#[allow(unused_imports)]
use crate::*;
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

/// A grid whose pitch is a little under the default size, as a 28-characters-a-line Chinese
/// official document has (16 pt text on a 15.79 pt pitch): (document, default size, pitch).
fn tight_grid(text: &str) -> (Document, f32, f32) {
    let lines = wordcraft_doc::SectionProps { doc_grid: Some(DocGrid { kind: DocGridType::Lines, ..Default::default() }), ..Default::default() };
    let size = para::Grid::of(&Document::from_text(""), &lines).font_size;
    let d = with_grid(text, DocGridType::LinesAndChars, DocGrid::char_space_for(size - 0.2, size));
    let pitch = d.last_section.grid_char_pitch(size).unwrap();
    (d, size, pitch)
}

fn first_para(l: &DocLayout) -> std::sync::Arc<para::ParaLayout> {
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap()
}

#[test]
fn larger_east_asian_text_takes_whole_cells_and_never_overlaps() {
    // A title at 1.375 times the default size (22 pt over a 16 pt grid): two cells a character,
    // not one cell narrower than the glyph.
    let (mut d, size, pitch) = tight_grid("关于通知");
    if let Some(Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
        p.runs[0].props.size = Some(size * 1.375);
    }
    let g = first_para(&lay(&d));
    for c in &g.clusters {
        assert!((c.adv - 2.0 * pitch).abs() < 0.01, "advance {} for pitch {pitch}", c.adv);
        assert!(c.adv >= size * 1.375, "a cell narrower than its glyph");
    }
    // Text at the default size keeps one cell, even on a pitch a little under that size.
    let (d, _, pitch) = tight_grid("关于通知");
    assert!(first_para(&lay(&d)).clusters.iter().all(|c| (c.adv - pitch).abs() < 0.01));
}

#[test]
fn character_indents_count_grid_cells() {
    // `w:firstLineChars="200"` on a character grid is two cells, not two font sizes: on a pitch
    // under the font size the first line must not lose a character to the indent.
    let text = "中".repeat(400);
    let (mut d, _, pitch) = tight_grid(&text);
    if let Some(Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
        p.props.indent_first_chars = Some(2.0);
    }
    let g = first_para(&lay(&d));
    let (first, second) = (&g.lines[0], &g.lines[1]);
    assert!(((first.xs[0] - second.xs[0]) - 2.0 * pitch).abs() < 0.01, "indent {} for pitch {pitch}", first.xs[0] - second.xs[0]);
    let count = |l: &para::Line| l.c1 - l.c0;
    assert_eq!(count(first) + 2, count(second), "first line {} cells, full line {}", count(first), count(second));
    // Off the grid (`snapToGrid` off) the font size stays the unit.
    let mut off = d.clone();
    if let Some(Block::Para(p)) = off.body.first_mut().map(std::sync::Arc::make_mut) {
        p.props.snap_to_grid = Some(false);
    }
    let g = first_para(&lay(&off));
    assert!(((g.lines[0].xs[0] - g.lines[1].xs[0]) - 2.0 * pitch).abs() > 0.1);
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
