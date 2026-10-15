//! Outline view (View › Outline): the body as an outline of its headings. No pages: the text runs
//! on one long sheet like Draft. Each paragraph is indented by its outline level and has a symbol
//! in front of it: a circled plus for a heading with something under it, a circled minus for a
//! heading with nothing under it, a small dot for body text. Collapsed headings hide what is under
//! them and are marked with a wavy line; Show Level hides everything deeper than a level; Show
//! First Line Only shows body paragraphs' first lines; without text formatting every paragraph is
//! drawn in the Normal style. Paragraph indents and alignment aren't shown.
//!
//! What's collapsed is view state (not saved in the document): the caller passes it in
//! [`OutlineView`] by body block index.

use std::collections::BTreeSet;
use std::sync::Arc;

use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::{Align, Border, BorderStyle, CharProps, Rgb};
use wordcraft_doc::{Block, Document, Paragraph, StoryRef};
use wordcraft_geom::Rect;

use crate::{DocLayout, Page, ParaLayout, Placed};

/// The outline level of body text (headings are 1–9).
pub const BODY: u8 = 10;
/// How far each outline level is indented, points.
pub const LEVEL_INDENT: f32 = 21.6;
/// The gutter in front of each paragraph that holds its symbol, points.
pub const SYMBOL_W: f32 = 16.0;

const SYMBOL_GREY: Rgb = Rgb(0x6E, 0x6E, 0x6E);

/// What the outline shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct OutlineView {
    /// Collapsed headings (body block indices).
    pub collapsed: BTreeSet<usize>,
    /// Show Level: 1–9 shows headings down to that level (no body text); anything else shows all.
    pub show_level: u8,
    /// Show First Line Only: body paragraphs show their first line.
    pub first_line_only: bool,
    /// Show Text Formatting off: every paragraph is drawn in the Normal style.
    pub plain: bool,
}

impl OutlineView {
    /// The deepest level shown (body text is [`BODY`]).
    pub fn deepest(&self) -> u8 {
        if (1..=9).contains(&self.show_level) { self.show_level } else { BODY }
    }
}

/// The outline level of a paragraph: 1–9 for headings (its style's or its own outline level),
/// [`BODY`] for body text.
pub fn para_level(doc: &Document, p: &Paragraph) -> u8 {
    doc.styles.resolve_para(&p.props).outline_level.filter(|l| *l < 9).map(|l| l + 1).unwrap_or(BODY)
}

/// The outline level of every body block (tables are body text).
pub fn levels(doc: &Document) -> Vec<u8> {
    doc.body
        .iter()
        .map(|b| match &**b {
            Block::Para(p) => para_level(doc, p),
            Block::Table(_) => BODY,
        })
        .collect()
}

/// The end (exclusive) of what is under block `i`: the blocks after a heading up to the next
/// heading of its level or higher. Body text has nothing under it (`i + 1`).
pub fn subtree_end(levels: &[u8], i: usize) -> usize {
    let Some(&l) = levels.get(i) else { return i };
    if l >= BODY {
        return i.saturating_add(1);
    }
    levels.iter().enumerate().skip(i.saturating_add(1)).find(|(_, x)| **x <= l).map(|(j, _)| j).unwrap_or(levels.len())
}

/// Whether heading `i` has anything under it.
pub fn has_children(levels: &[u8], i: usize) -> bool {
    subtree_end(levels, i) > i.saturating_add(1)
}

/// Which blocks the outline hides: deeper than Show Level, or under a collapsed heading.
pub fn hidden(levels: &[u8], view: &OutlineView) -> Vec<bool> {
    let deepest = view.deepest();
    // While inside a collapsed heading: its level (blocks deeper than it are hidden).
    let mut under: Option<u8> = None;
    levels
        .iter()
        .enumerate()
        .map(|(i, &l)| {
            if let Some(h) = under {
                if l > h {
                    return true;
                }
                under = None;
            }
            if l > deepest {
                return true;
            }
            if l < BODY && view.collapsed.contains(&i) {
                under = Some(l);
            }
            false
        })
        .collect()
}

/// "Collapsed by default" headings ([`ResolvedPara::collapsed`], #1745) as the other views show
/// them: which blocks they hide. A collapsed heading hides what follows it up to the next heading
/// of its level or higher, or the end of the story; the heading itself shows; body text can't
/// collapse. `collapsed` is by block index (missing entries are not collapsed).
///
/// [`ResolvedPara::collapsed`]: wordcraft_doc::resolve::ResolvedPara::collapsed
pub fn fold(levels: &[u8], collapsed: &[bool]) -> Vec<bool> {
    let view = OutlineView { collapsed: collapsed.iter().enumerate().filter(|(_, c)| **c).map(|(i, _)| i).collect(), ..Default::default() };
    hidden(levels, &view)
}

/// The body blocks collapsed headings hide ([`fold`]), by block index; empty (nothing hidden)
/// when no paragraph or style is collapsed, so documents without folds pay nothing. Linear in
/// the body's length; headings inside table cells don't fold.
pub fn folded(doc: &Document) -> Vec<bool> {
    let st = &doc.styles;
    let any = st.default_para.collapsed == Some(true)
        || st.styles.iter().any(|s| s.para.collapsed == Some(true))
        || doc.body.iter().any(|b| b.as_para().is_some_and(|p| p.props.collapsed == Some(true)));
    if !any {
        return Vec::new();
    }
    let (levels, collapsed): (Vec<u8>, Vec<bool>) = doc
        .body
        .iter()
        .map(|b| match &**b {
            Block::Para(p) => {
                let rp = st.resolve_para(&p.props);
                (rp.outline_level.filter(|l| *l < 9).map(|l| l + 1).unwrap_or(BODY), rp.collapsed)
            }
            Block::Table(_) => (BODY, false),
        })
        .unzip();
    fold(&levels, &collapsed)
}

/// How far block `i` is indented: a heading by its level, body text one level deeper than the
/// heading it follows.
pub fn indent(levels: &[u8], i: usize) -> f32 {
    let Some(&l) = levels.get(i) else { return 0.0 };
    let depth = if l < BODY { l.saturating_sub(1) } else { levels.get(..i).unwrap_or(&[]).iter().rev().find(|x| **x < BODY).copied().unwrap_or(0) };
    f32::from(depth) * LEVEL_INDENT
}

/// Character formatting kept without text formatting: what the text is (hidden, tracked, a
/// link, its language and direction), not how it looks.
fn plain_chars(c: &CharProps) -> CharProps {
    CharProps {
        hidden: c.hidden,
        lang: c.lang.clone(),
        no_proof: c.no_proof,
        rtl: c.rtl,
        cs: c.cs,
        lang_bidi: c.lang_bidi.clone(),
        link: c.link.clone(),
        ins: c.ins,
        del: c.del,
        ..Default::default()
    }
}

/// The paragraph as the outline draws it: no indents, start-aligned, and in the Normal style
/// without text formatting (`plain`).
pub(crate) fn outline_para(doc: &Document, p: &Paragraph, plain: bool) -> Paragraph {
    let mut q = p.clone();
    q.props.indent_left = Some(0.0);
    q.props.indent_right = Some(0.0);
    q.props.indent_first = Some(0.0);
    q.props.align = Some(Align::Left);
    q.props.page_break_before = Some(false);
    q.props.drop_cap = None;
    if plain {
        let rp = doc.styles.resolve_para(&p.props);
        q.props.style = None;
        q.props.numbering = p.props.numbering.or(rp.numbering);
        q.props.outline_level = None;
        q.props.shading = None;
        q.props.borders = None;
        for r in &mut q.runs {
            r.props = plain_chars(&r.props);
        }
        q.mark = plain_chars(&q.mark);
    }
    q
}

/// The symbol in front of a paragraph whose first line `pl` starts at (`x`, `y`) (the text's left
/// edge): a circled plus or minus for a heading, a dot for body text; and a wavy line under a
/// collapsed heading's first line.
pub(crate) fn symbol_items(pl: &Arc<ParaLayout>, x: f32, y: f32, level: u8, children: bool, collapsed: bool) -> Vec<Placed> {
    let Some(first) = pl.lines.first() else { return Vec::new() };
    let asc = (first.baseline - first.top).max(4.0);
    let cy = y + asc * 0.62;
    let cx = x - SYMBOL_W + 6.0;
    let mut out = Vec::new();
    let disc = |r: f32| Placed::Shape {
        rect: Rect::new(cx - r, cy - r, r * 2.0, r * 2.0),
        kind: ShapeKind::Ellipse,
        fill: Some(SYMBOL_GREY),
        stroke: None,
        stroke_width: 0.0,
        freeform: None,
        effects: Default::default(),
        spin: Default::default(),
        extra: Default::default(),
    };
    let white = Border { style: BorderStyle::Single, width: 1.1, color: Some(Rgb(0xFF, 0xFF, 0xFF)), space: 0.0 };
    if level >= BODY {
        out.push(disc(2.0));
        return out;
    }
    let r = 4.6;
    out.push(disc(r));
    out.push(Placed::Rule { x0: cx - 2.6, y0: cy, x1: cx + 2.6, y1: cy, border: white });
    if children {
        out.push(Placed::Rule { x0: cx, y0: cy - 2.6, x1: cx, y1: cy + 2.6, border: white });
    }
    if collapsed {
        let w = first.xs.last().copied().unwrap_or(0.0).max(12.0);
        let by = y + (first.baseline - first.top) + 3.0;
        out.push(Placed::Rule {
            x0: x,
            y0: by,
            x1: x + w,
            y1: by,
            border: Border { style: BorderStyle::Wave, width: 0.75, color: Some(SYMBOL_GREY), space: 0.0 },
        });
    }
    out
}

impl DocLayout {
    /// The outline symbol at page point (`x`, `y`), as the body block it belongs to (Outline view:
    /// clicking a symbol selects the heading and what is under it).
    pub fn outline_symbol_at(&self, page: usize, x: f32, y: f32) -> Option<usize> {
        let page: &Page = self.pages.get(page)?;
        page.items.iter().find_map(|it| {
            let Placed::Lines { story: StoryRef::Body, path, para, l0: 0, x: lx, y: ly, .. } = it else { return None };
            let [block] = path.0.as_slice() else { return None };
            let first = para.lines.first()?;
            let r = Rect::new(lx - SYMBOL_W, *ly, SYMBOL_W, first.height.max(8.0));
            (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h).then_some(*block as usize)
        })
    }
}

/// How wide a heading's fold arrow is, points, and its gap from the text.
pub const FOLD_ARROW: f32 = 9.0;
const FOLD_GAP: f32 = 3.0;

/// A heading's fold arrow (#1745): in the margin beside its first line, on its start side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldArrow {
    /// The heading's body block.
    pub block: usize,
    /// Where it sits on the page, points.
    pub rect: Rect,
    /// The heading is collapsed (the arrow points along the line, not down).
    pub collapsed: bool,
    /// The heading reads right to left: the arrow is on the right, pointing left when collapsed.
    pub rtl: bool,
    /// The left and right edges of the heading's first line on the page: pointing anywhere
    /// between them and the arrow shows it.
    pub line: (f32, f32),
}

impl DocLayout {
    /// The fold arrows of the body headings that start on `page` (top-level paragraphs with an
    /// outline level; not in Outline view, which has its own symbols).
    pub fn fold_arrows(&self, page: usize) -> Vec<FoldArrow> {
        let Some(page) = self.pages.get(page) else { return Vec::new() };
        page.items
            .iter()
            .filter_map(|it| {
                let Placed::Lines { story: StoryRef::Body, path, para, l0: 0, x, y, turn: wordcraft_doc::props::TextDirection::Horizontal, .. } = it
                else {
                    return None;
                };
                let [block] = path.0.as_slice() else { return None };
                para.rp.outline_level.filter(|l| *l < 9)?;
                let first = para.lines.first()?;
                let side = if first.rtl { x + first.right + FOLD_GAP } else { x + first.left - FOLD_GAP - FOLD_ARROW };
                Some(FoldArrow {
                    block: *block as usize,
                    rect: Rect::new(side, *y, FOLD_ARROW, first.height.max(FOLD_ARROW)),
                    collapsed: para.rp.collapsed,
                    rtl: first.rtl,
                    line: (x + first.left, x + first.right),
                })
            })
            .collect()
    }

    /// The heading whose fold arrow is at page point (`x`, `y`), with a little slack around it.
    pub fn fold_arrow_at(&self, page: usize, x: f32, y: f32) -> Option<usize> {
        self.fold_arrows(page)
            .into_iter()
            .find(|a| x >= a.rect.x - 2.0 && x <= a.rect.x + a.rect.w + 2.0 && y >= a.rect.y && y <= a.rect.y + a.rect.h)
            .map(|a| a.block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_level_and_collapse_hide_what_is_under_a_heading() {
        // H1, body, H2, body, H1, body
        let levels = [1, BODY, 2, BODY, 1, BODY];
        let all = OutlineView::default();
        assert_eq!(hidden(&levels, &all), vec![false; 6]);
        let one = OutlineView { show_level: 1, ..Default::default() };
        assert_eq!(hidden(&levels, &one), vec![false, true, true, true, false, true]);
        let two = OutlineView { show_level: 2, ..Default::default() };
        assert_eq!(hidden(&levels, &two), vec![false, true, false, true, false, true]);
        // Collapsing the first heading hides its whole subtree, not the next Heading 1.
        let c = OutlineView { collapsed: [0].into_iter().collect(), ..Default::default() };
        assert_eq!(hidden(&levels, &c), vec![false, true, true, true, false, false]);
        // Collapsing body text does nothing.
        let b = OutlineView { collapsed: [1].into_iter().collect(), ..Default::default() };
        assert_eq!(hidden(&levels, &b), vec![false; 6]);
        assert!(has_children(&levels, 0) && has_children(&levels, 2) && !has_children(&levels, 1));
        assert_eq!(subtree_end(&levels, 0), 4);
        assert_eq!(subtree_end(&levels, 99), 99);
        assert_eq!(indent(&levels, 3), 2.0 * LEVEL_INDENT);
        assert_eq!(indent(&levels, 0), 0.0);
    }

    /// "Collapsed by default" (#1745): a collapsed heading hides what follows it up to the next
    /// heading of its level or higher; nested collapsed headings and tables; body text and the
    /// last heading fold nothing.
    #[test]
    fn collapsed_headings_fold_up_to_the_next_heading_of_their_level() {
        // H1, body, H2, table, H3, body, H2, body, H1, body
        let levels = [1, BODY, 2, BODY, 3, BODY, 2, BODY, 1, BODY];
        let at = |ix: &[usize]| -> Vec<bool> { (0..levels.len()).map(|i| ix.contains(&i)).collect() };
        assert_eq!(fold(&levels, &at(&[])), vec![false; 10]);
        assert_eq!(fold(&levels, &at(&[0])), at(&[1, 2, 3, 4, 5, 6, 7]));
        assert_eq!(fold(&levels, &at(&[2])), at(&[3, 4, 5]));
        assert_eq!(fold(&levels, &at(&[4])), at(&[5]));
        // Nested: the outer fold covers the inner one, and both end where the outer one does.
        assert_eq!(fold(&levels, &at(&[0, 2, 4])), at(&[1, 2, 3, 4, 5, 6, 7]));
        assert_eq!(fold(&levels, &at(&[2, 6])), at(&[3, 4, 5, 7]));
        // Body text can't collapse; the last heading runs to the end of the story.
        assert_eq!(fold(&levels, &at(&[1, 3])), vec![false; 10]);
        assert_eq!(fold(&levels, &at(&[8])), at(&[9]));
        // Short or long flag lists don't matter.
        assert_eq!(fold(&levels, &[true]), at(&[1, 2, 3, 4, 5, 6, 7]));
        assert_eq!(fold(&[], &[true]), Vec::<bool>::new());
    }

    /// The document version: levels from styles or direct formatting, tables are body text, and
    /// a document without collapsed headings costs nothing (an empty list).
    #[test]
    fn folded_reads_the_flag_from_paragraphs_and_styles() {
        use wordcraft_doc::props::ParaProps;
        use wordcraft_doc::{Table, para_block};
        let para = |text: &str, props: ParaProps| {
            let mut p = Paragraph::with_text(text, CharProps::default());
            p.props = props;
            para_block(p)
        };
        let h = |n: u8| ParaProps { style: Some(format!("Heading{n}")), ..ParaProps::default() };
        let mut doc = Document::new();
        doc.body = vec![
            para("Intro", ParaProps { collapsed: Some(true), ..h(1) }),
            para("Body one", ParaProps::default()),
            Arc::new(Block::Table(Table::new(1, 1, 100.0))),
            para("Part", h(2)),
            para("Next", h(1)),
            para("Level 2 by hand", ParaProps { outline_level: Some(1), collapsed: Some(true), ..ParaProps::default() }),
            para("Under it", ParaProps::default()),
            para("Body can't fold", ParaProps { collapsed: Some(true), ..ParaProps::default() }),
        ];
        assert_eq!(folded(&doc), vec![false, true, true, true, false, false, true, true]);
        // A collapsed style folds its headings; direct formatting switches it off again.
        let mut styled = doc.clone();
        styled.body = doc
            .body
            .iter()
            .map(|b| match b.as_para() {
                Some(p) => {
                    let mut p = p.clone();
                    p.props.collapsed = None;
                    para_block(p)
                }
                None => b.clone(),
            })
            .collect();
        assert!(folded(&styled).is_empty());
        if let Some(s) = styled.styles.styles.iter_mut().find(|s| s.id == "Heading2") {
            s.para.collapsed = Some(true);
        }
        assert_eq!(folded(&styled), vec![false, false, false, false, false, false, false, false]);
        styled.body.insert(4, para("Under Part", ParaProps::default()));
        assert_eq!(folded(&styled), vec![false, false, false, false, true, false, false, false, false]);
    }
}
