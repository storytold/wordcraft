//! `w:textAlignment` (ECMA-376 §17.3.1.39): where the characters of a line that mixes sizes sit,
//! drawn beside an inline picture in a document-grid text box. `w:adjustLineHeightInTable`
//! (§17.15.3.1): whether the grid's line pitch reaches the lines of a table cell. Assertions are
//! relations between the line's own metrics and its drawing, never font metrics.

#[allow(unused_imports)]
use crate::*;
use wordcraft_doc::props::{LineSpacing, ParaProps, TextAlignment};
use wordcraft_doc::section::{DocGrid, DocGridType};
use wordcraft_doc::{Path, Pos, Table};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

/// "Alpha" with an inline picture 60 pt tall after it, in a paragraph with `align`.
fn picture_after_text(align: Option<TextAlignment>) -> Document {
    let mut d = Document::from_text("Alpha");
    let at = Pos::body(0, 0);
    d.format_paragraphs(&at, &at, &|p: &mut ParaProps| {
        p.space_before = Some(0.0);
        p.space_after = Some(0.0);
        p.line_spacing = Some(LineSpacing::Multiple(1.0));
        p.text_alignment = align;
    })
    .unwrap();
    let pic = wordcraft_doc::InlineObject::Image {
        media: "m".into(),
        w: 30.0,
        h: 60.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_object(5, pic, &Default::default()).unwrap();
    d
}

/// The first body paragraph's layout and its line's top on the page.
fn first_para(l: &DocLayout) -> (Arc<para::ParaLayout>, f32) {
    for it in &l.pages[0].items {
        if let Placed::Lines { story: StoryRef::Body, path, para, y, .. } = it
            && path.0 == vec![0]
        {
            return (para.clone(), *y);
        }
    }
    panic!("no body paragraph on the first page");
}

/// The text's and the picture's cluster indices on the line.
fn text_and_picture(pl: &para::ParaLayout, line: &para::Line) -> (usize, usize) {
    let kinds = |want: fn(&para::ClKind) -> bool| (line.c0..line.c1).find(|&k| want(&pl.clusters[k].kind)).unwrap();
    (kinds(|k| *k == para::ClKind::Text), kinds(|k| matches!(k, para::ClKind::Object(_))))
}

/// The text's glyph baseline and the picture's drawn rectangle, in page coordinates.
fn drawn(d: &Document, l: &DocLayout) -> (f32, Rect) {
    let draws = display::page_display(d, &l.pages[0], &Default::default());
    let glyph_y = draws
        .iter()
        .find_map(|it| if let display::Draw::Glyphs { glyphs, text, .. } = it { (text.contains("Alpha")).then(|| glyphs[0].2) } else { None })
        .unwrap();
    let rect = draws.iter().find_map(|it| if let display::Draw::Image { rect, .. } = it { Some(*rect) } else { None }).unwrap();
    (glyph_y, rect)
}

#[test]
fn auto_and_baseline_keep_the_characters_on_the_baseline() {
    for align in [None, Some(TextAlignment::Auto), Some(TextAlignment::Baseline)] {
        let d = picture_after_text(align);
        let l = lay(&d);
        let (pl, _) = first_para(&l);
        let line = &pl.lines[0];
        assert!(line.shifts.is_empty(), "{align:?}: {:?}", line.shifts);
        let (text, pic) = text_and_picture(&pl, line);
        assert_eq!((line.shift_at(text), line.shift_at(pic)), (0.0, 0.0), "{align:?}");
    }
}

#[test]
fn top_puts_the_text_box_on_the_line_top_and_keeps_the_picture() {
    let d = picture_after_text(Some(TextAlignment::Top));
    let l = lay(&d);
    let (pl, y) = first_para(&l);
    let line = &pl.lines[0];
    let (text, pic) = text_and_picture(&pl, line);
    // The picture is the tallest thing on the line, so its top already is the line's top.
    assert!(line.shift_at(pic).abs() < 0.01, "picture shift {}", line.shift_at(pic));
    // The text's box (its ascent above the baseline) starts at the line's top.
    let ascent = pl.styles[pl.clusters[text].style as usize].line_extent().0;
    assert!(((line.baseline - line.top) + line.shift_at(text) - ascent).abs() < 0.01);
    // Drawn: the glyphs sit that ascent below the line's top, the picture where it was.
    let (glyph_y, rect) = drawn(&d, &l);
    assert!((glyph_y - (y + ascent)).abs() < 0.01, "glyph baseline {glyph_y} vs {}", y + ascent);
    assert!((rect.y + rect.h - (y + line.baseline - line.top + line.shift_at(pic))).abs() < 0.01, "picture {rect:?}");
}

#[test]
fn bottom_puts_the_picture_on_the_line_bottom_and_keeps_the_text() {
    let d = picture_after_text(Some(TextAlignment::Bottom));
    let l = lay(&d);
    let (pl, y) = first_para(&l);
    let line = &pl.lines[0];
    let (text, pic) = text_and_picture(&pl, line);
    // The text's descent reaches the line's bottom already, so the text stays on the baseline.
    assert!(line.shift_at(text).abs() < 0.01, "text shift {}", line.shift_at(text));
    // The picture's bottom edge drops to the line's bottom.
    assert!(line.shift_at(pic) > 0.0, "picture shift {}", line.shift_at(pic));
    let (glyph_y, rect) = drawn(&d, &l);
    assert!((glyph_y - (y + line.baseline - line.top)).abs() < 0.01, "text stays on the baseline: {glyph_y}");
    assert!((rect.y + rect.h - (y + line.height)).abs() < 0.01, "picture {rect:?} bottom vs the line's bottom {}", y + line.height);
}

#[test]
fn center_puts_the_text_box_in_the_middle_of_the_line() {
    let d = picture_after_text(Some(TextAlignment::Center));
    let l = lay(&d);
    let (pl, _) = first_para(&l);
    let line = &pl.lines[0];
    let (text, _) = text_and_picture(&pl, line);
    let st = &pl.styles[pl.clusters[text].style as usize];
    let (a, dsc) = st.line_extent();
    let top_gap = (line.baseline - line.top) + line.shift_at(text) - a;
    let bottom_gap = line.height - ((line.baseline - line.top) + line.shift_at(text) + dsc);
    assert!(top_gap > 0.0 && (top_gap - bottom_gap).abs() < 0.01, "gaps {top_gap} and {bottom_gap}");
}

/// A body of one table with one cell of text, under a document grid of 18 pt lines, with the
/// settings' `adjustLineHeightInTable` as `adjust`.
fn grid_table(adjust: bool) -> Document {
    let mut d = Document::from_text("Lead");
    d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 18.0, char_space: 0 });
    d.settings.adjust_line_height_in_table = adjust;
    let mut t = Table::new(1, 1, 300.0);
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Cell", Default::default()))];
    d.body.insert(0, Arc::new(Block::Table(t)));
    d
}

/// Heights of the lines of the paragraphs inside table cells.
fn cell_line_heights(l: &DocLayout) -> Vec<f32> {
    let mut hs = Vec::new();
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = it
                && path.0.len() > 1
            {
                hs.extend(para.lines[*l0..*l1].iter().map(|line| line.height));
            }
        }
    }
    hs
}

#[test]
fn the_grid_line_pitch_reaches_cell_lines_only_with_adjust_line_height_in_table() {
    let off = cell_line_heights(&lay(&grid_table(false)));
    assert!(!off.is_empty(), "the cell's text is laid out");
    assert!(off.iter().all(|h| (h / 18.0 - (h / 18.0).round()).abs() > 1e-3), "cell lines keep their own height: {off:?}");
    let on = cell_line_heights(&lay(&grid_table(true)));
    assert!(!on.is_empty());
    assert!(on.iter().all(|h| (h / 18.0 - (h / 18.0).round()).abs() < 1e-3 && *h >= 18.0), "cell lines snap to the pitch: {on:?}");
}

/// Like [`grid_table`], with a text box inside the cell after its "Cell" text.
fn grid_table_with_text_box(adjust: bool) -> Document {
    let mut d = Document::from_text("Lead");
    d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 18.0, char_space: 0 });
    d.settings.adjust_line_height_in_table = adjust;
    // Single spacing: the grid makes a line whole pitches only at a multiple of 1.
    let mut boxed = wordcraft_doc::Paragraph::with_text("Box", Default::default());
    boxed.props.line_spacing = Some(LineSpacing::Multiple(1.0));
    let id = d.add_part(wordcraft_doc::PartKind::TextBox, vec![wordcraft_doc::para_block(boxed)]);
    d.parts.get_mut(&id).unwrap().body = wordcraft_doc::TextBody::default();
    let shape = wordcraft_doc::InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 120.0,
        h: 40.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let mut cell = wordcraft_doc::Paragraph::with_text("Cell", Default::default());
    cell.props.line_spacing = Some(LineSpacing::Multiple(1.0));
    cell.insert_object(4, shape, &Default::default()).unwrap();
    let mut t = Table::new(1, 1, 300.0);
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cell)];
    d.body.insert(0, Arc::new(Block::Table(t)));
    d
}

/// Heights of the lines of the paragraphs in text boxes' stories.
fn text_box_line_heights(l: &DocLayout) -> Vec<f32> {
    let mut hs = Vec::new();
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Part(_), para, l0, l1, .. } = it {
                hs.extend(para.lines[*l0..*l1].iter().map(|line| line.height));
            }
        }
    }
    hs
}

#[test]
fn a_text_box_in_a_cell_keeps_the_grid_whatever_the_cells_own_text_does() {
    let on_pitch = |h: &f32| (h / 18.0 - (h / 18.0).round()).abs() < 1e-3 && *h >= 18.0;
    // The cell's own text ignores the pitch without the flag; the box inside it is a story of its own.
    let off = lay(&grid_table_with_text_box(false));
    let (cell_off, boxed_off) = (cell_line_heights(&off), text_box_line_heights(&off));
    assert!(!cell_off.is_empty() && cell_off.iter().all(|h| !on_pitch(h)), "cell lines keep their own height: {cell_off:?}");
    assert!(!boxed_off.is_empty() && boxed_off.iter().all(on_pitch), "the text box keeps the grid: {boxed_off:?}");
    // With the flag, the cell's lines take the pitch too.
    let on = lay(&grid_table_with_text_box(true));
    let (cell_on, boxed_on) = (cell_line_heights(&on), text_box_line_heights(&on));
    assert!(!cell_on.is_empty() && cell_on.iter().all(on_pitch), "cell lines snap with the flag: {cell_on:?}");
    assert!(!boxed_on.is_empty() && boxed_on.iter().all(on_pitch), "the text box keeps the grid: {boxed_on:?}");
}
