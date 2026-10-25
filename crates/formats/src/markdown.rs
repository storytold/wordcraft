//! Markdown: a CommonMark subset plus GitHub tables and strikethrough.
//!
//! Import: ATX and setext headings, paragraphs with soft/hard breaks, block quotes (→ Quote
//! style), fenced and indented code (→ one Code paragraph per line), thematic breaks (→ a
//! bottom-border paragraph), bullet and ordered lists (nested by indentation), GFM pipe tables,
//! and inlines: emphasis/strong (the CommonMark delimiter algorithm), `~~strike~~`, code spans,
//! inline links, autolinks, images (`data:` URIs become pictures, others their alt text),
//! backslash escapes, entities, HTML comments (`<!-- ... -->`), `<br>` and `<a id>` anchors.
//!
//! Export writes the same constructs; pictures become `![alt](media-name)` placeholders.

use std::collections::HashMap;

use wordcraft_doc::{Align, Document};

use crate::html::decode_entity;
use crate::model::{self, Cell, FBlock, FTable, Fmt, Inline, Kind, ListInfo, Para, make_img};

/// Nesting limit for block containers (quotes, list items) and inline links.
const MAX_NEST: usize = 32;

// ---------------------------------------------------------------------------------------------
// Import

/// Parse Markdown text into a document.
pub fn import(text: &str) -> Document {
    model::to_doc(&parse(text))
}

/// [`import`] onto `page`: Markdown gives no page size (#1381).
pub fn import_on(text: &str, page: &wordcraft_doc::SectionProps) -> Document {
    model::to_doc_in(&parse(text), page.clone())
}

/// Link reference definitions: the normalised label to its destination.
pub type Refs = HashMap<String, String>;

/// A label as CommonMark compares them: case-folded, and any run of white space one space
/// (§4.7, "matches"). `[FOO bar]` and `[foo   BAR]` are the same label.
fn ref_label(raw: &str) -> String {
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

// A closing block-comment line can contain another comment before visible source text.
// Walk those delimiters without copying the remaining line at each opener.
fn block_comment_tail(mut text: &str) -> Option<&str> {
    loop {
        let end = text.find("-->")?;
        text = text.get(end + 3..)?;
        if !text.trim_start().starts_with("<!--") {
            return Some(text);
        }
    }
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
fn take_definitions(lines: &mut Vec<String>) -> Refs {
    let mut refs = Refs::new();
    let mut open: Option<(char, usize)> = None;
    let mut in_comment = false;
    // The start of the document begins a block.
    let mut starts_block = true;
    lines.retain(|line| {
        let t = line.trim_start_matches([' ', '\t']);
        if in_comment {
            let tail = block_comment_tail(line);
            in_comment = tail.is_none();
            // parse_blocks reprocesses visible tails as source, including opening fences.
            open = tail.filter(|s| indent(s) < 4).and_then(|s| fence(s.trim_start_matches([' ', '\t'])));
            starts_block = tail.is_some_and(|s| is_blank(s) || atx_heading(s.trim_start()).is_some() || thematic_break(s.trim_start()));
            return true;
        }
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
        if indent(line) < 4 && t.starts_with("<!--") {
            let tail = block_comment_tail(line);
            in_comment = tail.is_none();
            // parse_blocks reprocesses visible tails as source, including opening fences.
            open = tail.filter(|s| indent(s) < 4).and_then(|s| fence(s.trim_start_matches([' ', '\t'])));
            starts_block = tail.is_some_and(|s| is_blank(s) || atx_heading(s.trim_start()).is_some() || thematic_break(s.trim_start()));
            return true;
        }
        if starts_block && let Some((label, dest)) = link_definition(line) {
            // The first definition of a label wins, as CommonMark says, and definitions may
            // follow one another.
            refs.entry(label).or_insert(dest);
            return false;
        }
        starts_block = is_blank(line) || atx_heading(t).is_some() || thematic_break(t);
        true
    });
    refs
}

/// Parse Markdown text into the flow model.
pub fn parse(text: &str) -> model::Flow {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut lines: Vec<String> = text.split('\n').map(|l| expand_tabs(l.trim_end_matches('\r'))).collect();
    let refs = take_definitions(&mut lines);
    let mut blocks = parse_blocks(lines, 0, 0, &refs);
    // A leading "Title" from front matter isn't supported; a first-level heading stays a heading.
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
    }
    model::Flow { blocks, meta: Default::default() }
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

fn thematic_break(t: &str) -> bool {
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

fn split_row(l: &str) -> Vec<String> {
    let t = l.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if let Some(before) = t.strip_suffix('|') {
        let num_bs = before.chars().rev().take_while(|&c| c == '\\').count();
        if num_bs % 2 == 0 { before } else { t }
    } else {
        t
    };
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some(&'|') => {
                    cur.push('|');
                    chars.next();
                }
                Some(&'\\') => {
                    cur.push('\\');
                    cur.push('\\');
                    chars.next();
                }
                _ => cur.push('\\'),
            },
            '|' => cells.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn delimiter_row(l: &str) -> Option<Vec<Option<Align>>> {
    if !l.contains('-') {
        return None;
    }
    let cells = split_row(l);
    let mut out = Vec::new();
    for c in cells {
        let c = c.trim();
        let left = c.starts_with(':');
        let right = c.ends_with(':');
        let core = c.trim_matches(':');
        if core.is_empty() || !core.chars().all(|x| x == '-') {
            return None;
        }
        out.push(match (left, right) {
            (true, true) => Some(Align::Center),
            (false, true) => Some(Align::Right),
            (true, false) => Some(Align::Left),
            _ => None,
        });
    }
    Some(out)
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

fn flush_para(lines: &mut Vec<String>, out: &mut Vec<FBlock>, depth: usize, refs: &Refs) {
    if lines.is_empty() {
        return;
    }
    let text = join_lines(lines);
    lines.clear();
    let inlines = parse_inlines(&text, &Fmt::default(), depth, refs);
    if inlines.is_empty() {
        return;
    }
    let p = Para { inlines, ..Default::default() };
    out.push(FBlock::Para(p));
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

fn code_para(line: &str) -> FBlock {
    let mut p = Para::new(Kind::Code);
    p.push_text(line, &Fmt::default());
    FBlock::Para(p)
}

/// Parse block structure. `level` is the list nesting level for lists found here.
fn parse_blocks(mut lines: Vec<String>, depth: usize, level: u8, refs: &Refs) -> Vec<FBlock> {
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        if is_blank(line) {
            flush_para(&mut para, &mut out, depth, refs);
            i += 1;
            continue;
        }
        let ind = indent(line);
        if ind >= 4 {
            if !para.is_empty() {
                para.push(line.clone());
                i += 1;
                continue;
            }
            // Indented code.
            let mut code: Vec<&str> = Vec::new();
            while let Some(l) = lines.get(i) {
                if is_blank(l) {
                    code.push("");
                } else if indent(l) >= 4 {
                    code.push(strip_indent(l, 4));
                } else {
                    break;
                }
                i += 1;
            }
            while code.last().is_some_and(|l| l.is_empty()) {
                code.pop();
            }
            out.extend(code.into_iter().map(code_para));
            continue;
        }
        let t = line.trim_start_matches([' ', '\t']);
        // Setext heading underline.
        if !para.is_empty() {
            let u = t.trim_end();
            if !u.is_empty() && (u.chars().all(|c| c == '=') || (u.chars().all(|c| c == '-'))) {
                let lvl = if u.starts_with('=') { 1 } else { 2 };
                let text = join_lines(&para);
                para.clear();
                let mut p = Para::new(Kind::Heading(lvl));
                p.inlines = parse_inlines(&text, &Fmt::default(), depth, refs);
                out.push(FBlock::Para(p));
                i += 1;
                continue;
            }
        }
        if let Some((n, body)) = atx_heading(t) {
            flush_para(&mut para, &mut out, depth, refs);
            let mut p = Para::new(Kind::Heading(n));
            p.inlines = parse_inlines(body, &Fmt::default(), depth, refs);
            out.push(FBlock::Para(p));
            i += 1;
            continue;
        }
        if thematic_break(t) {
            flush_para(&mut para, &mut out, depth, refs);
            out.push(FBlock::Para(Para::new(Kind::Rule)));
            i += 1;
            continue;
        }
        if let Some((fc, fl)) = fence(t) {
            flush_para(&mut para, &mut out, depth, refs);
            i += 1;
            while let Some(l) = lines.get(i) {
                i += 1;
                let lt = l.trim_start_matches([' ', '\t']);
                if indent(l) < 4 && fence(lt).is_some_and(|(c, n)| c == fc && n >= fl) && lt.trim_start_matches(fc).trim().is_empty() {
                    break;
                }
                out.push(code_para(strip_indent(l, ind)));
            }
            continue;
        }
        if t.starts_with("<!--") {
            flush_para(&mut para, &mut out, depth, refs);
            while let Some(l) = lines.get(i) {
                if let Some(tail) = block_comment_tail(l) {
                    if is_blank(tail) {
                        i += 1;
                    } else {
                        let tail = tail.to_string();
                        if let Some(line) = lines.get_mut(i) {
                            *line = tail;
                        }
                    }
                    break;
                }
                i += 1;
            }
            continue;
        }
        if t.starts_with('>') {
            flush_para(&mut para, &mut out, depth, refs);
            let mut inner: Vec<String> = Vec::new();
            while let Some(l) = lines.get(i) {
                let lt = l.trim_start_matches([' ', '\t']);
                if indent(l) < 4 && lt.starts_with('>') {
                    let s = &lt[1..];
                    inner.push(s.strip_prefix(' ').unwrap_or(s).to_string());
                } else if !is_blank(l) && !interrupts(l) && inner.last().is_some_and(|x| !is_blank(x)) {
                    inner.push(l.clone());
                } else {
                    break;
                }
                i += 1;
            }
            if depth >= MAX_NEST {
                let mut p = Para::new(Kind::Quote);
                p.push_text(&inner.join(" "), &Fmt::default());
                out.push(FBlock::Para(p));
            } else {
                for mut b in parse_blocks(inner, depth + 1, level, refs) {
                    if let FBlock::Para(p) = &mut b
                        && p.kind == Kind::Normal
                    {
                        p.kind = Kind::Quote;
                    }
                    out.push(b);
                }
            }
            continue;
        }
        if para.is_empty()
            && t.contains('|')
            && let Some(aligns) = lines.get(i + 1).and_then(|l| delimiter_row(l))
        {
            let head = split_row(t);
            if head.len() == aligns.len() {
                let mut table = FTable::default();
                let cell = |s: &str, header: bool, a: Option<Align>| {
                    let p = Para { align: a, inlines: parse_inlines(s, &Fmt::default(), depth, refs), ..Default::default() };
                    Cell { blocks: vec![FBlock::Para(p)], header, ..Default::default() }
                };
                table.rows.push(head.iter().zip(&aligns).map(|(s, a)| cell(s, true, *a)).collect());
                i += 2;
                while let Some(l) = lines.get(i) {
                    if is_blank(l) || !l.contains('|') || interrupts(l) {
                        break;
                    }
                    let mut cells = split_row(l);
                    cells.resize(aligns.len(), String::new());
                    table.rows.push(cells.iter().zip(&aligns).map(|(s, a)| cell(s, false, *a)).collect());
                    i += 1;
                }
                out.push(FBlock::Table(table));
                continue;
            }
        }
        if let Some(m) = list_marker(t)
            && (para.is_empty() || ((!m.ordered || m.start == 1) && !is_blank(t.get(m.len..).unwrap_or(""))))
        {
            flush_para(&mut para, &mut out, depth, refs);
            i = parse_list(&lines, i, depth, level, &mut out, refs);
            continue;
        }
        para.push(line.clone());
        i += 1;
    }
    flush_para(&mut para, &mut out, depth, refs);
    out
}

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
fn parse_list(lines: &[String], mut i: usize, depth: usize, level: u8, out: &mut Vec<FBlock>, refs: &Refs) -> usize {
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
        let li = ListInfo { ordered: first.ordered, level: level.min(8), start: first.start };
        let inner = if depth >= MAX_NEST {
            let mut p = Para::default();
            p.push_text(&item.join(" "), &Fmt::default());
            vec![FBlock::Para(p)]
        } else {
            parse_blocks(item, depth + 1, level.saturating_add(1).min(8), refs)
        };
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

// ---- inlines ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Emph {
    Em,
    Strong,
    Strike,
}

enum Node {
    Text(String, Fmt),
    Inl(Inline),
    Delim { ch: char, orig: usize, left: usize, open: bool, close: bool, opens: Vec<Emph>, closes: Vec<Emph> },
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace() && !c.is_control())
}

/// Parse inline Markdown into formatted spans (on top of `base`).
pub fn parse_inlines(s: &str, base: &Fmt, depth: usize, refs: &Refs) -> Vec<Inline> {
    let nodes = tokenize(s, base, depth, refs);
    emit(nodes)
}

fn push_text(nodes: &mut Vec<Node>, s: &str, f: &Fmt) {
    if s.is_empty() {
        return;
    }
    if let Some(Node::Text(t, pf)) = nodes.last_mut()
        && pf == f
    {
        t.push_str(s);
        return;
    }
    nodes.push(Node::Text(s.to_string(), f.clone()));
}

/// The closing `]` matching the `[` at byte `open` (escapes and code spans respected). Gives up
/// after looking `max` bytes ahead. Records in `known` every bracket pair it passes, and every `[`
/// still open at the end of the text: a scan from one of those would take the same path.
fn matching_bracket(s: &str, open: usize, max: usize, ticks: &Backticks, known: &mut HashMap<usize, Option<usize>>) -> Option<usize> {
    let b = s.as_bytes();
    let mut opens: Vec<usize> = Vec::new();
    let mut i = open;
    while let Some(&c) = b.get(i) {
        if i - open > max {
            return None;
        }
        match c {
            b'\\' => i += 1,
            b'`' => {
                let n = b.get(i..).map(|r| r.iter().take_while(|x| **x == b'`').count()).unwrap_or(1);
                if let Some(end) = ticks.find(i + n, n) {
                    i = end + n - 1;
                } else {
                    i += n - 1;
                }
            }
            b'[' => opens.push(i),
            b']' => {
                if let Some(o) = opens.pop() {
                    known.insert(o, Some(i));
                }
                if opens.is_empty() {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    for o in opens {
        known.insert(o, None);
    }
    None
}

/// Where a text's runs of backticks start, by run length. Finding the run that closes a code span
/// is then a lookup: scanning the rest of the text for it, again for every run nothing closes
/// (each of a different length) and from every link bracket over them, was superlinear.
struct Backticks(HashMap<usize, Vec<usize>>);

impl Backticks {
    fn new(s: &str) -> Self {
        let b = s.as_bytes();
        let mut runs: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut i = 0;
        while let Some(&c) = b.get(i) {
            if c == b'`' {
                let k = b.get(i..).map(|r| r.iter().take_while(|x| **x == b'`').count()).unwrap_or(1);
                runs.entry(k).or_default().push(i);
                i += k;
            } else {
                i += 1;
            }
        }
        Self(runs)
    }

    /// Byte index of a run of exactly `n` backticks at or after `from` (which is not inside a run).
    fn find(&self, from: usize, n: usize) -> Option<usize> {
        let starts = self.0.get(&n)?;
        starts.get(starts.partition_point(|&p| p < from)).copied()
    }
}

/// Parse `(dest "title")` at byte `at` (which holds `(`): (destination, index after `)`).
fn link_dest(s: &str, at: usize) -> Option<(String, usize)> {
    let rest = s.get(at..)?.strip_prefix('(')?;
    let mut i = at + 1;
    let ws = rest.len() - rest.trim_start().len();
    i += ws;
    let r = s.get(i..)?;
    let (dest, used) = if let Some(inner) = r.strip_prefix('<') {
        let end = inner.find(['>', '\n'])?;
        if inner.get(end..)?.starts_with('\n') {
            return None;
        }
        (inner.get(..end)?.to_string(), end + 2)
    } else {
        let mut depth = 0i32;
        let mut end = r.len();
        let mut chars = r.char_indices().peekable();
        while let Some((k, c)) = chars.next() {
            match c {
                '\\' => {
                    chars.next();
                }
                '(' => depth += 1,
                ')' if depth == 0 => {
                    end = k;
                    break;
                }
                ')' => depth -= 1,
                c if c.is_whitespace() => {
                    end = k;
                    break;
                }
                _ => {}
            }
        }
        (r.get(..end)?.to_string(), end)
    };
    i += used;
    let r = s.get(i..)?;
    let ws = r.len() - r.trim_start().len();
    i += ws;
    let r = s.get(i..)?;
    if let Some(q) = r.chars().next().filter(|c| matches!(c, '"' | '\'' | '(')) {
        let close = if q == '(' { ')' } else { q };
        let end = r.get(1..)?.find(close)?;
        i += end + 2;
        let r = s.get(i..)?;
        i += r.len() - r.trim_start().len();
    }
    if s.get(i..)?.starts_with(')') { Some((unescape(&dest), i + 1)) } else { None }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(n) = chars.peek().copied().filter(char::is_ascii_punctuation)
        {
            out.push(n);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// The value of attribute `name` in an HTML tag's text.
fn tag_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(k) = lower.get(from..)?.find(name) {
        let at = from + k;
        let before = lower.get(..at)?.chars().next_back();
        let after = lower.get(at + name.len()..)?.trim_start();
        if before.is_some_and(char::is_whitespace) && after.starts_with('=') {
            let v = after.get(1..)?.trim_start();
            let q = v.chars().next()?;
            return Some(if q == '"' || q == '\'' {
                let inner = v.get(1..)?;
                let off = tag.len() - inner.len();
                let end = inner.find(q)?;
                tag.get(off..off + end)?.to_string()
            } else {
                let end = v.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(v.len());
                let off = tag.len() - v.len();
                tag.get(off..off + end)?.to_string()
            });
        }
        from = at + name.len();
    }
    None
}

fn tokenize(s: &str, base: &Fmt, depth: usize, refs: &Refs) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    // Bytes the link and image scans below may look ahead in total. Each unmatched `[` or `![(`
    // would otherwise rescan the rest of the text, which is quadratic on hostile input; once the
    // budget is gone, further brackets are plain text. A scan pays for what it reads; brackets an
    // earlier scan passed are looked up in `known` instead.
    let mut scan_budget = s.len().saturating_mul(8).saturating_add(8192);
    let ticks = Backticks::new(s);
    let mut comment_search_exhausted = false;
    let mut known: HashMap<usize, Option<usize>> = HashMap::new();
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut text_start = 0usize;
    macro_rules! flush {
        () => {
            if text_start < i {
                push_text(&mut nodes, s.get(text_start..i).unwrap_or(""), base);
            }
        };
    }
    while let Some(&c) = b.get(i) {
        match c {
            b'\\' => {
                flush!();
                match s.get(i + 1..).and_then(|r| r.chars().next()) {
                    Some(n) if n.is_ascii_punctuation() => {
                        push_text(&mut nodes, &n.to_string(), base);
                        i += 2;
                    }
                    _ => {
                        push_text(&mut nodes, "\\", base);
                        i += 1;
                    }
                }
                text_start = i;
            }
            b'`' => {
                flush!();
                let n = b.get(i..).map(|r| r.iter().take_while(|x| **x == b'`').count()).unwrap_or(1);
                match ticks.find(i + n, n) {
                    Some(end) => {
                        let mut code = s.get(i + n..end).unwrap_or("").replace('\n', " ");
                        if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty() {
                            code = code[1..code.len() - 1].to_string();
                        }
                        let mut f = base.clone();
                        f.code = true;
                        push_text(&mut nodes, &code, &f);
                        i = end + n;
                    }
                    None => {
                        push_text(&mut nodes, s.get(i..i + n).unwrap_or(""), base);
                        i += n;
                    }
                }
                text_start = i;
            }
            b'*' | b'_' | b'~' => {
                flush!();
                let ch = c as char;
                let n = b.get(i..).map(|r| r.iter().take_while(|x| **x == c).count()).unwrap_or(1);
                let before = s.get(..i).and_then(|r| r.chars().next_back()).unwrap_or(' ');
                let after = s.get(i + n..).and_then(|r| r.chars().next()).unwrap_or(' ');
                let lf = !after.is_whitespace() && (!is_punct(after) || before.is_whitespace() || is_punct(before));
                let rf = !before.is_whitespace() && (!is_punct(before) || after.is_whitespace() || is_punct(after));
                let (open, close) = match ch {
                    '_' => (lf && (!rf || is_punct(before)), rf && (!lf || is_punct(after))),
                    '~' if n != 2 => (false, false),
                    _ => (lf, rf),
                };
                if open || close {
                    nodes.push(Node::Delim { ch, orig: n, left: n, open, close, opens: Vec::new(), closes: Vec::new() });
                } else {
                    push_text(&mut nodes, s.get(i..i + n).unwrap_or(""), base);
                }
                i += n;
                text_start = i;
            }
            b'!' | b'[' if depth < MAX_NEST => {
                let img = c == b'!';
                let open = if img { i + 1 } else { i };
                let parsed = if b.get(open) == Some(&b'[') {
                    let found = match known.get(&open).copied() {
                        Some(found) => found,
                        None if scan_budget > 0 => {
                            let found = matching_bracket(s, open, scan_budget, &ticks, &mut known);
                            scan_budget = scan_budget.saturating_sub(found.unwrap_or(s.len()).saturating_sub(open));
                            found
                        }
                        None => None,
                    };
                    found.filter(|_| scan_budget > 0).and_then(|close| {
                        let text = s.get(open + 1..close)?;
                        if b.get(close + 1) == Some(&b'(') {
                            // A destination that does not parse may have been read to the end of the text.
                            let dest = link_dest(s, close + 1);
                            scan_budget = scan_budget.saturating_sub(dest.as_ref().map_or(s.len(), |d| d.1).saturating_sub(close));
                            let (dest, end) = dest?;
                            return Some((text.to_string(), dest, end));
                        }
                        // A reference link: `[text][label]`, `[text][]` and `[text]` all take
                        // their destination from a definition collected before parsing (#3238).
                        // The shortcut form is only a link when the label is defined, so an
                        // ordinary bracketed phrase is still text.
                        let (label, end) = match s.get(close + 1..).and_then(|rest| rest.strip_prefix('[')) {
                            Some(rest) => {
                                let shut = rest.find(']')?;
                                let inner = rest.get(..shut)?;
                                let label = if inner.trim().is_empty() { text } else { inner };
                                (label, close + 2 + shut + 1)
                            }
                            None => (text, close + 1),
                        };
                        Some((text.to_string(), refs.get(&ref_label(label))?.clone(), end))
                    })
                } else {
                    None
                };
                match parsed {
                    Some((inner, dest, end)) if img => {
                        flush!();
                        let alt: String = parse_inlines(&inner, &Fmt::default(), depth + 1, refs)
                            .iter()
                            .filter_map(|x| if let Inline::Text(t, _) = x { Some(t.as_str()) } else { None })
                            .collect();
                        match model::data_uri(&dest).and_then(|d| make_img(d, None, None, &alt)) {
                            Some(im) => nodes.push(Node::Inl(Inline::Image(im))),
                            None => push_text(&mut nodes, &alt, base),
                        }
                        i = end;
                        text_start = i;
                    }
                    Some((inner, dest, end)) if base.link.is_none() => {
                        flush!();
                        let mut f = base.clone();
                        f.link = Some(dest);
                        for x in parse_inlines(&inner, &f, depth + 1, refs) {
                            nodes.push(Node::Inl(x));
                        }
                        i = end;
                        text_start = i;
                    }
                    _ => i += 1,
                }
            }
            b'<' => {
                let comment_end = s.get(i..).filter(|r| r.starts_with("<!--")).and_then(|rest| {
                    if rest.starts_with("<!-->") {
                        return Some(5);
                    }
                    if rest.starts_with("<!--->") {
                        return Some(6);
                    }
                    if comment_search_exhausted {
                        return None;
                    }
                    let end = rest.get(4..).and_then(|r| r.find("-->")).map(|p| p + 7);
                    // Successful searches are consumed below. After a miss no later opener
                    // can have a closer, so scanning the same suffix again would be quadratic.
                    comment_search_exhausted = end.is_none();
                    end
                });
                if let Some(len) = comment_end {
                    flush!();
                    i += len;
                    text_start = i;
                    continue;
                }
                let rest = s.get(i + 1..).unwrap_or("");
                let end = within(rest, MAX_TAG).find('>');
                let inner = end.and_then(|e| rest.get(..e)).unwrap_or("");
                let lower = inner.to_ascii_lowercase();
                let is_auto = !inner.is_empty()
                    && !inner.contains([' ', '<', '\n'])
                    && (inner.contains("://") || lower.starts_with("mailto:") || (inner.contains('@') && !inner.contains('/')));
                if is_auto {
                    flush!();
                    let mut f = base.clone();
                    f.link = Some(if inner.contains('@') && !inner.contains(':') { format!("mailto:{inner}") } else { inner.to_string() });
                    push_text(&mut nodes, inner, &f);
                    i += inner.len() + 2;
                    text_start = i;
                } else if matches!(lower.trim_end_matches('/').trim(), "br") {
                    flush!();
                    push_text(&mut nodes, "\n", base);
                    i += inner.len() + 2;
                    text_start = i;
                } else if lower.starts_with("a ") && rest.get(inner.len() + 1..).is_some_and(|r| r.to_ascii_lowercase().starts_with("</a>")) {
                    flush!();
                    if let Some(name) = tag_attr(inner, "id").or_else(|| tag_attr(inner, "name")) {
                        nodes.push(Node::Inl(Inline::Anchor(name)));
                    }
                    i += inner.len() + 2 + 4;
                    text_start = i;
                } else {
                    i += 1;
                }
            }
            b'&' => {
                let rest = s.get(i..).unwrap_or("");
                match within(rest, 33).find(';').and_then(|k| decode_entity(rest.get(1..k)?).map(|ch| (ch, k))) {
                    Some((ch, k)) => {
                        flush!();
                        push_text(&mut nodes, &ch, base);
                        i += k + 1;
                        text_start = i;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    flush!();
    nodes
}

/// Longest `<...>` tag or autolink taken as inline HTML.
const MAX_TAG: usize = 4096;

/// The first `n` bytes of `s` (fewer if that would cut a character), so a search for a closing
/// character stays short however long the text is.
fn within(s: &str, n: usize) -> &str {
    let mut end = n.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

fn emph_index(ch: char) -> usize {
    match ch {
        '*' => 0,
        '_' => 1,
        _ => 2,
    }
}

/// The CommonMark "process emphasis" pass over the delimiter runs, then flattening into spans.
fn emit(mut nodes: Vec<Node>) -> Vec<Inline> {
    // Delimiter stack of node indexes; bottoms per (char, closer length % 3, closer can open).
    let mut stack: Vec<usize> = Vec::new();
    let mut bottom = [[[None::<usize>; 2]; 3]; 3];
    for c in 0..nodes.len() {
        let (ch, corig, copen, cclose) = match nodes.get(c) {
            Some(Node::Delim { ch, orig, open, close, .. }) => (*ch, *orig, *open, *close),
            _ => continue,
        };
        if cclose {
            loop {
                let cleft = match nodes.get(c) {
                    Some(Node::Delim { left, .. }) => *left,
                    _ => 0,
                };
                if cleft == 0 {
                    break;
                }
                let bot = bottom.get(emph_index(ch)).and_then(|x| x.get(corig % 3)).and_then(|x| x.get(copen as usize)).copied().flatten();
                let mut found = None;
                for (k, &o) in stack.iter().enumerate().rev() {
                    if bot.is_some_and(|b| o <= b) {
                        break;
                    }
                    if let Some(Node::Delim { ch: och, orig: oorig, left: oleft, open: oopen, close: oclose, .. }) = nodes.get(o) {
                        if *och != ch || !*oopen || *oleft == 0 {
                            continue;
                        }
                        if ch == '~' && (*oleft < 2 || cleft < 2) {
                            continue;
                        }
                        let odd = (*oclose || copen) && (oorig + corig) % 3 == 0 && !(oorig % 3 == 0 && corig % 3 == 0);
                        if odd {
                            continue;
                        }
                        found = Some((k, o, *oleft));
                        break;
                    }
                }
                let Some((k, o, oleft)) = found else {
                    if let Some(slot) = bottom.get_mut(emph_index(ch)).and_then(|x| x.get_mut(corig % 3)).and_then(|x| x.get_mut(copen as usize)) {
                        *slot = Some(c.saturating_sub(1));
                    }
                    break;
                };
                let (used, kind) = if ch == '~' {
                    (2, Emph::Strike)
                } else if oleft >= 2 && cleft >= 2 {
                    (2, Emph::Strong)
                } else {
                    (1, Emph::Em)
                };
                if let Some(Node::Delim { left, opens, .. }) = nodes.get_mut(o) {
                    *left -= used;
                    opens.push(kind);
                }
                if let Some(Node::Delim { left, closes, .. }) = nodes.get_mut(c) {
                    *left -= used;
                    closes.push(kind);
                }
                // Delimiters between opener and closer can no longer match.
                stack.truncate(k + 1);
                if oleft - used == 0 {
                    stack.truncate(k);
                }
            }
        }
        let left = match nodes.get(c) {
            Some(Node::Delim { left, .. }) => *left,
            _ => 0,
        };
        if copen && left > 0 {
            stack.push(c);
        }
    }
    // Flatten.
    let mut out: Vec<Inline> = Vec::new();
    let (mut em, mut strong, mut strike) = (0usize, 0usize, 0usize);
    let push = |out: &mut Vec<Inline>, t: &str, f: &Fmt, em: usize, strong: usize, strike: usize| {
        if t.is_empty() {
            return;
        }
        let mut f = f.clone();
        f.italic |= em > 0;
        f.bold |= strong > 0;
        f.strike |= strike > 0;
        if let Some(Inline::Text(pt, pf)) = out.last_mut()
            && *pf == f
        {
            pt.push_str(t);
            return;
        }
        out.push(Inline::Text(t.to_string(), f));
    };
    let base_of = |nodes: &[Node], k: usize| -> Fmt {
        // Delimiter literals take the formatting (link, …) of neighbouring text.
        for j in (0..k).rev() {
            if let Some(Node::Text(_, f)) = nodes.get(j) {
                return Fmt { bold: false, italic: false, strike: false, ..f.clone() };
            }
        }
        Fmt::default()
    };
    for (k, n) in nodes.iter().enumerate() {
        match n {
            Node::Text(t, f) => push(&mut out, t, f, em, strong, strike),
            Node::Inl(Inline::Text(t, f)) => push(&mut out, t, f, em, strong, strike),
            Node::Inl(other) => out.push(other.clone()),
            Node::Delim { ch, left, opens, closes, .. } => {
                for e in closes {
                    match e {
                        Emph::Em => em = em.saturating_sub(1),
                        Emph::Strong => strong = strong.saturating_sub(1),
                        Emph::Strike => strike = strike.saturating_sub(1),
                    }
                }
                let lit: String = std::iter::repeat_n(*ch, *left).collect();
                let f = base_of(&nodes, k);
                push(&mut out, &lit, &f, em, strong, strike);
                for e in opens {
                    match e {
                        Emph::Em => em += 1,
                        Emph::Strong => strong += 1,
                        Emph::Strike => strike += 1,
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Export

/// Escape text for Markdown (inline context).
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '|' | '&' => {
                out.push('\\');
                out.push(c);
            }
            '\u{000C}' | '\u{000E}' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Escape what would start a block at the beginning of a line. Leading spaces are kept, but at
/// most three (a tab counts as four): four would turn the line into an indented code block.
fn escape_line_start(s: String) -> String {
    let t = s.trim_start_matches([' ', '\t']);
    let lead = s.get(..s.len() - t.len()).unwrap_or("");
    let cols: usize = lead.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
    let indent = " ".repeat(cols.min(3));
    let needs = t.starts_with('#')
        || t.starts_with('>')
        || t.starts_with("- ")
        || t.starts_with("+ ")
        || t.starts_with("-\t")
        || t.starts_with("+\t")
        || t == "-"
        || t == "+"
        || t.starts_with('=')
        || (t.starts_with('-') && t.trim_end_matches([' ', '\t']).chars().all(|c| c == '-'))
        || thematic_break(t)
        || {
            let d = t.chars().take_while(char::is_ascii_digit).count();
            d > 0
                && t.get(d..).is_some_and(|r| {
                    r.starts_with(". ") || r.starts_with(") ") || r.starts_with(".\t") || r.starts_with(")\t") || r == "." || r == ")"
                })
        };
    if !needs {
        return if cols > 3 || lead.contains('\t') { format!("{indent}{t}") } else { s };
    }
    if let Some(d) = t.find(['.', ')']).filter(|_| t.starts_with(|c: char| c.is_ascii_digit())) {
        return format!("{indent}{}\\{}", &t[..d], &t[d..]);
    }
    format!("{indent}\\{t}")
}

/// [`escape_line_start`] for every line of a body, including those after a manual line break.
fn escape_lines(body: String) -> String {
    if !body.contains("\\\n") {
        return escape_line_start(body);
    }
    body.split("\\\n").map(|l| escape_line_start(l.to_string())).collect::<Vec<_>>().join("\\\n")
}

fn code_span(t: &str) -> String {
    let t = t.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let mut longest = 0;
    let mut cur = 0;
    for c in t.chars() {
        if c == '`' {
            cur += 1;
            longest = longest.max(cur);
        } else {
            cur = 0;
        }
    }
    let ticks = "`".repeat(longest + 1);
    let pad = if t.starts_with('`') || t.ends_with('`') || (t.starts_with(' ') && t.ends_with(' ') && !t.trim().is_empty()) { " " } else { "" };
    format!("{ticks}{pad}{t}{pad}{ticks}")
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Mark {
    Bold,
    Italic,
    Strike,
}

/// Inline spans → Markdown with properly nested markers.
fn inlines_md(inlines: &[Inline]) -> String {
    inlines_md_impl(inlines, false)
}

fn inlines_md_table(inlines: &[Inline]) -> String {
    inlines_md_impl(inlines, true)
}

fn inlines_md_impl(inlines: &[Inline], in_table: bool) -> String {
    // Split into (text, fmt) pieces with whitespace moved outside emphasis.
    let mut pieces: Vec<(String, Fmt, Option<String>)> = Vec::new();
    let eq = crate::model::equations_as_text(inlines);
    // Markdown keeps no ruby: its base text goes through the text path below.
    let inlines = crate::model::ruby_as_text(&eq);
    for i in inlines.iter() {
        match i {
            Inline::Text(t, f) => {
                let has_emph = f.bold || f.italic || f.strike;
                if has_emph && !f.code {
                    let core = t.trim_matches([' ', '\t', '\n']);
                    let lead = &t[..t.len() - t.trim_start_matches([' ', '\t', '\n']).len()];
                    let trail = &t[t.trim_end_matches([' ', '\t', '\n']).len()..];
                    let plain = Fmt { bold: false, italic: false, strike: false, ..f.clone() };
                    if !lead.is_empty() {
                        pieces.push((lead.to_string(), plain.clone(), None));
                    }
                    if !core.is_empty() {
                        pieces.push((core.to_string(), f.clone(), None));
                    }
                    if !trail.is_empty() && !core.is_empty() {
                        pieces.push((trail.to_string(), plain, None));
                    }
                } else {
                    pieces.push((t.clone(), f.clone(), None));
                }
            }
            Inline::Image(img) => {
                let name = format!("image.{}", img.ext);
                pieces.push((String::new(), Fmt::default(), Some(format!("![{}]({})", escape(&img.alt), name))));
            }
            Inline::Anchor(a) => {
                let safe: String = a.chars().filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')).collect();
                pieces.push((String::new(), Fmt::default(), Some(format!("<a id=\"{safe}\"></a>"))));
            }
            Inline::Figure(alt) => pieces.push((String::new(), Fmt::default(), Some(format!("![{}]()", escape(alt))))),
            Inline::Equation { .. } => {}
            // Read as its base text by `ruby_as_text` above.
            Inline::Ruby { .. } => {}
        }
    }
    let mut out = String::new();
    let mut stack: Vec<Mark> = Vec::new();
    let mut link: Option<String> = None;
    let close_all = |out: &mut String, stack: &mut Vec<Mark>| {
        while let Some(m) = stack.pop() {
            out.push_str(marker(m));
        }
    };
    for (text, f, raw) in pieces {
        // Links are the outermost construct; a script or other non-web target is plain text.
        let f_link = f.link.clone().filter(|l| wordcraft_doc::link::is_safe_href(l));
        if f_link != link || raw.is_some() {
            close_all(&mut out, &mut stack);
            if let Some(l) = &link {
                out.push_str(&format!("]({})", link_target(l)));
            }
            link = None;
            if let Some(r) = raw {
                out.push_str(&r);
                continue;
            }
            if let Some(l) = f_link {
                out.push('[');
                link = Some(l);
            }
        }
        let want: Vec<Mark> =
            [(f.bold, Mark::Bold), (f.italic, Mark::Italic), (f.strike, Mark::Strike)].iter().filter(|x| x.0).map(|x| x.1).collect();
        // Close until the stack is a prefix of what we want.
        while let Some(pos) = stack.iter().position(|m| !want.contains(m)) {
            while stack.len() > pos {
                if let Some(m) = stack.pop() {
                    out.push_str(marker(m));
                }
            }
        }
        for m in want {
            if !stack.contains(&m) {
                out.push_str(marker(m));
                stack.push(m);
            }
        }
        if f.code {
            if in_table {
                let t = text.replace('|', "\\|");
                out.push_str(&code_span(&t));
            } else {
                out.push_str(&code_span(&text));
            }
        } else {
            out.push_str(&escape(&text).replace('\n', "\\\n").replace('\t', "    "));
        }
    }
    close_all(&mut out, &mut stack);
    if let Some(l) = &link {
        out.push_str(&format!("]({})", link_target(l)));
    }
    out
}

fn link_target(l: &str) -> String {
    if l.contains([' ', '(', ')', '<', '>']) { format!("<{}>", l.replace(['<', '>', '\n'], "")) } else { l.to_string() }
}

fn marker(m: Mark) -> &'static str {
    match m {
        Mark::Bold => "**",
        Mark::Italic => "*",
        Mark::Strike => "~~",
    }
}

fn table_md(t: &FTable, out: &mut Vec<String>) {
    let cols = t.cols().max(1);
    let cell_md = |c: &Cell| -> String {
        let mut parts = Vec::new();
        for b in &c.blocks {
            match b {
                FBlock::Para(p) => parts.push(inlines_md_table(&p.inlines).replace("\\\n", "<br>")),
                FBlock::Table(inner) => parts.push(escape(&inner.rows.iter().flat_map(|r| r.iter().map(Cell::text)).collect::<Vec<_>>().join(" "))),
            }
        }
        parts.join("<br>").replace('\n', " ")
    };
    let align_of = |c: &Cell| c.blocks.iter().find_map(|b| if let FBlock::Para(p) = b { Some(p.align) } else { None }).flatten();
    let row_md = |r: &Vec<Cell>| -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for c in r {
            v.push(if c.covered { String::new() } else { cell_md(c) });
            for _ in 1..c.colspan.clamp(1, 63) {
                v.push(String::new());
            }
        }
        v.resize(cols, String::new());
        v
    };
    let Some(head) = t.rows.first() else { return };
    out.push(format!("| {} |", row_md(head).join(" | ")));
    let mut aligns: Vec<Option<Align>> = Vec::new();
    for c in head {
        aligns.push(align_of(c));
        for _ in 1..c.colspan.clamp(1, 63) {
            aligns.push(None);
        }
    }
    aligns.resize(cols, None);
    let delim: Vec<&str> = aligns
        .iter()
        .map(|a| match a {
            Some(Align::Center) => ":---:",
            Some(Align::Right) => "---:",
            Some(Align::Left) => ":---",
            _ => "---",
        })
        .collect();
    out.push(format!("| {} |", delim.join(" | ")));
    for r in t.rows.iter().skip(1) {
        out.push(format!("| {} |", row_md(r).join(" | ")));
    }
}

/// Markdown text of a document.
pub fn export(doc: &Document) -> String {
    let flow = model::from_doc(doc);
    let mut out: Vec<String> = Vec::new();
    write_blocks(&flow.blocks, &mut out);
    let mut s = out.join("\n");
    while s.ends_with("\n\n") {
        s.pop();
    }
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn write_blocks(blocks: &[FBlock], out: &mut Vec<String>) {
    let mut i = 0;
    let mut counters = [0u32; 9];
    let mut prev_list = false;
    let mut prev_top_ordered = false;
    while let Some(b) = blocks.get(i) {
        match b {
            FBlock::Table(t) => {
                table_md(t, out);
                out.push(String::new());
                prev_list = false;
            }
            FBlock::Para(p) if p.kind == Kind::Code && p.list.is_none() => {
                let mut lines = Vec::new();
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if q.kind != Kind::Code || q.list.is_some() {
                        break;
                    }
                    lines.push(q.text());
                    i += 1;
                }
                let longest = lines.iter().map(|l| l.chars().filter(|c| *c == '`').count()).max().unwrap_or(0);
                // The fence must be longer than the longest run in the content, or a content
                // line is read back as the closing fence (the `.min(64)` cap did exactly that).
                let fence = "`".repeat(3.max(longest + 1));
                out.push(fence.clone());
                out.extend(lines);
                out.push(fence);
                out.push(String::new());
                prev_list = false;
                continue;
            }
            FBlock::Para(p) => {
                let body = inlines_md(&p.inlines);
                match p.list {
                    Some(li) => {
                        let lv = li.level.min(8) as usize;
                        if !prev_list || (lv == 0 && prev_top_ordered != li.ordered) {
                            counters = [0; 9];
                        }
                        if lv == 0 {
                            prev_top_ordered = li.ordered;
                        }
                        let n = counters.get_mut(lv).map(|c| {
                            if *c == 0 {
                                *c = li.start.saturating_sub(1);
                            }
                            *c = c.saturating_add(1);
                            *c
                        });
                        for d in counters.iter_mut().skip(lv + 1) {
                            *d = 0;
                        }
                        let marker = if li.ordered { format!("{}.", n.unwrap_or(1)) } else { "-".to_string() };
                        let pad = " ".repeat(4 * lv);
                        let cont = " ".repeat(4 * lv + marker.len() + 1);
                        let body = if p.kind == Kind::Code {
                            code_span(&p.text())
                        } else {
                            let body = escape_lines(body).replace("\\\n", &format!("\\\n{cont}"));
                            // The bullet marker can turn a short run of hyphens into a rule.
                            if thematic_break(&format!("{marker} {body}")) { format!("\\{body}") } else { body }
                        };
                        out.push(format!("{pad}{marker} {body}").trim_end().to_string());
                        prev_list = true;
                        let next_is_list = matches!(blocks.get(i + 1), Some(FBlock::Para(q)) if q.list.is_some());
                        if !next_is_list {
                            out.push(String::new());
                        }
                    }
                    None => {
                        prev_list = false;
                        let line = match p.kind {
                            Kind::Heading(n) => format!("{} {}", "#".repeat(n.clamp(1, 6) as usize), body.replace("\\\n", " ")),
                            Kind::Title => format!("# {}", body.replace("\\\n", " ")),
                            Kind::Rule => "---".to_string(),
                            Kind::Quote => {
                                let b = escape_lines(body);
                                format!("> {}", b.replace("\\\n", "\\\n> "))
                            }
                            _ => {
                                if body.is_empty() {
                                    i += 1;
                                    continue;
                                }
                                escape_lines(body)
                            }
                        };
                        out.push(line);
                        out.push(String::new());
                    }
                }
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inl(s: &str) -> Vec<Inline> {
        parse_inlines(s, &Fmt::default(), 0, &Refs::new())
    }

    fn spans(s: &str) -> Vec<(String, bool, bool)> {
        inl(s).into_iter().filter_map(|i| if let Inline::Text(t, f) = i { Some((t, f.bold, f.italic)) } else { None }).collect()
    }

    #[test]
    fn emphasis() {
        assert_eq!(spans("a *b* c"), vec![("a ".into(), false, false), ("b".into(), false, true), (" c".into(), false, false)]);
        assert_eq!(spans("**b**"), vec![("b".into(), true, false)]);
        assert_eq!(spans("***x***"), vec![("x".into(), true, true)]);
        assert_eq!(spans("*a **b** c*"), vec![("a ".into(), false, true), ("b".into(), true, true), (" c".into(), false, true)]);
        assert_eq!(spans("snake_case_name"), vec![("snake_case_name".into(), false, false)]);
        assert_eq!(spans("2 * 3 * 4"), vec![("2 * 3 * 4".into(), false, false)]);
        assert_eq!(spans("**unclosed"), vec![("**unclosed".into(), false, false)]);
        let s = inl("~~gone~~");
        assert!(matches!(&s[0], Inline::Text(t, f) if t == "gone" && f.strike));
    }

    #[test]
    fn links_code_escapes() {
        let s = inl("see [the *site*](http://x.org \"T\") and `a*b*` \\*lit\\* &amp; <https://y.com>");
        let link: Vec<_> = s
            .iter()
            .filter_map(|i| if let Inline::Text(t, f) = i { f.link.as_ref().map(|l| (t.clone(), l.clone(), f.italic)) } else { None })
            .collect();
        assert_eq!(link[0], ("the ".into(), "http://x.org".into(), false));
        assert_eq!(link[1], ("site".into(), "http://x.org".into(), true));
        assert!(link.iter().any(|l| l.1 == "https://y.com"));
        assert!(s.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "a*b*" && f.code)));
        let text: String = s.iter().filter_map(|i| if let Inline::Text(t, _) = i { Some(t.as_str()) } else { None }).collect();
        assert!(text.contains("*lit* & "), "{text}");
    }

    /// Reference links: `[text][label]`, `[text][]` and `[text]` take their destination from a
    /// definition line, and the definition itself leaves the document. Before, none of the three
    /// was a link and the definition stayed as a paragraph of its own (#3238).
    #[test]
    fn reference_links_resolve_and_their_definitions_leave_the_body() {
        let links = |md: &str| -> Vec<(String, String)> {
            match &*parse(md).blocks {
                [FBlock::Para(p), ..] => p
                    .inlines
                    .iter()
                    .filter_map(|x| match x {
                        Inline::Text(t, f) => f.link.clone().map(|l| (t.clone(), l)),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            }
        };
        let def = "\n\n[ref1]: https://example.com/ref\n";
        let to_ref = |text: &str| vec![(text.to_string(), "https://example.com/ref".to_string())];

        assert_eq!(links(&format!("[click here][ref1]{def}")), to_ref("click here"), "full");
        assert_eq!(links(&format!("[ref1][]{def}")), to_ref("ref1"), "collapsed");
        assert_eq!(links(&format!("[ref1]{def}")), to_ref("ref1"), "shortcut");
        // Labels match case-folded with runs of white space collapsed (CommonMark §4.7).
        assert_eq!(links(&format!("[click here][  REF1  ]{def}")), to_ref("click here"), "folded label");
        // An undefined label is not a link: an ordinary bracketed phrase stays text.
        assert_eq!(links(&format!("[nope]{def}")), vec![], "undefined");
        // Inline links are unaffected.
        assert_eq!(links("[x](https://inline.example)"), vec![("x".into(), "https://inline.example".into())], "inline");

        // The definition is not a paragraph, and its text is nowhere in the body.
        let doc = import(&format!("Reference link: [click here][ref1]{def}"));
        let text = doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert_eq!(text, "Reference link: click here", "{text:?}");

        // A definition inside a fence is code, not a definition.
        let fenced = "```\n[ref1]: https://example.com/ref\n```\n\n[ref1]\n";
        let body = import(fenced).plain_text(wordcraft_doc::StoryRef::Body);
        assert!(body.contains("[ref1]: https://example.com/ref"), "the fenced definition is still code: {body:?}");
        assert_eq!(links(fenced), vec![], "a fenced definition defines nothing");
    }

    /// A definition has to start a block (CommonMark §4.7), and a fence opens and closes by the
    /// same rules the block parser uses. Reading a line without that context deleted text: a
    /// definition-shaped line inside a paragraph, and every such line inside a code block whose
    /// fence this pass thought had closed.
    #[test]
    fn a_definition_shaped_line_that_is_not_a_definition_stays_in_the_document() {
        let body = |md: &str| import(md).plain_text(wordcraft_doc::StoryRef::Body);

        // A definition cannot interrupt a paragraph: this is the author's text, lines and all.
        let para = "Status update\n[Warning]: unstable\nmore text\n";
        assert_eq!(body(para), "Status update [Warning]: unstable more text", "{:?}", body(para));

        // `` ```rust `` holds an info string, so it does not close a fence: the definition-shaped
        // line after it is still code.
        let code = "```\n```rust\n[foo]: /bar\n```\n\nSee [foo].\n";
        assert!(body(code).contains("[foo]: /bar"), "{:?}", body(code));
        assert!(body(code).ends_with("See [foo]."), "nothing defines foo: {:?}", body(code));

        // Four spaces of indentation is code, not a fence, so the definition that follows is
        // taken and the reference resolves.
        let indented = "    ```\n\n[x]: /y\n\nSee [x].\n";
        let doc = import(indented);
        let text = doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(text.contains("```") && text.ends_with("See x."), "{text:?}");

        // A definition still gives up its line after a blank line, at the start of the document,
        // after a heading, and after another definition.
        let starts = "[a]: /a\n[b]: /b\n\n# Head\n[c]: /c\n\nSee [a], [b], [c].\n";
        assert_eq!(body(starts), "Head\nSee a, b, c.", "{:?}", body(starts));

        // A footnote definition is not a link definition: it stays in the body.
        let footnote = "x[^1]\n\n[^1]: Note\n";
        assert!(body(footnote).contains("[^1]: Note"), "{:?}", body(footnote));
    }

    #[test]
    fn blocks() {
        let f = parse(
            "# Title\n\nPara one\ncontinues.\n\n> quoted\n\n- a\n- b\n    1. nested\n\n```\ncode *x*\n```\n\n---\n\n| A | B |\n|---|:-:|\n| 1 | 2 |\n",
        );
        let kinds: Vec<String> = f
            .blocks
            .iter()
            .map(|b| match b {
                FBlock::Para(p) => format!("{:?}{:?}:{}", p.kind, p.list.map(|l| (l.ordered, l.level)), p.text()),
                FBlock::Table(t) => format!("table{}", t.rows.len()),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "Heading(1)None:Title",
                "NormalNone:Para one continues.",
                "QuoteNone:quoted",
                "NormalSome((false, 0)):a",
                "NormalSome((false, 0)):b",
                "NormalSome((true, 1)):nested",
                "CodeNone:code *x*",
                "RuleNone:",
                "table2",
            ]
        );
    }

    #[test]
    fn export_escapes_round_trip() {
        let mut p = Para::default();
        p.push_text("1. not a list *really*", &Fmt::default());
        let md = escape_line_start(inlines_md(&p.inlines));
        let back = parse(&md);
        let FBlock::Para(q) = &back.blocks[0] else { panic!() };
        assert_eq!(q.text(), "1. not a list *really*");
        assert!(q.list.is_none());
    }

    #[test]
    fn escape_line_start_preserves_indentation() {
        assert_eq!(escape_line_start("   indented text".to_string()), "   indented text");
        assert_eq!(escape_line_start("   # not a heading".to_string()), "   \\# not a heading");
        assert_eq!(escape_line_start("  1. not a list".to_string()), "  1\\. not a list");
        assert_eq!(escape_line_start("  > quote".to_string()), "  \\> quote");
        // Four spaces or a tab would make an indented code block: the indent is capped at three.
        assert_eq!(escape_line_start("      deep".to_string()), "   deep");
        assert_eq!(escape_line_start("\tTabbed # text".to_string()), "   Tabbed # text");
        assert_eq!(escape_line_start("    # not a heading".to_string()), "   \\# not a heading");
        assert_eq!(escape_line_start(" \t1. not a list".to_string()), "   1\\. not a list");
    }

    #[test]
    fn leading_unicode_whitespace_is_kept_and_does_not_start_blocks() {
        let kinds_text = |md: &str| -> Vec<(Kind, bool, String)> {
            parse(md).blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some((p.kind, p.list.is_some(), p.text())) } else { None }).collect()
        };
        assert_eq!(kinds_text("\u{3000}\u{3000}日本語の段落\n"), vec![(Kind::Normal, false, "\u{3000}\u{3000}日本語の段落".to_string())]);
        for src in ["\u{a0}# not a heading", "\u{3000}1. not a list", "\u{a0}- x", "\u{a0}> not a quote"] {
            let mut p = Para::default();
            p.push_text(src, &Fmt::default());
            let md = escape_line_start(inlines_md(&p.inlines));
            assert_eq!(kinds_text(&md), vec![(Kind::Normal, false, src.to_string())], "{src:?} via {md:?}");
        }
    }

    #[test]
    fn single_backtick_code_span_escaped_pipe() {
        let md = "| Command | Status |\n| --- | --- |\n| `cmd \\| grep` | pass |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1].len(), 2);
        let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
        assert_eq!(p0.inlines.len(), 1);
        assert!(matches!(&p0.inlines[0], Inline::Text(t, f) if t == "cmd | grep" && f.code));
        let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
        assert_eq!(p1.text(), "pass");

        let mut out = Vec::new();
        table_md(t, &mut out);
        let exported = out.join("\n");
        assert!(exported.contains("`cmd \\| grep`"), "Exported: {exported}");
    }

    #[test]
    fn double_backtick_code_span_escaped_pipe() {
        let md = "| Syntax | Note |\n| --- | --- |\n| ``a \\| b`` | second |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1].len(), 2);
        let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
        assert_eq!(p0.inlines.len(), 1);
        assert!(matches!(&p0.inlines[0], Inline::Text(t, f) if t == "a | b" && f.code));
        let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
        assert_eq!(p1.text(), "second");
    }

    #[test]
    fn code_span_literal_backtick_and_escaped_pipe() {
        let md = "| Expression | Detail |\n| --- | --- |\n| ``a`b\\|c`` | distinct |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1].len(), 2);
        let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
        assert_eq!(p0.inlines.len(), 1);
        assert!(matches!(&p0.inlines[0], Inline::Text(t, f) if t == "a`b|c" && f.code));
        let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
        assert_eq!(p1.text(), "distinct");
    }

    #[test]
    fn three_column_table_with_distinctive_text() {
        let md = "| Col1 | Col2 | Col3 |\n| --- | --- | --- |\n| `p1\\|q1` | Distinctive Two | Distinctive Three |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1].len(), 3);
        assert_eq!(t.rows[1][0].text(), "p1|q1");
        assert_eq!(t.rows[1][1].text(), "Distinctive Two");
        assert_eq!(t.rows[1][2].text(), "Distinctive Three");
        let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
        assert!(matches!(&p0.inlines[0], Inline::Text(_, f) if f.code));
        let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
        assert!(matches!(&p1.inlines[0], Inline::Text(_, f) if !f.code));
        let FBlock::Para(p2) = &t.rows[1][2].blocks[0] else { panic!() };
        assert!(matches!(&p2.inlines[0], Inline::Text(_, f) if !f.code));
    }

    #[test]
    fn regular_table_cells_containing_escaped_pipes() {
        let md = "| Option | Description |\n| --- | --- |\n| Regular \\| Pipe | Normal text cell |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1].len(), 2);
        assert_eq!(t.rows[1][0].text(), "Regular | Pipe");
        assert_eq!(t.rows[1][1].text(), "Normal text cell");
        let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
        assert!(matches!(&p0.inlines[0], Inline::Text(_, f) if !f.code));

        let mut out = Vec::new();
        table_md(t, &mut out);
        let exported = out.join("\n");
        assert!(exported.contains("Regular \\| Pipe"), "Exported: {exported}");
        assert!(!exported.contains("Regular \\\\| Pipe"), "Exported: {exported}");
    }

    #[test]
    fn unescaped_pipes_behave_as_gfm_column_separators() {
        let md = "| Header 1 | Header 2 |\n| --- | --- |\n| `raw|pipe` | Extra Column |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1].len(), 2);
        assert_eq!(t.rows[1][0].text(), "`raw");
        assert_eq!(t.rows[1][1].text(), "pipe`");
    }

    #[test]
    fn ordinary_tables_and_standalone_code_spans_unchanged() {
        let md = "| Alpha | Beta |\n| --- | --- |\n| 1 | 2 |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!() };
        let mut out = Vec::new();
        table_md(t, &mut out);
        assert_eq!(out.join("\n"), "| Alpha | Beta |\n| --- | --- |\n| 1 | 2 |");

        let mut p = Para::default();
        let f = Fmt { code: true, ..Default::default() };
        p.push_text("cmd | grep", &f);
        let exported = inlines_md(&p.inlines);
        assert_eq!(exported, "`cmd | grep`");
    }

    #[test]
    fn code_span_line_endings_stay_literal_on_export() {
        let cases = [
            ("a\n# b", "a # b"),
            ("a\n<div>b</div>", "a <div>b</div>"),
            ("a\n- b", "a - b"),
            ("a\n```", "a ```"),
            ("a\r# b", "a # b"),
            ("a\r\n# b", "a # b"),
            ("a\n\n> b", "a  > b"),
            ("\na\n", " a "),
            ("`a\n# b`", "`a # b`"),
            ("cmd | grep", "cmd | grep"),
        ];
        let shapes = [
            (Kind::Normal, None),
            (Kind::Normal, Some(ListInfo { ordered: false, level: 0, start: 1 })),
            (Kind::Code, Some(ListInfo { ordered: true, level: 0, start: 1 })),
        ];
        for (source, expected) in cases {
            for (kind, list) in shapes {
                let mut p = Para::new(kind);
                p.list = list;
                p.push_text(source, &Fmt { code: true, ..Default::default() });
                let mut doc = model::to_doc(&model::Flow { blocks: vec![FBlock::Para(p)], ..Default::default() });
                // The flow builder removes CR controls; keep the native document's raw input.
                let p = doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
                p.text = source.to_string();
                p.runs[0].len = source.len();
                let bytes = crate::export("md", &doc).unwrap().unwrap();
                let md = String::from_utf8(bytes).unwrap();
                assert_eq!(md.trim_end().lines().count(), 1, "{source:?} {kind:?} {list:?} -> {md:?}");
                assert!(!md.contains('\r'), "{md:?}");
                let back = crate::import("md", md.as_bytes()).unwrap().unwrap();
                let flow = model::from_doc(&back);
                let [FBlock::Para(q)] = flow.blocks.as_slice() else { panic!("{md:?}") };
                assert_eq!(q.text(), expected, "{source:?} {kind:?} {list:?} -> {md:?}");
                assert_eq!(q.list, list, "{md:?}");
                assert!(q.inlines.iter().all(|i| matches!(i, Inline::Text(_, f) if f.code)), "{md:?}");
            }
        }
        let fenced = "```\na\n# b\n```";
        assert_eq!(export(&import(fenced)).trim_end(), fenced);
    }

    #[test]
    fn code_span_line_endings_in_table_cells_stay_literal() {
        for ending in ["\n", "\r", "\r\n"] {
            let source = format!("a{ending}# b|c");
            let mut p = Para::default();
            p.push_text(&source, &Fmt { code: true, ..Default::default() });
            let cell = Cell { blocks: vec![FBlock::Para(p)], ..Default::default() };
            let table = FTable { rows: vec![vec![cell, Cell::default()], vec![Cell::default(), Cell::default()]], ..Default::default() };
            let mut doc = model::to_doc(&model::Flow { blocks: vec![FBlock::Table(table)], ..Default::default() });
            let p = doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path(vec![0, 0, 0, 0])).unwrap();
            p.text = source.clone();
            p.runs[0].len = source.len();
            let md = export(&doc);
            let back = model::from_doc(&import(&md));
            let [FBlock::Table(table), ..] = back.blocks.as_slice() else { panic!("{md:?}") };
            assert!(back.blocks.iter().skip(1).all(|b| matches!(b, FBlock::Para(p) if p.text().is_empty())), "{md:?}");
            assert_eq!(table.rows.len(), 2, "{md:?}");
            assert_eq!(table.rows[0].len(), 2, "{md:?}");
            let [FBlock::Para(p)] = table.rows[0][0].blocks.as_slice() else { panic!("{md:?}") };
            assert_eq!(p.text(), "a # b|c", "{md:?}");
            assert!(p.inlines.iter().all(|i| matches!(i, Inline::Text(_, f) if f.code)), "{md:?}");
        }
    }

    #[test]
    fn table_code_pipe_export_import_round_trip() {
        let mut table = FTable::default();
        let mut cell_a = Cell::default();
        let mut para_a = Para::default();
        let fmt_a = Fmt { code: true, ..Default::default() };
        para_a.push_text("a`b|c", &fmt_a);
        cell_a.blocks.push(FBlock::Para(para_a));

        let mut cell_b = Cell::default();
        let mut para_b = Para::default();
        para_b.push_text("CRITICAL_DISTINCTIVE_INFO", &Fmt::default());
        cell_b.blocks.push(FBlock::Para(para_b));

        let mut head_a = Cell::default();
        let mut head_para_a = Para::default();
        head_para_a.push_text("Col A", &Fmt::default());
        head_a.blocks.push(FBlock::Para(head_para_a));

        let mut head_b = Cell::default();
        let mut head_para_b = Para::default();
        head_para_b.push_text("Col B", &Fmt::default());
        head_b.blocks.push(FBlock::Para(head_para_b));

        table.rows.push(vec![head_a, head_b]);
        table.rows.push(vec![cell_a, cell_b]);

        let mut out = Vec::new();
        table_md(&table, &mut out);
        let exported_md = out.join("\n");

        assert!(exported_md.contains("``a`b\\|c``"), "Exported: {exported_md}");
        assert!(exported_md.contains("CRITICAL\\_DISTINCTIVE\\_INFO"), "Exported: {exported_md}");

        let back_flow = parse(&exported_md);
        let FBlock::Table(back_table) = &back_flow.blocks[0] else { panic!("expected table") };

        assert_eq!(back_table.rows.len(), 2, "Both rows must exist");
        let row1 = &back_table.rows[1];
        assert_eq!(row1.len(), 2, "Both cells must exist in row 1");

        let back_cell_a = &row1[0];
        let back_cell_b = &row1[1];
        assert_eq!(back_cell_a.text(), "a`b|c", "Cell A text must be preserved");
        assert_eq!(back_cell_b.text(), "CRITICAL_DISTINCTIVE_INFO", "Cell B text must be preserved");

        let FBlock::Para(back_p_a) = &back_cell_a.blocks[0] else { panic!() };
        assert_eq!(back_p_a.inlines.len(), 1);
        assert!(
            matches!(&back_p_a.inlines[0], Inline::Text(t, f) if t == "a`b|c" && f.code),
            "Code formatting must remain intact on Cell A: {:?}",
            back_p_a.inlines
        );

        let FBlock::Para(back_p_b) = &back_cell_b.blocks[0] else { panic!() };
        assert_eq!(back_p_b.inlines.len(), 1);
        assert!(
            matches!(&back_p_b.inlines[0], Inline::Text(t, f) if t == "CRITICAL_DISTINCTIVE_INFO" && !f.code),
            "Cell B formatting must remain plain text: {:?}",
            back_p_b.inlines
        );

        assert_eq!(back_cell_a.text(), "a`b|c");
        assert_eq!(back_cell_b.text(), "CRITICAL_DISTINCTIVE_INFO");

        assert!(!back_cell_a.text().contains("CRITICAL_DISTINCTIVE_INFO"));
        assert!(!back_cell_b.text().contains("a`b|c"));

        let doc = import(&exported_md);
        let re_exported = export(&doc);
        assert!(re_exported.contains("``a`b\\|c``"), "Re-exported: {re_exported}");
        assert!(re_exported.contains("CRITICAL\\_DISTINCTIVE\\_INFO"), "Re-exported: {re_exported}");
        let doc2 = import(&re_exported);
        let back_flow2 = model::from_doc(&doc2);
        let FBlock::Table(back_table2) = &back_flow2.blocks[0] else { panic!() };
        assert_eq!(back_table2.rows[1][0].text(), "a`b|c");
        assert_eq!(back_table2.rows[1][1].text(), "CRITICAL_DISTINCTIVE_INFO");
    }

    #[test]
    fn ordered_list_keeps_start_number() {
        let txt = |md: &str| String::from_utf8_lossy(&crate::txt::export(&import(md))).into_owned();
        assert_eq!(txt("5. a\n6. b"), "5.\ta\r\n6.\tb");
        assert_eq!(txt("3. a\n- b\n4. c"), "3.\ta\r\n•\tb\r\n4.\tc");
        assert_eq!(export(&import("5. a\n6. b")).trim_end(), "5. a\n6. b");
    }

    #[test]
    fn lines_after_line_break_stay_in_paragraph() {
        use wordcraft_doc::{CharProps, Document, Paragraph, StoryRef, para_block};
        let cases = ["Shopping\n- milk\n# not a heading", "a\n1. x", "a\n2) x", "a\n+ x", "a\n---", "a\n***", "a\n==", "a\n--", "a\n> q"];
        for s in cases {
            let d = Document { body: vec![para_block(Paragraph::with_text(s, CharProps::default()))], ..Default::default() };
            let md = export(&d);
            let back = import(&md);
            assert_eq!(back.body.len(), 1, "{s:?} -> {md:?}");
            assert_eq!(back.plain_text(StoryRef::Body), s, "{md:?}");
        }
    }

    #[test]
    fn lines_after_line_break_stay_in_list_item_and_quote() {
        let cases = ["a\n- x", "a\n# h", "a\n1. x", "a\n--", "a\n---", "a\n==", "a\n> q"];
        let shapes: [(Kind, Option<ListInfo>); 5] = [
            (Kind::Normal, Some(ListInfo { ordered: false, level: 0, start: 1 })),
            (Kind::Normal, Some(ListInfo { ordered: true, level: 0, start: 1 })),
            (Kind::Normal, Some(ListInfo { ordered: false, level: 1, start: 1 })),
            (Kind::Normal, Some(ListInfo { ordered: true, level: 1, start: 1 })),
            (Kind::Quote, None),
        ];
        for s in cases {
            for (kind, list) in shapes {
                let mut blocks = Vec::new();
                if let Some(li) = list.filter(|l| l.level > 0) {
                    let mut parent = Para::new(Kind::Normal);
                    parent.list = Some(ListInfo { ordered: li.ordered, level: 0, start: 1 });
                    parent.push_text("top", &Fmt::default());
                    blocks.push(FBlock::Para(parent));
                }
                let mut p = Para::new(kind);
                p.list = list;
                p.push_text(s, &Fmt::default());
                blocks.push(FBlock::Para(p));
                let mut out = Vec::new();
                write_blocks(&blocks, &mut out);
                let md = out.join("\n");
                let back = parse(&md);
                assert_eq!(back.blocks.len(), blocks.len(), "{s:?} {kind:?} {list:?} -> {md:?}");
                let Some(FBlock::Para(q)) = back.blocks.last() else { panic!("{md:?}") };
                assert_eq!(q.kind, kind, "{md:?}");
                assert_eq!(q.list, list, "{md:?}");
                assert_eq!(q.text(), s, "{md:?}");
            }
        }
    }

    #[test]
    fn table_column_alignments_import_and_export() {
        let md = "| Left | Center | Right | Default |\n| :--- | :---: | ---: | --- |\n| 1 | 2 | 3 | 4 |\n";
        let flow = parse(md);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
        assert_eq!(t.rows.len(), 2);
        let aligns: Vec<Option<Align>> =
            t.rows[0].iter().map(|c| c.blocks.iter().find_map(|b| if let FBlock::Para(p) = b { Some(p.align) } else { None }).flatten()).collect();
        assert_eq!(aligns, vec![Some(Align::Left), Some(Align::Center), Some(Align::Right), None]);

        let mut out = Vec::new();
        table_md(t, &mut out);
        let exported = out.join("\n");
        assert!(exported.contains("| :--- | :---: | ---: | --- |"), "Exported: {exported}");
    }

    #[test]
    fn tab_separated_list_markers() {
        let f = parse("-\tbullet item\n*\tstar item\n+\tplus item\n1.\tone item\n2)\ttwo item\n");
        assert_eq!(f.blocks.len(), 5);
        for (i, b) in f.blocks.iter().enumerate() {
            let FBlock::Para(p) = b else { panic!("block {i} not a para") };
            assert!(p.list.is_some(), "block {i} should be a list item: {:?}", p);
        }
        let FBlock::Para(p0) = &f.blocks[0] else { panic!() };
        assert_eq!(p0.text(), "bullet item");
        assert!(!p0.list.unwrap().ordered);

        let FBlock::Para(p3) = &f.blocks[3] else { panic!() };
        assert_eq!(p3.text(), "one item");
        assert!(p3.list.unwrap().ordered);
    }

    #[test]
    fn html_comments_are_ignored_in_blocks_and_inlines() {
        let md = "<!-- block comment -->\n\nHello <!-- inline comment -->world!\n\n<!--\nmultiline\ncomment\n-->\n\nAfter comment.";
        let flow = parse(md);
        assert_eq!(flow.blocks.len(), 2);
        let texts: Vec<String> = flow.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.text()) } else { None }).collect();
        assert_eq!(texts, ["Hello world!", "After comment."]);
    }
    #[test]
    fn comment_definitions_cannot_shadow_visible_definitions() {
        for md in [
            "<!--\n\n[x]: /wrong\n-->\n\n[x]: /right\n\n[x]",
            "<!--\n[x]: /wrong\n-->\n[x]: /right\n\n[x]",
            "<!-- first --> <!-- second\n\n[x]: /wrong\n-->\n[x]: /right\n\n[x]",
        ] {
            let flow = parse(md);
            let links: Vec<_> = flow
                .blocks
                .iter()
                .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
                .flat_map(|p| &p.inlines)
                .filter_map(|i| if let Inline::Text(_, f) = i { f.link.as_deref() } else { None })
                .collect();
            assert_eq!(links, ["/right"], "{md}");
        }
        for code in ["```\n<!--\n```", "    <!--"] {
            let flow = parse(&format!("{code}\n\n[x]: /right\n\n[x]"));
            assert!(flow.blocks.iter().any(
                |b| matches!(b, FBlock::Para(p) if p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) if f.link.as_deref() == Some("/right"))))
            ));
        }
    }

    #[test]
    fn fences_after_comment_tails_keep_definitions_as_code() {
        for (comment, marker) in [("<!-- -->", "```"), ("<!--\n-->", "~~~"), ("<!-- first --><!-- second -->  ", "```")] {
            let flow = parse(&format!("{comment}{marker}\n\n[x]: /wrong\n{marker}\n[x]: /right\n\n[x]"));
            assert!(flow.blocks.iter().any(|b| matches!(b, FBlock::Para(p) if p.kind == Kind::Code && p.text() == "[x]: /wrong")));
            let links: Vec<_> = flow
                .blocks
                .iter()
                .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
                .flat_map(|p| &p.inlines)
                .filter_map(|i| if let Inline::Text(_, f) = i { f.link.as_deref() } else { None })
                .collect();
            assert_eq!(links, ["/right"]);
        }
    }

    #[test]
    fn closing_comments_keep_following_source_and_short_inline_forms() {
        for md in ["<!-- note -->Visible text", "<!--\nnote\n-->Visible text", "<!-- first --><!-- second -->Visible text"] {
            let flow = parse(md);
            assert!(matches!(&flow.blocks[..], [FBlock::Para(p)] if p.text() == "Visible text"), "{md}");
        }
        let flow = parse("<!-- note --># Heading\n\nBefore <!-->middle<!---> after");
        assert!(matches!(&flow.blocks[0], FBlock::Para(p) if p.kind == Kind::Heading(1) && p.text() == "Heading"));
        assert!(matches!(&flow.blocks[1], FBlock::Para(p) if p.text() == "Before middle after"));
    }

    #[test]
    fn long_comments_and_many_unclosed_inline_openers_preserve_text() {
        let unmatched = format!("prefix {}suffix", "<!--".repeat(100_000));
        let flow = parse(&unmatched);
        assert!(matches!(&flow.blocks[..], [FBlock::Para(p)] if p.text() == unmatched));
        let matched = format!("before <!--{}-->after", "hidden ".repeat(100_000));
        let flow = parse(&matched);
        assert!(matches!(&flow.blocks[..], [FBlock::Para(p)] if p.text() == "before after"));
    }
}
