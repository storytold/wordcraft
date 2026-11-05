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

/// Multiple spacing on an 18 pt grid, against Word for Mac's line heights (grid probe, Roboto
/// Regular): 11 and 14 pt text is m pitches tall; 18 pt text, whose natural height (about 21 pt)
/// is over one pitch, is 36 pt up to 2× and 54 pt at 3×. Natural heights are passed in directly,
/// so the assertions hold whatever font the machine has.
#[test]
fn multiple_spacing_follows_word_on_an_18_point_grid() {
    let g = para::Grid { line: Some(18.0), ..Default::default() };
    for natural in [13.0, 16.5] {
        for (m, want) in [(1.0, 18.0), (1.5, 27.0), (2.0, 36.0), (3.0, 54.0)] {
            assert_eq!(g.multiple_height(natural, m), want, "natural {natural} at {m}x");
        }
    }
    for (m, want) in [(1.0, 36.0), (1.5, 36.0), (2.0, 36.0), (3.0, 54.0)] {
        assert_eq!(g.multiple_height(21.1, m), want, "18 pt text at {m}x");
    }
    // Below single spacing the multiple still scales the snapped height.
    assert_eq!(g.multiple_height(13.0, 0.5), 9.0);
    // Without a line grid the natural height is simply multiplied.
    assert_eq!(para::Grid::default().multiple_height(21.0, 1.5), 31.5);
}

/// A top-and-bottom shape whose top lies just under the first grid line of 18 pt text at 3×
/// (54 pt tall, measured). The float check estimates the line before measuring it: with the
/// estimate at the plain `natural × 3` (over 63 pt) the line would be taken to meet the shape and
/// pushed below it, although it ends above the shape's top at 55 pt.
#[test]
fn grid_multiple_spacing_estimates_the_line_the_float_check_meets() {
    use wordcraft_doc::para::{Anchor, Float, InlineObject, ShapeKind, Wrap};
    use wordcraft_doc::{CharProps, Paragraph, Pos};
    let mut d = Document::new();
    d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 18.0, char_space: 0 });
    let ch = CharProps { font: Some("Source Sans 3".into()), size: Some(18.0), ..Default::default() };
    d.body = (1..=3)
        .map(|n| {
            let mut p = Paragraph::with_text(&format!("P{n}"), ch.clone());
            p.mark = ch.clone();
            p.props = wordcraft_doc::props::ParaProps {
                space_before: Some(0.0),
                space_after: Some(0.0),
                line_spacing: Some(LineSpacing::Multiple(3.0)),
                widow_control: Some(false),
                ..Default::default()
            };
            wordcraft_doc::para_block(p)
        })
        .collect();
    let shape = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 468.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 72.0 + 55.0, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let l = lay(&d);
    let first = l.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { path, y, para, story: StoryRef::Body, .. } if path.0.len() == 1 => Some((*y, para.lines[0].height)),
            _ => None,
        })
        .expect("first paragraph's lines");
    assert!((first.1 - 54.0).abs() < 0.01, "a 3x line on an 18 pt grid is 3 pitches: {first:?}");
    assert!((first.0 - 72.0).abs() < 0.01, "the line ends above the shape at 127 pt, so it stays at the top: {first:?}");
}
