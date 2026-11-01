use super::blocks::thematic_break;
use super::inlines::{is_punct, within};
use super::{Align, Arc, Cell, Document, FBlock, FTable, Fmt, Inline, Kind, model};

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
pub(super) fn escape_line_start(s: String) -> String {
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
pub(super) fn inlines_md(inlines: &[Inline]) -> String {
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
    let mut link: Option<Arc<str>> = None;
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
        let body = if f.code {
            if in_table { code_span(&text.replace('|', "\\|")) } else { code_span(&text) }
        } else {
            escape(&text).replace('\n', "\\\n").replace('\t', "    ")
        };
        // Close until the stack is a prefix of what we want.
        let mut close = String::new();
        while let Some(pos) = stack.iter().position(|m| !want.contains(m)) {
            while stack.len() > pos {
                if let Some(m) = stack.pop() {
                    close.push_str(marker(m));
                }
            }
        }
        let mut open = String::new();
        for m in want {
            if !stack.contains(&m) {
                open.push_str(marker(m));
                stack.push(m);
            }
        }
        // CommonMark only reads a closing run after punctuation as closing when whitespace or
        // punctuation follows it, and an opening run before punctuation as opening when whitespace
        // or punctuation precedes it. Where a letter sits there instead, or an opening run would
        // directly follow a closing run (the two would merge into one run), an empty HTML comment (which readers drop) supplies
        // the punctuation.
        let prev = out.chars().next_back();
        let next = body.chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace() && !is_punct(c));
        out.push_str(&close);
        let close_needs = !close.is_empty() && prev.is_some_and(is_punct) && (!open.is_empty() || word(next));
        let open_needs = !open.is_empty() && next.is_some_and(is_punct) && (!close.is_empty() || word(prev));
        if close_needs || open_needs {
            out.push_str(EMPTY_COMMENT);
        }
        out.push_str(&open);
        out.push_str(&body);
    }
    close_all(&mut out, &mut stack);
    if let Some(l) = &link {
        out.push_str(&format!("]({})", link_target(l)));
    }
    out
}

/// `l` as a Markdown link destination that reads back as exactly `l`, the target
/// `is_safe_href` passed. What Markdown would decode (a backslash escape, an entity reference)
/// or end the destination on (`<`, `>`) is backslash-escaped, never deleted: deleting `<` or `>`
/// could turn an accepted relative target into `javascript:…` or `//host`. Control characters
/// can't be written in a destination and are left out, as the safety check ignores them.
fn link_target(l: &str) -> String {
    let l: String = l.chars().filter(|c| !c.is_ascii_control()).collect();
    let mut out = String::with_capacity(l.len() + 2);
    for (i, c) in l.char_indices() {
        let rest = l.get(i + c.len_utf8()..).unwrap_or("");
        match c {
            '<' | '>' => out.push('\\'),
            // A backslash is literal unless punctuation (or the closing `)`/`>`) follows it.
            '\\' if rest.chars().next().is_none_or(|n| n.is_ascii_punctuation()) => out.push('\\'),
            '&' if entity_like(rest) => out.push('\\'),
            _ => {}
        }
        out.push(c);
    }
    if l.contains(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>')) { format!("<{out}>") } else { out }
}

/// Whether `rest` (the text after a `&`) starts like an entity reference (`amp;`, `#106;`,
/// `#x6A;`), which a Markdown reader would decode.
fn entity_like(rest: &str) -> bool {
    let Some(end) = within(rest, 34).find(';') else { return false };
    let body = rest.get(..end).unwrap_or("");
    let body = body.strip_prefix('#').map_or(body, |n| n.strip_prefix(['x', 'X']).unwrap_or(n));
    !body.is_empty() && body.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Inline HTML that Markdown readers drop, used to separate emphasis markers from letters.
const EMPTY_COMMENT: &str = "<!-- -->";

fn marker(m: Mark) -> &'static str {
    match m {
        Mark::Bold => "**",
        Mark::Italic => "*",
        Mark::Strike => "~~",
    }
}

pub(super) fn table_md(t: &FTable, out: &mut Vec<String>) {
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

/// A heading's text on one line, with a trailing run of '#' escaped so a reader does not take it
/// for the optional closing sequence (`## Item #` reads back as "Item").
fn heading_text(body: &str) -> String {
    let mut s = body.replace("\\\n", " ");
    let t = s.trim_end();
    let core = t.trim_end_matches('#');
    if core.len() != t.len() && (core.is_empty() || core.ends_with([' ', '\t'])) {
        s.insert(core.len(), '\\');
    }
    s
}

pub(super) fn write_blocks(blocks: &[FBlock], out: &mut Vec<String>) {
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
                            Kind::Heading(n) => format!("{} {}", "#".repeat(n.clamp(1, 6) as usize), heading_text(&body)),
                            Kind::Title => format!("# {}", heading_text(&body)),
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
