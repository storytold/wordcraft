//! Run-time guard for chat members: a policy for cooperating agents, not a sandbox (see
//! `chat_gate`).
//!
//! After a member's command the document may differ from the snapshot taken before it only by
//! changes tracked under the member's own name, plus formatting from a small allowed set (which
//! the caller announces in the chat). Anything else (untracked text, structure, another author's
//! revision marks, comments of others, document-wide settings, other formatting) is refused, and
//! [`crate::chat_gate::run_as_member`] puts the snapshot back.
//!
//! The check compares "views" of the two documents. The member view takes the member's tracked
//! changes away (its insertions dropped, its deletion marks cleared, its inserted paragraph marks
//! joined, its own comments ignored); everything left must be equal. A paragraph joined that way
//! keeps the properties of EVERY piece that still holds text not inserted by the member (both
//! halves of an owner paragraph split by a member's Enter), so the Enter cannot hide a change to
//! the owner's paragraph. A member's own new paragraph (only its tracked text) is free.
//! Accept/reject commands are checked with the accepted or rejected view instead: they may only
//! resolve revisions, and never in a comment by someone else.
//!
//! Every comparison below destructures the document structs field by field (no `..`): a new
//! upstream field breaks the build here until someone decides how the guard treats it.
//!
//! The guard fails closed: what the views cannot see is never ignored. A document with tables
//! nested deeper than [`MAX_DEPTH`] levels gets no change from a member at all.

use std::collections::{BTreeMap, BTreeSet};

use wordcraft_doc::numbering::{AbstractNum, Num, Numbering};
use wordcraft_doc::para::{InlineObject, OBJ, Paragraph};
use wordcraft_doc::{Block, Blocks, CharProps, Comment, Document, ParaProps, Part, PartKind, RevisionKind, SectionProps, Table};

/// Prefix of every refusal.
pub const REFUSED: &str = "untracked change refused";

const TEXT: &str = "text, structure, comments or another author's revision marks changed without a tracked change";
const FORMAT: &str = "formatting outside the allowed set (bold, italic, underline, strike, font, size, colour, highlight, sub/superscript; alignment, spacing, indent, paragraph style); lists, borders, shading, tabs, keep/page-break, outline level and direction of the owner's paragraphs are the owner's";
const DOC: &str = "document-wide settings (page setup, sections, styles, theme) are the owner's";
const LIST: &str = "list definitions (numbers, bullets, label text) are the owner's";
const JUDGE: &str = "accept/reject did more than resolve tracked changes";
const COMMENT: &str = "the text of a comment by someone else cannot change (only your own comments and replies)";
const DEEP: &str = "tables nested more than 16 levels deep: the guard cannot check them, so it refuses every change to this document";
/// Reason for a command that is not supposed to edit and changed the document or the undo stack.
pub const PURE: &str = "this command must not change the document, and it did";

/// The deepest table nesting the views see: blocks in a cell of a table that is nested deeper
/// are not in them (the same limit as [`Document::para_paths`]).
pub const MAX_DEPTH: usize = 16;

/// What a member command did to the document.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing, or only changes tracked under the member's name.
    Clean,
    /// Only formatting from the allowed set (to announce in the chat).
    Format,
    /// Anything else: put the snapshot back and answer with this reason.
    Refuse(&'static str),
}

/// A command that is not accept/reject, run by `handle`.
pub fn check_member(before: &Document, after: &Document, handle: &str) -> Verdict {
    if too_deep(before) || too_deep(after) {
        return Verdict::Refuse(DEEP);
    }
    if others_comments_changed(before, after, handle) {
        return Verdict::Refuse(COMMENT);
    }
    let own: BTreeSet<u32> = before.comments.iter().chain(after.comments.iter()).filter(|(_, c)| c.author == handle).map(|(k, _)| *k).collect();
    let b = view(before, Lens::Member(handle), &own);
    let a = view(after, Lens::Member(handle), &own);
    if !b.masked(Mask::All).same(&a.masked(Mask::All)) {
        return Verdict::Refuse(TEXT);
    }
    if !b.masked(Mask::Allowed).same(&a.masked(Mask::Allowed)) {
        return Verdict::Refuse(FORMAT);
    }
    if !same_settings(before, after) {
        return Verdict::Refuse(DOC);
    }
    if !lists_kept(&before.numbering, &after.numbering) {
        return Verdict::Refuse(LIST);
    }
    // New list definitions (a list on the member's own paragraph) are announced.
    if b.same(&a) && before.numbering == after.numbering { Verdict::Clean } else { Verdict::Format }
}

/// The text of a comment is shown plain everywhere (balloon, pane, review.comments): even a
/// tracked change there would be invisible. Comments of others stay exactly as they were.
fn others_comments_changed(before: &Document, after: &Document, handle: &str) -> bool {
    before.comments.values().filter(|c| c.author != handle).any(|c| before.parts.get(&c.part) != after.parts.get(&c.part))
}

/// Some story holds blocks the views do not see (tables nested deeper than [`MAX_DEPTH`]).
fn too_deep(doc: &Document) -> bool {
    hides(&doc.body, 0) || doc.parts.values().any(|p| hides(&p.blocks, 0))
}

/// [`Viewer::blocks`] at `depth` leaves out a block of `bl`.
fn hides(bl: &Blocks, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return !bl.is_empty();
    }
    bl.iter().any(|b| match &**b {
        Block::Para(_) => false,
        Block::Table(t) => t.rows.iter().flat_map(|r| r.cells.iter()).any(|c| hides(&c.blocks, depth.saturating_add(1))),
    })
}

/// A command that is not supposed to edit (`.pure()`): any change to the document is refused.
pub fn check_unchanged(before: &Document, after: &Document) -> Verdict {
    if before == after { Verdict::Clean } else { Verdict::Refuse(PURE) }
}

/// Tracked characters per author in every story (body, headers, footers, notes, comments):
/// inserted and deleted characters, an inserted or deleted paragraph mark counts as one. Counting
/// characters (not changes) makes a partial accept/reject show too. It sees what the views see
/// ([`MAX_DEPTH`]); [`check_judging`] refuses a document that holds more.
fn tally(doc: &Document) -> BTreeMap<String, usize> {
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    let author = |r: u32| doc.revisions.get(r as usize).map(|x| x.author.clone()).unwrap_or_else(|| "?".into());
    let mut count = |r: Option<u32>, n: usize| {
        if let Some(r) = r
            && n > 0
        {
            let e = out.entry(author(r)).or_default();
            *e = e.saturating_add(n);
        }
    };
    for story in std::iter::once(wordcraft_doc::StoryRef::Body).chain(doc.parts.keys().map(|k| wordcraft_doc::StoryRef::Part(*k))) {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            for (r, c) in p.run_ranges() {
                let n = p.text.get(r).map(|t| t.chars().count()).unwrap_or(0);
                count(c.ins, n);
                count(c.del, n);
            }
            count(p.mark.ins, 1);
            count(p.mark.del, 1);
        }
    }
    out
}

/// Tracked characters an accept/reject resolved: how many, and whose (sorted).
pub fn judged(before: &Document, after: &Document) -> (usize, Vec<String>) {
    let (b, a) = (tally(before), tally(after));
    let mut n: usize = 0;
    let mut authors = Vec::new();
    for (who, k) in b {
        let gone = k.saturating_sub(a.get(&who).copied().unwrap_or(0));
        if gone > 0 {
            n = n.saturating_add(gone);
            authors.push(who);
        }
    }
    (n, authors)
}

/// Paragraph marks inserted as tracked changes, in every story.
pub fn inserted_paragraphs(doc: &Document) -> usize {
    std::iter::once(wordcraft_doc::StoryRef::Body)
        .chain(doc.parts.keys().map(|k| wordcraft_doc::StoryRef::Part(*k)))
        .flat_map(|story| doc.para_paths(story).into_iter().map(move |p| (story, p)))
        .filter(|(story, path)| doc.para(*story, path).is_some_and(|p| p.mark.ins.is_some()))
        .count()
}

/// Comments whose resolved state changed: (author, resolved now).
pub fn resolved_comments(before: &Document, after: &Document) -> Vec<(String, bool)> {
    after
        .comments
        .iter()
        .filter_map(|(k, c)| before.comments.get(k).filter(|b| b.resolved != c.resolved).map(|_| (c.author.clone(), c.resolved)))
        .collect()
}

/// Every list definition of `before` is still there, unchanged (new ones may be added).
fn lists_kept(before: &Numbering, after: &Numbering) -> bool {
    let Numbering { abstracts, nums } = before;
    let Numbering { abstracts: abstracts_after, nums: nums_after } = after;
    let abs_kept = abstracts.iter().all(|x| {
        let AbstractNum { id, name: _, levels: _ } = x;
        abstracts_after.iter().find(|y| y.id == *id) == Some(x)
    });
    let nums_kept = nums.iter().all(|x| {
        let Num { id, abstract_id: _, start_overrides: _ } = x;
        nums_after.iter().find(|y| y.id == *id) == Some(x)
    });
    abs_kept && nums_kept
}

/// `review.accept*` (`accept`) or `review.reject*` run by `handle`: the accepted (or rejected)
/// document must be the same before and after, and comments by others must not change (as in
/// [`check_member`]).
pub fn check_judging(before: &Document, after: &Document, accept: bool, handle: &str) -> Verdict {
    if too_deep(before) || too_deep(after) {
        return Verdict::Refuse(DEEP);
    }
    if others_comments_changed(before, after, handle) {
        return Verdict::Refuse(COMMENT);
    }
    let lens = if accept { Lens::Accepted } else { Lens::Rejected };
    let none = BTreeSet::new();
    let (b, a) = (view(before, lens, &none), view(after, lens, &none));
    if b.same(&a) && same_settings(before, after) && before.numbering == after.numbering { Verdict::Clean } else { Verdict::Refuse(JUDGE) }
}

/// Everything outside the stories, comments, revisions and lists (track-changes state excepted:
/// the gate sets it). The stories and comments are compared by the views, the revisions through
/// the marks that use them, the lists by [`lists_kept`].
fn same_settings(a: &Document, b: &Document) -> bool {
    let Document { body: _, last_section, parts: _, styles, numbering: _, comments: _, revisions: _, settings, core, sources, media, passthrough } =
        a;
    let Document {
        body: _,
        last_section: last_section_b,
        parts: _,
        styles: styles_b,
        numbering: _,
        comments: _,
        revisions: _,
        settings: settings_b,
        core: core_b,
        sources: sources_b,
        media: media_b,
        passthrough: passthrough_b,
    } = b;
    let mut sa = settings.clone();
    let mut sb = settings_b.clone();
    sa.track_changes = false;
    sb.track_changes = false;
    sa == sb
        && last_section == last_section_b
        && styles == styles_b
        && core == core_b
        && sources == sources_b
        && media.keys().eq(media_b.keys())
        && passthrough.keys().eq(passthrough_b.keys())
}

/// A revision as (kind, author, date): indexes into `Document::revisions` differ between copies.
type Rev = Option<(RevisionKind, String, String)>;

#[derive(Clone, Debug, PartialEq)]
struct Key {
    /// Direct formatting without ins/del (masked fields cleared).
    props: CharProps,
    ins: Rev,
    del: Rev,
    /// Hidden after styles are applied (a style can hide text too).
    hidden: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct NRun {
    text: String,
    objects: Vec<InlineObject>,
    key: Key,
}

/// A paragraph's own properties: direct ones, and the list it is in (styles included).
#[derive(Clone, Debug, PartialEq)]
struct NProps {
    props: ParaProps,
    list: Option<(u32, u8)>,
}

#[derive(Clone, Debug, PartialEq)]
struct NPara {
    runs: Vec<NRun>,
    /// One entry per paragraph joined into this one that holds text in this view (the last one
    /// when none does), consecutive duplicates merged.
    props: Vec<NProps>,
    /// Paragraph mark: formatting of the first piece that holds text (the first piece when none
    /// does), revisions of the last piece; hidden on any piece counts.
    mark: Key,
    /// The marks of the later pieces that hold text are as a split leaves them.
    tails: Tails,
    section: Option<Box<SectionProps>>,
}

/// Whether every later piece that holds text has the mark a split gives it (the formatting at
/// the split point, i.e. at the last character of the last earlier piece that holds text that the
/// member did not insert): exactly (`strict`), and leaving the
/// allowed formatting aside (`allowed`). A member who formats such a mark (it formats the list
/// label) is announced or refused like any other formatting.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tails {
    allowed: bool,
    strict: bool,
}

const TAILS_OK: Tails = Tails { allowed: true, strict: true };

#[derive(Clone, Debug, PartialEq)]
enum NBlock {
    Para(Box<NPara>),
    /// Table properties (cells emptied; `None` when masked) and the cells' blocks.
    Table(Option<Box<Table>>, Vec<Vec<Vec<NBlock>>>),
}

#[derive(Debug, PartialEq)]
struct NDoc {
    body: Vec<NBlock>,
    parts: BTreeMap<u32, (PartKind, Vec<NBlock>)>,
    /// Comments not ignored: (author, initials, date, parent, part). Resolving is not compared.
    comments: BTreeMap<u32, (String, String, String, Option<u32>, u32)>,
}

impl NDoc {
    /// Same body, same comments, and every story of `self` unchanged in `after` (new stories are
    /// fine: they are only seen through a reference, which is compared where it sits).
    fn same(&self, after: &NDoc) -> bool {
        self.body == after.body && self.comments == after.comments && self.parts.iter().all(|(id, p)| after.parts.get(id) == Some(p))
    }

    fn masked(&self, m: Mask) -> NDoc {
        NDoc {
            body: mask_blocks(&self.body, m),
            parts: self.parts.iter().map(|(k, (kind, bl))| (*k, (*kind, mask_blocks(bl, m)))).collect(),
            comments: self.comments.clone(),
        }
    }
}

/// Which differences a comparison ignores.
#[derive(Clone, Copy, PartialEq)]
enum Mask {
    /// The allowed formatting (bold, font, alignment…).
    Allowed,
    /// All formatting: text, structure and revision marks only.
    All,
}

/// Whose revision marks a view takes away.
#[derive(Clone, Copy)]
enum Lens<'a> {
    /// The member's own tracked changes (rejected) and comments (ignored).
    Member(&'a str),
    /// Every change accepted.
    Accepted,
    /// Every change rejected.
    Rejected,
}

struct Viewer<'a> {
    doc: &'a Document,
    lens: Lens<'a>,
    /// Comment ids whose anchors and text are ignored.
    skip: &'a BTreeSet<u32>,
}

fn view(doc: &Document, lens: Lens, skip: &BTreeSet<u32>) -> NDoc {
    let v = Viewer { doc, lens, skip };
    let skip_parts: BTreeSet<u32> = doc.comments.iter().filter(|(k, _)| skip.contains(k)).map(|(_, c)| c.part).collect();
    let part = |p: &Part| {
        let Part { kind, blocks } = p;
        (*kind, v.blocks(blocks, 0))
    };
    // `resolved` is not compared: resolving is announced by the gate, not refused.
    let comment = |c: &Comment| {
        let Comment { author, initials, date, parent, resolved: _, part } = c;
        (author.clone(), initials.clone(), date.clone(), *parent, *part)
    };
    NDoc {
        body: v.blocks(&doc.body, 0),
        parts: doc.parts.iter().filter(|(k, _)| !skip_parts.contains(k)).map(|(k, p)| (*k, part(p))).collect(),
        comments: doc.comments.iter().filter(|(k, _)| !skip.contains(k)).map(|(k, c)| (*k, comment(c))).collect(),
    }
}

impl Viewer<'_> {
    fn rev(&self, r: Option<u32>) -> Rev {
        r.map(|i| match self.doc.revisions.get(i as usize) {
            Some(x) => (x.kind, x.author.clone(), x.date.clone()),
            None => (RevisionKind::Insert, format!("?{i}"), String::new()),
        })
    }

    fn mine(&self, r: Option<u32>) -> bool {
        match self.lens {
            Lens::Member(h) => r.and_then(|i| self.doc.revisions.get(i as usize)).is_some_and(|x| x.author == h),
            Lens::Accepted | Lens::Rejected => false,
        }
    }

    /// Text that is not there in this view.
    fn drops(&self, c: &CharProps) -> bool {
        match self.lens {
            Lens::Member(_) => self.mine(c.ins),
            Lens::Accepted => c.del.is_some(),
            Lens::Rejected => c.ins.is_some(),
        }
    }

    /// A paragraph mark that is not there in this view: the paragraph joins the next one.
    fn joins(&self, mark: &CharProps) -> bool {
        match self.lens {
            Lens::Member(_) => self.mine(mark.ins),
            Lens::Accepted => false,
            Lens::Rejected => mark.ins.is_some(),
        }
    }

    fn key(&self, style: Option<&str>, c: &CharProps) -> Key {
        let keeps = matches!(self.lens, Lens::Member(_));
        let ins = if keeps && !self.mine(c.ins) { self.rev(c.ins) } else { None };
        let del = if keeps && !self.mine(c.del) { self.rev(c.del) } else { None };
        let mut props = c.clone();
        props.ins = None;
        props.del = None;
        let hidden = self.doc.styles.resolve_char(style, &props).hidden;
        Key { props, ins, del, hidden }
    }

    fn skips(&self, o: &InlineObject) -> bool {
        matches!(o, InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } if self.skip.contains(id))
    }

    /// The paragraph in this view, whether it holds text in it, and whether it joins the next one.
    fn para(&self, p: &Paragraph) -> (NPara, bool, bool) {
        // `runs` is read through `run_ranges`; `rev` is a cache counter.
        let Paragraph { text: ptext, runs: _, props, mark, objects, section, rev: _ } = p;
        let style = props.style.as_deref();
        let mut spans: Vec<(usize, usize, CharProps)> = p.run_ranges().map(|(r, c)| (r.start, r.end, c.clone())).collect();
        let covered = spans.last().map(|s| s.1).unwrap_or(0);
        if covered < ptext.len() {
            spans.push((covered, ptext.len(), CharProps::default()));
        }
        let mut objects = objects.iter();
        let mut runs: Vec<NRun> = Vec::new();
        for (a, b, c) in spans {
            let drop = self.drops(&c);
            let mut text = String::new();
            let mut objs = Vec::new();
            for ch in ptext.get(a..b).unwrap_or("").chars() {
                if ch == OBJ {
                    let o = objects.next();
                    if drop || o.is_some_and(|o| self.skips(o)) {
                        continue;
                    }
                    objs.extend(o.cloned());
                    text.push(OBJ);
                } else if !drop {
                    text.push(ch);
                }
            }
            if !drop {
                push_run(&mut runs, NRun { text, objects: objs, key: self.key(style, &c) });
            }
        }
        let list = self.doc.styles.resolve_para(props).numbering.filter(|n| n.num != 0).map(|n| (n.num, n.level));
        let holds = !runs.is_empty();
        let np = NPara {
            runs,
            props: vec![NProps { props: props.clone(), list }],
            mark: self.key(style, mark),
            tails: TAILS_OK,
            section: section.clone(),
        };
        (np, holds, self.joins(mark))
    }

    /// Is `tail`'s mark what a split after `before` leaves: the formatting at the end of
    /// `before`, read at its last character the member did not insert (the member may format its
    /// own text freely, so its text cannot set what the owner's mark should look like)?
    fn tail(&self, before: &Paragraph, tail: &Paragraph) -> Tails {
        let style = tail.props.style.as_deref();
        let last_not_mine = before.run_ranges().filter(|(r, c)| !r.is_empty() && !self.mine(c.ins)).last().map(|(_, c)| c);
        let expected = self.key(style, last_not_mine.unwrap_or_else(|| before.props_at(before.len())));
        let got = self.key(style, &tail.mark);
        let looks = |a: &Key, b: &Key| a.props == b.props && a.hidden == b.hidden;
        Tails { allowed: looks(&mask_key(&expected, Mask::Allowed), &mask_key(&got, Mask::Allowed)), strict: looks(&expected, &got) }
    }

    /// The view of `bl`; empty past [`MAX_DEPTH`] (the checks refuse such a document first, see
    /// [`too_deep`]).
    fn blocks(&self, bl: &Blocks, depth: usize) -> Vec<NBlock> {
        let mut out: Vec<NBlock> = Vec::new();
        if depth > MAX_DEPTH {
            return out;
        }
        // The pieces joined so far into the last paragraph of `out`: (properties, holds text).
        let mut group: Vec<(NProps, bool)> = Vec::new();
        // The group's mark formatting comes from a piece that holds text.
        let mut mark_from_text = false;
        // The last earlier piece of the group that holds text: a split leaves its end's
        // formatting on the later marks (a member's empty paragraph in between does not count).
        let mut last_text: Option<&Paragraph> = None;
        let mut join = false;
        for b in bl {
            match &**b {
                Block::Para(p) => {
                    let (mut np, holds, next) = self.para(p);
                    let own = np.props.pop();
                    match out.last_mut() {
                        // The joined paragraph keeps the mark revisions and section of the last
                        // piece (the split gave those to the new paragraph).
                        Some(NBlock::Para(prev)) if join => {
                            for r in np.runs {
                                push_run(&mut prev.runs, r);
                            }
                            prev.section = np.section;
                            if holds && !mark_from_text {
                                // A member Enter at the start of the owner's paragraph: its mark
                                // is this piece's.
                                prev.mark.props = np.mark.props.clone();
                                mark_from_text = true;
                            } else if holds && let Some(before) = last_text {
                                let t = self.tail(before, p);
                                prev.tails.allowed &= t.allowed;
                                prev.tails.strict &= t.strict;
                            }
                            prev.mark.ins = np.mark.ins;
                            prev.mark.del = np.mark.del;
                            prev.mark.hidden |= np.mark.hidden;
                        }
                        _ => {
                            close_group(&mut out, &mut group);
                            mark_from_text = holds;
                            last_text = None;
                            out.push(NBlock::Para(Box::new(np)));
                        }
                    }
                    group.extend(own.map(|o| (o, holds)));
                    if holds {
                        last_text = Some(p);
                    }
                    join = next;
                }
                Block::Table(t) => {
                    close_group(&mut out, &mut group);
                    last_text = None;
                    join = false;
                    let mut shell = t.clone();
                    for r in &mut shell.rows {
                        for c in &mut r.cells {
                            c.blocks.clear();
                        }
                    }
                    let cells = t.rows.iter().map(|r| r.cells.iter().map(|c| self.blocks(&c.blocks, depth.saturating_add(1))).collect()).collect();
                    out.push(NBlock::Table(Some(Box::new(shell)), cells));
                }
            }
        }
        close_group(&mut out, &mut group);
        out
    }
}

/// Give the last paragraph of `out` the properties of the pieces in `group` that hold text (or
/// of the last piece when none does), and start a new group.
fn close_group(out: &mut [NBlock], group: &mut Vec<(NProps, bool)>) {
    let any = group.iter().any(|(_, holds)| *holds);
    let mut keep: Vec<NProps> = Vec::new();
    let last = group.len().saturating_sub(1);
    for (i, (p, holds)) in group.drain(..).enumerate() {
        if holds || (!any && i == last) {
            keep.push(p);
        }
    }
    if let Some(NBlock::Para(prev)) = out.last_mut()
        && !keep.is_empty()
    {
        prev.props = merged(keep);
    }
}

/// Consecutive duplicates merged (an owner paragraph split in two keeps one entry).
fn merged(mut v: Vec<NProps>) -> Vec<NProps> {
    v.dedup();
    v
}

/// Append, merging with the previous run when it looks the same.
fn push_run(runs: &mut Vec<NRun>, r: NRun) {
    if r.text.is_empty() {
        return;
    }
    match runs.last_mut() {
        Some(last) if last.key == r.key => {
            last.text.push_str(&r.text);
            last.objects.extend(r.objects);
        }
        _ => runs.push(r),
    }
}

fn mask_blocks(bl: &[NBlock], m: Mask) -> Vec<NBlock> {
    bl.iter()
        .map(|b| match b {
            NBlock::Para(p) => NBlock::Para(Box::new(mask_para(p, m))),
            NBlock::Table(shell, cells) => NBlock::Table(
                if m == Mask::All { None } else { shell.clone() },
                cells.iter().map(|r| r.iter().map(|c| mask_blocks(c, m)).collect()).collect(),
            ),
        })
        .collect()
}

fn mask_para(p: &NPara, m: Mask) -> NPara {
    let mut runs = Vec::new();
    for r in &p.runs {
        push_run(&mut runs, NRun { text: r.text.clone(), objects: r.objects.clone(), key: mask_key(&r.key, m) });
    }
    let props = merged(p.props.iter().map(|x| mask_props(x, m)).collect());
    let tails = match m {
        Mask::All => TAILS_OK,
        Mask::Allowed => Tails { allowed: p.tails.allowed, strict: true },
    };
    NPara { runs, props, mark: mask_key(&p.mark, m), tails, section: if m == Mask::All { None } else { p.section.clone() } }
}

/// Paragraph properties without the allowed ones (`Allowed`) or without any (`All`). Lists stay
/// in `Allowed`: joining, leaving or changing the level of a list is the owner's.
fn mask_props(x: &NProps, m: Mask) -> NProps {
    if m == Mask::All {
        return NProps { props: ParaProps::default(), list: None };
    }
    let ParaProps {
        style: _,
        align: _,
        indent_left: _,
        indent_right: _,
        indent_first: _,
        space_before: _,
        space_after: _,
        line_spacing: _,
        contextual_spacing: _,
        keep_next,
        keep_lines,
        page_break_before,
        widow_control,
        outline_level,
        numbering,
        tabs,
        shading,
        borders,
        suppress_hyphens,
        suppress_line_numbers,
        bidi,
        drop_cap,
    } = &x.props;
    let props = ParaProps {
        style: None,
        align: None,
        indent_left: None,
        indent_right: None,
        indent_first: None,
        space_before: None,
        space_after: None,
        line_spacing: None,
        contextual_spacing: None,
        keep_next: *keep_next,
        keep_lines: *keep_lines,
        page_break_before: *page_break_before,
        widow_control: *widow_control,
        outline_level: *outline_level,
        numbering: *numbering,
        tabs: tabs.clone(),
        shading: *shading,
        borders: *borders,
        suppress_hyphens: *suppress_hyphens,
        suppress_line_numbers: *suppress_line_numbers,
        bidi: *bidi,
        drop_cap: *drop_cap,
    };
    NProps { props, list: x.list }
}

fn mask_key(k: &Key, m: Mask) -> Key {
    match m {
        Mask::All => Key { props: CharProps::default(), ins: k.ins.clone(), del: k.del.clone(), hidden: false },
        Mask::Allowed => {
            let CharProps {
                style,
                font: _,
                size: _,
                bold: _,
                italic: _,
                underline: _,
                underline_color: _,
                strike: _,
                double_strike: _,
                color: _,
                highlight: _,
                shading,
                vert_align: _,
                caps,
                small_caps,
                hidden,
                spacing,
                scale,
                position,
                kern,
                outline,
                shadow,
                emboss,
                engrave,
                lang,
                no_proof,
                rtl,
                link,
                ins,
                del,
            } = &k.props;
            let props = CharProps {
                style: style.clone(),
                font: None,
                size: None,
                bold: None,
                italic: None,
                underline: None,
                underline_color: None,
                strike: None,
                double_strike: None,
                color: None,
                highlight: None,
                shading: *shading,
                vert_align: None,
                caps: *caps,
                small_caps: *small_caps,
                hidden: *hidden,
                spacing: *spacing,
                scale: *scale,
                position: *position,
                kern: *kern,
                outline: *outline,
                shadow: *shadow,
                emboss: *emboss,
                engrave: *engrave,
                lang: lang.clone(),
                no_proof: *no_proof,
                rtl: *rtl,
                link: link.clone(),
                ins: *ins,
                del: *del,
            };
            Key { props, ..k.clone() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{Revision, StoryRef};

    /// "Alpha beta gamma." with an owner insertion " beta" (revision 0, "Owner").
    fn doc() -> Document {
        let mut d = Document::from_text("Alpha beta gamma.\nSecond.");
        d.revisions.push(Revision { kind: RevisionKind::Insert, author: "Owner".into(), date: "2026-10-09T00:00:00Z".into() });
        d.revisions.push(Revision { kind: RevisionKind::Insert, author: "@claude".into(), date: "2026-10-09T00:00:01Z".into() });
        d.revisions.push(Revision { kind: RevisionKind::Delete, author: "@claude".into(), date: "2026-10-09T00:00:01Z".into() });
        if let Ok(p) = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
            let _ = p.format(5, 10, &|c| c.ins = Some(0));
        }
        d
    }

    fn fmt(d: &mut Document, a: usize, b: usize, f: &dyn Fn(&mut CharProps)) {
        if let Ok(p) = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
            let _ = p.format(a, b, f);
        }
    }

    #[test]
    fn member_insertion_and_deletion_are_clean() {
        let before = doc();
        let mut after = before.clone();
        if let Ok(p) = after.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
            let _ = p.insert_text(0, "New ", &CharProps { ins: Some(1), ..Default::default() });
        }
        fmt(&mut after, 15, 20, &|c| c.del = Some(2));
        assert_eq!(check_member(&before, &after, "@claude"), Verdict::Clean);
    }

    #[test]
    fn untracked_text_is_refused() {
        let before = doc();
        let mut after = before.clone();
        if let Ok(p) = after.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
            let _ = p.insert_text(0, "New ", &CharProps::default());
        }
        assert_eq!(check_member(&before, &after, "@claude"), Verdict::Refuse(TEXT));
    }

    #[test]
    fn forged_owner_marks_are_refused() {
        let before = doc();
        // Owner's insertion mark copied onto other text.
        let mut a1 = before.clone();
        fmt(&mut a1, 0, 5, &|c| c.ins = Some(0));
        assert_eq!(check_member(&before, &a1, "@claude"), Verdict::Refuse(TEXT));
        // Owner's insertion mark removed (accepted without review).
        let mut a2 = before.clone();
        fmt(&mut a2, 5, 10, &|c| c.ins = None);
        assert_eq!(check_member(&before, &a2, "@claude"), Verdict::Refuse(TEXT));
        // Owner's insertion re-signed by the member.
        let mut a3 = before.clone();
        fmt(&mut a3, 5, 10, &|c| c.ins = Some(1));
        assert_eq!(check_member(&before, &a3, "@claude"), Verdict::Refuse(TEXT));
        // A deletion credited to someone else.
        let mut a4 = before.clone();
        a4.revisions.push(Revision { kind: RevisionKind::Delete, author: "Owner".into(), date: "x".into() });
        fmt(&mut a4, 11, 16, &|c| c.del = Some(3));
        assert_eq!(check_member(&before, &a4, "@claude"), Verdict::Refuse(TEXT));
    }

    #[test]
    fn allowed_formatting_is_format_and_other_formatting_is_refused() {
        let before = doc();
        let mut bold = before.clone();
        fmt(&mut bold, 0, 5, &|c| c.bold = Some(true));
        assert_eq!(check_member(&before, &bold, "@claude"), Verdict::Format);
        let mut hidden = before.clone();
        fmt(&mut hidden, 0, 5, &|c| c.hidden = Some(true));
        assert_eq!(check_member(&before, &hidden, "@claude"), Verdict::Refuse(FORMAT));
        let mut caps = before.clone();
        fmt(&mut caps, 0, 5, &|c| c.caps = Some(true));
        assert_eq!(check_member(&before, &caps, "@claude"), Verdict::Refuse(FORMAT));
    }

    #[test]
    fn owner_comment_text_and_removal_are_refused_member_comment_is_ignored() {
        let mut before = doc();
        let part = before.add_part(PartKind::Comment, vec![wordcraft_doc::para_block(Paragraph::with_text("owner's note", CharProps::default()))]);
        before.comments.insert(0, wordcraft_doc::Comment { author: "Owner".into(), part, ..Default::default() });
        let mut gone = before.clone();
        gone.comments.remove(&0);
        assert_eq!(check_member(&before, &gone, "@claude"), Verdict::Refuse(TEXT));
        let mut edited = before.clone();
        if let Ok(p) = edited.para_mut(StoryRef::Part(part), &wordcraft_doc::Path::top(0)) {
            let _ = p.insert_text(0, "café ", &CharProps::default());
        }
        assert_eq!(check_member(&before, &edited, "@claude"), Verdict::Refuse(COMMENT));
        let mut mine = before.clone();
        let p2 = mine.add_part(PartKind::Comment, vec![wordcraft_doc::para_block(Paragraph::with_text("mine", CharProps::default()))]);
        mine.comments.insert(1, wordcraft_doc::Comment { author: "@claude".into(), part: p2, ..Default::default() });
        let _ = mine.insert_object(&wordcraft_doc::Pos::body(0, 2), InlineObject::CommentStart { id: 1 }, &CharProps::default());
        assert_eq!(check_member(&before, &mine, "@claude"), Verdict::Clean);
        let mut forged = before.clone();
        let p3 = forged.add_part(PartKind::Comment, vec![wordcraft_doc::para_block(Paragraph::with_text("fake", CharProps::default()))]);
        forged.comments.insert(1, wordcraft_doc::Comment { author: "Owner".into(), part: p3, ..Default::default() });
        assert_eq!(check_member(&before, &forged, "@claude"), Verdict::Refuse(TEXT));
    }

    #[test]
    fn member_paragraph_split_is_clean() {
        let before = doc();
        let mut after = before.clone();
        let at = wordcraft_doc::Pos::body(0, 5);
        let _ = after.split_paragraph(&at);
        if let Ok(p) = after.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
            p.mark.ins = Some(1);
        }
        if let Ok(p) = after.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)) {
            p.mark.ins = None;
        }
        assert_eq!(check_member(&before, &after, "@claude"), Verdict::Clean);
        // The same split without a mark is an untracked structure change.
        let mut plain = before.clone();
        let _ = plain.split_paragraph(&at);
        if let Ok(p) = plain.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)) {
            p.mark.ins = None;
        }
        assert_eq!(check_member(&before, &plain, "@claude"), Verdict::Refuse(TEXT));
    }

    #[test]
    fn judging_only_resolves() {
        let before = doc();
        // Accept: the owner's insertion loses its mark; accepted view unchanged.
        let mut acc = before.clone();
        fmt(&mut acc, 5, 10, &|c| c.ins = None);
        assert_eq!(check_judging(&before, &acc, true, "@claude"), Verdict::Clean);
        // "Accept" that also changes text.
        let mut bad = acc.clone();
        if let Ok(p) = bad.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)) {
            let _ = p.insert_text(0, "X", &CharProps::default());
        }
        assert_eq!(check_judging(&before, &bad, true, "@claude"), Verdict::Refuse(JUDGE));
        // Reject: the owner's insertion is removed; rejected view unchanged.
        let mut rej = before.clone();
        if let Ok(p) = rej.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
            let _ = p.delete(5, 10);
        }
        assert_eq!(check_judging(&before, &rej, false, "@claude"), Verdict::Clean);
        assert_eq!(check_judging(&before, &rej, true, "@claude"), Verdict::Refuse(JUDGE));
    }

    /// `before` with a comment by `author` whose text holds the owner's tracked insertion
    /// "NOT " (revision 0), and that comment's part.
    fn comment_with_owner_insertion(author: &str) -> (Document, u32) {
        let mut d = doc();
        let part = d.add_part(PartKind::Comment, vec![wordcraft_doc::para_block(Paragraph::with_text("NOT I accept", CharProps::default()))]);
        if let Ok(p) = d.para_mut(StoryRef::Part(part), &wordcraft_doc::Path::top(0)) {
            let _ = p.format(0, 4, &|c| c.ins = Some(0));
        }
        d.comments.insert(0, wordcraft_doc::Comment { author: author.into(), part, ..Default::default() });
        (d, part)
    }

    #[test]
    fn judging_never_changes_a_comment_by_someone_else() {
        for (author, verdict) in [("Owner", Verdict::Refuse(COMMENT)), ("@claude", Verdict::Clean)] {
            let (before, part) = comment_with_owner_insertion(author);
            // Accept: the mark goes, the text stays.
            let mut acc = before.clone();
            if let Ok(p) = acc.para_mut(StoryRef::Part(part), &wordcraft_doc::Path::top(0)) {
                let _ = p.format(0, 4, &|c| c.ins = None);
            }
            assert_eq!(check_judging(&before, &acc, true, "@claude"), verdict, "accept, comment by {author}");
            // Reject: the text goes.
            let mut rej = before.clone();
            if let Ok(p) = rej.para_mut(StoryRef::Part(part), &wordcraft_doc::Path::top(0)) {
                let _ = p.delete(0, 4);
            }
            assert_eq!(check_judging(&before, &rej, false, "@claude"), verdict, "reject, comment by {author}");
        }
    }

    /// "Top.", then `depth` 1x1 tables, each one in the cell of the one before, around "Deep.".
    fn nested(depth: usize) -> Document {
        let mut d = Document::from_text("Top.");
        let mut inner: Blocks = vec![wordcraft_doc::para_block(Paragraph::with_text("Deep.", CharProps::default()))];
        for _ in 0..depth {
            let mut t = Table::default();
            t.rows.push(wordcraft_doc::Row { cells: vec![wordcraft_doc::Cell { blocks: inner, ..Default::default() }], ..Default::default() });
            inner = vec![std::sync::Arc::new(Block::Table(t))];
        }
        d.body.extend(inner.iter().cloned());
        d
    }

    #[test]
    fn a_document_deeper_than_the_views_is_refused_whole() {
        assert!(DEEP.contains(&format!(" {MAX_DEPTH} ")));
        for depth in [0, 1, MAX_DEPTH] {
            assert!(!too_deep(&nested(depth)), "{depth}");
        }
        let seen = nested(MAX_DEPTH);
        let deep = nested(MAX_DEPTH + 1);
        assert!(too_deep(&deep));
        // Equal views, yet every check refuses: the change could be where the views do not look.
        assert_eq!(check_member(&deep, &deep, "@claude"), Verdict::Refuse(DEEP));
        assert_eq!(check_judging(&deep, &deep, true, "@claude"), Verdict::Refuse(DEEP));
        assert_eq!(check_judging(&seen, &deep, false, "@claude"), Verdict::Refuse(DEEP));
        assert_eq!(check_member(&seen, &seen, "@claude"), Verdict::Clean);
        // In a comment or a header too.
        let mut d = doc();
        let blocks = nested(MAX_DEPTH + 1).body.clone();
        d.add_part(PartKind::Comment, blocks);
        assert!(too_deep(&d));
    }

    #[test]
    fn page_setup_is_refused() {
        let before = doc();
        let mut after = before.clone();
        after.last_section.margin_left += 10.0;
        assert_eq!(check_member(&before, &after, "@claude"), Verdict::Refuse(DOC));
    }
}
