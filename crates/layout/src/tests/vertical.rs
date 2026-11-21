//! Vertical sections (`w:textDirection` in `w:sectPr`): lines run down the page and stack from
//! the right (`tbRl`), or run up and stack from the left (`btLr`).

use crate::*;
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
    crate::test_fonts();
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
    crate::test_fonts();
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
    crate::test_fonts();
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
    crate::test_fonts();
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

#[test]
fn vertical_section_reserves_space_for_inherited_header() {
    use wordcraft_doc::section::SectionStart;
    let mut d = Document::from_text("Sec1\nSec2 vertical");
    let tall: Vec<_> =
        (0..5).map(|i| wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("Header line {i}"), Default::default()))).collect();
    let id = d.add_part(wordcraft_doc::PartKind::Header, tall);

    // Section 1 defines a tall default header
    let first = SectionProps {
        start: SectionStart::NextPage,
        headers: wordcraft_doc::section::HeaderSet { default: Some(id), ..Default::default() },
        ..Default::default()
    };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));

    // Section 2 is vertical, but has NO headers of its own (inherits section 1's tall header)
    d.last_section.start = SectionStart::NextPage;
    d.last_section.text_direction = TextDirection::Down;
    d.last_section.headers = wordcraft_doc::section::HeaderSet::default();

    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    // Page 1 (Section 2) has inherited the header, and its body top must account for it
    assert!(
        l.pages[1].body.y > d.last_section.margin_top + 10.0,
        "body.y {} should exceed margin_top {}",
        l.pages[1].body.y,
        d.last_section.margin_top
    );
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

#[test]
fn vertical_section_inherits_percentage_width_header_table_using_defining_width() {
    use wordcraft_doc::section::SectionStart;
    let mut d = Document::from_text("Section 1 text\nSection 2 vertical text");

    let mut table = wordcraft_doc::Table::new(1, 1, 0.0);
    table.props.width_pct = Some(100.0);
    let p = wordcraft_doc::Paragraph::with_text(
        "A somewhat long line of text inside the percentage header table cell that wraps differently depending on width",
        Default::default(),
    );
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(p)];
    let h_id = d.add_part(wordcraft_doc::PartKind::Header, vec![Block::Table(table).into()]);

    let s1 = SectionProps {
        start: SectionStart::NextPage,
        page_w: 600.0,
        page_h: 800.0,
        margin_left: 50.0,
        margin_right: 50.0,
        margin_top: 50.0,
        headers: wordcraft_doc::section::HeaderSet { default: Some(h_id), ..Default::default() },
        ..Default::default()
    };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(s1));

    d.last_section.start = SectionStart::NextPage;
    d.last_section.page_w = 300.0;
    d.last_section.page_h = 800.0;
    d.last_section.margin_left = 50.0;
    d.last_section.margin_right = 50.0;
    d.last_section.margin_top = 50.0;
    d.last_section.text_direction = TextDirection::Down;
    d.last_section.headers = wordcraft_doc::section::HeaderSet::default();

    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    assert_eq!(l.pages[1].body.y, l.pages[0].body.y, "inheriting turned section uses defining section's header height");
    assert!(l.pages[1].body.y > d.last_section.margin_top, "header table reserves room on vertical page");

    // Control: if the header table were measured at the inheriting section's 200 pt width,
    // the text wraps across more lines and yields a strictly greater body top boundary.
    let mut d_control = d.clone();
    if let Some(s) = d_control.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section.as_mut() {
        s.page_w = 300.0;
    }
    let l_control = lay(&d_control);
    assert!(l.pages[1].body.y < l_control.pages[0].body.y, "defining width (500 pt) header height is shorter than wrapped narrow width (200 pt)");
}

/// #4847: A vertical section with title_page=true and no first header does not reserve space for
/// an inherited tall default header on its first page.
#[test]
fn vertical_section_inheriting_default_header_with_blank_first_header_reserves_no_room_on_first_page() {
    let mut d = Document::from_text("Section 1 text\nSection 2 vertical text");

    let p = wordcraft_doc::Paragraph::with_text(
        "A multi-line tall header paragraph that pushes the body top boundary downward by many points.",
        Default::default(),
    );
    let h_id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(p)]);

    let s1 = SectionProps {
        start: SectionStart::NextPage,
        page_w: 600.0,
        page_h: 800.0,
        margin_left: 50.0,
        margin_right: 50.0,
        margin_top: 50.0,
        headers: wordcraft_doc::section::HeaderSet { default: Some(h_id), ..Default::default() },
        ..Default::default()
    };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(s1));

    d.last_section.start = SectionStart::NextPage;
    d.last_section.page_w = 300.0;
    d.last_section.page_h = 800.0;
    d.last_section.margin_left = 50.0;
    d.last_section.margin_right = 50.0;
    d.last_section.margin_top = 50.0;
    d.last_section.text_direction = TextDirection::Down;
    d.last_section.title_page = true;
    d.last_section.headers = wordcraft_doc::section::HeaderSet::default(); // default: None (inherits h_id), first: None

    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    // Page 1 is the first page of the turned section with title_page=true and no first header:
    assert_eq!(
        l.pages[1].body.y, d.last_section.margin_top,
        "first page with blank first header does not reserve space for inherited default header"
    );
}

/// #4847: Measuring headers for turned sections preserves list numbering so body items do not advance.
#[test]
fn vertical_section_with_numbered_header_does_not_advance_body_numbering() {
    let mut d = Document::from_text("First section\nNumbered item in body");

    let num = std::sync::Arc::make_mut(&mut d.numbering).add_list(wordcraft_doc::ListKind::Numbered);

    // Give the second paragraph list numbering (num: num, level: 0)
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(1).map(std::sync::Arc::make_mut) {
        p.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
    }

    // Numbered header paragraph using the same num, level: 0
    let mut hp = wordcraft_doc::Paragraph::with_text("Header item", Default::default());
    hp.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
    let h_id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);

    let s1 = SectionProps {
        start: SectionStart::NextPage,
        page_w: 600.0,
        page_h: 800.0,
        margin_top: 50.0,
        headers: wordcraft_doc::section::HeaderSet { default: Some(h_id), ..Default::default() },
        ..Default::default()
    };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(s1));

    d.last_section.start = SectionStart::NextPage;
    d.last_section.page_w = 400.0;
    d.last_section.page_h = 800.0;
    d.last_section.margin_top = 50.0;
    d.last_section.text_direction = TextDirection::Down;
    d.last_section.headers = wordcraft_doc::section::HeaderSet::default(); // inherits numbered header

    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    // The body paragraph on page 2 should have list item marker starting with 1 (not 2 from header advance):
    let body_label_text = l.pages[1].items.iter().find_map(|item| {
        if let crate::Placed::Lines { para, .. } = item { para.label.as_ref().and_then(|lab| lab.runs.first().map(|r| r.text.clone())) } else { None }
    });
    assert_eq!(body_label_text.as_deref(), Some("1."), "body paragraph numbering starts at 1, not advanced by header measurement");
}

/// #4847: A vertical section starting on an even page (via page_num_start) selects the even header
/// when even/odd headers are enabled, reserving room for it.
#[test]
fn vertical_section_with_even_page_start_selects_even_header() {
    crate::test_fonts();
    let mut d = Document::from_text("Vertical section starting on even page");
    d.settings.even_odd_headers = true;

    let p = wordcraft_doc::Paragraph::with_text(
        "A multi-line tall even header paragraph that pushes the body top boundary downward by many points.",
        Default::default(),
    );
    let even_id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(p)]);

    d.last_section.page_w = 400.0;
    d.last_section.page_h = 800.0;
    d.last_section.margin_left = 50.0;
    d.last_section.margin_right = 50.0;
    d.last_section.margin_top = 50.0;
    d.last_section.page_num_start = Some(2); // starts on even page (page 2)
    d.last_section.text_direction = TextDirection::Down;
    d.last_section.headers = wordcraft_doc::section::HeaderSet { even: Some(even_id), ..Default::default() };

    let l = lay(&d);
    assert_eq!(l.pages.len(), 1);
    assert_eq!(l.pages[0].number, 2);
    assert!(l.pages[0].body.y > d.last_section.margin_top, "first page (page 2) selects and reserves space for the even header");
}

/// #4847: When a manual page break leaves an empty page at the end of the preceding section,
/// `start_section_page` retracts it; `turned_section` must see the retracted page number so the
/// header slot measured for vertical body clearance matches the rendered page parity.
#[test]
fn vertical_section_after_page_break_retracts_empty_page_before_selecting_even_header() {
    crate::test_fonts();
    use wordcraft_doc::para::PAGE_BREAK;
    use wordcraft_doc::section::HeaderSet;

    let mut d = Document::from_text(&format!("Page 1{PAGE_BREAK}\nSection 2 vertical"));
    d.settings.even_odd_headers = true;

    // Short odd/default header fits inside margin_top; tall even header pushes body down.
    let short =
        d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Odd", Default::default()))]);
    let tall_paras: Vec<_> = (0..5)
        .map(|i| wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("Even header line {i}"), Default::default())))
        .collect();
    let even_id = d.add_part(wordcraft_doc::PartKind::Header, tall_paras);

    let s1 = SectionProps { start: SectionStart::NextPage, margin_top: 50.0, ..Default::default() };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(s1));

    d.last_section.start = SectionStart::NextPage;
    d.last_section.margin_top = 50.0;
    d.last_section.text_direction = TextDirection::Down;
    d.last_section.headers = HeaderSet { default: Some(short), even: Some(even_id), ..Default::default() };

    let l = lay(&d);
    assert_eq!(l.pages.len(), 2, "empty break page retracted");
    assert_eq!(l.pages[1].number, 2);
    assert_eq!(l.pages[1].hf_slot, HfSlot::Even);
    assert!(
        l.pages[1].body.y > d.last_section.margin_top + 10.0,
        "vertical page 2 reserves space for even header instead of default header: body.y={}",
        l.pages[1].body.y
    );
}

/// #4847: Measuring a turned section's header passes the actual page number to `body_top_for`
/// so an inside/outside-aligned object with one-sided wrapping on an even page gives the same
/// header height during measurement as when rendered.
#[test]
fn vertical_section_header_measurement_uses_actual_page_number_for_inside_outside_float() {
    crate::test_fonts();
    use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap, WrapText};

    let mut d = Document::from_text("Vertical section with inside float in header");
    let mut hp = wordcraft_doc::Paragraph::with_text("Header text wrapped around inside float", Default::default());
    let float = Float {
        wrap: Wrap::Square,
        wrap_text: WrapText::Right,
        h_rel: Anchor::Margin,
        h_align: Some(FloatAlign::Inside),
        v_rel: Anchor::Paragraph,
        ..Default::default()
    };
    hp.insert_object(0, super::rect_shape(100.0, 120.0, float), &Default::default()).unwrap();
    let hid = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);

    d.last_section.page_w = 400.0;
    d.last_section.page_h = 800.0;
    d.last_section.margin_left = 50.0;
    d.last_section.margin_right = 50.0;
    d.last_section.margin_top = 80.0;
    d.last_section.header = 20.0;
    d.last_section.page_num_start = Some(2); // even page: Inside is right edge, so WrapText::Right wraps below the 120pt shape
    d.last_section.text_direction = TextDirection::Down;
    d.last_section.headers = wordcraft_doc::section::HeaderSet { default: Some(hid), ..Default::default() };

    let l = lay(&d);
    assert_eq!(l.pages.len(), 1);
    assert_eq!(l.pages[0].number, 2);
    let header_bottom = l.pages[0]
        .header
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { y, para, l0, l1, .. } => crate::page::item_bottom(*y, para, *l0, *l1),
            _ => None,
        })
        .fold(0.0f32, f32::max);
    assert!(header_bottom > d.last_section.margin_top + 40.0, "header text wraps below 120pt float on even page: bottom={header_bottom}");
    assert!(l.pages[0].body.y >= header_bottom - 0.5, "vertical body.y ({}) clears wrapped even-page header ({header_bottom})", l.pages[0].body.y);

    // Control: on an odd page (page 1), Inside is left edge so WrapText::Right fits beside the float inside margin_top.
    let mut d_odd = d.clone();
    d_odd.last_section.page_num_start = Some(1);
    let l_odd = lay(&d_odd);
    assert!((l_odd.pages[0].body.y - d_odd.last_section.margin_top).abs() < 0.01);
}
