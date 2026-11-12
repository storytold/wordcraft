use super::inlines::parse_inlines;
use super::{Align, Arc, Cell, Document, FBlock, FTable, Fmt, HashMap, Kind, ListInfo, MAX_COLS, MAX_ROWS, Para, model};

/// Nesting limit for block containers (quotes, list items) and inline links.
pub(super) const MAX_NEST: usize = 32;

// ---------------------------------------------------------------------------------------------
// Import

/// Parse Markdown text into a document.
pub fn import(text: &str) -> Document {
    model::begin_import();
    model::to_doc(&parse(text))
}

/// [`import`] onto `page`: Markdown gives no page size (#1381).
pub fn import_on(text: &str, page: &wordcraft_doc::SectionProps) -> Document {
    model::begin_import();
    model::to_doc_in(&parse(text), page.clone())
}

/// Link reference definitions: the normalised label to its destination.
/// A destination is shared by every reference to it, not copied into each (#3713).
pub type Refs = HashMap<String, Arc<str>>;

/// A label as CommonMark compares them: case-folded, and any run of white space one space
/// (§4.7, "matches"). `[FOO bar]` and `[foo   BAR]` are the same label.
pub(super) fn ref_label(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A link reference definition on its own line: `[label]: destination "optional title"`. The
/// destination may be wrapped in `<...>`. Returns the label and the destination.
///
/// CommonMark allows the title on the following line and the label to span lines; a definition
/// written across lines is left as text here, which is what it was before these were read at all.
fn link_definition(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    // Up to three spaces of indentation; four makes it indented code.
    if line.len() - t.len() > 3 || !t.starts_with('[') {
        return None;
    }
    let bytes = t.as_bytes();
    // The label ends at the first unescaped `]`, and cannot be empty or hold an unescaped `[`.
    let mut i = 1;
    let mut close = None;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'[' => return None,
            b']' => {
                close = Some(i);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let close = close?;
    let label = ref_label(t.get(1..close)?);
    // `[^label]: ...` is a footnote definition, which this importer leaves as text.
    if label.starts_with('^') {
        return None;
    }
    if label.is_empty() || t.as_bytes().get(close + 1) != Some(&b':') {
        return None;
    }
    let rest = t.get(close + 2..)?.trim_start();
    if rest.is_empty() {
        return None;
    }
    // The destination: `<...>`, else up to the next space. What follows may only be a title.
    let (dest, after) = match rest.strip_prefix('<') {
        Some(inner) => {
            let end = inner.find('>')?;
            (inner.get(..end)?.to_string(), inner.get(end + 1..)?)
        }
        None => {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (rest.get(..end)?.to_string(), rest.get(end..)?)
        }
    };
    if dest.is_empty() {
        return None;
    }
    let title = after.trim();
    let titled =
        title.is_empty() || (title.len() >= 2 && matches!((title.chars().next()?, title.chars().last()?), ('"', '"') | ('\'', '\'') | ('(', ')')));
    titled.then_some((label, dest))
}

/// Take the link reference definitions out of `lines`, leaving the rest. Lines inside a fenced
/// code block are left alone: a definition there is code, not a definition. Fences open and
/// close by the same rules [`parse_blocks`] applies, or a line it reads as code would be read
/// here as a definition and deleted.
///
/// A definition has to start a block: CommonMark §4.7 forbids one from interrupting a paragraph,
/// and a line like `[Warning]: unstable` in the middle of one is the author's text. This pass
/// sees lines without the block structure around them, so it takes a definition only after a
/// blank line, at the start of the document, after a heading or thematic break, or after another
/// definition. A definition anywhere else stays in the document and reads as text, which is what
/// it did before definitions were understood at all; deleting a line that was part of a
/// paragraph would lose it silently.
///
/// Returns the parsed references and the 0-based indexes of the removed lines.
fn take_definitions(lines: &mut Vec<String>) -> (Refs, Vec<usize>) {
    let mut refs = Refs::new();
    let mut open: Option<(char, usize)> = None;
    // The start of the document begins a block.
    let mut starts_block = true;
    let mut removed = Vec::new();
    let mut at = 0usize;
    lines.retain(|line| {
        let this = at;
        at += 1;
        let t = line.trim_start_matches([' ', '\t']);
        // Four spaces of indentation is code, not a fence (`parse_blocks`), on both sides.
        let marker = (indent(line) < 4).then(|| fence(t)).flatten();
        match open {
            Some((c, n)) => {
                // A closing fence holds nothing but its own run of fence characters.
                if marker.is_some_and(|(fc, fl)| fc == c && fl >= n) && t.trim_start_matches(c).trim().is_empty() {
                    open = None;
                    starts_block = true;
                }
                return true;
            }
            None => {
                if let Some(f) = marker {
                    open = Some(f);
                    return true;
                }
            }
        }
        if starts_block && let Some((label, dest)) = link_definition(line) {
            // The first definition of a label wins, as CommonMark says, and definitions may
            // follow one another.
            refs.entry(label).or_insert(dest.into());
            removed.push(this);
            return false;
        }
        starts_block = is_blank(line) || atx_heading(t).is_some() || thematic_break(t);
        true
    });
    (refs, removed)
}

/// Maps monotonically non-decreasing line indexes in lines filtered of link definitions back to
/// their original source lines in a single linear pass.
pub(super) fn remap_original_lines(block_lines: &mut [usize], removed: &[usize]) {
    let mut rem_iter = removed.iter().copied().peekable();
    let mut shift = 0;
    for l in block_lines.iter_mut() {
        while let Some(&r) = rem_iter.peek() {
            if r <= *l + shift {
                shift += 1;
                rem_iter.next();
            } else {
                break;
            }
        }
        *l += shift;
    }
}

/// Context for block parsing: depth, nesting, reference definitions, cell budget, and line recording.
struct BlockCtx<'a> {
    depth: usize,
    level: u8,
    refs: &'a Refs,
    budget: &'a std::cell::Cell<usize>,
    lines_out: Option<&'a mut Vec<usize>>,
}

impl BlockCtx<'_> {
    /// Records the starting line of a top-level block if line tracking is active.
    fn record_line(&mut self, line: usize) {
        if self.depth == 0
            && let Some(lo) = self.lines_out.as_mut()
        {
            lo.push(line);
        }
    }

    /// Records `count` blocks originating at `line` if line tracking is active.
    fn record_lines(&mut self, line: usize, count: usize) {
        if self.depth == 0
            && let Some(lo) = self.lines_out.as_mut()
        {
            for _ in 0..count {
                lo.push(line);
            }
        }
    }
}

/// Parse Markdown text into the flow model.
pub fn parse(text: &str) -> model::Flow {
    parse_with_lines(text).0
}

/// [`parse`] returning the flow model and the 0-based starting line of each top-level block.
pub fn parse_with_lines(text: &str) -> (model::Flow, Vec<usize>) {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut lines: Vec<String> = text.split('\n').map(|l| expand_tabs(l.trim_end_matches('\r'))).collect();
    let (refs, removed) = take_definitions(&mut lines);
    let budget = std::cell::Cell::new(super::MAX_TABLE_CELLS);
    let mut block_lines = Vec::new();
    let mut ctx = BlockCtx { depth: 0, level: 0, refs: &refs, budget: &budget, lines_out: Some(&mut block_lines) };
    let mut blocks = parse_blocks(&lines, &mut ctx);
    // A leading "Title" from front matter isn't supported; a first-level heading stays a heading.
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
        block_lines.push(0);
    }
    remap_original_lines(&mut block_lines, &removed);
    (model::Flow { blocks, meta: Default::default() }, block_lines)
}

/// [`import_on`] returning the document and the 0-based starting line of each top-level block.
pub fn import_on_with_lines(text: &str, page: &wordcraft_doc::SectionProps) -> (Document, Vec<usize>) {
    model::begin_import();
    let (flow, mut lines) = parse_with_lines(text);
    let doc = model::to_doc_in(&flow, page.clone());
    if doc.body.len() > lines.len() {
        let end_line = text.as_bytes().iter().filter(|&&b| b == b'\n').count();
        lines.resize(doc.body.len(), end_line);
    }
    (doc, lines)
}

/// Leading tabs → spaces (tab stops of 4); the rest of the line is kept.
fn expand_tabs(l: &str) -> String {
    if !l.starts_with([' ', '\t']) || !l.contains('\t') {
        return l.to_string();
    }
    let mut out = String::with_capacity(l.len() + 8);
    let mut col = 0usize;
    let mut rest = l;
    while let Some(c) = rest.chars().next() {
        match c {
            ' ' => {
                out.push(' ');
                col += 1;
            }
            '\t' => {
                let n = 4 - col % 4;
                out.push_str(&" ".repeat(n));
                col += n;
            }
            _ => break,
        }
        rest = &rest[c.len_utf8()..];
    }
    out.push_str(rest);
    out
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start_matches(' ').len()
}

fn is_blank(l: &str) -> bool {
    l.trim().is_empty()
}

/// Strip up to `n` leading spaces.
fn strip_indent(l: &str, n: usize) -> &str {
    let k = indent(l).min(n);
    l.get(k..).unwrap_or("")
}

fn atx_heading(t: &str) -> Option<(u8, &str)> {
    let n = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&n) {
        return None;
    }
    let rest = t.get(n..)?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let mut body = rest.trim();
    // Optional closing sequence.
    let trimmed = body.trim_end_matches('#');
    if trimmed.len() != body.len() && (trimmed.is_empty() || trimmed.ends_with(' ')) {
        body = trimmed.trim_end();
    }
    Some((n as u8, body))
}

pub(super) fn thematic_break(t: &str) -> bool {
    let mut ch = None;
    let mut n = 0;
    for c in t.chars() {
        match c {
            ' ' | '\t' => {}
            '*' | '-' | '_' => {
                if ch.is_some_and(|x| x != c) {
                    return false;
                }
                ch = Some(c);
                n += 1;
            }
            _ => return false,
        }
    }
    n >= 3
}

fn fence(t: &str) -> Option<(char, usize)> {
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    if c == '`' && t.get(n..).is_some_and(|info| info.contains('`')) {
        return None;
    }
    Some((c, n))
}

#[derive(Clone, Copy, PartialEq)]
struct Marker {
    ordered: bool,
    /// Bullet char or ordered delimiter.
    ch: char,
    /// Bytes of the marker itself (after the indent).
    len: usize,
    start: u32,
}

fn list_marker(t: &str) -> Option<Marker> {
    let first = t.chars().next()?;
    let (m, len) = if matches!(first, '-' | '+' | '*') {
        (Marker { ordered: false, ch: first, len: 1, start: 1 }, 1)
    } else {
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        let d = t.get(digits..)?.chars().next()?;
        if d != '.' && d != ')' {
            return None;
        }
        let start = t.get(..digits)?.parse::<u32>().unwrap_or(1);
        (Marker { ordered: true, ch: d, len: digits + 1, start }, digits + 1)
    };
    let rest = t.get(len..)?;
    if rest.is_empty() || rest.starts_with([' ', '\t']) { Some(m) } else { None }
}

/// A table row without its leading and trailing pipes (an escaped last pipe stays).
fn row_body(l: &str) -> &str {
    let t = l.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    if let Some(before) = t.strip_suffix('|') {
        let num_bs = before.chars().rev().take_while(|&c| c == '\\').count();
        if num_bs % 2 == 0 { before } else { t }
    } else {
        t
    }
}

/// The first `keep` cells of a table row, and how many cells it has: the rest are only counted.
pub(super) fn split_row(l: &str, keep: usize) -> (Vec<String>, usize) {
    let mut cells = Vec::new();
    let mut count = 0usize;
    let mut cur = String::new();
    let mut chars = row_body(l).chars().peekable();
    while let Some(c) = chars.next() {
        let full = cells.len() >= keep;
        match c {
            '\\' => match chars.peek() {
                Some(&'|') => {
                    if !full {
                        cur.push('|');
                    }
                    chars.next();
                }
                Some(&'\\') => {
                    if !full {
                        cur.push_str("\\\\");
                    }
                    chars.next();
                }
                _ if !full => cur.push('\\'),
                _ => {}
            },
            '|' => {
                if !full {
                    cells.push(std::mem::take(&mut cur).trim().to_string());
                }
                count += 1;
            }
            _ if !full => cur.push(c),
            _ => {}
        }
    }
    if cells.len() < keep {
        cells.push(cur.trim().to_string());
    }
    (cells, count + 1)
}

/// The alignments of the first `keep` cells of a delimiter row, and how many cells it has; every
/// cell is checked, so a bad one past `keep` still means it isn't a delimiter row.
fn delimiter_row(l: &str, keep: usize) -> Option<(Vec<Option<Align>>, usize)> {
    if !l.contains('-') {
        return None;
    }
    let mut out = Vec::new();
    let mut count = 0usize;
    // A valid cell is only dashes, colons and spaces, so an escaped pipe can't be in one.
    for c in row_body(l).split('|') {
        let c = c.trim();
        let left = c.starts_with(':');
        let right = c.ends_with(':');
        let core = c.trim_matches(':');
        if core.is_empty() || !core.chars().all(|x| x == '-') {
            return None;
        }
        if out.len() < keep {
            out.push(match (left, right) {
                (true, true) => Some(Align::Center),
                (false, true) => Some(Align::Right),
                (true, false) => Some(Align::Left),
                _ => None,
            });
        }
        count += 1;
    }
    Some((out, count))
}

/// Does this line start a block that interrupts a paragraph?
fn interrupts(l: &str) -> bool {
    if indent(l) >= 4 {
        return false;
    }
    let t = l.trim_start_matches([' ', '\t']);
    atx_heading(t).is_some()
        || thematic_break(t)
        || fence(t).is_some()
        || t.starts_with('>')
        || t.starts_with("<!--")
        || list_marker(t).is_some_and(|m| (!m.ordered || m.start == 1) && !is_blank(t.get(m.len..).unwrap_or("")))
}

/// Flushes accumulated paragraph text into `out`, recording its starting line if tracked.
fn flush_para(lines: &mut Vec<String>, start_line: &mut Option<usize>, out: &mut Vec<FBlock>, ctx: &mut BlockCtx<'_>) {
    if lines.is_empty() {
        *start_line = None;
        return;
    }
    let line_no = start_line.take().unwrap_or(0);
    let text = join_lines(lines);
    lines.clear();
    let inlines = parse_inlines(&text, &Fmt::default(), ctx.depth, ctx.refs);
    if inlines.is_empty() {
        return;
    }
    let p = Para { inlines, ..Default::default() };
    out.push(FBlock::Para(p));
    ctx.record_line(line_no);
}

/// Paragraph lines → text with `\n` for hard breaks and spaces for soft ones.
fn join_lines(lines: &[String]) -> String {
    let mut s = String::new();
    let n = lines.len();
    for (i, l) in lines.iter().enumerate() {
        let l = l.trim_start_matches([' ', '\t']);
        if i + 1 == n {
            s.push_str(l.trim_end());
        } else if l.ends_with("  ") {
            s.push_str(l.trim_end());
            s.push('\n');
        } else if l.ends_with('\\') && !l.ends_with("\\\\") {
            s.push_str(&l[..l.len() - 1]);
            s.push('\n');
        } else {
            s.push_str(l.trim_end());
            s.push(' ');
        }
    }
    s
}

/// Creates a single code block paragraph for an indented code line.
fn code_para(line: &str) -> FBlock {
    let mut p = Para::new(Kind::Code);
    p.push_text(line, &Fmt::default());
    FBlock::Para(p)
}

/// Attempts to parse a table starting at `lines[*i]`. Returns `true` if a table was parsed.
fn parse_table(lines: &[String], i: &mut usize, t: &str, ctx: &mut BlockCtx<'_>, out: &mut Vec<FBlock>) -> bool {
    let Some((aligns, width)) = lines.get(*i + 1).and_then(|l| delimiter_row(l, MAX_COLS)) else { return false };
    let (head, head_width) = split_row(t, MAX_COLS);
    if head_width != width {
        return false;
    }
    let table_start = *i;
    let mut table = FTable::default();
    let cell = |s: &str, header: bool, a: Option<Align>| {
        let p = Para { align: a, inlines: parse_inlines(s, &Fmt::default(), ctx.depth, ctx.refs), ..Default::default() };
        Cell { blocks: vec![FBlock::Para(p)], header, ..Default::default() }
    };
    let cols = aligns.len();
    let fits = |rows: usize| rows < MAX_ROWS && ctx.budget.get() >= cols;
    let mut cut = head_width > cols;
    if fits(0) {
        ctx.budget.set(ctx.budget.get() - cols);
        table.rows.push(head.iter().zip(&aligns).map(|(s, a)| cell(s, true, *a)).collect());
    } else {
        cut = true;
    }
    *i += 2;
    while let Some(l) = lines.get(*i) {
        if is_blank(l) || !l.contains('|') || interrupts(l) {
            break;
        }
        if !table.rows.is_empty() && fits(table.rows.len()) {
            ctx.budget.set(ctx.budget.get() - cols);
            let (mut cells, _) = split_row(l, cols);
            cells.resize(cols, String::new());
            table.rows.push(cells.iter().zip(&aligns).map(|(s, a)| cell(s, false, *a)).collect());
        } else {
            cut = true;
        }
        *i += 1;
    }
    if cut {
        model::dropped(model::Dropped::TableCells);
    }
    if !table.rows.is_empty() {
        out.push(FBlock::Table(table));
        ctx.record_line(table_start);
    }
    true
}

/// Parse block structure.
fn parse_blocks(lines: &[String], ctx: &mut BlockCtx<'_>) -> Vec<FBlock> {
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut para_start = None;
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        if is_blank(line) {
            flush_para(&mut para, &mut para_start, &mut out, ctx);
            i += 1;
            continue;
        }
        let ind = indent(line);
        if ind >= 4 {
            if !para.is_empty() {
                if para_start.is_none() {
                    para_start = Some(i);
                }
                para.push(line.clone());
                i += 1;
                continue;
            }
            parse_indented_code(lines, &mut i, &mut out, ctx);
            continue;
        }
        let t = line.trim_start_matches([' ', '\t']);
        // Setext heading underline.
        if !para.is_empty() {
            let u = t.trim_end();
            if !u.is_empty() && (u.chars().all(|c| c == '=') || (u.chars().all(|c| c == '-'))) {
                let lvl = if u.starts_with('=') { 1 } else { 2 };
                let line_no = para_start.take().unwrap_or(i);
                let text = join_lines(&para);
                para.clear();
                let mut p = Para::new(Kind::Heading(lvl));
                p.inlines = parse_inlines(&text, &Fmt::default(), ctx.depth, ctx.refs);
                out.push(FBlock::Para(p));
                ctx.record_line(line_no);
                i += 1;
                continue;
            }
        }
        if let Some((n, body)) = atx_heading(t) {
            flush_para(&mut para, &mut para_start, &mut out, ctx);
            let mut p = Para::new(Kind::Heading(n));
            p.inlines = parse_inlines(body, &Fmt::default(), ctx.depth, ctx.refs);
            out.push(FBlock::Para(p));
            ctx.record_line(i);
            i += 1;
            continue;
        }
        if thematic_break(t) {
            flush_para(&mut para, &mut para_start, &mut out, ctx);
            out.push(FBlock::Para(Para::new(Kind::Rule)));
            ctx.record_line(i);
            i += 1;
            continue;
        }
        if let Some((fc, fl)) = fence(t) {
            flush_para(&mut para, &mut para_start, &mut out, ctx);
            i += 1;
            while let Some(l) = lines.get(i) {
                let cur = i;
                i += 1;
                let lt = l.trim_start_matches([' ', '\t']);
                if indent(l) < 4 && fence(lt).is_some_and(|(c, n)| c == fc && n >= fl) && lt.trim_start_matches(fc).trim().is_empty() {
                    break;
                }
                out.push(code_para(strip_indent(l, ind)));
                ctx.record_line(cur);
            }
            continue;
        }
        if t.starts_with("<!--") {
            flush_para(&mut para, &mut para_start, &mut out, ctx);
            while let Some(l) = lines.get(i) {
                i += 1;
                if l.contains("-->") {
                    break;
                }
            }
            continue;
        }
        if t.starts_with('>') {
            flush_para(&mut para, &mut para_start, &mut out, ctx);
            let quote_start = i;
            let before = out.len();
            parse_quote(lines, &mut i, ctx, &mut out);
            ctx.record_lines(quote_start, out.len() - before);
            continue;
        }
        if para.is_empty() && t.contains('|') && parse_table(lines, &mut i, t, ctx, &mut out) {
            continue;
        }
        if let Some(m) = list_marker(t)
            && (para.is_empty() || ((!m.ordered || m.start == 1) && !is_blank(t.get(m.len..).unwrap_or(""))))
        {
            flush_para(&mut para, &mut para_start, &mut out, ctx);
            i = parse_list(lines, i, ctx, &mut out);
            continue;
        }
        if para_start.is_none() {
            para_start = Some(i);
        }
        para.push(line.clone());
        i += 1;
    }
    flush_para(&mut para, &mut para_start, &mut out, ctx);
    out
}

/// Parses indented code block starting at `lines[*i]`.
fn parse_indented_code(lines: &[String], i: &mut usize, out: &mut Vec<FBlock>, ctx: &mut BlockCtx<'_>) {
    let mut code: Vec<(usize, &str)> = Vec::new();
    while let Some(line) = lines.get(*i) {
        let cur = *i;
        if is_blank(line) {
            code.push((cur, ""));
        } else if indent(line) >= 4 {
            code.push((cur, strip_indent(line, 4)));
        } else {
            break;
        }
        *i += 1;
    }
    while code.last().is_some_and(|(_, line)| line.is_empty()) {
        code.pop();
    }
    for (line_idx, text) in code {
        out.push(code_para(text));
        ctx.record_line(line_idx);
    }
}

/// Parses block quote starting at `lines[*i]`.
fn parse_quote(lines: &[String], i: &mut usize, ctx: &BlockCtx<'_>, out: &mut Vec<FBlock>) {
    let mut inner = Vec::new();
    while let Some(line) = lines.get(*i) {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if indent(line) < 4 && trimmed.starts_with('>') {
            let content = &trimmed[1..];
            inner.push(content.strip_prefix(' ').unwrap_or(content).to_string());
        } else if !is_blank(line) && !interrupts(line) && inner.last().is_some_and(|line| !is_blank(line)) {
            inner.push(line.clone());
        } else {
            break;
        }
        *i += 1;
    }
    if ctx.depth >= MAX_NEST {
        let mut p = Para::new(Kind::Quote);
        p.push_text(&inner.join(" "), &Fmt::default());
        out.push(FBlock::Para(p));
        return;
    }
    let mut child = BlockCtx { depth: ctx.depth + 1, level: ctx.level, refs: ctx.refs, budget: ctx.budget, lines_out: None };
    for mut block in parse_blocks(&inner, &mut child) {
        if let FBlock::Para(p) = &mut block
            && p.kind == Kind::Normal
        {
            p.kind = Kind::Quote;
        }
        out.push(block);
    }
}

/// Computes columns and byte count for spaces following a list item marker.
fn marker_space(ind: usize, marker_len: usize, after: &str) -> (usize, usize) {
    let mut cols = 0;
    let mut bytes = 0;
    let mut rest = after;
    while let Some(c) = rest.chars().next() {
        match c {
            ' ' => {
                cols += 1;
                bytes += 1;
                rest = &rest[1..];
                if cols >= 4 {
                    break;
                }
            }
            '\t' => {
                let col = ind + marker_len + cols;
                let n = 4 - col % 4;
                cols += n;
                bytes += 1;
                break;
            }
            _ => break,
        }
    }
    (cols, bytes)
}

/// Parse a list starting at `lines[i]`; returns the index after it.
fn parse_list(lines: &[String], mut i: usize, ctx: &mut BlockCtx<'_>, out: &mut Vec<FBlock>) -> usize {
    let Some(first) = lines.get(i).and_then(|l| list_marker(l.trim_start_matches([' ', '\t']))) else { return i + 1 };
    while let Some(line) = lines.get(i) {
        let ind = indent(line);
        if ind >= 4 {
            break;
        }
        let t = line.trim_start_matches([' ', '\t']);
        let Some(m) = list_marker(t) else { break };
        if m.ordered != first.ordered || m.ch != first.ch {
            break;
        }
        let item_start = i;
        let after = t.get(m.len..).unwrap_or("");
        let (spaces, skip) = if indent(after) > 4 {
            (1, 1)
        } else if after.starts_with([' ', '\t']) {
            marker_space(ind, m.len, after)
        } else {
            (0, 0)
        };
        let content_indent = if is_blank(after) { ind + m.len + 1 } else { ind + m.len + spaces };
        let item0 = if is_blank(after) { String::new() } else { after.get(skip.min(after.len())..).unwrap_or("").to_string() };
        let mut item: Vec<String> = vec![item0];
        i += 1;
        while let Some(l) = lines.get(i) {
            if is_blank(l) {
                item.push(String::new());
            } else if indent(l) >= content_indent {
                item.push(strip_indent(l, content_indent).to_string());
            } else if item.last().is_some_and(|x| !is_blank(x)) && !interrupts(l) && list_marker(l.trim_start_matches([' ', '\t'])).is_none() {
                // Lazy continuation of the item's paragraph.
                item.push(l.trim_start_matches([' ', '\t']).to_string());
            } else {
                break;
            }
            i += 1;
        }
        // Blank lines at the end of an item belong between items.
        while item.len() > 1 && item.last().is_some_and(String::is_empty) {
            item.pop();
        }
        let li = ListInfo { ordered: first.ordered, level: ctx.level.min(8), start: first.start, ..Default::default() };
        let inner = if ctx.depth >= MAX_NEST {
            let mut p = Para::default();
            p.push_text(&item.join(" "), &Fmt::default());
            vec![FBlock::Para(p)]
        } else {
            let mut child =
                BlockCtx { depth: ctx.depth + 1, level: ctx.level.saturating_add(1).min(8), refs: ctx.refs, budget: ctx.budget, lines_out: None };
            parse_blocks(&item, &mut child)
        };
        let before_count = out.len();
        let mut placed = false;
        if inner.is_empty() {
            out.push(FBlock::Para(Para { list: Some(li), ..Default::default() }));
            placed = true;
        }
        for b in inner {
            match b {
                FBlock::Para(mut p) if p.list.is_none() => {
                    if !placed {
                        p.list = Some(li);
                        placed = true;
                        out.push(FBlock::Para(p));
                    } else if let Some(FBlock::Para(prev)) = out.last_mut()
                        && prev.list.is_some_and(|x| x.level == li.level)
                    {
                        // A continuation paragraph joins the item with a line break.
                        prev.push_text("\n", &Fmt::default());
                        prev.inlines.extend(p.inlines);
                    } else {
                        p.list = Some(li);
                        out.push(FBlock::Para(p));
                    }
                }
                FBlock::Para(p) => {
                    if !placed {
                        // The item starts with a nested list: give it an empty first line.
                        out.push(FBlock::Para(Para { list: Some(li), ..Default::default() }));
                        placed = true;
                    }
                    out.push(FBlock::Para(p));
                }
                other => {
                    placed = true;
                    out.push(other);
                }
            }
        }
        ctx.record_lines(item_start, out.len() - before_count);
        // Blank lines between items.
        while lines.get(i).is_some_and(|l| is_blank(l)) {
            let next_is_item = lines.get(i + 1).is_some_and(|l| {
                indent(l) < 4 && list_marker(l.trim_start_matches([' ', '\t'])).is_some_and(|m| m.ordered == first.ordered && m.ch == first.ch)
            });
            if !next_is_item {
                return i;
            }
            i += 1;
        }
    }
    i
}
