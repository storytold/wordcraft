use super::from_doc::uses_kind;
use super::types::{
    CODE_STYLE, Cell, Dropped, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, MAX_DEPTH, MAX_RUN_STRINGS, MONO_FONT, Para, RUBY_BASE_SIZE,
    RevMark, clean_text, dropped, ruby_object,
};
use super::{
    AbstractNum, Arc, Block, Blocks, Border, BorderStyle, Borders, CellProps, CharProps, Document, InlineObject, Level, ListKind, Num, NumRef, OBJ,
    ParaProps, Paragraph, PropChange, Revision, RevisionKind, Rgb, RowProps, Run, Style, StyleKind, Table, TextColor, Underline, VMerge, VertAlign,
    levels_for, para_block,
};

pub fn char_props(f: &Fmt) -> CharProps {
    let on = |b: bool| if b { Some(true) } else { None };
    CharProps {
        bold: on(f.bold),
        italic: on(f.italic),
        underline: if f.underline { Some(Underline::Single) } else { None },
        strike: on(f.strike),
        vert_align: if f.sup {
            Some(VertAlign::Superscript)
        } else if f.sub {
            Some(VertAlign::Subscript)
        } else {
            None
        },
        link: f.link.as_deref().filter(|l| !l.is_empty()).map(str::to_string),
        color: f.color.map(TextColor::Rgb),
        size: f.size.filter(|s| s.is_finite() && *s >= 1.0 && *s <= 1638.0),
        font: if f.code { Some(MONO_FONT.to_string()) } else { f.font.as_deref().filter(|s| !s.is_empty()).map(str::to_string) },
        shading: f.background,
        ..Default::default()
    }
    .with_extra(f)
}

/// [`Fmt::extra`] laid over the character properties of the other [`Fmt`] fields.
trait WithExtra {
    fn with_extra(self, f: &Fmt) -> CharProps;
}

impl WithExtra for CharProps {
    fn with_extra(mut self, f: &Fmt) -> CharProps {
        let Some(x) = f.extra.as_deref() else { return self };
        if f.underline
            && let Some(u) = x.underline
        {
            self.underline = Some(u);
        }
        if f.underline {
            self.underline_color = x.underline_color;
        }
        if x.double_strike == Some(true) {
            (self.strike, self.double_strike) = (None, Some(true));
        }
        let finite = |v: Option<f32>, lo: f32, hi: f32| v.filter(|v| v.is_finite()).map(|v| v.clamp(lo, hi));
        self.caps = x.caps;
        self.small_caps = x.small_caps;
        self.hidden = x.hidden;
        self.spacing = finite(x.spacing, -1584.0, 1584.0);
        self.scale = finite(x.scale, 1.0, 600.0);
        self.position = finite(x.position, -1584.0, 1584.0);
        self.outline = x.outline;
        self.shadow = x.shadow;
        self.emboss = x.emboss;
        self.engrave = x.engrave;
        self.lang = x.lang.as_deref().map(clean_text).filter(|l| !l.is_empty() && l.len() <= 64);
        self
    }
}

fn code_style() -> Style {
    Style {
        id: CODE_STYLE.into(),
        name: CODE_STYLE.into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        para: ParaProps { space_after: Some(0.0), space_before: Some(0.0), line_spacing: Some(Default::default()), ..Default::default() },
        chr: CharProps { font: Some(MONO_FONT.into()), size: Some(10.0), ..Default::default() },
        quick: false,
        ..Default::default()
    }
}

fn rule_borders() -> Borders {
    Borders { bottom: Some(Border { style: BorderStyle::Single, width: 0.75, color: Some(Rgb(0xA0, 0xA0, 0xA0)), space: 1.0 }), ..Default::default() }
}

/// Append text (without object characters) or one inline object to the end of `p`.
fn append(p: &mut Paragraph, text: &str, props: &CharProps, obj: Option<InlineObject>) {
    let text: std::borrow::Cow<str> = if text.contains(OBJ) { text.chars().filter(|c| *c != OBJ).collect::<String>().into() } else { text.into() };
    let piece = match obj {
        Some(o) => {
            p.objects.push(o);
            OBJ.to_string()
        }
        None => text.into_owned(),
    };
    if piece.is_empty() {
        return;
    }
    p.text.push_str(&piece);
    match p.runs.last_mut() {
        Some(last) if last.props == *props => last.len += piece.len(),
        _ => p.runs.push(Run { len: piece.len(), props: props.clone() }),
    }
}

/// The paragraph properties of a flow paragraph (its list as instance `list_num`, when given).
fn para_props(p: &Para, list_num: Option<u32>) -> ParaProps {
    let mut out = ParaProps {
        style: match p.kind {
            Kind::Heading(n) => Some(format!("Heading{}", n.clamp(1, 6))),
            Kind::Title => Some("Title".into()),
            Kind::Quote => Some("Quote".into()),
            Kind::Code => Some(CODE_STYLE.into()),
            Kind::Rule | Kind::Normal => None,
        },
        ..Default::default()
    };
    if p.kind == Kind::Rule {
        out.borders = Some(rule_borders());
    } else if p.borders.is_some() {
        out.borders = p.borders;
    }
    if let (Some(li), Some(num)) = (p.list, list_num) {
        if out.style.is_none() {
            out.style = Some("ListParagraph".into());
        }
        out.numbering = Some(NumRef { num, level: li.level.min(8) });
    }
    out.align = p.align;
    let sp = p.spacing;
    out.space_before = sp.before;
    out.space_after = sp.after;
    out.line_spacing = sp.line;
    out.indent_left = sp.left;
    out.indent_right = sp.right;
    out.indent_first = sp.first;
    if p.page_break {
        out.page_break_before = Some(true);
    }
    out
}

struct Builder<'a> {
    doc: &'a mut Document,
    /// Revisions already in `doc` by (kind, author, date), so each change is added once.
    revs: std::collections::HashMap<(u8, RevMark), u32>,
    /// Link and font bytes given to runs so far ([`MAX_RUN_STRINGS`]).
    run_strings: usize,
    /// The document's comment for each flow comment id.
    comments: std::collections::HashMap<u32, u32>,
    /// The document's list instance for each flow list number ([`ListInfo::list`]).
    lists: std::collections::HashMap<u32, u32>,
}

fn rev_key(kind: RevisionKind) -> u8 {
    match kind {
        RevisionKind::Insert => 0,
        RevisionKind::Delete => 1,
        RevisionKind::Format => 2,
    }
}

impl<'a> Builder<'a> {
    fn new(doc: &'a mut Document) -> Builder<'a> {
        let mut revs = std::collections::HashMap::new();
        for (i, r) in doc.revisions.iter().enumerate() {
            if r.move_name.is_none() {
                let mark = RevMark { author: r.author.clone(), date: r.date.clone(), id: None };
                revs.entry((rev_key(r.kind), mark)).or_insert(i as u32);
            }
        }
        Builder { doc, revs, run_strings: 0, comments: Default::default(), lists: Default::default() }
    }

    /// The document revision for a tracked change, added the first time it is seen.
    fn revision(&mut self, kind: RevisionKind, m: &RevMark) -> u32 {
        let key = (rev_key(kind), m.clone());
        if let Some(i) = self.revs.get(&key) {
            return *i;
        }
        let i = u32::try_from(self.doc.revisions.len()).unwrap_or(u32::MAX);
        self.doc.revisions.push(Revision::new(kind, m.author.clone(), m.date.clone()));
        self.revs.insert(key, i);
        i
    }

    /// `cp` as a run may keep it: without its link and font once the runs' copies of them are
    /// over budget.
    fn budget(&mut self, mut cp: CharProps) -> CharProps {
        let n = cp.link.as_ref().map_or(0, String::len).saturating_add(cp.font.as_ref().map_or(0, String::len));
        match self.run_strings.checked_add(n).filter(|&t| t <= MAX_RUN_STRINGS) {
            Some(t) => self.run_strings = t,
            None => {
                if cp.link.is_some() || cp.font.is_some() {
                    dropped(Dropped::RunStrings);
                }
                (cp.link, cp.font) = (None, None);
            }
        }
        cp
    }

    /// Comment `c` (flow id `id`) added to the document with its text, as a reply when its parent
    /// was added before it; its document id, or `None` when the document has no room for it or
    /// the flow already used `id`.
    fn comment(&mut self, id: u32, c: &super::types::FComment) -> Option<u32> {
        if self.comments.contains_key(&id) {
            return None;
        }
        let blocks = self.blocks(&c.blocks, 1);
        let part = self.doc.try_add_part(wordcraft_doc::PartKind::Comment, blocks).ok()?;
        let ids = &self.doc.comments;
        let Some(doc_id) = wordcraft_doc::free_id(ids.keys().next_back().copied(), ids.keys().copied(), 0) else {
            self.doc.parts.remove(&part);
            return None;
        };
        let parent = c.parent.and_then(|p| self.comments.get(&p).copied());
        let comment = wordcraft_doc::Comment {
            author: clean_text(&c.author),
            initials: clean_text(&c.initials),
            date: clean_text(&c.date),
            parent,
            resolved: c.resolved,
            part,
            ..Default::default()
        };
        self.doc.comments.insert(doc_id, comment);
        self.comments.insert(id, doc_id);
        Some(doc_id)
    }

    /// A note story holding `blocks`, as Word's: its paragraphs in the note text style, the first
    /// starting with the note's mark and a space. The reference to put in the text, or `None`
    /// when the document has no room for another story.
    fn note(&mut self, endnote: bool, custom: &str, blocks: &[FBlock]) -> Option<InlineObject> {
        use wordcraft_doc::para::NoteKind;
        let (kind, part, text, mark) = if endnote {
            (NoteKind::Endnote, wordcraft_doc::PartKind::Endnote, "EndnoteText", "EndnoteReference")
        } else {
            (NoteKind::Footnote, wordcraft_doc::PartKind::Footnote, "FootnoteText", "FootnoteReference")
        };
        let custom: String = clean_text(custom).chars().filter(|c| !c.is_control()).take(10).collect();
        let id = self.doc.try_add_part(part, Vec::new()).ok()?;
        let mut story = self.blocks(blocks, 1);
        if story.is_empty() {
            story.push(para_block(Paragraph::new()));
        }
        for (i, b) in story.iter_mut().enumerate() {
            if let Block::Para(p) = Arc::make_mut(b) {
                if p.props.style.is_none() {
                    p.props.style = Some(text.into());
                }
                if i == 0 {
                    let starts_blank = p.text.starts_with([' ', '\t']);
                    let reference = InlineObject::NoteRef { kind, id, custom: custom.clone() };
                    if p.insert_object(0, reference, &CharProps { style: Some(mark.into()), ..Default::default() }).is_ok() && !starts_blank {
                        let at = p.text.len().min(wordcraft_doc::para::OBJ.len_utf8());
                        let _ = p.insert_text(at, " ", &CharProps::default());
                    }
                }
            }
        }
        if let Some(part) = self.doc.parts.get_mut(&id) {
            part.blocks = story;
        }
        Some(InlineObject::NoteRef { kind, id, custom })
    }

    fn para(&mut self, p: &Para, list_num: Option<u32>) -> Paragraph {
        let mut out = Paragraph::new();
        out.props = para_props(p, list_num);
        if let Some(c) = &p.fmt_change {
            // A list item stays in its list instance (only its level may differ); an old list on a
            // paragraph that is no list now gets an instance of its own, so Reject can restore it.
            // The same goes for an item whose list kind changed (numbered to bulleted or back).
            let old_num = match (c.old.list, p.list, list_num) {
                (Some(old), Some(now), Some(num)) if old.ordered == now.ordered => Some(num),
                (Some(old), _, _) => Some(self.new_list(&[old])),
                _ => None,
            };
            let old = para_props(&c.old, old_num);
            out.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, &c.mark), old);
        }
        if p.keep_next {
            out.props.keep_next = Some(true);
        }
        out.section.clone_from(&p.section);
        // Inlines are appended in order, so build the runs directly: inserting each one through
        // `Paragraph::insert_text` rescans every earlier run and is quadratic in their number.
        for i in &p.inlines {
            match i {
                Inline::Text(t, f) => {
                    let t = clean_text(t);
                    let mut cp = self.budget(char_props(f));
                    cp.ins = f.ins.as_ref().map(|m| self.revision(RevisionKind::Insert, m));
                    cp.del = f.del.as_ref().map(|m| self.revision(RevisionKind::Delete, m));
                    if let Some(c) = &f.fmt_change {
                        let old = self.budget(char_props(&c.old));
                        cp.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, &c.mark), old);
                    }
                    if p.kind == Kind::Code {
                        // Code's font comes from its style, before a change as after it.
                        if cp.font.as_deref() == Some(MONO_FONT) {
                            cp.font = None;
                        }
                        if let Some(c) = cp.fmt_change.as_deref_mut()
                            && c.old.font.as_deref() == Some(MONO_FONT)
                        {
                            c.old.font = None;
                        }
                    }
                    append(&mut out, &t, &cp, None);
                }
                Inline::Image(img) => {
                    let key = self.doc.add_media(img.data.to_vec(), &img.ext);
                    let (w, h) = (finite_or(img.w, 72.0).clamp(1.0, 1584.0), finite_or(img.h, 72.0).clamp(1.0, 1584.0));
                    let obj = InlineObject::Image { media: key, w, h, alt: img.alt.clone(), float: Default::default(), crop: [0.0; 4], ole: None };
                    append(&mut out, "", &CharProps::default(), Some(obj));
                }
                Inline::Anchor(name) => {
                    append(&mut out, "", &CharProps::default(), Some(InlineObject::BookmarkStart { name: name.clone() }));
                    append(&mut out, "", &CharProps::default(), Some(InlineObject::BookmarkEnd { name: name.clone() }));
                }
                Inline::Figure(_) => {}
                Inline::Comment { id, comment } => {
                    // A reply is the comment's, not the text's: it has no range.
                    if let Some(doc_id) = self.comment(*id, comment)
                        && comment.parent.is_none()
                    {
                        append(&mut out, "", &CharProps::default(), Some(InlineObject::CommentStart { id: doc_id }));
                    }
                }
                Inline::CommentEnd(id) => {
                    if let Some(doc_id) = self.comments.get(id) {
                        append(&mut out, "", &CharProps::default(), Some(InlineObject::CommentEnd { id: *doc_id }));
                    }
                }
                Inline::Note { endnote, custom, blocks } => {
                    if let Some(obj) = self.note(*endnote, custom, blocks) {
                        let style = if *endnote { "EndnoteReference" } else { "FootnoteReference" };
                        append(&mut out, "", &CharProps { style: Some(style.into()), ..Default::default() }, Some(obj));
                    }
                }
                Inline::Field { instr, result, fmt } => {
                    let cp = self.budget(char_props(fmt));
                    let (instr, result) = (clean_text(instr), clean_text(result));
                    if instr.trim().is_empty() {
                        append(&mut out, &result, &cp, None);
                    } else {
                        let obj = InlineObject::Field { instr, result, locked: false, dirty: false, code: false, form: String::new() };
                        append(&mut out, "", &cp, Some(obj));
                    }
                }
                Inline::Equation { linear, display } => {
                    let linear = clean_text(linear);
                    if !linear.is_empty() {
                        let obj = InlineObject::Equation { linear, display: *display, math: Default::default() };
                        append(&mut out, "", &CharProps::default(), Some(obj));
                    }
                }
                Inline::Ruby { base, ruby, fmt } => {
                    let (base, ruby) = (clean_text(base), clean_text(ruby));
                    let cp = self.budget(char_props(fmt));
                    // A source without a base or without a reading (malformed markup) is just its
                    // text: a guide that annotates nothing is no guide.
                    if base.is_empty() || ruby.is_empty() {
                        append(&mut out, if base.is_empty() { &ruby } else { &base }, &cp, None);
                    } else {
                        // Ruby attributes a format without them doesn't state (HTML): the
                        // Phonetic Guide's defaults for a new guide over this run's text.
                        let obj = ruby_object(base, ruby, cp.size.unwrap_or(RUBY_BASE_SIZE));
                        append(&mut out, "", &cp, Some(obj));
                    }
                }
            }
        }
        out.mark.ins = p.mark_ins.as_deref().map(|m| self.revision(RevisionKind::Insert, m));
        out.mark.del = p.mark_del.as_deref().map(|m| self.revision(RevisionKind::Delete, m));
        out.normalize();
        out.touch();
        out
    }

    /// A new list instance whose levels follow the kinds used in `group` (first use per level).
    fn new_list(&mut self, group: &[ListInfo]) -> u32 {
        let num_levels = levels_for(ListKind::Numbered);
        let bul_levels = levels_for(ListKind::Bullet);
        let mut kinds: [Option<bool>; 9] = [None; 9];
        for li in group {
            if let Some(k) = kinds.get_mut(li.level.min(8) as usize)
                && k.is_none()
            {
                *k = Some(li.ordered);
            }
        }
        let mut starts: [Option<u32>; 9] = [None; 9];
        let mut labels: [Option<super::types::Label>; 9] = [None; 9];
        for li in group {
            if let Some(s) = starts.get_mut(li.level.min(8) as usize)
                && s.is_none()
            {
                *s = Some(li.start);
            }
            if let Some(l) = labels.get_mut(li.level.min(8) as usize)
                && l.is_none()
            {
                *l = li.label;
            }
        }
        // Levels nobody used follow the first level's kind.
        let first = group.first().map(|l| l.ordered).unwrap_or(false);
        let levels: Vec<Level> = (0..9)
            .map(|i| {
                let ordered = kinds.get(i).copied().flatten().unwrap_or(first);
                let src = if ordered { &num_levels } else { &bul_levels };
                let mut level = src.get(i).cloned().unwrap_or_default();
                if ordered && let Some(s) = starts.get(i).copied().flatten() {
                    level.start = s;
                }
                // The source's own label: its format and text, or its bullet.
                match labels.get(i).copied().flatten() {
                    Some(l) if ordered && l.format != wordcraft_doc::section::NumFormat::Bullet => {
                        level.format = l.format;
                        level.text = l.text(i as u8);
                    }
                    Some(l) if !ordered => {
                        if let Some(c) = l.bullet().filter(|c| !c.is_control() && !level.text.starts_with(*c)) {
                            let custom = levels_for(ListKind::BulletChar(c));
                            level.text = c.to_string();
                            level.chr.font = custom.first().and_then(|l| l.chr.font.clone());
                        }
                    }
                    _ => {}
                }
                level
            })
            .collect();
        let numbering = &mut self.doc.numbering;
        let aid = numbering.abstracts.iter().map(|a| a.id + 1).max().unwrap_or(0);
        numbering.abstracts.push(AbstractNum { id: aid, name: None, levels, ..Default::default() });
        let nid = numbering.nums.iter().map(|n| n.id + 1).max().unwrap_or(1).max(1);
        numbering.nums.push(Num { id: nid, abstract_id: aid, ..Default::default() });
        nid
    }

    fn blocks(&mut self, blocks: &[FBlock], depth: usize) -> Blocks {
        let mut out = Blocks::new();
        let mut i = 0;
        while let Some(b) = blocks.get(i) {
            match b {
                FBlock::Para(p) if p.list.is_some() => {
                    // A run of consecutive list paragraphs is one list, until an item of another
                    // list or (when the source doesn't tell lists apart) a top-level item of the
                    // other kind (bulleted or numbered) starts a new one. A list the source
                    // numbers goes on where it shows up again.
                    let mut j = i;
                    let mut group: Vec<ListInfo> = Vec::new();
                    let mut top: Option<bool> = None;
                    let id = p.list.map_or(0, |l| l.list);
                    while let Some(FBlock::Para(q)) = blocks.get(j) {
                        let Some(li) = q.list else { break };
                        if li.list != id {
                            break;
                        }
                        if li.level == 0 && id == 0 {
                            if top.is_some_and(|o| o != li.ordered) {
                                break;
                            }
                            top = Some(li.ordered);
                        }
                        group.push(li);
                        j += 1;
                    }
                    let num = match self.lists.get(&id) {
                        Some(num) if id != 0 => *num,
                        _ => {
                            let num = self.new_list(&group);
                            if id != 0 {
                                self.lists.insert(id, num);
                            }
                            num
                        }
                    };
                    for q in blocks.get(i..j).unwrap_or(&[]) {
                        if let FBlock::Para(q) = q {
                            let para = self.para(q, Some(num));
                            out.push(para_block(para));
                        }
                    }
                    i = j.max(i + 1);
                    continue;
                }
                FBlock::Para(p) => out.push(para_block(self.para(p, None))),
                FBlock::Table(t) => {
                    if depth >= MAX_DEPTH {
                        // Too deep: flatten into paragraphs.
                        for row in &t.rows {
                            for c in row {
                                out.extend(self.blocks(&c.blocks, depth));
                            }
                        }
                    } else if let Some(tb) = self.table(t, depth) {
                        out.push(Arc::new(Block::Table(tb)));
                    }
                }
            }
            i += 1;
        }
        out
    }

    fn table(&mut self, t: &FTable, depth: usize) -> Option<Table> {
        let rows: Vec<&Vec<Cell>> = t.rows.iter().filter(|r| !r.is_empty()).take(wordcraft_doc::table::MAX_ROWS).collect();
        if rows.is_empty() {
            return None;
        }
        let cols = t.cols().clamp(1, wordcraft_doc::table::MAX_COLS);
        let width = self.doc.last_section.text_width().max(72.0);
        // One template row for the grid widths and props; the real rows are built below. Sizing it
        // to `rows.len()` would allocate a full rows × cols cell grid only to clear it, which a
        // tall table (many rows, wide from merged cells) turns into a large transient spike.
        let mut tb = Table::new(1, cols, width);
        if t.borderless {
            tb.props.style = None;
        }
        if t.widths.len() == cols && t.widths.iter().all(|w| w.is_finite() && *w > 1.0) {
            tb.grid = t.widths.iter().map(|w| w.clamp(6.0, 1584.0)).collect();
        }
        tb.rows.clear();
        for r in rows {
            let mut row = wordcraft_doc::Row { props: RowProps::default(), cells: Vec::new(), controls: Default::default() };
            row.props.header = !r.is_empty() && r.iter().all(|c| c.header);
            let mut g = 0usize;
            for c in r.iter().take(wordcraft_doc::table::MAX_COLS) {
                let span = (c.colspan.clamp(1, 63) as usize).min(cols.saturating_sub(g).max(1));
                let w: f32 = tb.grid.get(g..(g + span).min(tb.grid.len())).map(|s| s.iter().sum()).unwrap_or(72.0);
                g += span;
                let mut blocks = self.blocks(&c.blocks, depth + 1);
                if blocks.is_empty() || matches!(blocks.last().map(|b| &**b), Some(Block::Table(_))) {
                    blocks.push(para_block(Paragraph::new()));
                }
                let vmerge = if c.covered {
                    VMerge::Continue
                } else if c.rowspan > 1 {
                    VMerge::Restart
                } else {
                    VMerge::None
                };
                row.cells.push(wordcraft_doc::Cell {
                    props: CellProps {
                        width: Some(w),
                        span: span as u32,
                        vmerge,
                        shading: c.shading,
                        borders: c.borders,
                        valign: c.valign,
                        ..Default::default()
                    },
                    blocks,
                    controls: Default::default(),
                });
            }
            if row.cells.is_empty() {
                row.cells.push(wordcraft_doc::Cell::empty());
            }
            tb.rows.push(row);
        }
        Some(tb)
    }
}

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// Build a document from a flow.
pub fn to_doc(flow: &Flow) -> Document {
    to_doc_in(flow, wordcraft_doc::SectionProps::default())
}

/// [`to_doc`] on a page of the source's own size and margins (`section`), which tables size to.
pub fn to_doc_in(flow: &Flow, section: wordcraft_doc::SectionProps) -> Document {
    let mut doc = Document::new();
    doc.last_section = section;
    doc.numbering = Default::default();
    let uses_code = uses_kind(&flow.blocks, Kind::Code, 0);
    if uses_code {
        doc.styles.upsert(code_style());
    }
    let body = Builder::new(&mut doc).blocks(&flow.blocks, 0);
    doc.body = body;
    let m = &flow.meta;
    doc.core.title = m.title.clone();
    doc.core.creator = m.author.clone();
    doc.core.subject = m.subject.clone();
    doc.core.keywords = m.keywords.clone();
    doc.core.description = m.description.clone();
    doc.core.created = m.created.clone();
    doc.ensure_nonempty();
    doc
}

/// `blocks` as document blocks of `doc` (their pictures go into its media), for a header,
/// footer or other part an importer adds itself.
pub fn part_blocks(doc: &mut Document, blocks: &[FBlock]) -> Blocks {
    let mut out = Builder::new(doc).blocks(blocks, 0);
    if out.is_empty() {
        out.push(para_block(Paragraph::new()));
    }
    out
}
