//! Vertical sections (`w:textDirection` in `w:sectPr`): lines run down the page and stack from
//! the right (`tbRl`), or run up and stack from the left (`btLr`).

use crate::*;
use wordcraft_doc::Pos;
use wordcraft_doc::props::{CharProps, EastAsianLayout, Underline};

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

/// Body paragraph `i`'s laid-out lines, on the page they are drawn on.
fn para_of(l: &DocLayout, i: usize) -> std::sync::Arc<ParaLayout> {
    for page in &l.pages {
        for it in &page.items {
            if let Placed::Lines { para, path, .. } = it
                && path.0 == [i as u32]
            {
                return para.clone();
            }
        }
    }
    panic!("paragraph {i} is laid out");
}

/// The upright glyphs drawn inside a page's turned lines: their text, page x and y, and spin.
fn rotated_marks(draws: &[display::Draw]) -> Vec<(String, f32, f32, f32)> {
    let mut out = Vec::new();
    for d in draws {
        let display::Draw::Turned { x, y, turn, items } = d else { continue };
        for it in items {
            let display::Draw::Rotated { cx, cy, spin, items } = it else { continue };
            let t: String =
                items.iter().filter_map(|i| if let display::Draw::Glyphs { text, .. } = i { Some(text.as_str()) } else { None }).collect();
            let (px, py) = crate::turn_point(*turn, *x, *y, *cx, *cy);
            out.push((t, px, py, spin.deg));
        }
    }
    out
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

/// 縦中横 (`w:eastAsianLayout/@w:vert`): a run in top-to-bottom text is drawn as the horizontal
/// text it is — its characters stand upright side by side across the line, inside the length the
/// run takes along it (a pair of half-width digits fits one character's height). Without `vert`
/// the digits lie with the line as before; horizontal text ignores `vert`, and the layout (line
/// breaking, cluster advances, carets) is the same either way.
#[test]
fn tate_chu_yoko_stands_upright_across_a_top_to_bottom_line() {
    let text = "日本語12テスト";
    let start = text.find('1').expect("a digit");
    let digits = start..start + 2;
    let mark = |d: &mut Document, on: bool| {
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        let digit_range = digits.clone();
        p.format(digit_range.start, digit_range.end, &|c: &mut CharProps| {
            c.east_asian_layout = if on { Some(EastAsianLayout { vert: Some(true), ..Default::default() }) } else { None };
        })
        .unwrap();
    };
    let mut d = Document::from_text(text);
    d.last_section.text_direction = TextDirection::Down;
    mark(&mut d, true);
    let l = lay(&d);
    let pl = para_of(&l, 0);
    // The layout is unchanged: the digits keep their shaped advances along the line.
    let mut off_doc = d.clone();
    mark(&mut off_doc, false);
    let off_pl = para_of(&lay(&off_doc), 0);
    assert_eq!(
        pl.clusters.iter().map(|c| (c.start, c.adv)).collect::<Vec<_>>(),
        off_pl.clusters.iter().map(|c| (c.start, c.adv)).collect::<Vec<_>>(),
        "vert does not change the layout"
    );
    let line = pl.lines.first().expect("one line");
    let k1 = pl.clusters.iter().position(|c| c.start == start).expect("the first digit's cluster");
    let k2 = k1 + 1;
    let len = line.cl_left(k2 + 1).unwrap_or(0.0) - line.cl_left(k1).unwrap_or(0.0);
    let size = pl.styles.get(pl.clusters[k1].style as usize).map(|s| s.size).unwrap_or(12.0);
    assert!(len > 0.0 && len < size * 1.6, "the pair takes about one character along the line: {len} pt at {size} pt");

    // The display turns the two digits upright, side by side across the line: one page height,
    // the first left of the second by half the pair's length.
    let draws = display::page_display(&d, &l.pages[0], &Default::default());
    let marks = rotated_marks(&draws);
    let one = marks.iter().find(|m| m.0 == "1").expect("the first digit drawn on its own");
    let two = marks.iter().find(|m| m.0 == "2").expect("the second digit drawn on its own");
    assert_eq!((one.3, two.3), (270.0, 270.0), "upright like the ideographs: {one:?} {two:?}");
    assert!((one.2 - two.2).abs() < 0.01, "one page height for both: {one:?} {two:?}");
    assert!(one.1 < two.1, "the first digit is left of the second: {one:?} {two:?}");
    assert!((two.1 - one.1 - len / 2.0).abs() < 0.05, "half the pair's length apart: {} against {}", two.1 - one.1, len / 2.0);
    // The text is still read and extracted in order.
    let read: String = draws
        .iter()
        .map(|d| match d {
            display::Draw::Turned { items, .. } => items
                .iter()
                .map(|i| match i {
                    display::Draw::Glyphs { text, .. } => text.clone(),
                    display::Draw::Rotated { items, .. } => {
                        items.iter().filter_map(|g| if let display::Draw::Glyphs { text, .. } = g { Some(text.clone()) } else { None }).collect()
                    }
                    _ => String::new(),
                })
                .collect::<String>(),
            _ => String::new(),
        })
        .collect();
    assert!(read.contains(text), "the line reads {text}: {read}");
    // Carets at the digit offsets are found again by a click.
    for off in [start, start + 1, start + 2] {
        let c = l.caret(&Pos::body(0, off)).unwrap();
        let hit = l.hit(c.page, c.x + c.width / 2.0, c.top + 0.2, StoryRef::Body).unwrap();
        assert_eq!(hit.off, off, "caret at {off} at {c:?}");
    }

    // Without vert the digits keep lying with the line: no upright glyphs of their own.
    let off_l = lay(&off_doc);
    let off_marks = rotated_marks(&display::page_display(&off_doc, &off_l.pages[0], &Default::default()));
    assert!(off_marks.iter().all(|m| m.0 != "1" && m.0 != "2"), "without vert the digits lie with the line: {off_marks:?}");
    let off_lying = display::page_display(&off_doc, &off_l.pages[0], &Default::default()).iter().any(|d| match d {
        display::Draw::Turned { items, .. } => items.iter().any(|i| matches!(i, display::Draw::Glyphs { text, .. } if text.contains("12"))),
        _ => false,
    });
    assert!(off_lying, "the lying digits are one text run");

    // Horizontal text ignores vert: no turns, no upright glyphs of its own, and the run reads as
    // one text either way.
    let mut h = d.clone();
    h.last_section.text_direction = TextDirection::Horizontal;
    let hl = lay(&h);
    let hdraws = display::page_display(&h, &hl.pages[0], &Default::default());
    assert!(hdraws.iter().all(|d| !matches!(d, display::Draw::Turned { .. })), "horizontal text is not turned");
    assert!(hdraws.iter().all(|d| !matches!(d, display::Draw::Rotated { .. })), "horizontal text draws no upright glyphs of its own");
}

/// A `vert` run keeps its decorations and tracked-change marks: its underline, strikethrough and
/// double strike, a deleted run's strike and an inserted run's underline each still emit a
/// [`display::Draw::Line`]. Drawn along the run's line span (Word draws them under each upright
/// glyph); skipping the block left a deleted or inserted `vert` run looking like ordinary text.
#[test]
fn tate_chu_yoko_keeps_its_decorations_and_revision_marks() {
    let text = "12";
    // The whole run, marked `vert`, carrying `f`'s formatting.
    let vert_doc = |f: &dyn Fn(&mut CharProps)| -> Document {
        let mut d = Document::from_text(text);
        d.last_section.text_direction = TextDirection::Down;
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.format(0, text.len(), &|c: &mut CharProps| {
            c.east_asian_layout = Some(EastAsianLayout { vert: Some(true), ..Default::default() });
            f(c);
        })
        .unwrap();
        d
    };
    // The lines drawn inside the turned line's frame, the run's decorations: x0, y0, x1, y1, stroke.
    let lines = |d: &Document| -> Vec<(f32, f32, f32, f32, display::Stroke)> {
        let l = lay(d);
        display::page_display(d, &l.pages[0], &Default::default())
            .into_iter()
            .flat_map(|x| match x {
                display::Draw::Turned { items, .. } => items,
                _ => Vec::new(),
            })
            .filter_map(|i| if let display::Draw::Line { x0, y0, x1, y1, stroke, .. } = i { Some((x0, y0, x1, y1, stroke)) } else { None })
            .collect()
    };
    let underline = lines(&vert_doc(&|c: &mut CharProps| c.underline = Some(Underline::Single)));
    assert_eq!(underline.len(), 1, "an underlined vert run draws its line: {underline:?}");
    let (x0, uy, x1, uy1, stroke) = underline[0];
    assert!(x1 > x0 && uy1 == uy, "the underline runs along the line: {underline:?}");
    assert_eq!(stroke, display::Stroke::Solid, "{underline:?}");
    let strike = lines(&vert_doc(&|c: &mut CharProps| c.strike = Some(true)));
    assert_eq!(strike.len(), 1, "a struck vert run draws its line: {strike:?}");
    assert!(strike[0].1 < uy, "the strike sits above the underline across the line: {strike:?} {underline:?}");
    let double = lines(&vert_doc(&|c: &mut CharProps| c.double_strike = Some(true)));
    assert_eq!(double.len(), 1, "a double-struck vert run draws its line: {double:?}");
    assert_eq!(double[0].4, display::Stroke::Double, "{double:?}");
    // A tracked deletion is struck through and an insertion underlined (the default marks), where
    // the run's own decoration goes.
    let mut deleted = vert_doc(&|c: &mut CharProps| c.del = Some(0));
    deleted.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "Ana", ""));
    let deletion = lines(&deleted);
    assert_eq!(deletion.len(), 1, "a deleted vert run is struck through: {deletion:?}");
    assert!((deletion[0].1 - strike[0].1).abs() < 0.01, "the deletion mark sits where the strike does: {deletion:?} {strike:?}");
    let mut inserted = vert_doc(&|c: &mut CharProps| c.ins = Some(0));
    inserted.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Insert, "Ana", ""));
    let insertion = lines(&inserted);
    assert_eq!(insertion.len(), 1, "an inserted vert run is underlined: {insertion:?}");
    assert!((insertion[0].1 - uy).abs() < 0.01, "the insertion mark sits where the underline does: {insertion:?} {underline:?}");
}

/// A `vert` run that ends a bidirectional line still takes its own length along the line: the
/// line's visible boxes stop at its last cluster, so the run ends at that cluster's trailing edge
/// (it read as zero, and the digits stood a whole em apart instead of half the run's length).
#[test]
fn tate_chu_yoko_at_the_end_of_a_bidi_line_takes_its_length() {
    let text = "שלום 12";
    let start = text.rfind('1').expect("a digit");
    let mut d = Document::from_text(text);
    d.last_section.text_direction = TextDirection::Down;
    d.para_mut(StoryRef::Body, &Path::top(0))
        .unwrap()
        .format(start, start + 2, &|c: &mut CharProps| {
            c.east_asian_layout = Some(EastAsianLayout { vert: Some(true), ..Default::default() });
        })
        .unwrap();
    let l = lay(&d);
    let pl = para_of(&l, 0);
    let line = pl.lines.first().expect("one line");
    assert!(!line.vis.is_empty(), "the Hebrew text makes the line bidirectional");
    let k1 = pl.clusters.iter().position(|c| c.start == start).expect("the first digit's cluster");
    let k2 = k1 + 1;
    assert_eq!(k2 + 1, line.c1, "the vert run ends the line");
    let len = line.cl_right(k2).expect("the run's trailing edge") - line.cl_left(k1).expect("the run's start");

    // Half the run's length apart across the line, as in a plain line.
    let marks = rotated_marks(&display::page_display(&d, &l.pages[0], &Default::default()));
    let one = marks.iter().find(|m| m.0 == "1").expect("the first digit drawn on its own");
    let two = marks.iter().find(|m| m.0 == "2").expect("the second digit drawn on its own");
    assert!((one.2 - two.2).abs() < 0.01, "one page height for both: {one:?} {two:?}");
    assert!(one.1 < two.1, "the first digit is left of the second: {one:?} {two:?}");
    assert!((two.1 - one.1 - len / 2.0).abs() < 0.05, "half the run's length apart: {} against {}", two.1 - one.1, len / 2.0);
}

/// A `vert` run of one half-width digit takes the glyph's own advance: centring it in a whole em
/// shifted its advance box along the line by the rest of the em (a quarter em for a half-width
/// digit). The cluster's advance also holds the automatic space before the next character.
#[test]
fn tate_chu_yoko_a_lone_digit_keeps_its_advance() {
    let text = "あ1い";
    let start = text.find('1').expect("a digit");
    let mut d = Document::from_text(text);
    d.last_section.text_direction = TextDirection::Down;
    d.para_mut(StoryRef::Body, &Path::top(0))
        .unwrap()
        .format(start, start + 1, &|c: &mut CharProps| {
            c.east_asian_layout = Some(EastAsianLayout { vert: Some(true), ..Default::default() });
        })
        .unwrap();
    let l = lay(&d);
    let pl = para_of(&l, 0);
    let k = pl.clusters.iter().position(|c| c.start == start).expect("the digit's cluster");
    let st = &pl.styles[pl.clusters[k].style as usize];
    let g = &pl.glyphs[pl.clusters[k].g0 as usize];
    let own = st.face.advance(g.gid) as f32 * st.size / st.face.upem.max(1.0) as f32;

    let draws = display::page_display(&d, &l.pages[0], &Default::default());
    let mut pens = Vec::new();
    for d in &draws {
        let display::Draw::Turned { items, .. } = d else { continue };
        for it in items {
            let display::Draw::Rotated { cx, items, .. } = it else { continue };
            for i in items {
                if let display::Draw::Glyphs { text, glyphs, .. } = i
                    && text == "1"
                {
                    pens.extend(glyphs.iter().map(|g| (g.1, *cx)));
                }
            }
        }
    }
    assert_eq!(pens.len(), 1, "the digit drawn once");
    let (pen_x, cx) = pens[0];
    assert!((2.0 * (cx - pen_x) - own).abs() < 0.05, "the digit's advance box centred on its slot: {} against {}", 2.0 * (cx - pen_x), own);
}

/// A `vert` cluster shaped to two glyphs (a digit and its combining mark) stands in one slot, its
/// text drawn once: each glyph took a slot of its own and the cluster was drawn twice.
#[test]
fn tate_chu_yoko_keeps_a_multi_glyph_cluster_in_one_slot() {
    let text = "あ1\u{301}い";
    let start = text.find('1').expect("a digit");
    let end = start + '1'.len_utf8() + '\u{301}'.len_utf8();
    let mut d = Document::from_text(text);
    d.last_section.text_direction = TextDirection::Down;
    d.para_mut(StoryRef::Body, &Path::top(0))
        .unwrap()
        .format(start, end, &|c: &mut CharProps| {
            c.east_asian_layout = Some(EastAsianLayout { vert: Some(true), ..Default::default() });
        })
        .unwrap();
    let l = lay(&d);
    let pl = para_of(&l, 0);
    let k = pl.clusters.iter().position(|c| c.start == start).expect("the digit's cluster");
    let c = &pl.clusters[k];
    assert_eq!(c.g1 - c.g0, 2, "the digit and its mark shape to two glyphs");
    let cluster = "1\u{301}";
    let marks = rotated_marks(&display::page_display(&d, &l.pages[0], &Default::default()));
    let drawn = marks.iter().filter(|m| m.0 == cluster).count();
    assert_eq!(drawn, 1, "the cluster is drawn in one slot, its text once: {marks:?}");
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

/// #2038 in a vertical section, where every line is turned: a click outside a floating table
/// goes to the text beside or after it, not to a cell of the table nearer the click.
#[test]
fn a_click_outside_a_floating_table_in_a_vertical_section_skips_its_cells() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::TableFloat;
    let mut d = Document::from_text("Text");
    d.last_section.text_direction = TextDirection::Down;
    let mut t = wordcraft_doc::Table::new(3, 2, 200.0);
    t.props.float = Some(TableFloat {
        h_rel: Anchor::Column,
        v_rel: Anchor::Paragraph,
        x: 150.0,
        dist: [9.0, 0.0, 9.0, 0.0],
        overlap: true,
        ..Default::default()
    });
    d.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();
    let l = lay(&d);
    let page = &l.pages[0];
    assert_eq!(page.float_tables, [Path::top(0)]);
    let area =
        page.items.iter().filter_map(|it| if let Placed::Cell { rect, .. } = it { Some(*rect) } else { None }).reduce(|a, b| a.union(&b)).unwrap();
    let turned: Vec<(&Path, Rect)> = page
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, .. } => it.turned_bounds().map(|r| (path, r)),
            _ => None,
        })
        .collect();
    assert!(!turned.is_empty() && hit::page_lines(page, StoryRef::Body).is_empty(), "only turned lines on the page");
    let band = |v: f32, lo: f32, len: f32| (lo - v).max(v - lo - len).max(0.0);
    let dist = |r: &Rect, x: f32, y: f32| band(x, r.x, r.w).hypot(band(y, r.y, r.h));
    // Points just outside the table, off every line, whose nearest line is one of the table's.
    let (cx, cy) = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    let points: Vec<(f32, f32)> = [(area.x - 12.0, cy), (area.right() + 12.0, cy), (cx, area.y - 12.0), (cx, area.bottom() + 12.0)]
        .into_iter()
        .filter(|&(x, y)| page.body.contains(Point::new(x, y)) && turned.iter().all(|(_, r)| !r.expand(2.0).contains(Point::new(x, y))))
        .filter(|&(x, y)| turned.iter().min_by(|a, b| dist(&a.1, x, y).total_cmp(&dist(&b.1, x, y))).is_some_and(|(p, _)| p.0.len() > 1))
        .collect();
    assert!(!points.is_empty(), "a click there would land in the table: {area:?}");
    for (x, y) in points {
        assert_eq!(l.hit(0, x, y, StoryRef::Body).unwrap().path, Path::top(1), "a click at ({x}, {y})");
    }
    // Inside the table a click still lands in a cell.
    assert!(l.hit(0, cx, cy, StoryRef::Body).unwrap().path.0.len() > 1);
}
