//! Markdown: a CommonMark subset plus GitHub tables and strikethrough.
//!
//! Import: ATX and setext headings, paragraphs with soft/hard breaks, block quotes (→ Quote
//! style), fenced and indented code (→ one Code paragraph per line), thematic breaks (→ a
//! bottom-border paragraph), bullet and ordered lists (nested by indentation), GFM pipe tables,
//! and inlines: emphasis/strong (the CommonMark delimiter algorithm), `~~strike~~`, code spans,
//! inline links, autolinks, images (`data:` URIs become pictures, others their alt text),
//! backslash escapes, entities, `<br>` and `<a id>` anchors.
//!
//! Export writes the same constructs; pictures become `![alt](media-name)` placeholders.

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

/// Parse Markdown text into the flow model.
pub fn parse(text: &str) -> model::Flow {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let lines: Vec<String> = text.split('\n').map(|l| expand_tabs(l.trim_end_matches('\r'))).collect();
    let mut blocks = parse_blocks(&lines, 0, 0);
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
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
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
    if rest.is_empty() || rest.starts_with(' ') { Some(m) } else { None }
}

fn split_row(l: &str) -> Vec<String> {
    let t = l.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") { &t[..t.len() - 1] } else { t };
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    let mut in_code = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '`' => {
                in_code = !in_code;
                cur.push(c);
            }
            '|' if !in_code => cells.push(std::mem::take(&mut cur).trim().to_string()),
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
    let t = l.trim_start();
    atx_heading(t).is_some()
        || thematic_break(t)
        || fence(t).is_some()
        || t.starts_with('>')
        || list_marker(t).is_some_and(|m| (!m.ordered || m.start == 1) && !is_blank(t.get(m.len..).unwrap_or("")))
}

fn flush_para(lines: &mut Vec<String>, out: &mut Vec<FBlock>, depth: usize) {
    if lines.is_empty() {
        return;
    }
    let text = join_lines(lines);
    lines.clear();
    let p = Para { inlines: parse_inlines(&text, &Fmt::default(), depth), ..Default::default() };
    out.push(FBlock::Para(p));
}

/// Paragraph lines → text with `\n` for hard breaks and spaces for soft ones.
fn join_lines(lines: &[String]) -> String {
    let mut s = String::new();
    let n = lines.len();
    for (i, l) in lines.iter().enumerate() {
        let l = l.trim_start();
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
fn parse_blocks(lines: &[String], depth: usize, level: u8) -> Vec<FBlock> {
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        if is_blank(line) {
            flush_para(&mut para, &mut out, depth);
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
        let t = line.trim_start();
        // Setext heading underline.
        if !para.is_empty() {
            let u = t.trim_end();
            if !u.is_empty() && (u.chars().all(|c| c == '=') || (u.chars().all(|c| c == '-'))) {
                let lvl = if u.starts_with('=') { 1 } else { 2 };
                let text = join_lines(&para);
                para.clear();
                let mut p = Para::new(Kind::Heading(lvl));
                p.inlines = parse_inlines(&text, &Fmt::default(), depth);
                out.push(FBlock::Para(p));
                i += 1;
                continue;
            }
        }
        if let Some((n, body)) = atx_heading(t) {
            flush_para(&mut para, &mut out, depth);
            let mut p = Para::new(Kind::Heading(n));
            p.inlines = parse_inlines(body, &Fmt::default(), depth);
            out.push(FBlock::Para(p));
            i += 1;
            continue;
        }
        if thematic_break(t) {
            flush_para(&mut para, &mut out, depth);
            out.push(FBlock::Para(Para::new(Kind::Rule)));
            i += 1;
            continue;
        }
        if let Some((fc, fl)) = fence(t) {
            flush_para(&mut para, &mut out, depth);
            i += 1;
            while let Some(l) = lines.get(i) {
                i += 1;
                let lt = l.trim_start();
                if indent(l) < 4 && fence(lt).is_some_and(|(c, n)| c == fc && n >= fl) && lt.trim_start_matches(fc).trim().is_empty() {
                    break;
                }
                out.push(code_para(strip_indent(l, ind)));
            }
            continue;
        }
        if t.starts_with('>') {
            flush_para(&mut para, &mut out, depth);
            let mut inner: Vec<String> = Vec::new();
            while let Some(l) = lines.get(i) {
                let lt = l.trim_start();
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
                for mut b in parse_blocks(&inner, depth + 1, level) {
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
                    let p = Para { align: a, inlines: parse_inlines(s, &Fmt::default(), depth), ..Default::default() };
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
            flush_para(&mut para, &mut out, depth);
            i = parse_list(lines, i, depth, level, &mut out);
            continue;
        }
        para.push(line.clone());
        i += 1;
    }
    flush_para(&mut para, &mut out, depth);
    out
}

/// Parse a list starting at `lines[i]`; returns the index after it.
fn parse_list(lines: &[String], mut i: usize, depth: usize, level: u8, out: &mut Vec<FBlock>) -> usize {
    let Some(first) = lines.get(i).and_then(|l| list_marker(l.trim_start())) else { return i + 1 };
    while let Some(line) = lines.get(i) {
        let ind = indent(line);
        if ind >= 4 {
            break;
        }
        let t = line.trim_start();
        let Some(m) = list_marker(t) else { break };
        if m.ordered != first.ordered || m.ch != first.ch {
            break;
        }
        let after = t.get(m.len..).unwrap_or("");
        let spaces = indent(after);
        let content_indent = if is_blank(after) || spaces > 4 { ind + m.len + 1 } else { ind + m.len + spaces };
        let mut item: Vec<String> =
            vec![if is_blank(after) { String::new() } else { after.get(spaces.min(4).min(after.len())..).unwrap_or("").to_string() }];
        if spaces > 4 {
            item[0] = after.get(1..).unwrap_or("").to_string();
        }
        i += 1;
        while let Some(l) = lines.get(i) {
            if is_blank(l) {
                item.push(String::new());
            } else if indent(l) >= content_indent {
                item.push(strip_indent(l, content_indent).to_string());
            } else if item.last().is_some_and(|x| !is_blank(x)) && !interrupts(l) && list_marker(l.trim_start()).is_none() {
                // Lazy continuation of the item's paragraph.
                item.push(l.trim_start().to_string());
            } else {
                break;
            }
            i += 1;
        }
        // Blank lines at the end of an item belong between items.
        while item.len() > 1 && item.last().is_some_and(|x| x.is_empty()) {
            item.pop();
        }
        let li = ListInfo { ordered: first.ordered, level: level.min(8) };
        let inner = if depth >= MAX_NEST {
            let mut p = Para::default();
            p.push_text(&item.join(" "), &Fmt::default());
            vec![FBlock::Para(p)]
        } else {
            parse_blocks(&item, depth + 1, level.saturating_add(1).min(8))
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
            let next_is_item = lines
                .get(i + 1)
                .is_some_and(|l| indent(l) < 4 && list_marker(l.trim_start()).is_some_and(|m| m.ordered == first.ordered && m.ch == first.ch));
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
pub fn parse_inlines(s: &str, base: &Fmt, depth: usize) -> Vec<Inline> {
    let nodes = tokenize(s, base, depth);
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

/// The closing `]` matching the `[` at byte `open` (escapes and code spans respected).
fn matching_bracket(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while let Some(&c) = b.get(i) {
        match c {
            b'\\' => i += 1,
            b'`' => {
                let n = b.get(i..).map(|r| r.iter().take_while(|x| **x == b'`').count()).unwrap_or(1);
                if let Some(end) = find_backticks(s, i + n, n) {
                    i = end + n - 1;
                } else {
                    i += n - 1;
                }
            }
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Byte index of a run of exactly `n` backticks at or after `from`.
fn find_backticks(s: &str, from: usize, n: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = from;
    while let Some(&c) = b.get(i) {
        if c == b'`' {
            let k = b.get(i..).map(|r| r.iter().take_while(|x| **x == b'`').count()).unwrap_or(1);
            if k == n {
                return Some(i);
            }
            i += k;
        } else {
            i += 1;
        }
    }
    None
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
            && let Some(n) = chars.peek().copied().filter(|n| n.is_ascii_punctuation())
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
        if before.is_some_and(|c| c.is_whitespace()) && after.starts_with('=') {
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

fn tokenize(s: &str, base: &Fmt, depth: usize) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
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
                match find_backticks(s, i + n, n) {
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
                    matching_bracket(s, open).and_then(|close| {
                        let (dest, end) = link_dest(s, close + 1)?;
                        Some((s.get(open + 1..close)?.to_string(), dest, end))
                    })
                } else {
                    None
                };
                match parsed {
                    Some((inner, dest, end)) if img => {
                        flush!();
                        let alt: String = parse_inlines(&inner, &Fmt::default(), depth + 1)
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
                        for x in parse_inlines(&inner, &f, depth + 1) {
                            nodes.push(Node::Inl(x));
                        }
                        i = end;
                        text_start = i;
                    }
                    _ => i += 1,
                }
            }
            b'<' => {
                let rest = s.get(i + 1..).unwrap_or("");
                let end = rest.find('>');
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
                match rest.find(';').filter(|k| *k <= 32).and_then(|k| decode_entity(rest.get(1..k)?).map(|ch| (ch, k))) {
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
fn escape(s: &str) -> String {
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

/// Escape what would start a block at the beginning of a line.
fn escape_line_start(s: String) -> String {
    let s = s.trim_start().to_string();
    let t = s.as_str();
    let lead = s.len() - t.len();
    let needs = t.starts_with('#')
        || t.starts_with('>')
        || t.starts_with("- ")
        || t.starts_with("+ ")
        || t == "-"
        || t == "+"
        || t.starts_with('=')
        || thematic_break(t)
        || {
            let d = t.chars().take_while(|c| c.is_ascii_digit()).count();
            d > 0 && t.get(d..).is_some_and(|r| r.starts_with(". ") || r.starts_with(") ") || r == "." || r == ")")
        };
    if !needs {
        return s;
    }
    if let Some(d) = t.find(['.', ')']).filter(|_| t.starts_with(|c: char| c.is_ascii_digit())) {
        return format!("{}{}\\{}", &s[..lead], &t[..d], &t[d..]);
    }
    format!("{}\\{}", &s[..lead], t)
}

fn code_span(t: &str) -> String {
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
    // Split into (text, fmt) pieces with whitespace moved outside emphasis.
    let mut pieces: Vec<(String, Fmt, Option<String>)> = Vec::new();
    let inlines = crate::model::equations_as_text(inlines);
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
        // Links are the outermost construct.
        if f.link != link || raw.is_some() {
            close_all(&mut out, &mut stack);
            if let Some(l) = &link {
                out.push_str(&format!("]({})", link_target(l)));
            }
            link = None;
            if let Some(r) = raw {
                out.push_str(&r);
                continue;
            }
            if let Some(l) = &f.link {
                out.push('[');
                link = Some(l.clone());
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
            out.push_str(&code_span(&text));
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
                FBlock::Para(p) => parts.push(inlines_md(&p.inlines).replace("\\\n", "<br>")),
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
                let fence = "`".repeat(3.max(longest + 1).min(64));
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
                            *c += 1;
                            *c
                        });
                        for d in counters.iter_mut().skip(lv + 1) {
                            *d = 0;
                        }
                        let marker = if li.ordered { format!("{}.", n.unwrap_or(1)) } else { "-".to_string() };
                        let pad = " ".repeat(4 * lv);
                        let cont = " ".repeat(4 * lv + marker.len() + 1);
                        let body = body.replace("\\\n", &format!("\\\n{cont}"));
                        let body = if p.kind == Kind::Code { code_span(&p.text()) } else { body };
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
                                let b = escape_line_start(body);
                                format!("> {}", b.replace("\\\n", "\\\n> "))
                            }
                            _ => {
                                if body.is_empty() {
                                    i += 1;
                                    continue;
                                }
                                escape_line_start(body)
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
        parse_inlines(s, &Fmt::default(), 0)
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
}
