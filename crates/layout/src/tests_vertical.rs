//! Vertical sections (`w:textDirection` in `w:sectPr`): lines run down the page and stack from
//! the right (`tbRl`), or run up and stack from the left (`btLr`).

use super::*;
use wordcraft_doc::Pos;

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn vertical_doc(dir: TextDirection) -> Document {
    let text = "Vertical sections lay their lines out down the page, one after another from right to left. ".repeat(4);
    let mut d = Document::from_text(&format!("{text}\n日本語のテキスト"));
    d.last_section.text_direction = dir;
    d
}

/// Body lines on page 0: (x, y, turn) of each piece.
fn body_lines(l: &DocLayout) -> Vec<(f32, f32, TextDirection)> {
    l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, x, y, turn, .. } => Some((*x, *y, *turn)),
            _ => None,
        })
        .collect()
}

#[test]
fn top_to_bottom_section_stacks_lines_from_the_right() {
    let d = vertical_doc(TextDirection::Down);
    let l = lay(&d);
    let page = &l.pages[0];
    // The page keeps its size; the text area is the real one (inside the margins).
    assert_eq!((page.w, page.h), (d.last_section.page_w, d.last_section.page_h));
    assert!(page.body.x >= d.last_section.margin_left - 0.5 && page.body.right() <= page.w - d.last_section.margin_right + 0.5);
    let lines = body_lines(&l);
    assert!(lines.len() >= 2);
    assert!(lines.iter().all(|(.., t)| *t == TextDirection::Down));
    // The first paragraph's first line is the rightmost, the next paragraph left of it.
    let p0 = Pos::body(0, 0);
    let c0 = l.caret(&p0).unwrap();
    let c1 = l.caret(&Pos::body(1, 0)).unwrap();
    assert!(c1.x < c0.x, "second paragraph left of the first: {c1:?} vs {c0:?}");
    assert!(c0.x <= page.body.right() + 0.5 && c0.top >= page.body.y - 0.5);
    // Characters progress downward along a line; the caret lies across it.
    let c5 = l.caret(&Pos::body(0, 2)).unwrap();
    assert!(c5.top > c0.top && (c5.x - c0.x).abs() < 0.5, "{c5:?} vs {c0:?}");
    assert!(c0.width > c0.height);
    // Line two of the first paragraph is left of line one.
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    let wrap = l.index.get(&(StoryRef::Body, Path::top(0))).map(|v| v[0]).unwrap();
    let Placed::Lines { para, .. } = &l.pages[wrap.0].items[wrap.1] else { panic!() };
    assert!(para.lines.len() >= 2 && para.lines[1].start < p.len());
    let c_next = l.caret(&Pos::body(0, para.lines[1].start + 1)).unwrap();
    assert!(c_next.x < c0.x - 1.0);
    // Clicking a caret position finds it again.
    for off in [0, 2, para.lines[1].start + 1] {
        let c = l.caret(&Pos::body(0, off)).unwrap();
        let hit = l.hit(c.page, c.x + c.width / 2.0, c.top + 0.2, StoryRef::Body).unwrap();
        assert_eq!(hit.off, off);
    }
    // Clicking in the empty space below a short line lands on that line (its end).
    let c_cjk = l.caret(&Pos::body(1, 0)).unwrap();
    let below = l.hit(0, c_cjk.x + c_cjk.width / 2.0, page.body.bottom() - 2.0, StoryRef::Body).unwrap();
    assert_eq!(below.path, Path::top(1));
}

#[test]
fn bottom_to_top_section_stacks_lines_from_the_left() {
    let l = lay(&vertical_doc(TextDirection::Up));
    assert!(body_lines(&l).iter().all(|(.., t)| *t == TextDirection::Up));
    let c0 = l.caret(&Pos::body(0, 0)).unwrap();
    let c1 = l.caret(&Pos::body(1, 0)).unwrap();
    let c5 = l.caret(&Pos::body(0, 2)).unwrap();
    assert!(c1.x > c0.x, "second paragraph right of the first");
    assert!(c5.top < c0.top, "characters progress upward");
}

#[test]
fn east_asian_characters_stand_upright_in_top_to_bottom_text() {
    let l = lay(&vertical_doc(TextDirection::Down));
    let d = vertical_doc(TextDirection::Down);
    let draws = display::page_display(&d, &l.pages[0], &Default::default());
    let turned: Vec<&Vec<display::Draw>> =
        draws.iter().filter_map(|x| if let display::Draw::Turned { items, .. } = x { Some(items) } else { None }).collect();
    let text_of = |ds: &[display::Draw]| -> String {
        ds.iter()
            .map(|x| match x {
                display::Draw::Glyphs { text, .. } => text.clone(),
                display::Draw::Rotated { items, .. } => {
                    items.iter().map(|i| if let display::Draw::Glyphs { text, .. } = i { text.clone() } else { String::new() }).collect()
                }
                _ => String::new(),
            })
            .collect()
    };
    // Each ideograph turned back upright on its own; the Latin text lies with the line.
    let cjk = turned.iter().find(|ds| text_of(ds).contains('日')).expect("the Japanese line is drawn turned");
    let upright = cjk.iter().filter(|x| matches!(x, display::Draw::Rotated { spin, .. } if spin.deg == 270.0)).count();
    assert_eq!(upright, "日本語のテキスト".chars().count());
    assert!(turned.iter().any(|ds| text_of(ds).contains("Vertical") && ds.iter().all(|x| !matches!(x, display::Draw::Rotated { .. }))));
}

#[test]
fn headers_stay_horizontal_and_a_vertical_section_starts_a_page() {
    use wordcraft_doc::section::SectionStart;
    let mut d = Document::from_text("Horizontal\nVertical");
    // Section 1 ends after paragraph 0 (continuous), section 2 is vertical.
    let first = SectionProps { start: SectionStart::Continuous, ..Default::default() };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));
    d.last_section.start = SectionStart::Continuous;
    d.last_section.text_direction = TextDirection::Down;
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    assert_eq!(l.caret(&Pos::body(0, 0)).unwrap().width, 0.0);
    assert!(l.caret(&Pos::body(1, 0)).unwrap().width > 0.0);
}
