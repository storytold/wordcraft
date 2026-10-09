//! HTML.
//!
//! Import is a tolerant tag-soup reader (no DOM): a tokenizer plus a stack of open elements with
//! HTML's implicit closes for `p`, `li`, `tr`, `td`/`th` and headings. It understands `p`, `div`,
//! `h1`–`h6`, `b`/`strong`, `i`/`em`, `u`, `s`/`strike`/`del`, `sup`/`sub`, `code`/`kbd`, `a`
//! (links and `id`/`name` anchors), `br`, `hr`, `ul`/`ol`/`li` (nested), `table`/`tr`/`td`/`th`
//! (colspan/rowspan), `img` (`data:` URIs, other sources through a caller's [`ImageLoader`]), `blockquote`, `pre`, `font`, and the CSS
//! properties `color`, `background-color`, `font-weight`, `font-style`, `font-size`,
//! `font-family`, `text-decoration`, `vertical-align`, `text-align`, `white-space` and page
//! breaks. Scripts and styles are skipped; nesting is capped.
//!
//! Export writes clean semantic HTML with inline CSS and pictures as `data:` URIs.

use wordcraft_doc::{Align, Document};

use crate::model::{
    self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, base64_encode, data_uri, make_img, mime_of, parse_color,
};

/// Open elements kept at most (deeper start tags are ignored).
const MAX_STACK: usize = 256;

// ---------------------------------------------------------------------------------------------
// Entities and decoding

/// Decode an entity body (`amp`, `#38`, `#x26`) to its text.
pub fn decode_entity(name: &str) -> Option<String> {
    if let Some(num) = name.strip_prefix('#') {
        let v = if let Some(h) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok()? } else { num.parse::<u32>().ok()? };
        let c = match v {
            0 => '\u{FFFD}',
            0x80..=0x9F => crate::txt::cp1252(v as u8),
            _ => char::from_u32(v).unwrap_or('\u{FFFD}'),
        };
        return Some(c.to_string());
    }
    let c = match name {
        "amp" | "AMP" => '&',
        "lt" | "LT" => '<',
        "gt" | "GT" => '>',
        "quot" | "QUOT" => '"',
        "apos" => '\'',
        "nbsp" => '\u{00A0}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "shy" => '\u{00AD}',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "deg" => '°',
        "plusmn" => '±',
        "times" => '×',
        "divide" => '÷',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "sect" => '§',
        "para" => '¶',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "dagger" => '†',
        "Dagger" => '‡',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "harr" => '↔',
        "le" => '≤',
        "ge" => '≥',
        "ne" => '≠',
        "infin" => '∞',
        "check" => '✓',
        "iexcl" => '¡',
        "iquest" => '¿',
        "szlig" => 'ß',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "eacute" => 'é',
        "egrave" => 'è',
        "ecirc" => 'ê',
        "Eacute" => 'É',
        "aacute" => 'á',
        "agrave" => 'à',
        "acirc" => 'â',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "ntilde" => 'ñ',
        "Ntilde" => 'Ñ',
        "ccedil" => 'ç',
        "Ccedil" => 'Ç',
        "aring" => 'å',
        "Aring" => 'Å',
        "oslash" => 'ø',
        "Oslash" => 'Ø',
        "aelig" => 'æ',
        "AElig" => 'Æ',
        "alpha" => 'α',
        "beta" => 'β',
        "gamma" => 'γ',
        "delta" => 'δ',
        "pi" => 'π',
        "sigma" => 'σ',
        "mu" => 'μ',
        "Omega" => 'Ω',
        "omega" => 'ω',
        "zwj" => '\u{200D}',
        "zwnj" => '\u{200C}',
        _ => return None,
    };
    Some(c.to_string())
}

/// Replace entity references in `s`.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        out.push_str(&rest[..k]);
        let after = &rest[k + 1..];
        let end = after.find(';').filter(|e| *e <= 32);
        match end.and_then(|e| decode_entity(&after[..e]).map(|t| (t, e))) {
            Some((t, e)) => {
                out.push_str(&t);
                rest = &after[e + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Decode HTML bytes: BOM, then a `charset` in the first 2 KB, else UTF-8 with a Windows-1252
/// fallback.
pub fn decode(b: &[u8]) -> String {
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) || b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return crate::txt::decode(b);
    }
    let head = String::from_utf8_lossy(b.get(..b.len().min(2048)).unwrap_or(b)).to_ascii_lowercase();
    let latin = head.contains("charset=windows-1252") || head.contains("charset=iso-8859-1") || head.contains("charset=\"windows-1252\"");
    if !latin && let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    crate::txt::decode(b)
}

// ---------------------------------------------------------------------------------------------
// Tokenizer

#[derive(Debug)]
enum Tok {
    Text(String),
    Start { name: String, attrs: Vec<(String, String)>, self_close: bool },
    End(String),
}

struct Lexer<'a> {
    s: &'a str,
    pos: usize,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        self.s.get(self.pos..).unwrap_or("")
    }

    /// Skip past the next occurrence of `pat` (ASCII case-insensitive), or to the end.
    fn skip_past(&mut self, pat: &str) -> usize {
        let start = self.pos;
        let hay = self.rest().as_bytes();
        let p = pat.as_bytes();
        let found = hay.windows(p.len().max(1)).position(|w| w.eq_ignore_ascii_case(p));
        match found {
            Some(k) => self.pos += k + p.len(),
            None => self.pos = self.s.len(),
        }
        found.map(|k| start + k).unwrap_or(self.s.len())
    }

    fn next(&mut self) -> Option<Tok> {
        let rest = self.rest();
        if rest.is_empty() {
            return None;
        }
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let t = unescape(&rest[..end]);
            self.pos += end;
            return Some(Tok::Text(t));
        }
        let b = rest.as_bytes();
        if rest.starts_with("<!--") {
            self.pos += 4;
            self.skip_past("-->");
            return Some(Tok::Text(String::new()));
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            self.skip_past(">");
            return Some(Tok::Text(String::new()));
        }
        if rest.starts_with("</") {
            let name: String =
                rest.strip_prefix("</").unwrap_or("").chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-').collect();
            if name.is_empty() {
                self.pos += 2;
                return Some(Tok::Text("</".into()));
            }
            self.skip_past(">");
            return Some(Tok::End(name.to_ascii_lowercase()));
        }
        if !b.get(1).is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
            return Some(Tok::Text("<".into()));
        }
        // Start tag.
        let name: String = rest[1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-').collect();
        let mut i = 1 + name.len();
        let mut attrs = Vec::new();
        let mut self_close = false;
        loop {
            while b.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
            match b.get(i) {
                None => break,
                Some(b'>') => {
                    i += 1;
                    break;
                }
                Some(b'/') => {
                    if b.get(i + 1) == Some(&b'>') {
                        self_close = true;
                        i += 2;
                        break;
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let a0 = i;
            while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'=' | b'>' | b'/')) {
                i += 1;
            }
            let an = rest.get(a0..i).unwrap_or("").to_ascii_lowercase();
            if an.is_empty() {
                i += 1;
                continue;
            }
            while b.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
            let mut val = String::new();
            if b.get(i) == Some(&b'=') {
                i += 1;
                while b.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                    i += 1;
                }
                match b.get(i) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        let v0 = i + 1;
                        let e = rest.get(v0..).and_then(|r| r.find(q as char)).map(|k| v0 + k).unwrap_or(rest.len());
                        val = unescape(rest.get(v0..e).unwrap_or(""));
                        i = e + 1;
                    }
                    _ => {
                        let v0 = i;
                        while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>') {
                            i += 1;
                        }
                        val = unescape(rest.get(v0..i).unwrap_or(""));
                    }
                }
            }
            if attrs.len() < 64 {
                attrs.push((an, val));
            }
        }
        self.pos += i.min(rest.len());
        Some(Tok::Start { name: name.to_ascii_lowercase(), attrs, self_close })
    }
}

// ---------------------------------------------------------------------------------------------
// Builder

#[derive(Clone, Copy, PartialEq, Debug)]
enum ElKind {
    Inline,
    Block,
    Heading(u8),
    Quote,
    Pre,
    List(bool),
    Li,
    Table,
    Tr,
    Cell,
    Head,
}

#[derive(Clone, Debug)]
struct El {
    name: String,
    kind: ElKind,
    fmt: Fmt,
    align: Option<Align>,
    preserve: bool,
    page_break: bool,
    /// Li: a paragraph of this item was already emitted.
    used: bool,
}

struct TableB {
    rows: Vec<Vec<Cell>>,
    row: Option<Vec<Cell>>,
}

/// Loads the bytes of an image an `img` names by a source other than a `data:` URI (a path
/// relative to the HTML file). `None` when it can't be found.
pub type ImageLoader<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

struct Builder<'r> {
    images: ImageLoader<'r>,
    containers: Vec<Vec<FBlock>>,
    cells: Vec<Cell>,
    tables: Vec<TableB>,
    stack: Vec<El>,
    para: Option<Para>,
    /// The current paragraph continues a list item (joins the previous paragraph).
    para_cont: bool,
    meta: Meta,
    title: Option<String>,
}

fn is_void(n: &str) -> bool {
    matches!(
        n,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
            | "basefont"
            | "frame"
    )
}

fn block_kind(n: &str) -> Option<ElKind> {
    Some(match n {
        "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav" | "aside" | "figure" | "figcaption" | "address" | "dl" | "dt"
        | "dd" | "center" | "form" | "fieldset" | "body" | "html" | "caption" | "thead" | "tbody" | "tfoot" | "details" | "summary" | "legend"
        | "hgroup" => ElKind::Block,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => ElKind::Heading(n.get(1..2).and_then(|d| d.parse().ok()).unwrap_or(1)),
        "blockquote" => ElKind::Quote,
        "pre" | "listing" | "xmp" | "plaintext" => ElKind::Pre,
        "ul" | "menu" | "dir" => ElKind::List(false),
        "ol" => ElKind::List(true),
        "li" => ElKind::Li,
        "table" => ElKind::Table,
        "tr" => ElKind::Tr,
        "td" | "th" => ElKind::Cell,
        "head" => ElKind::Head,
        _ => return None,
    })
}

/// Closes an open `p` (HTML's "close a p element" on block starts).
fn closes_p(n: &str) -> bool {
    matches!(
        n,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "div"
            | "dl"
            | "fieldset"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
            | "figure"
            | "center"
            | "dd"
            | "dt"
            | "li"
    )
}

fn css_len_pt(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    if let Some(x) = v.strip_suffix("pt") {
        num(x)
    } else if let Some(x) = v.strip_suffix("px") {
        num(x).map(|p| p * 0.75)
    } else if let Some(x) = v.strip_suffix("rem").or_else(|| v.strip_suffix("em")) {
        num(x).map(|p| p * 12.0)
    } else if let Some(x) = v.strip_suffix("in") {
        num(x).map(|p| p * 72.0)
    } else if let Some(x) = v.strip_suffix("cm") {
        num(x).map(|p| p * 72.0 / 2.54)
    } else if let Some(x) = v.strip_suffix("mm") {
        num(x).map(|p| p * 72.0 / 25.4)
    } else if let Some(x) = v.strip_suffix('%') {
        num(x).map(|p| p * 0.12)
    } else {
        match v.as_str() {
            "xx-small" => Some(7.0),
            "x-small" => Some(7.5),
            "small" => Some(10.0),
            "medium" => Some(12.0),
            "large" => Some(13.5),
            "x-large" => Some(18.0),
            "xx-large" => Some(24.0),
            _ => num(&v).map(|p| p * 0.75),
        }
    }
}

/// Apply a `style` attribute to formatting / block properties.
fn apply_style(css: &str, f: &mut Fmt, align: &mut Option<Align>, preserve: &mut bool, page_break: &mut bool) {
    for decl in css.split(';').take(64) {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().trim_end_matches("!important").trim();
        let lv = v.to_ascii_lowercase();
        match k.as_str() {
            "font-weight" => f.bold = lv.starts_with("bold") || lv.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => f.italic = lv == "italic" || lv == "oblique",
            "text-decoration" | "text-decoration-line" => {
                if lv.contains("none") {
                    f.underline = false;
                    f.strike = false;
                }
                if lv.contains("underline") {
                    f.underline = true;
                }
                if lv.contains("line-through") {
                    f.strike = true;
                }
            }
            "color" => {
                if let Some(c) = parse_color(v) {
                    f.color = Some(c);
                }
            }
            "background-color" | "background" => {
                if let Some(c) = v.split_whitespace().find_map(parse_color) {
                    f.background = Some(c);
                }
            }
            "font-size" => {
                if let Some(p) = css_len_pt(v).filter(|p| *p >= 1.0 && *p <= 1638.0) {
                    f.size = Some(p);
                }
            }
            "font-family" => {
                let fam = v.split(',').next().unwrap_or("").trim().trim_matches(['"', '\'']).to_string();
                if model::is_mono(&fam) || fam.eq_ignore_ascii_case("monospace") {
                    f.code = true;
                } else if !fam.is_empty() && !matches!(fam.to_ascii_lowercase().as_str(), "serif" | "sans-serif" | "inherit" | "initial") {
                    f.font = Some(fam);
                }
            }
            "vertical-align" => {
                f.sup = lv == "super";
                f.sub = lv == "sub";
            }
            "text-align" => {
                *align = match lv.as_str() {
                    "center" => Some(Align::Center),
                    "right" | "end" => Some(Align::Right),
                    "justify" => Some(Align::Justify),
                    "left" | "start" => Some(Align::Left),
                    _ => *align,
                }
            }
            "white-space" => *preserve = lv.starts_with("pre"),
            "page-break-before" | "break-before" => *page_break = lv == "always" || lv == "page",
            _ => {}
        }
    }
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

impl<'r> Builder<'r> {
    fn new(images: ImageLoader<'r>) -> Builder<'r> {
        Builder {
            images,
            containers: vec![Vec::new()],
            cells: Vec::new(),
            tables: Vec::new(),
            stack: Vec::new(),
            para: None,
            para_cont: false,
            meta: Meta::default(),
            title: None,
        }
    }

    fn fmt(&self) -> Fmt {
        self.stack.last().map(|e| e.fmt.clone()).unwrap_or_default()
    }

    fn preserve(&self) -> bool {
        self.stack.last().is_some_and(|e| e.preserve)
    }

    fn in_head(&self) -> bool {
        self.stack.iter().any(|e| e.kind == ElKind::Head)
    }

    /// Paragraph context from the open elements (innermost first, up to the enclosing cell).
    fn start_para(&mut self) -> &mut Para {
        if self.para.is_none() {
            let mut p = Para::default();
            let mut list_level: Option<(bool, usize)> = None;
            let mut li_at: Option<usize> = None;
            let mut lists = 0usize;
            for (i, e) in self.stack.iter().enumerate().rev() {
                match e.kind {
                    ElKind::Cell | ElKind::Table => break,
                    ElKind::Heading(n) if p.kind == Kind::Normal => p.kind = Kind::Heading(n.clamp(1, 6)),
                    ElKind::Pre if p.kind == Kind::Normal => p.kind = Kind::Code,
                    ElKind::Quote if p.kind == Kind::Normal => p.kind = Kind::Quote,
                    ElKind::Li if li_at.is_none() => li_at = Some(i),
                    ElKind::List(o) => {
                        if list_level.is_none() {
                            list_level = Some((o, 0));
                        }
                        lists += 1;
                    }
                    _ => {}
                }
                if p.align.is_none() && e.align.is_some() && e.kind != ElKind::Inline {
                    p.align = e.align;
                }
            }
            if let (Some((o, _)), Some(li)) = (list_level, li_at) {
                p.list = Some(ListInfo { ordered: o, level: lists.saturating_sub(1).min(8) as u8 });
                self.para_cont = self.stack.get(li).is_some_and(|e| e.used);
                if let Some(e) = self.stack.get_mut(li) {
                    e.used = true;
                }
            } else {
                self.para_cont = false;
            }
            p.page_break = self.stack.iter().rev().take_while(|e| e.kind != ElKind::Cell).any(|e| e.page_break);
            self.para = Some(p);
        }
        self.para.get_or_insert_with(Para::default)
    }

    fn container(&mut self) -> &mut Vec<FBlock> {
        if self.containers.is_empty() {
            self.containers.push(Vec::new());
        }
        let n = self.containers.len() - 1;
        &mut self.containers[n]
    }

    fn flush(&mut self) {
        let Some(mut p) = self.para.take() else { return };
        if p.kind != Kind::Code {
            p.trim();
        }
        if p.inlines.len() == 1 && p.text() == "\u{00A0}" {
            // `<p>&nbsp;</p>`: an empty paragraph on purpose.
            p.inlines.clear();
            self.para_cont = false;
            self.container().push(FBlock::Para(p));
            return;
        }
        if p.is_empty() && p.kind != Kind::Code && p.list.is_none() {
            // Whitespace-only text between blocks.
            return;
        }
        let cont = std::mem::take(&mut self.para_cont);
        let level = p.list.map(|l| l.level);
        let c = self.container();
        if cont
            && let Some(FBlock::Para(prev)) = c.last_mut()
            && prev.list.is_some()
            && prev.list.map(|l| l.level) == level
        {
            prev.push_text("\n", &Fmt::default());
            prev.inlines.extend(p.inlines);
            return;
        }
        c.push(FBlock::Para(p));
    }

    fn text(&mut self, t: &str) {
        if self.in_head() {
            if let Some(title) = &mut self.title {
                title.push_str(t);
            }
            return;
        }
        if t.is_empty() {
            return;
        }
        let inside_table_only = matches!(
            self.stack.iter().rev().find(|e| matches!(e.kind, ElKind::Table | ElKind::Tr | ElKind::Cell)).map(|e| e.kind),
            Some(ElKind::Table | ElKind::Tr)
        );
        if inside_table_only && t.trim().is_empty() {
            return;
        }
        let code = self.stack.iter().any(|e| e.kind == ElKind::Pre);
        let f = self.fmt();
        if code {
            let mut first = true;
            for line in t.split('\n') {
                if !first {
                    // A newline right after <pre> is ignored.
                    let empty_start = self.para.as_ref().is_none_or(|p| p.inlines.is_empty()) && self.pre_fresh();
                    if !empty_start {
                        self.start_para();
                        self.flush();
                    }
                }
                first = false;
                let line = line.trim_end_matches('\r');
                if !line.is_empty() {
                    self.start_para().push_text(line, &Fmt { code: false, ..f.clone() });
                    self.mark_pre_used();
                }
            }
            return;
        }
        if self.preserve() {
            self.start_para().push_text(t, &f);
            return;
        }
        // Collapse whitespace.
        let mut s = String::with_capacity(t.len());
        let mut prev_space = self.para.as_ref().is_none_or(|p| match p.inlines.last() {
            Some(Inline::Text(x, _)) => x.ends_with([' ', '\n']),
            Some(_) => false,
            None => true,
        });
        for c in t.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000C}') {
                if !prev_space {
                    s.push(' ');
                    prev_space = true;
                }
            } else {
                s.push(c);
                prev_space = false;
            }
        }
        if s.is_empty() {
            return;
        }
        self.start_para().push_text(&s, &f);
    }

    fn pre_fresh(&self) -> bool {
        self.stack.iter().rev().find(|e| e.kind == ElKind::Pre).is_some_and(|e| !e.used)
    }

    fn mark_pre_used(&mut self) {
        if let Some(e) = self.stack.iter_mut().rev().find(|e| e.kind == ElKind::Pre) {
            e.used = true;
        }
    }

    /// Pop elements down to and including index `k`.
    fn pop_to(&mut self, k: usize) {
        while self.stack.len() > k {
            let Some(e) = self.stack.pop() else { break };
            self.close(&e);
        }
    }

    fn close(&mut self, e: &El) {
        if e.kind != ElKind::Inline {
            self.flush();
        }
        match e.kind {
            ElKind::Cell => {
                self.flush();
                let blocks = if self.containers.len() > 1 { self.containers.pop().unwrap_or_default() } else { Vec::new() };
                let mut cell = self.cells.pop().unwrap_or_default();
                cell.blocks = blocks;
                if let Some(t) = self.tables.last_mut() {
                    t.row.get_or_insert_with(Vec::new).push(cell);
                }
            }
            ElKind::Tr => {
                if let Some(t) = self.tables.last_mut()
                    && let Some(r) = t.row.take()
                {
                    t.rows.push(r);
                }
            }
            ElKind::Table => {
                if let Some(mut t) = self.tables.pop() {
                    if let Some(r) = t.row.take() {
                        t.rows.push(r);
                    }
                    let mut ft = FTable { rows: t.rows.into_iter().filter(|r| !r.is_empty()).collect(), widths: Vec::new() };
                    if !ft.rows.is_empty() {
                        ft.insert_covered();
                        self.container().push(FBlock::Table(ft));
                    }
                }
            }
            ElKind::Head => {}
            _ => {}
        }
    }

    /// Index of the innermost open element named `name`, not looking past a table boundary
    /// unless `cross_tables`.
    fn find_open(&self, pred: impl Fn(&El) -> bool, stop: impl Fn(&El) -> bool) -> Option<usize> {
        for (i, e) in self.stack.iter().enumerate().rev() {
            if pred(e) {
                return Some(i);
            }
            if stop(e) {
                return None;
            }
        }
        None
    }

    fn start(&mut self, name: &str, attrs: &[(String, String)], self_close: bool) {
        // Head metadata.
        match name {
            "title" => {
                self.title = Some(String::new());
            }
            "meta" => {
                let n = attr(attrs, "name").unwrap_or("").to_ascii_lowercase();
                let c = attr(attrs, "content").unwrap_or("").to_string();
                match n.as_str() {
                    "author" => self.meta.author = c,
                    "description" => self.meta.description = c,
                    "keywords" => self.meta.keywords = c,
                    "subject" => self.meta.subject = c,
                    _ => {}
                }
                return;
            }
            _ => {}
        }
        let is_table_part = matches!(name, "tr" | "td" | "th");
        let in_table = self.tables.len() < model::MAX_DEPTH;
        // Implicit closes.
        if closes_p(name)
            && let Some(k) = self.find_open(|e| e.name == "p", |e| matches!(e.kind, ElKind::Cell | ElKind::Table | ElKind::Li | ElKind::Quote))
        {
            self.pop_to(k);
        }
        if matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") && self.stack.last().is_some_and(|e| matches!(e.kind, ElKind::Heading(_))) {
            let k = self.stack.len() - 1;
            self.pop_to(k);
        }
        if name == "li"
            && let Some(k) = self.find_open(|e| e.kind == ElKind::Li, |e| matches!(e.kind, ElKind::List(_) | ElKind::Table | ElKind::Cell))
        {
            self.pop_to(k);
        }
        if matches!(name, "dt" | "dd")
            && let Some(k) =
                self.find_open(|e| e.name == "dt" || e.name == "dd", |e| matches!(e.kind, ElKind::Table | ElKind::Cell) || e.name == "dl")
        {
            self.pop_to(k);
        }
        if is_table_part && !self.tables.is_empty() {
            if matches!(name, "td" | "th")
                && let Some(k) = self.find_open(|e| e.kind == ElKind::Cell, |e| matches!(e.kind, ElKind::Table | ElKind::Tr))
            {
                self.pop_to(k);
            }
            if name == "tr"
                && let Some(k) = self.find_open(|e| matches!(e.kind, ElKind::Tr | ElKind::Cell), |e| e.kind == ElKind::Table)
            {
                // Pop to the outermost of cell/row inside the current table.
                let mut kk = k;
                while kk > 0 && self.stack.get(kk - 1).is_some_and(|e| matches!(e.kind, ElKind::Tr | ElKind::Cell)) {
                    kk -= 1;
                }
                self.pop_to(kk);
            }
        }
        if name == "a"
            && !self.in_head()
            && let Some(id) = attr(attrs, "id").or_else(|| attr(attrs, "name")).filter(|s| !s.is_empty())
        {
            self.start_para().inlines.push(Inline::Anchor(id.to_string()));
        }
        // Element-specific work.
        match name {
            "br" => {
                if self.stack.iter().any(|e| e.kind == ElKind::Pre) {
                    self.start_para();
                    self.flush();
                } else {
                    let f = self.fmt();
                    self.start_para().push_text("\n", &f);
                }
                return;
            }
            "hr" => {
                self.flush();
                self.container().push(FBlock::Para(Para::new(Kind::Rule)));
                return;
            }
            "img" => {
                let alt = attr(attrs, "alt").unwrap_or("").to_string();
                let dim = |k: &str| attr(attrs, k).and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok()).map(|p| p * 0.75);
                let mut w = dim("width");
                let mut h = dim("height");
                if let Some(st) = attr(attrs, "style") {
                    for decl in st.split(';') {
                        if let Some((k, v)) = decl.split_once(':') {
                            match k.trim().to_ascii_lowercase().as_str() {
                                "width" => w = css_len_pt(v).or(w),
                                "height" => h = css_len_pt(v).or(h),
                                _ => {}
                            }
                        }
                    }
                }
                let img = attr(attrs, "src")
                    .and_then(|src| if src.trim_start().starts_with("data:") { data_uri(src) } else { (self.images)(src.trim()) })
                    .and_then(|d| make_img(d, w, h, &alt));
                let f = self.fmt();
                match img {
                    Some(im) => self.start_para().inlines.push(Inline::Image(im)),
                    None if !alt.is_empty() => self.start_para().push_text(&alt, &f),
                    None => {}
                }
                return;
            }
            _ => {}
        }
        if is_void(name) || self_close && block_kind(name).is_none() && !matches!(name, "a" | "span") {
            return;
        }
        if self.stack.len() >= MAX_STACK {
            return;
        }
        let mut kind = block_kind(name).unwrap_or(ElKind::Inline);
        if matches!(kind, ElKind::Table | ElKind::Tr | ElKind::Cell) && !in_table {
            kind = ElKind::Block;
        }
        if matches!(kind, ElKind::Tr | ElKind::Cell) && self.tables.is_empty() {
            kind = ElKind::Block;
        }
        if kind != ElKind::Inline {
            self.flush();
        }
        let mut fmt = self.fmt();
        let mut align = None;
        let mut preserve = self.preserve();
        let mut page_break = false;
        match name {
            "b" | "strong" => fmt.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" | "address" => fmt.italic = true,
            "u" | "ins" => fmt.underline = true,
            "s" | "strike" | "del" => fmt.strike = true,
            "code" | "kbd" | "tt" | "samp" => fmt.code = true,
            "sup" => fmt.sup = true,
            "sub" => fmt.sub = true,
            "mark" => fmt.background = Some(wordcraft_doc::Rgb(255, 255, 0)),
            "font" => {
                if let Some(c) = attr(attrs, "color").and_then(parse_color) {
                    fmt.color = Some(c);
                }
                if let Some(face) = attr(attrs, "face") {
                    let fam = face.split(',').next().unwrap_or("").trim().to_string();
                    if model::is_mono(&fam) {
                        fmt.code = true;
                    } else if !fam.is_empty() {
                        fmt.font = Some(fam);
                    }
                }
                if let Some(sz) = attr(attrs, "size").and_then(|s| s.trim().parse::<i32>().ok()) {
                    fmt.size = Some([7.5, 10.0, 12.0, 13.5, 18.0, 24.0, 36.0].get((sz.clamp(1, 7) - 1) as usize).copied().unwrap_or(12.0));
                }
            }
            "a" => {
                if let Some(h) =
                    attr(attrs, "href").filter(|h| !h.trim().is_empty() && !h.trim_start().to_ascii_lowercase().starts_with("javascript:"))
                {
                    fmt.link = Some(h.trim().to_string());
                }
            }
            _ => {}
        }
        if let Some(a) = attr(attrs, "align") {
            align = match a.to_ascii_lowercase().as_str() {
                "center" | "middle" => Some(Align::Center),
                "right" => Some(Align::Right),
                "justify" => Some(Align::Justify),
                "left" => Some(Align::Left),
                _ => None,
            };
        }
        if name == "center" {
            align = Some(Align::Center);
        }
        if let Some(st) = attr(attrs, "style") {
            apply_style(st, &mut fmt, &mut align, &mut preserve, &mut page_break);
        }
        if kind == ElKind::Pre {
            preserve = true;
        }
        match kind {
            ElKind::Table => self.tables.push(TableB { rows: Vec::new(), row: None }),
            ElKind::Tr => {
                if let Some(t) = self.tables.last_mut()
                    && let Some(r) = t.row.take()
                {
                    t.rows.push(r);
                }
            }
            ElKind::Cell => {
                let tb = self.find_open(|e| e.kind == ElKind::Tr, |e| e.kind == ElKind::Table).is_none();
                if tb && let Some(t) = self.tables.last_mut() {
                    // A cell without a row: start one.
                    if let Some(r) = t.row.take() {
                        t.rows.push(r);
                    }
                    t.row = Some(Vec::new());
                }
                let num = |k: &str| attr(attrs, k).and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.clamp(1, 63));
                let shading = attr(attrs, "bgcolor").and_then(parse_color).or(fmt.background);
                self.cells.push(Cell {
                    blocks: Vec::new(),
                    colspan: num("colspan").unwrap_or(1),
                    rowspan: attr(attrs, "rowspan").and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.clamp(1, 1000)).unwrap_or(1),
                    covered: false,
                    header: name == "th",
                    shading,
                });
                fmt.background = None;
                self.containers.push(Vec::new());
            }
            _ => {}
        }
        self.stack.push(El { name: name.to_string(), kind, fmt, align, preserve, page_break, used: false });
    }

    fn end(&mut self, name: &str) {
        if name == "title" {
            if let Some(t) = self.title.take() {
                self.meta.title = t.split_whitespace().collect::<Vec<_>>().join(" ");
            }
            return;
        }
        let crosses = matches!(name, "table");
        let target = if matches!(name, "td" | "th") {
            self.find_open(|e| e.kind == ElKind::Cell, |e| e.kind == ElKind::Table)
        } else if name == "tr" {
            self.find_open(|e| e.kind == ElKind::Tr, |e| e.kind == ElKind::Table)
        } else {
            self.find_open(|e| e.name == name, |e| !crosses && matches!(e.kind, ElKind::Table | ElKind::Cell))
        };
        if let Some(k) = target {
            self.pop_to(k);
        }
    }

    fn finish(mut self) -> Flow {
        self.pop_to(0);
        self.flush();
        // Unclosed cells: fold remaining containers into the body.
        while self.containers.len() > 1 {
            let c = self.containers.pop().unwrap_or_default();
            self.container().extend(c);
        }
        let blocks = self.containers.pop().unwrap_or_default();
        if let Some(t) = self.title.take() {
            self.meta.title = t;
        }
        Flow { blocks, meta: self.meta }
    }
}

/// Parse HTML text into the flow model.
pub fn parse(s: &str) -> Flow {
    parse_with(s, &|_| None)
}

/// [`parse`], loading pictures that aren't `data:` URIs through `images`.
pub fn parse_with(s: &str, images: ImageLoader) -> Flow {
    let mut lx = Lexer { s, pos: 0 };
    let mut b = Builder::new(images);
    let mut guard = 0usize;
    while let Some(t) = lx.next() {
        guard += 1;
        if guard > 50_000_000 {
            break;
        }
        match t {
            Tok::Text(t) => b.text(&t),
            Tok::Start { name, attrs, self_close } => {
                if matches!(name.as_str(), "script" | "style" | "noscript" | "template" | "textarea") && !self_close {
                    lx.skip_past(&format!("</{name}"));
                    lx.skip_past(">");
                    continue;
                }
                b.start(&name, &attrs, self_close);
            }
            Tok::End(name) => b.end(&name),
        }
    }
    b.finish()
}

/// Parse HTML into a document.
pub fn import(s: &str) -> Document {
    model::to_doc(&parse(s))
}

/// [`import`], loading pictures that aren't `data:` URIs through `images`.
pub fn import_with(s: &str, images: ImageLoader) -> Document {
    model::to_doc(&parse_with(s, images))
}

// ---------------------------------------------------------------------------------------------
// Export

/// Escape text for HTML content and attribute values.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\u{000C}' | '\u{000E}' => {}
            _ => o.push(c),
        }
    }
    o
}

fn align_css(a: Option<Align>) -> Option<&'static str> {
    match a? {
        Align::Center => Some("text-align:center"),
        Align::Right => Some("text-align:right"),
        Align::Justify | Align::Distribute => Some("text-align:justify"),
        Align::Left => None,
    }
}

fn text_html(t: &str) -> String {
    let mut o = String::new();
    let mut prev_space = false;
    for c in t.chars() {
        match c {
            '\n' => {
                o.push_str("<br>");
                prev_space = true;
                continue;
            }
            '\t' => o.push_str("<span style=\"white-space:pre\">\t</span>"),
            ' ' if prev_space => o.push_str("&nbsp;"),
            _ => o.push_str(&esc(&c.to_string())),
        }
        prev_space = c == ' ';
    }
    o
}

/// Inline spans → HTML.
pub fn inlines_html(inlines: &[Inline]) -> String {
    let mut o = String::new();
    for i in inlines {
        match i {
            Inline::Text(t, f) => {
                let mut open = String::new();
                let mut close: Vec<&str> = Vec::new();
                if let Some(l) = &f.link {
                    open.push_str(&format!("<a href=\"{}\">", esc(l)));
                    close.push("</a>");
                }
                let mut tag = |o: &str, c: &'static str| {
                    open.push_str(o);
                    close.push(c);
                };
                if f.bold {
                    tag("<strong>", "</strong>");
                }
                if f.italic {
                    tag("<em>", "</em>");
                }
                if f.underline {
                    tag("<u>", "</u>");
                }
                if f.strike {
                    tag("<s>", "</s>");
                }
                if f.sup {
                    tag("<sup>", "</sup>");
                } else if f.sub {
                    tag("<sub>", "</sub>");
                }
                if f.code {
                    tag("<code>", "</code>");
                }
                let mut css = Vec::new();
                if let Some(c) = f.color {
                    css.push(format!("color:#{}", c.hex()));
                }
                if let Some(c) = f.background {
                    css.push(format!("background-color:#{}", c.hex()));
                }
                if let Some(s) = f.size {
                    css.push(format!("font-size:{}pt", trim_num(s)));
                }
                if let Some(fam) = &f.font {
                    css.push(format!("font-family:'{}'", esc(&fam.replace('\'', ""))));
                }
                if !css.is_empty() {
                    open.push_str(&format!("<span style=\"{}\">", css.join(";")));
                    close.push("</span>");
                }
                o.push_str(&open);
                o.push_str(&text_html(t));
                for c in close.iter().rev() {
                    o.push_str(c);
                }
            }
            Inline::Image(img) => {
                o.push_str(&format!(
                    "<img src=\"data:{};base64,{}\" alt=\"{}\" width=\"{}\" height=\"{}\">",
                    mime_of(&img.ext),
                    base64_encode(&img.data),
                    esc(&img.alt),
                    (img.w / 0.75).round(),
                    (img.h / 0.75).round()
                ));
            }
            Inline::Anchor(a) => o.push_str(&format!("<a id=\"{}\"></a>", esc(a))),
        }
    }
    o
}

fn trim_num(v: f32) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn para_attrs(p: &Para) -> String {
    let mut css: Vec<&str> = Vec::new();
    if let Some(a) = align_css(p.align) {
        css.push(a);
    }
    if p.page_break {
        css.push("break-before:page;page-break-before:always");
    }
    if css.is_empty() { String::new() } else { format!(" style=\"{}\"", css.join(";")) }
}

fn para_html(p: &Para) -> String {
    let body = inlines_html(&p.inlines);
    let attrs = para_attrs(p);
    match p.kind {
        Kind::Heading(n) => format!("<h{n}{attrs}>{body}</h{n}>", n = n.clamp(1, 6)),
        Kind::Title => format!("<h1{attrs}>{body}</h1>"),
        Kind::Rule => "<hr>".to_string(),
        _ if body.is_empty() => format!("<p{attrs}>&nbsp;</p>"),
        _ => format!("<p{attrs}>{body}</p>"),
    }
}

fn table_html(t: &FTable, out: &mut String, depth: usize) {
    out.push_str("<table style=\"border-collapse:collapse\">\n");
    for row in &t.rows {
        out.push_str("<tr>");
        for c in row {
            if c.covered {
                continue;
            }
            let tag = if c.header { "th" } else { "td" };
            let mut attrs = String::new();
            if c.colspan > 1 {
                attrs.push_str(&format!(" colspan=\"{}\"", c.colspan));
            }
            if c.rowspan > 1 {
                attrs.push_str(&format!(" rowspan=\"{}\"", c.rowspan));
            }
            let mut css = vec!["border:1px solid #999".to_string(), "padding:4px 6px".to_string(), "vertical-align:top".to_string()];
            if let Some(s) = c.shading {
                css.push(format!("background-color:#{}", s.hex()));
            }
            let single = match c.blocks.as_slice() {
                [FBlock::Para(p)] if p.kind == Kind::Normal && p.list.is_none() && !p.page_break => Some(p),
                _ => None,
            };
            if let Some(a) = single.and_then(|p| align_css(p.align)) {
                css.push(a.to_string());
            }
            out.push_str(&format!("<{tag}{attrs} style=\"{}\">", css.join(";")));
            match single {
                Some(p) => out.push_str(&inlines_html(&p.inlines)),
                None => {
                    out.push('\n');
                    blocks_html(&c.blocks, out, depth + 1);
                }
            }
            out.push_str(&format!("</{tag}>"));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}

fn blocks_html(blocks: &[FBlock], out: &mut String, depth: usize) {
    let mut i = 0;
    while let Some(b) = blocks.get(i) {
        match b {
            FBlock::Table(t) => {
                if depth < model::MAX_DEPTH {
                    table_html(t, out, depth);
                }
                i += 1;
            }
            FBlock::Para(p) if p.list.is_some() => {
                // Nested lists from consecutive list paragraphs.
                let mut stack: Vec<(bool, bool)> = Vec::new(); // (ordered, li open)
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    let Some(li) = q.list else { break };
                    let lv = li.level.min(8) as usize;
                    while stack.len() > lv + 1 {
                        close_list(&mut stack, out);
                    }
                    if stack.len() == lv + 1 && stack.last().is_some_and(|s| s.0 != li.ordered) {
                        close_list(&mut stack, out);
                    }
                    while stack.len() < lv + 1 {
                        if let Some(top) = stack.last_mut()
                            && !top.1
                        {
                            out.push_str("<li>");
                            top.1 = true;
                        }
                        out.push_str(if li.ordered { "<ol>" } else { "<ul>" });
                        stack.push((li.ordered, false));
                    }
                    if let Some(top) = stack.last_mut() {
                        if top.1 {
                            out.push_str("</li>\n");
                        }
                        top.1 = true;
                    }
                    let body = inlines_html(&q.inlines);
                    let attrs = para_attrs(q);
                    out.push_str(&format!("<li{attrs}>{body}"));
                    i += 1;
                }
                while !stack.is_empty() {
                    close_list(&mut stack, out);
                }
                out.push('\n');
            }
            FBlock::Para(p) if p.kind == Kind::Code => {
                out.push_str("<pre><code>");
                let mut first = true;
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if q.kind != Kind::Code || q.list.is_some() {
                        break;
                    }
                    if !first {
                        out.push('\n');
                    }
                    first = false;
                    out.push_str(&esc(&q.text()));
                    i += 1;
                }
                out.push_str("</code></pre>\n");
            }
            FBlock::Para(p) if p.kind == Kind::Quote => {
                out.push_str("<blockquote>\n");
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if q.kind != Kind::Quote || q.list.is_some() {
                        break;
                    }
                    out.push_str(&para_html(&Para { kind: Kind::Normal, ..q.clone() }));
                    out.push('\n');
                    i += 1;
                }
                out.push_str("</blockquote>\n");
            }
            FBlock::Para(p) => {
                out.push_str(&para_html(p));
                out.push('\n');
                i += 1;
            }
        }
    }
}

fn close_list(stack: &mut Vec<(bool, bool)>, out: &mut String) {
    if let Some((ordered, li_open)) = stack.pop() {
        if li_open {
            out.push_str("</li>");
        }
        out.push_str(if ordered { "</ol>" } else { "</ul>" });
    }
}

/// A complete HTML document.
pub fn export(doc: &Document) -> String {
    let flow = model::from_doc(doc);
    export_flow(&flow, doc.styles.default_chr.lang.as_deref().unwrap_or("en"))
}

pub fn export_flow(flow: &Flow, lang: &str) -> String {
    let m = &flow.meta;
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n");
    out.push_str(&format!("<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n", esc(lang)));
    out.push_str("<meta name=\"generator\" content=\"WordCraft\">\n");
    out.push_str(&format!("<title>{}</title>\n", esc(&m.title)));
    for (k, v) in [("author", &m.author), ("description", &m.description), ("keywords", &m.keywords), ("subject", &m.subject)] {
        if !v.is_empty() {
            out.push_str(&format!("<meta name=\"{k}\" content=\"{}\">\n", esc(v)));
        }
    }
    out.push_str(
        "<style>\nbody{font-family:Aptos,Calibri,'Segoe UI',Arial,sans-serif;font-size:12pt;line-height:1.15;max-width:48em;margin:2em auto;padding:0 1em}\n\
         table{border-collapse:collapse;margin:0.5em 0}\nblockquote{margin:0.5em 2em;font-style:italic;color:#404040}\n\
         pre{background:#f5f5f5;padding:0.5em;font-family:'Courier New',monospace;font-size:10pt}\n</style>\n</head>\n<body>\n",
    );
    blocks_html(&flow.blocks, &mut out, 0);
    out.push_str("</body>\n</html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(f: &Flow) -> Vec<String> {
        f.blocks
            .iter()
            .map(|b| match b {
                FBlock::Para(p) => format!("{:?}{:?}:{}", p.kind, p.list.map(|l| (l.ordered, l.level)), p.text()),
                FBlock::Table(t) => format!("table{}x{}", t.rows.len(), t.cols()),
            })
            .collect()
    }

    #[test]
    fn tag_soup() {
        let f = parse(
            "<html><head><title>T &amp; U</title><style>p{}</style><script>if(a<b)x</script></head><body><h1>Head</h1><p>One <b>bold<p>Two <i>it</i>\n  text<ul><li>a<li>b<ol><li>c</ol></ul><table><tr><td>x<td colspan=2>y<tr><th>z</table><blockquote>q</blockquote><pre>l1\nl2</pre>",
        );
        assert_eq!(f.meta.title, "T & U");
        assert_eq!(
            texts(&f),
            vec![
                "Heading(1)None:Head",
                "NormalNone:One bold",
                "NormalNone:Two it text",
                "NormalSome((false, 0)):a",
                "NormalSome((false, 0)):b",
                "NormalSome((true, 1)):c",
                "table2x3",
                "QuoteNone:q",
                "CodeNone:l1",
                "CodeNone:l2",
            ]
        );
        let FBlock::Para(p) = &f.blocks[1] else { panic!() };
        assert!(matches!(&p.inlines[1], Inline::Text(t, f) if t == "bold" && f.bold));
    }

    #[test]
    fn styles_and_links() {
        let f = parse("<p style='text-align:center'><span style=\"color:#ff0000;font-weight:700\">red</span> <a href=\"http://x\">l</a><br>n</p>");
        let FBlock::Para(p) = &f.blocks[0] else { panic!() };
        assert_eq!(p.align, Some(Align::Center));
        assert!(matches!(&p.inlines[0], Inline::Text(t, f) if t == "red" && f.bold && f.color == Some(wordcraft_doc::Rgb(255,0,0))));
        assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "l" && f.link.as_deref() == Some("http://x"))));
        assert!(p.text().ends_with("l\nn"));
    }

    #[test]
    fn entities() {
        assert_eq!(unescape("a &lt;b&gt; &#65;&#x42; &bogus; &"), "a <b> AB &bogus; &");
    }
}
