use super::inlines::parse_inlines;
use super::{Align, Arc, Cell, Document, FBlock, FTable, Fmt, HashMap, Kind, ListInfo, Para, model};

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
fn take_definitions(lines: &mut Vec<String>) -> Refs {
    let mut refs = Refs::new();
    let mut open: Option<(char, usize)> = None;
    // The start of the document begins a block.
    let mut starts_block = true;
    lines.retain(|line| {
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
    let mut blocks = parse_blocks(&lines, 0, 0, &refs);
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
fn parse_blocks(lines: &[String], depth: usize, level: u8, refs: &Refs) -> Vec<FBlock> {
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
            parse_indented_code(lines, &mut i, &mut out);
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
                i += 1;
                if l.contains("-->") {
                    break;
                }
            }
            continue;
        }
        if t.starts_with('>') {
            flush_para(&mut para, &mut out, depth, refs);
            parse_quote(lines, &mut i, depth, level, refs, &mut out);
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
            i = parse_list(lines, i, depth, level, &mut out, refs);
            continue;
        }
        para.push(line.clone());
        i += 1;
    }
    flush_para(&mut para, &mut out, depth, refs);
    out
}

fn parse_indented_code(lines: &[String], i: &mut usize, out: &mut Vec<FBlock>) {
    let mut code: Vec<&str> = Vec::new();
    while let Some(line) = lines.get(*i) {
        if is_blank(line) {
            code.push("");
        } else if indent(line) >= 4 {
            code.push(strip_indent(line, 4));
        } else {
            break;
        }
        *i += 1;
    }
    while code.last().is_some_and(|line| line.is_empty()) {
        code.pop();
    }
    out.extend(code.into_iter().map(code_para));
}

fn parse_quote(lines: &[String], i: &mut usize, depth: usize, level: u8, refs: &Refs, out: &mut Vec<FBlock>) {
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
    if depth >= MAX_NEST {
        let mut p = Para::new(Kind::Quote);
        p.push_text(&inner.join(" "), &Fmt::default());
        out.push(FBlock::Para(p));
        return;
    }
    for mut block in parse_blocks(&inner, depth + 1, level, refs) {
        if let FBlock::Para(p) = &mut block
            && p.kind == Kind::Normal
        {
            p.kind = Kind::Quote;
        }
        out.push(block);
    }
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
            parse_blocks(&item, depth + 1, level.saturating_add(1).min(8), refs)
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
