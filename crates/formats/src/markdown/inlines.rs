use super::blocks::{MAX_NEST, Refs, ref_label};
use super::{Arc, Fmt, HashMap, Inline, decode_entity, make_img, model};

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

struct LinkScan<'a> {
    text: &'a str,
    refs: &'a Refs,
    ticks: &'a Backticks,
    known: HashMap<usize, Option<usize>>,
    budget: usize,
}

impl LinkScan<'_> {
    fn parse_at(&mut self, open: usize) -> Option<(String, Arc<str>, usize)> {
        let bytes = self.text.as_bytes();
        let close = match self.known.get(&open).copied() {
            Some(found) => found,
            None if self.budget > 0 => {
                let found = matching_bracket(self.text, open, self.budget, self.ticks, &mut self.known);
                self.budget = self.budget.saturating_sub(found.unwrap_or(self.text.len()).saturating_sub(open));
                found
            }
            None => None,
        }?;
        if self.budget == 0 {
            return None;
        }
        let label_text = self.text.get(open + 1..close)?;
        if bytes.get(close + 1) == Some(&b'(') {
            let destination = link_dest(self.text, close + 1);
            self.budget = self.budget.saturating_sub(destination.as_ref().map_or(self.text.len(), |d| d.1).saturating_sub(close));
            let (destination, end) = destination?;
            return Some((label_text.to_string(), Arc::from(destination), end));
        }
        // Full and shortcut reference links use definitions collected before inline parsing.
        let (label, end) = match self.text.get(close + 1..).and_then(|rest| rest.strip_prefix('[')) {
            Some(rest) => {
                let shut = rest.find(']')?;
                let inner = rest.get(..shut)?;
                let label = if inner.trim().is_empty() { label_text } else { inner };
                (label, close + 2 + shut + 1)
            }
            None => (label_text, close + 1),
        };
        Some((label_text.to_string(), self.refs.get(&ref_label(label))?.clone(), end))
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
        // The first `>` not escaped by a backslash ends it; a line break first means no link.
        let b = inner.as_bytes();
        let mut k = 0;
        let end = loop {
            match b.get(k)? {
                b'\\' => k += 1,
                b'>' => break k,
                b'\n' => return None,
                _ => {}
            }
            k += 1;
        };
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
    let ticks = Backticks::new(s);
    let mut links = LinkScan { text: s, refs, ticks: &ticks, known: HashMap::new(), budget: s.len().saturating_mul(8).saturating_add(8192) };
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
                let (code, fmt, end) = code_span(s, i, base, &ticks);
                push_text(&mut nodes, &code, &fmt);
                i = end;
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
                let parsed = if b.get(open) == Some(&b'[') { links.parse_at(open) } else { None };
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
                        // A destination too long to keep leaves its text unlinked.
                        f.link = (dest.len() <= model::MAX_LINK).then_some(dest);
                        if f.link.is_none() {
                            model::dropped(model::Dropped::Link);
                        }
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
                if let Some((end, parsed)) = angle_inlines(s, i, base) {
                    flush!();
                    nodes.extend(parsed);
                    i = end;
                    text_start = end;
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

fn angle_inlines(s: &str, start: usize, base: &Fmt) -> Option<(usize, Vec<Node>)> {
    if let Some(pos) = s.get(start..).and_then(|rest| rest.strip_prefix("<!--")).and_then(|rest| rest.find("-->")) {
        return Some((start + 4 + pos + 3, Vec::new()));
    }
    let rest = s.get(start + 1..).unwrap_or("");
    let tag_end = within(rest, MAX_TAG).find('>')?;
    let inner = rest.get(..tag_end)?;
    let lower = inner.to_ascii_lowercase();
    let end = start + inner.len() + 2;
    let mut nodes = Vec::new();
    if !inner.is_empty()
        && !inner.contains([' ', '<', '\n'])
        && (inner.contains("://") || lower.starts_with("mailto:") || (inner.contains('@') && !inner.contains('/')))
    {
        let mut fmt = base.clone();
        fmt.link = Some(if inner.contains('@') && !inner.contains(':') { format!("mailto:{inner}").into() } else { inner.into() });
        push_text(&mut nodes, inner, &fmt);
        Some((end, nodes))
    } else if matches!(lower.trim_end_matches('/').trim(), "br") {
        push_text(&mut nodes, "\n", base);
        Some((end, nodes))
    } else if lower.starts_with("a ") && rest.get(inner.len() + 1..).is_some_and(|tail| tail.to_ascii_lowercase().starts_with("</a>")) {
        if let Some(name) = tag_attr(inner, "id").or_else(|| tag_attr(inner, "name")) {
            nodes.push(Node::Inl(Inline::Anchor(name)));
        }
        Some((end + 4, nodes))
    } else {
        None
    }
}

fn code_span(s: &str, start: usize, base: &Fmt, ticks: &Backticks) -> (String, Fmt, usize) {
    let bytes = s.as_bytes();
    let count = bytes.get(start..).map(|tail| tail.iter().take_while(|byte| **byte == b'`').count()).unwrap_or(1);
    let Some(end) = ticks.find(start + count, count) else {
        return (s.get(start..start + count).unwrap_or("").to_string(), base.clone(), start + count);
    };
    let mut code = s.get(start + count..end).unwrap_or("").replace('\n', " ");
    if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty() {
        code = code[1..code.len() - 1].to_string();
    }
    let mut fmt = base.clone();
    fmt.code = true;
    (code, fmt, end + count)
}

/// Longest `<...>` tag or autolink taken as inline HTML.
const MAX_TAG: usize = 4096;

/// The first `n` bytes of `s` (fewer if that would cut a character), so a search for a closing
/// character stays short however long the text is.
pub(super) fn within(s: &str, n: usize) -> &str {
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
    // Delimiter literals take the formatting (link, …) of the text before them: the last text
    // node, tracked here rather than searched for backwards from every delimiter (quadratic on
    // a paragraph of delimiters).
    let mut last_text: Option<&Fmt> = None;
    for n in &nodes {
        match n {
            Node::Text(t, f) => {
                push(&mut out, t, f, em, strong, strike);
                last_text = Some(f);
            }
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
                if *left > 0 {
                    let lit: String = std::iter::repeat_n(*ch, *left).collect();
                    let f = last_text.map_or_else(Fmt::default, |f| Fmt { bold: false, italic: false, strike: false, ..f.clone() });
                    push(&mut out, &lit, &f, em, strong, strike);
                }
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
