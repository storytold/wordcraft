//! A list number's decorations, as Microsoft 365 Word draws them (hand-written DOCX files, list
//! `%1.` / `%1)` with a hanging indent, Word's PDFs): the number takes the paragraph mark's
//! strikethrough, double strikethrough, highlight and border, but its underline and shading come
//! from the list level alone, and it is never small caps. Lines run over the number's characters;
//! the highlight, shading and border also cover the gap up to the text.
use crate::display::{Draw, page_display};
use crate::*;
use std::sync::Arc;
use wordcraft_doc::props::{Border, Highlight, NumRef, Rgb, TextColor, Underline};
use wordcraft_doc::resolve::ResolvedChar;
use wordcraft_doc::styles::Style;
use wordcraft_doc::{ListKind, para_block};

const RED: Rgb = Rgb(255, 0, 0);
const BLUE: Rgb = Rgb(0, 0, 255);
const GREEN: Rgb = Rgb(0, 176, 80);
const LIME: Rgb = Rgb(0, 255, 0);

/// One numbered paragraph, laid out and drawn.
struct Laid {
    draws: Vec<Draw>,
    /// The number's left and right edges and where the paragraph's text starts (page x).
    left: f32,
    right: f32,
    text: f32,
    /// The number as drawn (its runs' text) and its width.
    shown: String,
    width: f32,
    /// The resolved properties the number is drawn with.
    rc: ResolvedChar,
}

/// A horizontal line: (x0, x1, y, width, colour, stroke as text).
type HLine = (f32, f32, f32, f32, Rgb, String);

impl Laid {
    fn hlines(&self) -> Vec<HLine> {
        self.draws
            .iter()
            .filter_map(|d| match d {
                Draw::Line { x0, y0, x1, y1, width, color, stroke, .. } if (y0 - y1).abs() < 0.01 && (x1 - x0).abs() > 0.5 => {
                    Some((x0.min(*x1), x0.max(*x1), *y0, *width, *color, format!("{stroke:?}")))
                }
                _ => None,
            })
            .collect()
    }
    /// The horizontal lines drawn on the number (anywhere from its left edge to the text).
    fn on_number(&self) -> Vec<HLine> {
        self.hlines().into_iter().filter(|l| l.0 >= self.left - 0.5 && l.1 <= self.text + 0.5).collect()
    }
    /// The horizontal lines drawn on the text.
    fn on_text(&self) -> Vec<HLine> {
        self.hlines().into_iter().filter(|l| l.0 >= self.text - 0.5).collect()
    }
    /// Text backgrounds (highlight or shading): (x0, x1, y, height, colour), with the index of
    /// each in the draw list.
    fn fills(&self) -> Vec<(usize, f32, f32, f32, f32, Rgb)> {
        self.draws
            .iter()
            .enumerate()
            .filter_map(|(i, d)| match d {
                Draw::Fill { rect, color, text_bg: true, .. } => Some((i, rect.x, rect.x + rect.w, rect.y, rect.h, *color)),
                _ => None,
            })
            .collect()
    }
    /// The backgrounds that start at the number.
    fn fills_on_number(&self) -> Vec<(usize, f32, f32, f32, f32, Rgb)> {
        self.fills().into_iter().filter(|f| f.1 < self.text - 0.5).collect()
    }
    /// Index of the number's glyphs in the draw list.
    fn number_glyphs(&self) -> usize {
        self.draws
            .iter()
            .position(
                |d| matches!(d, Draw::Glyphs { text, glyphs, .. } if *text == self.shown && glyphs.first().is_some_and(|g| g.1 < self.text - 0.5)),
            )
            .expect("the number's glyphs are drawn")
    }
    fn assert_spans_number(&self, l: &HLine, what: &str) {
        assert!(
            (l.0 - self.left).abs() < 0.5 && (l.1 - self.right).abs() < 0.5,
            "{what} runs over the number's characters only: {l:?}, number {}..{}",
            self.left,
            self.right
        );
    }
    fn assert_spans_number_and_gap(&self, x0: f32, x1: f32, what: &str) {
        assert!(
            (x0 - self.left).abs() < 0.5 && (x1 - self.text).abs() < 0.5,
            "{what} covers the number and the gap up to the text: {x0}..{x1}, number at {}, text at {}",
            self.left,
            self.text
        );
    }
}

/// A one-paragraph list of `kind` whose first level has the run properties `level`, whose
/// paragraph mark has `mark` and whose text ("Alpha") has `run`; `style` adds a paragraph style
/// with those run properties and applies it.
fn lay_with(kind: ListKind, level: CharProps, mark: CharProps, run: CharProps, style: Option<CharProps>) -> Laid {
    crate::test_fonts();
    let mut d = Document::new();
    let num = Arc::make_mut(&mut d.numbering).add_list(kind);
    let numbering = Arc::make_mut(&mut d.numbering);
    let abstract_id = numbering.nums.iter().find(|n| n.id == num).expect("the new list").abstract_id;
    let lvl = numbering.abstracts.iter_mut().find(|a| a.id == abstract_id).and_then(|a| a.levels.first_mut()).expect("its first level");
    lvl.chr = lvl.chr.clone().overlaid(&level);
    let mut p = Paragraph::with_text("Alpha", run);
    p.props.numbering = Some(NumRef { num, level: 0 });
    p.mark = mark;
    if let Some(chr) = style {
        Arc::make_mut(&mut d.styles).upsert(Style { id: "Marked".into(), name: "Marked".into(), chr, ..Style::default() });
        p.props.style = Some("Marked".into());
    }
    d.body = vec![para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = l.pages.first().expect("one page");
    let (pl, x) = page
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { para, x, .. } if para.label.is_some() => Some((para, *x)),
            _ => None,
        })
        .expect("the numbered paragraph");
    let label = pl.label.as_ref().expect("a list number");
    let line = pl.lines.first().expect("a line");
    let text = (line.c0..line.c1)
        .filter(|&k| pl.clusters.get(k).is_some_and(|c| matches!(c.kind, para::ClKind::Text)))
        .filter_map(|k| line.cl_left(k))
        .fold(f32::MAX, f32::min)
        + x;
    let left = x + label.x;
    Laid {
        draws: page_display(&d, page, &Default::default()),
        left,
        right: left + label.width,
        text,
        shown: label.runs.iter().map(|r| r.text.as_str()).collect(),
        width: label.width,
        rc: pl.styles.get(label.style as usize).expect("the number's style").rc.as_ref().clone(),
    }
}

fn lay(level: CharProps, mark: CharProps, run: CharProps) -> Laid {
    lay_with(ListKind::Numbered, level, mark, run, None)
}

fn plain() -> CharProps {
    CharProps::default()
}

fn underlined(u: Underline) -> CharProps {
    CharProps { underline: Some(u), ..CharProps::default() }
}

#[test]
fn a_plain_number_has_a_gap_before_the_text_and_no_decoration() {
    let l = lay(plain(), plain(), plain());
    assert_eq!(l.shown.trim(), "1.");
    assert!(l.right + 1.0 < l.text, "the number {}..{} ends before the text at {}", l.left, l.right, l.text);
    assert_eq!(l.on_number(), vec![], "no line on a plain number");
    assert_eq!(l.fills(), vec![], "no background on a plain paragraph");
}

#[test]
fn the_levels_underline_is_drawn_under_the_number_like_a_text_underline() {
    for u in [Underline::Single, Underline::Double, Underline::Thick, Underline::Dotted, Underline::Dash, Underline::Wave] {
        let l = lay(underlined(u), plain(), underlined(u));
        let (number, text) = (l.on_number(), l.on_text());
        assert_eq!(number.len(), 1, "{u:?}: one underline on the number: {number:?}");
        assert_eq!(text.len(), 1, "{u:?}: one underline on the text: {text:?}");
        l.assert_spans_number(&number[0], "the underline");
        assert!((number[0].2 - text[0].2).abs() < 0.01, "{u:?}: at the text underline's height: {number:?} vs {text:?}");
        assert!((number[0].3 - text[0].3).abs() < 0.01, "{u:?}: as thick as the text's: {number:?} vs {text:?}");
        assert_eq!(number[0].5, text[0].5, "{u:?}: in the text underline's stroke");
        assert_eq!(l.rc.underline, u);
    }
}

#[test]
fn words_only_underline_is_one_line_under_the_number() {
    let l = lay(underlined(Underline::Words), plain(), plain());
    let number = l.on_number();
    assert_eq!(number.len(), 1, "{number:?}");
    l.assert_spans_number(&number[0], "the underline");
    assert_eq!(l.on_text(), vec![], "the text is not underlined");
}

#[test]
fn the_underline_takes_its_own_colour_or_the_numbers() {
    // w:u w:color: a green line under a black number.
    let l = lay(CharProps { underline_color: Some(GREEN), ..underlined(Underline::Single) }, plain(), plain());
    assert_eq!(l.on_number().iter().map(|x| x.4).collect::<Vec<_>>(), [GREEN]);
    // No underline colour: the number's colour, here the level's blue, which wins over the mark's red.
    let level = CharProps { color: Some(TextColor::Rgb(BLUE)), ..underlined(Underline::Double) };
    let mark = CharProps { color: Some(TextColor::Rgb(RED)), ..CharProps::default() };
    let l = lay(level, mark.clone(), plain());
    assert_eq!(l.on_number().iter().map(|x| x.4).collect::<Vec<_>>(), [BLUE]);
    // ... and the mark's red when the level has no colour.
    let l = lay(underlined(Underline::Single), mark, plain());
    assert_eq!(l.on_number().iter().map(|x| x.4).collect::<Vec<_>>(), [RED]);
}

#[test]
fn the_marks_underline_never_reaches_the_number() {
    // Underlined text and paragraph mark, plain level: Word underlines the text only.
    for u in [Underline::Single, Underline::Double] {
        let mark = CharProps { underline_color: Some(RED), ..underlined(u) };
        let l = lay(plain(), mark, underlined(u));
        assert_eq!(l.on_number(), vec![], "{u:?}: the number is not underlined");
        assert_eq!(l.on_text().len(), 1, "{u:?}: the text is");
        assert_eq!(l.rc.underline, Underline::None);
        assert_eq!(l.rc.underline_color, None);
    }
    // The same from the paragraph's style.
    let l = lay_with(ListKind::Numbered, plain(), plain(), plain(), Some(underlined(Underline::Single)));
    assert_eq!(l.on_number(), vec![], "a style's underline is the text's, not the number's");
    assert_eq!(l.on_text().len(), 1);
    assert_eq!(l.rc.underline, Underline::None);
}

#[test]
fn the_mark_cannot_change_the_levels_underline() {
    for mark in [underlined(Underline::None), underlined(Underline::Single)] {
        let l = lay(underlined(Underline::Double), mark, plain());
        let number = l.on_number();
        assert_eq!(number.len(), 1, "{number:?}");
        assert_eq!(l.rc.underline, Underline::Double);
        let reference = lay(underlined(Underline::Double), plain(), plain()).on_number();
        assert_eq!(number[0].5, reference[0].5, "still a double underline");
    }
}

#[test]
fn strikethrough_from_the_mark_the_style_or_the_level_is_drawn_over_the_number() {
    let strike = CharProps { strike: Some(true), ..CharProps::default() };
    let double = CharProps { double_strike: Some(true), ..CharProps::default() };
    for (what, level, mark, style) in [
        ("mark", plain(), strike.clone(), None),
        ("level", strike.clone(), plain(), None),
        ("style", plain(), plain(), Some(strike.clone())),
        ("mark, double", plain(), double.clone(), None),
        ("level, double", double.clone(), plain(), None),
    ] {
        let is_double = what.ends_with("double");
        // A struck text run of the same size is the reference for height, thickness and stroke.
        let run = if is_double { double.clone() } else { strike.clone() };
        let l = lay_with(ListKind::Numbered, level, mark, run, style);
        let (number, text) = (l.on_number(), l.on_text());
        assert_eq!(number.len(), 1, "{what}: one line through the number: {number:?}");
        assert_eq!(text.len(), 1, "{what}: one line through the text: {text:?}");
        l.assert_spans_number(&number[0], "the strikethrough");
        assert!((number[0].2 - text[0].2).abs() < 0.01, "{what}: at the text strikethrough's height");
        assert!((number[0].3 - text[0].3).abs() < 0.01, "{what}: as thick as the text's");
        assert_eq!(number[0].5, text[0].5, "{what}: in the text strikethrough's stroke");
    }
    // Single and double differ.
    let one = lay(plain(), strike, plain()).on_number();
    let two = lay(plain(), double, plain()).on_number();
    assert_ne!(one[0].5, two[0].5);
}

#[test]
fn a_struck_and_underlined_number_has_the_strikethrough_above_the_underline() {
    let l = lay(underlined(Underline::Single), CharProps { strike: Some(true), ..CharProps::default() }, plain());
    let mut ys: Vec<f32> = l.on_number().iter().map(|x| x.2).collect();
    assert_eq!(ys.len(), 2, "{ys:?}");
    ys.sort_by(f32::total_cmp);
    assert!(ys[1] - ys[0] > 2.0, "strikethrough through the digits, underline below them: {ys:?}");
}

#[test]
fn highlight_from_the_mark_or_the_level_covers_the_number_and_the_gap() {
    let yellow = Highlight::Yellow.rgb().expect("yellow");
    let hl = CharProps { highlight: Some(Highlight::Yellow), ..CharProps::default() };
    // The text run is highlighted in another colour: the reference for the band's top and height.
    let run = CharProps { highlight: Some(Highlight::Turquoise), ..CharProps::default() };
    for (what, level, mark, style) in
        [("mark", plain(), hl.clone(), None), ("level", hl.clone(), plain(), None), ("style", plain(), plain(), Some(hl.clone()))]
    {
        let l = lay_with(ListKind::Numbered, level, mark, run.clone(), style);
        let number = l.fills_on_number();
        assert_eq!(number.len(), 1, "{what}: one background on the number: {number:?}");
        let (at, x0, x1, y, h, color) = number[0];
        assert_eq!(color, yellow, "{what}");
        l.assert_spans_number_and_gap(x0, x1, "the highlight");
        let text = l.fills().into_iter().find(|f| f.1 >= l.text - 0.5).expect("the text's highlight");
        assert!((y - text.3).abs() < 0.01 && (h - text.4).abs() < 0.01, "{what}: as tall as the text's highlight: {y}+{h} vs {}+{}", text.3, text.4);
        assert!(at < l.number_glyphs(), "{what}: the background is painted before the number");
    }
}

#[test]
fn shading_comes_from_the_level_only() {
    let shade = CharProps { shading: Some(LIME), ..CharProps::default() };
    let l = lay(shade.clone(), plain(), plain());
    let number = l.fills_on_number();
    assert_eq!(number.len(), 1, "the level's shading is drawn: {number:?}");
    assert_eq!(number[0].5, LIME);
    l.assert_spans_number_and_gap(number[0].1, number[0].2, "the shading");
    assert_eq!(l.rc.shading, Some(LIME));
    // The mark's (or the style's) shading is the text's alone.
    let l = lay(plain(), shade.clone(), shade.clone());
    assert_eq!(l.fills_on_number(), vec![], "the mark's shading is not drawn on the number");
    assert!(l.fills().iter().any(|f| f.5 == LIME), "the text's is");
    assert_eq!(l.rc.shading, None);
    let l = lay_with(ListKind::Numbered, plain(), plain(), plain(), Some(shade));
    assert_eq!(l.fills_on_number(), vec![]);
    assert_eq!(l.rc.shading, None);
    // A level with both: the highlight shows, as on text.
    let both = CharProps { shading: Some(LIME), highlight: Some(Highlight::Yellow), ..CharProps::default() };
    let l = lay(both, plain(), plain());
    assert_eq!(l.fills_on_number().iter().map(|f| f.5).collect::<Vec<_>>(), [Highlight::Yellow.rgb().expect("yellow")]);
}

#[test]
fn a_border_from_the_mark_or_the_level_boxes_the_number_and_the_gap() {
    let boxed = CharProps { border: Some(Border::single(0.5)), ..CharProps::default() };
    for (what, level, mark) in [("mark", plain(), boxed.clone()), ("level", boxed.clone(), plain())] {
        let l = lay(level, mark, plain());
        let edges = l.on_number();
        assert_eq!(edges.len(), 2, "{what}: a top and a bottom edge: {edges:?}");
        for e in &edges {
            l.assert_spans_number_and_gap(e.0, e.1, "the box");
        }
        assert!((edges[0].2 - edges[1].2).abs() > 5.0, "{what}: top and bottom are a line apart: {edges:?}");
        let sides: Vec<f32> = l
            .draws
            .iter()
            .filter_map(|d| match d {
                Draw::Line { x0, y0, x1, y1, .. } if (x0 - x1).abs() < 0.01 && (y1 - y0).abs() > 5.0 => Some(*x0),
                _ => None,
            })
            .collect();
        assert_eq!(sides.len(), 2, "{what}: a left and a right edge: {sides:?}");
        assert!(
            sides.iter().any(|x| (x - l.left).abs() < 0.5) && sides.iter().any(|x| (x - l.text).abs() < 0.5),
            "{what}: at the number's left and the text's start: {sides:?}"
        );
    }
}

#[test]
fn a_list_number_is_never_small_caps() {
    let small = CharProps { small_caps: Some(true), ..CharProps::default() };
    let reference = lay_with(ListKind::LowerLetter, plain(), plain(), plain(), None);
    assert_eq!(reference.shown.trim(), "a)");
    for (what, level, mark, style) in
        [("mark", plain(), small.clone(), None), ("level", small.clone(), plain(), None), ("style", plain(), plain(), Some(small.clone()))]
    {
        let l = lay_with(ListKind::LowerLetter, level, mark, plain(), style);
        assert_eq!(l.shown, reference.shown, "{what}: the letter stays as the list gives it");
        assert!((l.width - reference.width).abs() < 0.01, "{what}: at full size: {} vs {}", l.width, reference.width);
        assert!(!l.rc.small_caps, "{what}");
    }
    // All caps still capitalises the number.
    let caps = CharProps { caps: Some(true), ..CharProps::default() };
    assert_eq!(lay_with(ListKind::LowerLetter, plain(), caps, plain(), None).shown.trim(), "A)");
}

#[test]
fn the_number_keeps_what_it_already_took_from_the_mark() {
    let mark = CharProps { bold: Some(true), italic: Some(true), size: Some(20.0), color: Some(TextColor::Rgb(RED)), ..CharProps::default() };
    let l = lay(plain(), mark, plain());
    assert!(l.rc.bold && l.rc.italic);
    assert_eq!(l.rc.size, 20.0);
    assert_eq!(l.rc.color, TextColor::Rgb(RED));
    assert_eq!(l.on_number(), vec![]);
    assert_eq!(l.fills(), vec![]);
}
