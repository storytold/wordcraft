//! Hide or show the footer (usually the page number) on one page.
//!
//! Only constructs that DOCX and ODT both keep are used, so the result survives a save:
//! - the first page of a section uses *Different First Page* with its own first-page footer
//!   (empty to hide it);
//! - any other page is put alone in a section of its own (next-page section breaks before and
//!   after it) whose footer is its own part, empty to hide it, while the section after it gets
//!   an explicit copy of the footer the page used to show, so every other page, and the page
//!   numbers, stay exactly as they were.
//!
//! A section break lives on the paragraph that ends a section, so the breaks go on the last
//! paragraph before the page and on the last paragraph on it; no empty paragraph is added. A
//! manual page break at the very end of such a paragraph is taken out, the section break being
//! a page break already (both together leave a blank page in Word and ODT readers).
//!
//! A page that starts or ends in the middle of a paragraph, or next to a table, has no paragraph
//! boundary to break at: the command is refused with the reason (and nothing changes). The one
//! exception is a page that starts or ends right after a manual page break typed inside a
//! paragraph (not a list item or a heading): the paragraph is cut there, which is what Word
//! makes of a page break followed by text.
//!
//! The command checks its own result by laying the document out again: the page count, the
//! page numbers and the header and footer of every other page must be unchanged, else it fails
//! and the session puts the document back.

use std::collections::BTreeMap;
use std::sync::Arc;

use wordcraft_doc::para::PAGE_BREAK;
use wordcraft_doc::section::{HeaderSet, SectionProps, SectionStart};
use wordcraft_doc::{Blocks, Document};
use wordcraft_layout::{DocLayout, HfSlot, Page, ParaLayout, Placed};

use super::header_footer::page_number_para;
use super::{Block, CmdError, CmdResult, Paragraph, PartKind, Path, Pos, Session, StoryRef, json, p, para_block};

/// Deepest table nesting looked into when deciding whether a footer is empty.
const BLANK_DEPTH: usize = 8;

/// What a page shows in the map of footers (see [`page_footers`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageFooterInfo {
    /// Page number in the layout, from 1.
    pub page: usize,
    /// The number the page displays (after section restarts and number formats).
    pub number: u32,
    /// The page shows a footer with something in it.
    pub footer: bool,
    /// The page shows a header with something in it.
    pub header: bool,
    /// Why the footer of this page cannot be hidden or shown on its own (`layout.pageFooterVisible`
    /// refuses it), if so.
    pub blocked: Option<String>,
}

/// Whether `blocks` show nothing: paragraphs without text or objects, and tables of such.
pub fn blocks_blank(blocks: &[Arc<Block>]) -> bool {
    blank_at(blocks, 0)
}

/// Whether the blocks hold nothing visible: only whitespace, no objects, and tables whose cells are blank too, down to `BLANK_DEPTH`.
fn blank_at(blocks: &[Arc<Block>], depth: usize) -> bool {
    blocks.iter().all(|b| match &**b {
        Block::Para(p) => p.objects.is_empty() && p.text.chars().all(char::is_whitespace),
        Block::Table(t) => depth < BLANK_DEPTH && t.rows.iter().all(|r| r.cells.iter().all(|c| blank_at(&c.blocks, depth + 1))),
    })
}

/// The blocks of the part with this id, if the id is set and the part exists.
fn blocks_of(doc: &Document, id: Option<u32>) -> Option<&Blocks> {
    id.and_then(|id| doc.parts.get(&id)).map(|part| &part.blocks)
}

/// Whether `page` shows a footer with something in it.
pub fn footer_visible(doc: &Document, page: &Page) -> bool {
    blocks_of(doc, page.footer_story).is_some_and(|b| !blocks_blank(b))
}

/// Whether `page` shows a header with something in it.
pub fn header_visible(doc: &Document, page: &Page) -> bool {
    blocks_of(doc, page.header_story).is_some_and(|b| !blocks_blank(b))
}

/// Every page of the layout with whether it shows a header and a footer: the map the footer
/// panel is drawn from.
pub fn page_footers(s: &mut Session) -> Vec<PageFooterInfo> {
    let layout = s.layout();
    (0..layout.pages.len()).filter_map(|i| info_of(&s.doc, &layout, i)).collect()
}

/// [`page_footers`] for the page with 0-based index `idx` only (the right-click menu's).
pub fn page_footer_info(s: &mut Session, idx: usize) -> Option<PageFooterInfo> {
    let layout = s.layout();
    info_of(&s.doc, &layout, idx)
}

/// The footer and header state of page `i`, and why its footer cannot be changed alone, if so.
fn info_of(doc: &Document, layout: &DocLayout, i: usize) -> Option<PageFooterInfo> {
    let pg = layout.pages.get(i)?;
    let blocked = match PageUse::of(doc, layout, i) {
        _ if first_of_section(layout, i) => None,
        Some(u) => u.refusal(doc, i + 1).err().map(reason),
        None => Some(reason(no_text(i + 1))),
    };
    Some(PageFooterInfo { page: i + 1, number: pg.number, footer: footer_visible(doc, pg), header: header_visible(doc, pg), blocked })
}

/// What a page holds of the body, by top-level block.
struct PageUse {
    first: usize,
    last: usize,
    /// The page starts in the middle of block `first` (a paragraph carried over, a table split).
    starts_mid: bool,
    /// Block `last` goes on onto the next page.
    ends_mid: bool,
    /// Where block `first` can be split so the page starts a paragraph: the page begins right
    /// after a manual page break inside the paragraph. Byte offset in the paragraph.
    split_before: Option<usize>,
    /// Likewise for the end of the page: block `last` goes on after a manual page break.
    split_after: Option<usize>,
}

#[derive(Default)]
struct BlockUse {
    table: bool,
    from: usize,
    to: usize,
    total: usize,
    seen: bool,
    /// The paragraph layout with the page's first line of the block, and that line.
    first_line: Option<(usize, Arc<ParaLayout>)>,
    /// The paragraph layout with the page's last line of the block, and the line after it.
    last_line: Option<(usize, Arc<ParaLayout>)>,
}

/// The top-level body blocks with something placed on `page`.
fn blocks_on(page: &Page) -> BTreeMap<usize, BlockUse> {
    let mut map: BTreeMap<usize, BlockUse> = BTreeMap::new();
    for it in &page.items {
        match it {
            Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } => {
                let Some(&b) = path.0.first() else { continue };
                let e = map.entry(b as usize).or_default();
                if path.0.len() > 1 {
                    e.table = true;
                } else {
                    if !e.seen || *l0 < e.from {
                        e.first_line = Some((*l0, Arc::clone(para)));
                    }
                    if !e.seen || *l1 > e.to {
                        e.last_line = Some((*l1, Arc::clone(para)));
                    }
                    e.from = if e.seen { e.from.min(*l0) } else { *l0 };
                    e.to = e.to.max(*l1);
                    e.total = para.lines.len();
                    e.seen = true;
                }
            }
            Placed::Cell { story: StoryRef::Body, table, .. } => {
                if let Some(&b) = table.0.first() {
                    map.entry(b as usize).or_default().table = true;
                }
            }
            _ => {}
        }
    }
    map
}

/// Whether paragraph `block` may be cut in two at byte `off`, just after a manual page break:
/// the two halves are what Word makes of a page break followed by text. Not a list item or
/// a heading, whose second half would be a second item or heading.
fn can_split_after_break(doc: &Document, block: usize, off: usize) -> bool {
    let Some(p) = doc.para(StoryRef::Body, &Path::top(block)) else { return false };
    let heading = p.props.style.as_deref().is_some_and(|st| st.starts_with("Heading") || st == "Title");
    p.text.get(..off).is_some_and(|before| before.ends_with(PAGE_BREAK))
        && off < p.text.len()
        && p.props.numbering.is_none()
        && p.props.outline_level.is_none()
        && !heading
}

impl PageUse {
    /// How page `idx` relates to the paragraphs and tables that touch it, or `None` when the page has no text of its own.
    fn of(doc: &Document, layout: &DocLayout, idx: usize) -> Option<PageUse> {
        let here = blocks_on(layout.pages.get(idx)?);
        let (&first, &last) = (here.keys().next()?, here.keys().next_back()?);
        let before = idx.checked_sub(1).and_then(|i| layout.pages.get(i)).map(blocks_on).unwrap_or_default();
        let after = layout.pages.get(idx + 1).map(blocks_on).unwrap_or_default();
        let (f, l) = (here.get(&first)?, here.get(&last)?);
        // A paragraph carried over from the page before, or a table that is on both pages.
        let starts_mid = if f.table { before.contains_key(&first) } else { f.from > 0 || before.contains_key(&first) };
        let ends_mid = if l.table { after.contains_key(&last) } else { l.to < l.total || after.contains_key(&last) };
        let split_before = f
            .first_line
            .as_ref()
            .filter(|_| starts_mid && !f.table && f.from > 0)
            .and_then(|(l0, para)| para.lines.get(*l0).map(|line| line.start))
            .filter(|off| can_split_after_break(doc, first, *off));
        let split_after = l
            .last_line
            .as_ref()
            .filter(|_| ends_mid && !l.table && l.to > 0 && l.to < l.total)
            .and_then(|(l1, para)| l1.checked_sub(1).and_then(|k| para.lines.get(k)).map(|line| line.stop))
            .filter(|off| can_split_after_break(doc, last, *off));
        Some(PageUse { first, last, starts_mid, ends_mid, split_before, split_after })
    }

    /// Why a section break can't go before and after this page, if so.
    fn refusal(&self, doc: &Document, n: usize) -> Result<(), CmdError> {
        if self.starts_mid && self.split_before.is_none() {
            return Err(CmdError::Disabled(format!(
                "page {n} starts in the middle of a paragraph or table, and section breaks go between paragraphs: put the paragraph that starts it on a page of its own (Page Break Before) first"
            )));
        }
        if self.ends_mid && self.split_after.is_none() {
            return Err(CmdError::Disabled(format!(
                "page {n} ends in the middle of a paragraph or table, and section breaks go between paragraphs: put a page break after that paragraph first"
            )));
        }
        let sections = doc.sections();
        let sidx = doc.section_index_of(self.first);
        if doc.section_index_of(self.last) != sidx {
            return Err(CmdError::Disabled(format!("page {n} holds a continuous section break: make it a next-page break first")));
        }
        let start = if sidx == 0 { 0 } else { sections.get(sidx - 1).map_or(0, |(e, _)| e + 1) };
        let end = sections.get(sidx).map_or(self.last, |(e, _)| *e);
        let para = |b: usize| matches!(doc.body.get(b).map(|x| &**x), Some(Block::Para(_)));
        if self.first > start && self.split_before.is_none() && !para(self.first - 1) {
            return Err(CmdError::Disabled(format!("page {n} follows a table, and a section break needs a paragraph before it")));
        }
        if self.last < end && self.split_after.is_none() && !para(self.last) {
            return Err(CmdError::Disabled(format!("page {n} ends with a table, and a section break needs a paragraph to hold it")));
        }
        Ok(())
    }
}

/// The reason an error gives, without the command's name.
fn reason(e: CmdError) -> String {
    match e {
        CmdError::Disabled(why) | CmdError::Failed(why) | CmdError::Params(why) => why,
        other => other.to_string(),
    }
}

/// The refusal for a page with no text of its own, where a section break cannot be placed.
fn no_text(n: usize) -> CmdError {
    CmdError::Disabled(format!("page {n} has no text of its own, so there is nothing to put a section break around"))
}

/// Whether page `idx` is the first page of its section.
fn first_of_section(layout: &DocLayout, idx: usize) -> bool {
    match idx.checked_sub(1) {
        None => true,
        Some(prev) => layout.pages.get(prev).zip(layout.pages.get(idx)).is_none_or(|(a, b)| a.section != b.section),
    }
}

/// The part id stored in one slot (first, even or default) of a header or footer set.
fn slot_get(set: &HeaderSet, slot: HfSlot) -> Option<u32> {
    match slot {
        HfSlot::First => set.first,
        HfSlot::Even => set.even,
        HfSlot::Default => set.default,
    }
}

/// Stores a part id in one slot (first, even or default) of a header or footer set.
fn slot_set(set: &mut HeaderSet, slot: HfSlot, id: Option<u32>) {
    match slot {
        HfSlot::First => set.first = id,
        HfSlot::Even => set.even = id,
        HfSlot::Default => set.default = id,
    }
}

/// The headers (or footers) section `sidx` shows: its own, else those it inherits.
fn linked(doc: &Document, sidx: usize, footer: bool) -> HeaderSet {
    let mut acc = HeaderSet::default();
    for (_, sec) in doc.sections().into_iter().take(sidx.saturating_add(1)) {
        let own = if footer { sec.footers } else { sec.headers };
        acc = HeaderSet { default: own.default.or(acc.default), first: own.first.or(acc.first), even: own.even.or(acc.even) };
    }
    acc
}

/// The footer slot as the layout reads it: an even page without its own even footer shows the default.
fn linked_slot(doc: &Document, sidx: usize, slot: HfSlot) -> Option<u32> {
    let set = linked(doc, sidx, true);
    match slot {
        HfSlot::Even => set.even.or(set.default),
        other => slot_get(&set, other),
    }
}

/// A section like `sec` for the pages that follow a break made here: same page setup, nothing
/// of its own to show, no restart of the numbering and no separate first page.
fn plain_copy(sec: &SectionProps) -> SectionProps {
    let mut c = sec.clone();
    c.headers = HeaderSet::default();
    c.footers = HeaderSet::default();
    c.title_page = false;
    c.page_num_start = None;
    c.start = SectionStart::NextPage;
    c.fmt_change = None;
    c
}

/// A footer with one empty paragraph in the "Footer" style.
fn blank_footer() -> Blocks {
    vec![para_block(Paragraph::new().styled("Footer"))]
}

/// Adds a part of the given kind to the document, mapping the error to a command error.
fn add_part(s: &mut Session, kind: PartKind, blocks: Blocks) -> Result<u32, CmdError> {
    Ok(s.doc.try_add_part(kind, blocks)?)
}

/// Ends a section on paragraph `block`. A page break at the very end of that paragraph is
/// taken out: the next-page break of the section does the same job, and both together make
/// Word and ODT readers add a blank page.
fn set_mark(s: &mut Session, block: usize, props: SectionProps) -> Result<(), CmdError> {
    let para = s.doc.para_mut(StoryRef::Body, &Path::top(block))?;
    if para.text.ends_with(PAGE_BREAK) {
        let end = para.text.len();
        para.delete(end.saturating_sub(PAGE_BREAK.len_utf8()), end)?;
    }
    para.section = Some(Box::new(props));
    Ok(())
}

/// Takes part `id` out of the document when no section names it any more.
fn drop_if_unreferenced(doc: &mut Document, id: Option<u32>) {
    let Some(id) = id else { return };
    let used =
        doc.sections().iter().any(|(_, sec)| [sec.headers, sec.footers].iter().any(|set| [set.default, set.first, set.even].contains(&Some(id))));
    if !used {
        doc.parts.remove(&id);
    }
}

/// The blocks of a header or footer that show something.
fn shown_blocks(doc: &Document, id: Option<u32>) -> Option<Blocks> {
    blocks_of(doc, id).filter(|b| !blocks_blank(b)).cloned()
}

/// What each page shows: its number, header and footer. Equal before and after a change for
/// every page but the one changed.
type View = Vec<(u32, Option<Blocks>, Option<Blocks>)>;

/// A snapshot of every page: its number, shown header blocks and shown footer blocks, used to check that nothing else changed.
fn view_of(doc: &Document, layout: &DocLayout) -> View {
    layout.pages.iter().map(|pg| (pg.number, shown_blocks(doc, pg.header_story), shown_blocks(doc, pg.footer_story))).collect()
}

/// Checks the document laid out again: page `idx` shows a footer or not as wanted, and nothing
/// else about the pages changed.
fn verify(s: &mut Session, before: &View, idx: usize, want_visible: bool) -> Result<(), CmdError> {
    s.touch();
    let layout = s.layout();
    let after = view_of(&s.doc, &layout);
    let shown = layout.pages.get(idx).is_some_and(|pg| footer_visible(&s.doc, pg));
    let same = before.len() == after.len() && before.iter().zip(&after).enumerate().all(|(i, (a, b))| i == idx || a == b);
    if shown == want_visible && same && before.get(idx).zip(after.get(idx)).is_some_and(|(a, b)| a.0 == b.0 && a.1 == b.1) {
        return Ok(());
    }
    Err(CmdError::Failed(format!(
        "page {} cannot be changed alone in this document without moving other pages or their headers and footers",
        idx + 1
    )))
}

/// `layout.pageFooterVisible`: hide or show the footer on one page of the current layout.
pub(crate) fn page_footer_visible(s: &mut Session, v: &serde_json::Value) -> CmdResult {
    let layout = s.layout();
    let given = v.get("page").is_some_and(|x| !x.is_null());
    let idx = match p::u64(v, "page") {
        None if given => return Err(CmdError::Params("page must be a whole number from 1".into())),
        Some(n) => {
            let n = usize::try_from(n).unwrap_or(usize::MAX);
            if n == 0 || n > layout.pages.len() {
                return Err(CmdError::Params(format!("no page {n}: pages are numbered from 1 to {}", layout.pages.len())));
            }
            n - 1
        }
        None => {
            let focus = s.sel.focus.clone();
            let at = if focus.story == StoryRef::Body { layout.page_of(&focus) } else { Some(s.page_hint) };
            at.unwrap_or(0).min(layout.pages.len().saturating_sub(1))
        }
    };
    let Some(pg) = layout.pages.get(idx) else { return Err(CmdError::Failed("the document has no pages".into())) };
    let now = footer_visible(&s.doc, pg);
    let want = p::bool(v, "visible").unwrap_or(!now);
    let result = |changed: bool| Ok(json!({"page": idx + 1, "visible": want, "changed": changed}));
    if want == now {
        return result(false);
    }
    let before = view_of(&s.doc, &layout);
    if want {
        show(s, &layout, idx, &before)?;
    } else {
        let orig = blocks_of(&s.doc, pg.footer_story).cloned().unwrap_or_else(blank_footer);
        set_page_footer(s, &layout, idx, blank_footer(), orig)?;
        verify(s, &before, idx, false)?;
    }
    result(true)
}

/// Shows the footer on page `idx`: it gets the neighbour's footer, or a page number, and the result is verified by laying out again.
fn show(s: &mut Session, layout: &DocLayout, idx: usize, before: &View) -> Result<(), CmdError> {
    let Some(pg) = layout.pages.get(idx) else { return Ok(()) };
    let content = match neighbour_footer(&s.doc, layout, idx) {
        Some(blocks) => blocks,
        None => vec![para_block(page_number_para("Footer")?)],
    };
    let orig = blocks_of(&s.doc, pg.footer_story).cloned().unwrap_or_else(blank_footer);
    // First the simple way back: undo what hiding did, which leaves the file as it was.
    let backup = s.doc.clone();
    let undone = if first_of_section(layout, idx) { drop_first_page(s, layout, idx) } else { relink(s, layout, idx) };
    if undone {
        if verify(s, before, idx, true).is_ok() {
            return Ok(());
        }
        s.doc = backup;
    }
    set_page_footer(s, layout, idx, content, orig)?;
    verify(s, before, idx, true)
}

/// The footer of the closest page that shows one in the same slot (else any), to show on a page
/// that has none.
fn neighbour_footer(doc: &Document, layout: &DocLayout, idx: usize) -> Option<Blocks> {
    let slot = layout.pages.get(idx)?.hf_slot;
    let order = || (0..idx).rev().chain(idx + 1..layout.pages.len());
    let pick = |same_slot: bool| order().filter_map(|j| layout.pages.get(j)).find(|pg| footer_visible(doc, pg) && (!same_slot || pg.hf_slot == slot));
    let pg = pick(true).or_else(|| pick(false))?;
    blocks_of(doc, pg.footer_story).cloned()
}

/// Gives page `idx` the footer `content`, leaving every other page as it was. `orig` is what
/// the page showed, which the pages after it in the same section must go on showing.
fn set_page_footer(s: &mut Session, layout: &DocLayout, idx: usize, content: Blocks, orig: Blocks) -> Result<(), CmdError> {
    if first_of_section(layout, idx) { set_first_page(s, layout, idx, content) } else { set_inner_page(s, layout, idx, content, orig) }
}

/// The first page of a section: Different First Page and an own first-page footer.
fn set_first_page(s: &mut Session, layout: &DocLayout, idx: usize, content: Blocks) -> Result<(), CmdError> {
    let Some(pg) = layout.pages.get(idx) else { return Ok(()) };
    let sections: Vec<(usize, SectionProps)> = s.doc.sections().into_iter().map(|(e, p)| (e, p.clone())).collect();
    let sidx = pg.section.min(sections.len().saturating_sub(1));
    let Some((end, sec)) = sections.get(sidx).cloned() else { return Ok(()) };
    let old_first_footer = linked(&s.doc, sidx, true).first;
    // With a first-page slot the page would show that slot's header: keep the one it shows now.
    if !sec.title_page {
        let first_header = linked(&s.doc, sidx, false).first;
        let shown = pg.header_story.filter(|id| s.doc.parts.contains_key(id));
        if first_header != shown {
            let blocks = blocks_of(&s.doc, shown).cloned().unwrap_or_else(|| vec![para_block(Paragraph::new().styled("Header"))]);
            let id = add_part(s, PartKind::Header, blocks)?;
            s.doc.section_mut(end).headers.first = Some(id);
        }
    }
    let id = add_part(s, PartKind::Footer, content)?;
    let replaced = {
        let target = s.doc.section_mut(end);
        let replaced = target.footers.first.replace(id);
        target.title_page = true;
        replaced
    };
    // Later sections that read the first-page footer from this one keep what they showed.
    for (n_end, n) in sections.iter().skip(sidx + 1) {
        if n.footers.first.is_some() {
            break;
        }
        if n.title_page {
            let blocks = blocks_of(&s.doc, old_first_footer).cloned().unwrap_or_else(blank_footer);
            let copy = add_part(s, PartKind::Footer, blocks)?;
            s.doc.section_mut(*n_end).footers.first = Some(copy);
            break;
        }
    }
    drop_if_unreferenced(&mut s.doc, replaced);
    Ok(())
}

/// Any other page: alone in a section of its own, see the module docs.
fn set_inner_page(s: &mut Session, layout: &DocLayout, idx: usize, content: Blocks, orig: Blocks) -> Result<(), CmdError> {
    let n = idx + 1;
    let Some(pg) = layout.pages.get(idx) else { return Ok(()) };
    let usage = PageUse::of(&s.doc, layout, idx).ok_or_else(|| no_text(n))?;
    usage.refusal(&s.doc, n)?;
    let slot = pg.hf_slot;
    if slot == HfSlot::First {
        return Err(CmdError::Failed(format!("page {n} uses a first-page footer but does not start its section")));
    }
    let (mut f, mut l) = (usage.first, usage.last);
    // A page that starts or ends right after a manual page break inside a paragraph: the
    // paragraph is cut there, as a page break followed by a new paragraph. The end goes first
    // so the offset of the start stays valid when both are in one paragraph.
    if let Some(off) = usage.split_after {
        cut_after_break(s, l, off)?;
    }
    if let Some(off) = usage.split_before {
        cut_after_break(s, f, off)?;
        f += 1;
        l += 1;
    }
    let sections: Vec<(usize, SectionProps)> = s.doc.sections().into_iter().map(|(e, p)| (e, p.clone())).collect();
    let sidx = s.doc.section_index_of(f);
    let Some((end, sec)) = sections.get(sidx).cloned() else { return Err(CmdError::Failed("page without a section".into())) };
    let start = if sidx == 0 { 0 } else { sections.get(sidx - 1).map_or(0, |(e, _)| e + 1) };

    let mut mid = if f > start { plain_copy(&sec) } else { sec.clone() };
    let replaced = slot_get(&mid.footers, slot);
    let mid_footer = add_part(s, PartKind::Footer, content)?;
    slot_set(&mut mid.footers, slot, Some(mid_footer));
    if f > start {
        set_mark(s, f - 1, sec.clone())?;
    }
    if l < end {
        set_mark(s, l, mid)?;
        let mut after = plain_copy(&sec);
        let copy = add_part(s, PartKind::Footer, orig)?;
        slot_set(&mut after.footers, slot, Some(copy));
        *s.doc.section_mut(end) = after;
    } else {
        *s.doc.section_mut(end) = mid;
        pin_next_section(s, &sections, sidx, slot, orig)?;
    }
    drop_if_unreferenced(&mut s.doc, replaced);
    Ok(())
}

/// Cuts paragraph `block` in two at byte `off`; the second half starts a page, so it keeps no
/// Page Break Before of its own.
fn cut_after_break(s: &mut Session, block: usize, off: usize) -> Result<(), CmdError> {
    s.doc.split_paragraph(&Pos::body(block, off))?;
    s.doc.para_mut(StoryRef::Body, &Path::top(block + 1))?.props.page_break_before = None;
    Ok(())
}

/// The section after `sidx` that read `slot` from the one before it gets its own copy of `orig`,
/// so a footer changed in the section before it no longer reaches it.
fn pin_next_section(s: &mut Session, sections: &[(usize, SectionProps)], sidx: usize, slot: HfSlot, orig: Blocks) -> Result<(), CmdError> {
    if let Some((n_end, n)) = sections.get(sidx + 1)
        && slot_get(&n.footers, slot).is_none()
    {
        let copy = add_part(s, PartKind::Footer, orig)?;
        slot_set(&mut s.doc.section_mut(*n_end).footers, slot, Some(copy));
    }
    Ok(())
}

/// Hiding a first page left Different First Page on with an empty first-page footer: turn both
/// off again when the page then looks the same apart from its footer. Whether anything changed.
fn drop_first_page(s: &mut Session, layout: &DocLayout, idx: usize) -> bool {
    let Some(pg) = layout.pages.get(idx) else { return false };
    let sections = s.doc.sections();
    let sidx = pg.section.min(sections.len().saturating_sub(1));
    let Some((end, sec)) = sections.get(sidx).map(|(e, p)| (*e, (*p).clone())) else { return false };
    let own = sec.footers.first;
    if !sec.title_page || !blocks_of(&s.doc, own).is_some_and(|b| blocks_blank(b)) {
        return false;
    }
    let heads = linked(&s.doc, sidx, false);
    if blocks_of(&s.doc, heads.first) != blocks_of(&s.doc, heads.default) {
        return false;
    }
    let own_head = sec.headers.first;
    let target = s.doc.section_mut(end);
    target.title_page = false;
    target.footers.first = None;
    if own_head.is_some() {
        target.headers.first = None;
    }
    drop_if_unreferenced(&mut s.doc, own);
    drop_if_unreferenced(&mut s.doc, own_head);
    true
}

/// A page hidden by an own empty footer in a section of its own shows the footer of the section
/// before again, and the section after it stops carrying its copy. Whether anything changed.
fn relink(s: &mut Session, layout: &DocLayout, idx: usize) -> bool {
    let Some(pg) = layout.pages.get(idx) else { return false };
    let slot = pg.hf_slot;
    let sections: Vec<(usize, SectionProps)> = s.doc.sections().into_iter().map(|(e, p)| (e, p.clone())).collect();
    let sidx = pg.section.min(sections.len().saturating_sub(1));
    // Only when the page is the whole section: the others must not change with it.
    if sidx == 0 || layout.pages.iter().filter(|p| p.section == sidx).count() != 1 {
        return false;
    }
    let Some((end, sec)) = sections.get(sidx).cloned() else { return false };
    let own = slot_get(&sec.footers, slot);
    if !blocks_of(&s.doc, own).is_some_and(|b| blocks_blank(b)) {
        return false;
    }
    let inherited = linked_slot(&s.doc, sidx - 1, slot);
    let Some(shown) = blocks_of(&s.doc, inherited).filter(|b| !blocks_blank(b)).cloned() else { return false };
    slot_set(&mut s.doc.section_mut(end).footers, slot, None);
    let mut dropped = vec![own];
    if let Some((n_end, n)) = sections.get(sidx + 1) {
        let n_own = slot_get(&n.footers, slot);
        if blocks_of(&s.doc, n_own) == Some(&shown) {
            slot_set(&mut s.doc.section_mut(*n_end).footers, slot, None);
            dropped.push(n_own);
        }
    }
    for id in dropped {
        drop_if_unreferenced(&mut s.doc, id);
    }
    true
}
#[cfg(test)]
mod tests;
