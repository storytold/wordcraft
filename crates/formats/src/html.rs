//! HTML.
//!
//! Import is a tolerant tag-soup reader (no DOM): a tokenizer plus a stack of open elements with
//! HTML's implicit closes for `p`, `li`, `tr`, `td`/`th` and headings. It understands `p`, `div`,
//! `h1`–`h6`, `b`/`strong`, `i`/`em`, `u`, `s`/`strike`/`del`, `sup`/`sub`, `code`/`kbd`, `a`
//! (links and `id`/`name` anchors), `br`, `hr`, `ul`/`ol`/`li` (nested), `table`/`tr`/`td`/`th`
//! (colspan/rowspan), `img` (`data:` URIs, other sources through a caller's [`ImageLoader`]), `blockquote`, `pre`, `font`, and the CSS
//! properties `color`, `background-color`, `font-weight`, `font-style`, `font-size`,
//! `font-family`, `text-decoration`, `vertical-align`, `text-align`, `white-space` and page
//! breaks, and on paragraphs `margin`, `text-indent` and `line-height`. CSS comes from `style`
//! attributes and from `style` elements' rules with simple selectors (`p`, `.note`, `p.note`),
//! applied in cascade order (specificity, then source order, then the `style` attribute).
//! Scripts are skipped; nesting is capped.
//!
//! Export writes clean semantic HTML with inline CSS and pictures as `data:` URIs.

use std::sync::Arc;

use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{Align, Document};

use crate::model::{
    self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, Spacing, base64_encode, data_uri, make_img, mime_of, parse_color,
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
        // An entity name is short; looking further would rescan the whole text for every stray `&`.
        let mut cut = after.len().min(33);
        while !after.is_char_boundary(cut) {
            cut -= 1;
        }
        let end = after.get(..cut).and_then(|w| w.find(';'));
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
    decode_as(b, None)
}

/// [`decode`] in the `charset` its container states (a MIME part's, a content type's), when it
/// states one. Single-byte Windows code pages (`windows-1250`…, `iso-8859-1`, `us-ascii` as 1252)
/// are decoded as such.
pub fn decode_as(b: &[u8], charset: Option<&str>) -> String {
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) || b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return crate::txt::decode(b);
    }
    let head = String::from_utf8_lossy(b.get(..b.len().min(2048)).unwrap_or(b)).to_ascii_lowercase();
    let named = charset.map(str::to_ascii_lowercase).or_else(|| {
        let at = head.find("charset=")? + "charset=".len();
        Some(head.get(at..)?.trim_start_matches(['"', '\'']).chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect())
    });
    let code_page = match named.as_deref().map(str::trim) {
        Some("iso-8859-1" | "latin1" | "us-ascii") => Some(1252),
        Some(cs) => cs
            .strip_prefix("windows-")
            .or_else(|| cs.strip_prefix("cp"))
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|cp| wordcraft_doc::encoding::SINGLE_BYTE_CODE_PAGES.contains(cp)),
        None => None,
    };
    match code_page {
        Some(cp) => b.iter().map(|&c| wordcraft_doc::encoding::windows_byte(cp, c)).collect(),
        None => match std::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            Err(_) => crate::txt::decode(b),
        },
    }
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
            let name: String = rest
                .strip_prefix("</")
                .unwrap_or("")
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-')
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if name.is_empty() {
                self.pos += 2;
                return Some(Tok::Text("</".into()));
            }
            self.skip_past(">");
            return Some(Tok::End(name));
        }
        if !b.get(1).is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
            return Some(Tok::Text("<".into()));
        }
        // Start tag.
        let name: String =
            rest[1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-').map(|c| c.to_ascii_lowercase()).collect();
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
        Some(Tok::Start { name, attrs, self_close })
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
    /// Margins, indents and line height its CSS states (blocks only; they don't inherit; on a
    /// page's spans, their margins).
    spacing: Spacing,
    /// A page's `p` or heading: margins its CSS leaves out (or writes invalidly) are HTML's
    /// automatic spacing.
    auto_margins: bool,
    align: Option<Align>,
    preserve: bool,
    page_break: bool,
    /// Li: a paragraph of this item was already emitted.
    used: bool,
}

struct TableB {
    rows: Vec<Vec<Cell>>,
    row: Option<Vec<Cell>>,
    /// The table or one of its cells asks for borders.
    bordered: bool,
    /// Column widths, points, from the cells that give one and span a single column.
    widths: Vec<Option<f32>>,
}

/// Whether an element's `border` attribute or its CSS declares a visible border.
fn declares_border(attrs: &[(String, String)], css: &str, is_table: bool) -> bool {
    if is_table && attr(attrs, "border").is_some_and(|b| b.trim().parse::<u32>().map(|n| n > 0).unwrap_or(b.trim().is_empty())) {
        return true;
    }
    decls(css).any(|(name, value)| {
        if !matches!(name.as_str(), "border" | "border-top" | "border-right" | "border-bottom" | "border-left") {
            return false;
        }
        let value = value.to_ascii_lowercase();
        !value.is_empty() && !value.split_whitespace().any(|tok| matches!(tok, "none" | "hidden" | "0" | "0px" | "0pt"))
    })
}

/// A declaration block's declarations, at most [`MAX_DECLS`]: (property in lower case, value
/// without `!important`).
fn decls(css: &str) -> impl Iterator<Item = (String, &str)> {
    css.split(';').take(MAX_DECLS).filter_map(|d| {
        let (k, v) = d.split_once(':')?;
        Some((k.trim().to_ascii_lowercase(), v.trim().trim_end_matches("!important").trim()))
    })
}

/// Loads the bytes of an image an `img` names by a source other than a `data:` URI (a path
/// relative to the HTML file). `None` when it can't be found. A picture used more than once can
/// share one buffer.
pub type ImageLoader<'a> = &'a dyn Fn(&str) -> Option<Arc<Vec<u8>>>;

struct Builder<'r> {
    images: ImageLoader<'r>,
    containers: Vec<Vec<FBlock>>,
    cells: Vec<Cell>,
    tables: Vec<TableB>,
    stack: Vec<El>,
    para: Option<Para>,
    /// The current paragraph continues a list item (joins the previous paragraph).
    para_cont: bool,
    /// Whether the current paragraph's first / last text was read with preserved white space
    /// (CSS `white-space: pre…`): those edge spaces are kept when the paragraph ends.
    pre_lead: Option<bool>,
    pre_tail: bool,
    meta: Meta,
    title: Option<String>,
    /// Rules of the page's `style` elements.
    sheet: Sheet,
    /// Read as Word opens a web page ([`import_page`]), not as a pasted fragment.
    page: bool,
    /// What is left of [`MAX_CSS_WORK`].
    css_work: usize,
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
    for (k, v) in decls(css) {
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

/// The paragraph spacing a block's CSS states: `margin` (and its sides) as space before / after
/// and left / right indents, `text-indent` as the first-line indent, `line-height` as line
/// spacing (a number or percentage: a multiple; a length: exactly). `auto` and values that don't
/// parse state nothing.
fn block_spacing(css: &str) -> Spacing {
    let mut sp = Spacing::default();
    // A length with a unit, or 0: a bare number, a percentage or a keyword states nothing.
    let len = |v: &str| match v.trim() {
        "0" => Some(0.0),
        v if v.ends_with('%') || v.parse::<f32>().is_ok() || !v.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')) => None,
        v => css_len_pt(v).filter(|x| x.abs() <= 1584.0),
    };
    for (k, v) in decls(css) {
        match k.as_str() {
            "margin" => {
                let parts: Vec<Option<f32>> = v.split_whitespace().map(len).collect();
                let [top, right, bottom, left] = match parts.as_slice() {
                    [a] => [*a; 4],
                    [a, b] => [*a, *b, *a, *b],
                    [a, b, c] => [*a, *b, *c, *b],
                    [a, b, c, d] => [*a, *b, *c, *d],
                    _ => continue,
                };
                sp.before = top.map(|x| x.max(0.0)).or(sp.before);
                sp.after = bottom.map(|x| x.max(0.0)).or(sp.after);
                sp.left = left.or(sp.left);
                sp.right = right.or(sp.right);
            }
            "margin-top" => sp.before = len(v).map(|x| x.max(0.0)).or(sp.before),
            "margin-bottom" => sp.after = len(v).map(|x| x.max(0.0)).or(sp.after),
            "margin-left" => sp.left = len(v).or(sp.left),
            "margin-right" => sp.right = len(v).or(sp.right),
            "text-indent" => sp.first = len(v).or(sp.first),
            "line-height" => {
                let lv = v.to_ascii_lowercase();
                let multiple = |m: f32| (m.is_finite() && m > 0.0).then(|| LineSpacing::Multiple(m.clamp(0.25, 10.0)));
                sp.line = if let Some(pc) = lv.strip_suffix('%') {
                    pc.trim().parse::<f32>().ok().and_then(|p| multiple(p / 100.0))
                } else if let Ok(m) = lv.parse::<f32>() {
                    multiple(m)
                } else if lv == "normal" {
                    Some(LineSpacing::Multiple(1.0))
                } else {
                    len(&lv).filter(|x| *x > 0.0).map(LineSpacing::Exactly)
                }
                .or(sp.line);
            }
            _ => {}
        }
    }
    sp
}

/// A rule of a `style` element with a simple selector: an element name, classes or both.
#[derive(Clone, Debug)]
struct Rule {
    tag: Option<String>,
    classes: Vec<String>,
    /// Classes count ten, an element name one.
    specificity: usize,
    /// Shared by the selectors of one block.
    decls: Arc<str>,
    /// How many declarations `decls` holds.
    count: usize,
}

/// Most rules kept from a page's style elements.
const MAX_RULES: usize = 10_000;
/// Most declarations read for one element (its rules' and its `style` attribute's).
const MAX_DECLS: usize = 512;
/// Most declarations read from one `style` attribute.
const INLINE_DECLS: usize = 64;
/// Rule matching (rules looked at plus bytes gathered) one page may spend.
const MAX_CSS_WORK: usize = 64 << 20;
/// Most bytes of declarations gathered from the rules one element matches.
const MAX_RULE_CSS: usize = 64 * 1024;
/// Most of an element's classes looked up.
const MAX_CLASSES: usize = 32;

/// A page's style rules, found by the element name or a class they need.
#[derive(Default)]
struct Sheet {
    rules: Vec<Rule>,
    /// Rules naming classes, by their first class.
    by_class: std::collections::HashMap<String, Vec<usize>>,
    /// Rules naming only an element, by its name.
    by_tag: std::collections::HashMap<String, Vec<usize>>,
    /// `*` rules.
    any: Vec<usize>,
}

impl Sheet {
    fn push(&mut self, r: Rule) {
        let i = self.rules.len();
        match (r.classes.first(), &r.tag) {
            (Some(c), _) => self.by_class.entry(c.clone()).or_default().push(i),
            (None, Some(t)) => self.by_tag.entry(t.clone()).or_default().push(i),
            (None, None) => self.any.push(i),
        }
        self.rules.push(r);
    }
}

/// Read the rules of a style sheet into `out`. At-rules (`@media`, `@page`…) and rules whose
/// selectors aren't simple (descendants, pseudo-classes, attributes…) are skipped.
fn parse_sheet(css: &str, out: &mut Sheet) {
    // Comments go, as do the `<!-- -->` old pages hide their styles in.
    let mut text = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        text.push_str(rest.get(..i).unwrap_or(""));
        rest = rest.get(i + 2..).and_then(|r| r.find("*/").and_then(|j| r.get(j + 2..))).unwrap_or("");
    }
    text.push_str(rest);
    let text = text.replace("<!--", " ").replace("-->", " ");
    let mut rest = text.as_str();
    while let Some(open) = rest.find('{') {
        // A statement at-rule before it (`@charset "x";`, `@import url(a);`) isn't a selector.
        let selectors = rest.get(..open).unwrap_or("").rsplit(';').next().unwrap_or("").trim();
        let body = rest.get(open + 1..).unwrap_or("");
        // The block's end, counting nested braces (an at-rule's own rules).
        let mut depth = 1usize;
        let mut end = body.len();
        for (i, c) in body.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let decls = body.get(..end).unwrap_or("");
        rest = body.get(end + 1..).unwrap_or("");
        if selectors.starts_with('@') || decls.contains('{') || decls.len() > MAX_RULE_CSS {
            continue;
        }
        let (count, decls) = (decls.split(';').count(), Arc::<str>::from(decls));
        for sel in selectors.split(',') {
            if out.rules.len() >= MAX_RULES {
                return;
            }
            if let Some((tag, classes)) = simple_selector(sel.trim()) {
                let specificity = classes.len() * 10 + usize::from(tag.is_some());
                out.push(Rule { tag, classes, specificity, decls: decls.clone(), count });
            }
        }
    }
}

/// `p`, `.note`, `p.note.wide` or `*`: (element name, classes). Class names may start with a
/// digit (`.000003`): browsers drop such a rule, but Word's HTML import applies it, and
/// converters write them.
fn simple_selector(sel: &str) -> Option<(Option<String>, Vec<String>)> {
    if sel == "*" {
        return Some((None, Vec::new()));
    }
    let mut parts = sel.split('.');
    let tag = parts.next()?;
    let tag = if tag.is_empty() {
        None
    } else if tag.chars().all(|c| c.is_ascii_alphanumeric()) && tag.starts_with(|c: char| c.is_ascii_alphabetic()) {
        Some(tag.to_ascii_lowercase())
    } else {
        return None;
    };
    let ident = |c: &str| !c.is_empty() && c.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-');
    let classes: Vec<String> = parts.map(str::to_string).collect();
    if classes.iter().any(|c| !ident(c)) || (tag.is_none() && classes.is_empty()) {
        return None;
    }
    Some((tag, classes))
}

impl Builder<'_> {
    /// An element's CSS: the declarations of the style sheet rules it matches, in cascade order,
    /// then its `style` attribute (later declarations win).
    fn css_of(&mut self, name: &str, attrs: &[(String, String)]) -> String {
        // The `style` attribute's first declarations always fit in what is read of the result.
        let inline: Vec<&str> = attr(attrs, "style").unwrap_or("").split(';').take(INLINE_DECLS).collect();
        let inline = inline.join(";");
        let sheet = &self.sheet;
        if sheet.rules.is_empty() || self.css_work == 0 {
            return inline;
        }
        let mut classes: Vec<&str> = attr(attrs, "class").map(|c| c.split_whitespace().take(MAX_CLASSES).collect()).unwrap_or_default();
        classes.sort_unstable();
        classes.dedup();
        let mut candidates: Vec<usize> = sheet.any.clone();
        candidates.extend(sheet.by_tag.get(name).into_iter().flatten());
        for c in &classes {
            candidates.extend(sheet.by_class.get(*c).into_iter().flatten());
        }
        let mut hits: Vec<(usize, usize)> = candidates
            .iter()
            .filter_map(|&i| sheet.rules.get(i).map(|r| (i, r)))
            .filter(|(_, r)| r.tag.as_deref().is_none_or(|t| t == name) && r.classes.iter().all(|c| classes.contains(&c.as_str())))
            .map(|(i, r)| (r.specificity, i))
            .collect();
        hits.sort_unstable();
        let mut css = String::new();
        let mut count = 0;
        for (_, i) in hits {
            let Some(r) = sheet.rules.get(i) else { continue };
            if css.len() + r.decls.len() > MAX_RULE_CSS || count + r.count > MAX_DECLS - INLINE_DECLS {
                break;
            }
            css.push_str(&r.decls);
            css.push(';');
            count += r.count;
        }
        // A page whose rules cost too much to apply reads on without them.
        self.css_work = self.css_work.saturating_sub(candidates.len() + css.len());
        css.push_str(&inline);
        css
    }
}

/// The borders a cell's CSS gives it (`border`, `border-top`…: style, width and colour in any
/// order; `windowtext` is the automatic colour). `None` when it states none.
fn cell_borders(css: &str) -> Option<wordcraft_doc::props::Borders> {
    use wordcraft_doc::props::{Border, BorderStyle, Borders};
    let mut b = Borders::default();
    let mut any = false;
    for (k, v) in decls(css) {
        let sides: &[u8] = match k.as_str() {
            "border" => b"tlbr",
            "border-top" => b"t",
            "border-left" => b"l",
            "border-bottom" => b"b",
            "border-right" => b"r",
            _ => continue,
        };
        // CSS's default width is `medium`.
        let (mut style, mut width, mut color) = (None, 2.25_f32, None);
        for tok in v.split_whitespace() {
            let lt = tok.to_ascii_lowercase();
            match lt.as_str() {
                "none" | "hidden" => style = Some(BorderStyle::None),
                "solid" => style = Some(BorderStyle::Single),
                "double" => style = Some(BorderStyle::Double),
                "dotted" => style = Some(BorderStyle::Dotted),
                "dashed" => style = Some(BorderStyle::Dashed),
                "thin" => width = 0.75,
                "medium" => width = 2.25,
                "thick" => width = 3.75,
                "windowtext" => {}
                _ => match css_len_pt(tok).filter(|_| tok.starts_with(|c: char| c.is_ascii_digit() || c == '.')) {
                    Some(w) => width = w.clamp(0.0, 12.0),
                    None => color = parse_color(tok).or(color),
                },
            }
        }
        // CSS draws no border without a style.
        let border = Border { style: style.unwrap_or(BorderStyle::None), width, color, space: 0.0 };
        for side in sides {
            let slot = match side {
                b't' => &mut b.top,
                b'l' => &mut b.left,
                b'b' => &mut b.bottom,
                _ => &mut b.right,
            };
            *slot = Some(border);
        }
        any = true;
    }
    any.then_some(b)
}

/// Most columns given widths in one table.
const MAX_TABLE_COLS: usize = 63;

/// A cell's width, points: its CSS `width`, else its `width` attribute (pixels); percentages
/// and widths that don't parse give none.
fn cell_width(css: &str, attrs: &[(String, String)]) -> Option<f32> {
    let from_css = decls(css).filter(|(k, v)| k == "width" && !v.ends_with('%')).filter_map(|(_, v)| css_len_pt(v));
    let from_attr = || {
        attr(attrs, "width")
            .map(str::trim)
            .filter(|v| !v.ends_with('%'))
            .and_then(|v| v.trim_end_matches("px").trim().parse::<f32>().ok())
            .map(|px| px * 0.75)
    };
    from_css.last().or_else(from_attr).filter(|w| w.is_finite() && *w > 1.0 && *w <= 1584.0)
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

impl<'r> Builder<'r> {
    fn new(images: ImageLoader<'r>, page: bool) -> Builder<'r> {
        Builder {
            images,
            containers: vec![Vec::new()],
            cells: Vec::new(),
            tables: Vec::new(),
            stack: Vec::new(),
            para: None,
            para_cont: false,
            pre_lead: None,
            pre_tail: false,
            meta: Meta::default(),
            title: None,
            sheet: Sheet::default(),
            page,
            css_work: MAX_CSS_WORK,
        }
    }

    fn fmt(&self) -> Fmt {
        match self.stack.last() {
            Some(e) => e.fmt.clone(),
            // A web page's text is Times New Roman 12 pt unless its CSS says otherwise.
            None if self.page => Fmt { font: Some(PAGE_FONT.into()), size: Some(12.0), ..Fmt::default() },
            None => Fmt::default(),
        }
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
            let mut spaced = false;
            // A page's first span stating margins (see below).
            let span = self
                .stack
                .iter()
                .rev()
                .take_while(|e| e.kind == ElKind::Inline)
                .find(|e| e.spacing.before.is_some() || e.spacing.after.is_some())
                .map(|e| (e.spacing.before, e.spacing.after));
            for (i, e) in self.stack.iter().enumerate().rev() {
                match e.kind {
                    ElKind::Cell => {
                        // CSS inherits `text-align` from the cell, its row, row group and table
                        // (#1381); the paragraph's own (found above) wins.
                        if p.align.is_none() {
                            p.align = self.stack.get(..=i).and_then(|outer| {
                                outer
                                    .iter()
                                    .rev()
                                    .take_while(|o| o.kind != ElKind::Table)
                                    .chain(outer.iter().rev().find(|o| o.kind == ElKind::Table))
                                    .find_map(|o| o.align)
                            });
                        }
                        break;
                    }
                    ElKind::Table => break,
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
                // Margins belong to the paragraph's own block, not to those around it.
                if !spaced && e.kind != ElKind::Inline {
                    p.spacing = e.spacing;
                    if self.page {
                        // Word opens a page's paragraphs with single lines; a margin a span
                        // states replaces the one its paragraph states, and one the paragraph
                        // leaves out is automatic spacing.
                        let (top, bottom) = span.unwrap_or_default();
                        let sp = &mut p.spacing;
                        sp.before_auto = sp.before.is_none() && e.auto_margins;
                        sp.after_auto = sp.after.is_none() && e.auto_margins;
                        sp.before = sp.before.map(|own| top.unwrap_or(own));
                        sp.after = sp.after.map(|own| bottom.unwrap_or(own));
                        sp.line = sp.line.or(Some(LineSpacing::Multiple(1.0)));
                    }
                    spaced = true;
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
        let (pre_lead, pre_tail) = (self.pre_lead.take().unwrap_or(false), std::mem::take(&mut self.pre_tail));
        if p.kind != Kind::Code {
            // Preserved white space keeps its spaces and tabs at the edges; line breaks there
            // (source formatting) still go.
            const ALL: &[char] = &[' ', '\t', '\r', '\n'];
            const BREAKS: &[char] = &['\r', '\n'];
            p.trim_with(if pre_lead { BREAKS } else { ALL }, if pre_tail { BREAKS } else { ALL });
            if p.text().trim().is_empty() {
                // Only white space (indentation between blocks): nothing to keep.
                p.trim();
            }
        }
        if p.inlines.len() == 1 && p.text() == "\u{00A0}" {
            // `<p>&nbsp;</p>`: an empty paragraph on purpose, whose line is as high as the
            // space's formatting makes it.
            if let Some(Inline::Text(_, f)) = p.inlines.first() {
                p.mark = Some(Box::new(f.clone()));
            }
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
            self.note_edge(true);
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
        self.note_edge(false);
        self.start_para().push_text(&s, &f);
    }

    /// Record whether text about to be added keeps its white space (see `pre_lead`).
    fn note_edge(&mut self, preserved: bool) {
        if self.para.as_ref().is_none_or(|p| p.inlines.is_empty()) {
            self.pre_lead = Some(preserved);
        }
        self.pre_tail = preserved;
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
                    let borderless = !t.bordered;
                    // Keep at most MAX_ROWS rows (the model drops the rest anyway): bounds the
                    // O(rows × rowspan) work of `insert_covered` on a table of millions of `<tr>`.
                    let rows: Vec<_> = t.rows.into_iter().filter(|r| !r.is_empty()).take(wordcraft_doc::table::MAX_ROWS).collect();
                    let mut ft = FTable { rows, widths: Vec::new(), borderless };
                    // Columns keep the widths their cells give when every column has one.
                    if t.widths.len() == ft.cols() {
                        ft.widths = t.widths.iter().copied().collect::<Option<Vec<f32>>>().unwrap_or_default();
                    }
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

    fn close_head_for_start(&mut self, name: &str) {
        // A body-content start tag implicitly ends an omitted </head>.
        if !matches!(
            name,
            "html" | "head" | "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "noscript" | "script" | "style" | "template" | "title"
        ) && let Some(k) = self.find_open(|e| e.kind == ElKind::Head, |_| false)
        {
            self.pop_to(k);
        }
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
                    .and_then(|src| if src.trim_start().starts_with("data:") { data_uri(src).map(Arc::new) } else { (self.images)(src.trim()) })
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
            // A header cell is bold, as in browsers and Word, unless its own CSS says otherwise.
            "b" | "strong" | "th" => fmt.bold = true,
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
                if let Some(h) = attr(attrs, "href").filter(|h| wordcraft_doc::link::is_safe_href(h)) {
                    fmt.link = Some(h.trim().to_string());
                }
            }
            _ => {}
        }
        // `align` on a table is about where the table sits (not imported); it doesn't align the
        // text in its cells.
        if let Some(a) = attr(attrs, "align").filter(|_| kind != ElKind::Table) {
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
        if self.page
            && let ElKind::Heading(n) = kind
        {
            // A page's headings are bold, black and sized as browsers show them.
            fmt.bold = true;
            fmt.color = Some(wordcraft_doc::Rgb(0, 0, 0));
            fmt.size = PAGE_HEADING_SIZES.get(usize::from(n.clamp(1, 6)) - 1).copied();
        }
        let css = self.css_of(name, attrs);
        apply_style(&css, &mut fmt, &mut align, &mut preserve, &mut page_break);
        if kind == ElKind::Pre {
            preserve = true;
        }
        match kind {
            ElKind::Table => {
                self.tables.push(TableB { rows: Vec::new(), row: None, bordered: declares_border(attrs, &css, true), widths: Vec::new() })
            }
            ElKind::Tr => {
                if let Some(t) = self.tables.last_mut()
                    && let Some(r) = t.row.take()
                {
                    t.rows.push(r);
                }
            }
            ElKind::Cell => {
                let tb = self.find_open(|e| e.kind == ElKind::Tr, |e| e.kind == ElKind::Table).is_none();
                if let Some(t) = self.tables.last_mut()
                    && declares_border(attrs, &css, false)
                {
                    t.bordered = true;
                }
                if tb && let Some(t) = self.tables.last_mut() {
                    // A cell without a row: start one.
                    if let Some(r) = t.row.take() {
                        t.rows.push(r);
                    }
                    t.row = Some(Vec::new());
                }
                let num = |k: &str| attr(attrs, k).and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.clamp(1, 63));
                let colspan = num("colspan").unwrap_or(1);
                if colspan == 1
                    && let Some(w) = cell_width(&css, attrs)
                    && let Some(t) = self.tables.last_mut()
                {
                    // shortcut: columns covered by a rowspan from a row above aren't counted; count them if tables with spanned rows lose widths.
                    let col = t.row.iter().flatten().map(|c| c.colspan.clamp(1, 63) as usize).sum::<usize>();
                    if col < MAX_TABLE_COLS {
                        if t.widths.len() <= col {
                            t.widths.resize(col + 1, None);
                        }
                        if let Some(slot) = t.widths.get_mut(col) {
                            slot.get_or_insert(w);
                        }
                    }
                }
                let shading = attr(attrs, "bgcolor").and_then(parse_color).or(fmt.background);
                self.cells.push(Cell {
                    blocks: Vec::new(),
                    colspan,
                    rowspan: attr(attrs, "rowspan").and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.clamp(1, 1000)).unwrap_or(1),
                    covered: false,
                    header: name == "th",
                    shading,
                    borders: cell_borders(&css),
                });
                fmt.background = None;
                self.containers.push(Vec::new());
            }
            _ => {}
        }
        // The page's own margins (`body`, `html`) aren't its paragraphs'.
        let spacing =
            if (kind != ElKind::Inline || self.page) && !matches!(name, "body" | "html") { block_spacing(&css) } else { Spacing::default() };
        let auto_margins = self.page && (name == "p" || matches!(kind, ElKind::Heading(_)));
        self.stack.push(El { name: name.to_string(), kind, fmt, spacing, auto_margins, align, preserve, page_break, used: false });
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
    parse_in(s, images, false)
}

fn parse_in(s: &str, images: ImageLoader, page: bool) -> Flow {
    let mut lx = Lexer { s, pos: 0 };
    let mut b = Builder::new(images, page);
    let mut guard = 0usize;
    while let Some(t) = lx.next() {
        guard += 1;
        if guard > 50_000_000 {
            break;
        }
        match t {
            Tok::Text(t) => b.text(&t),
            Tok::Start { name, attrs, self_close } => {
                b.close_head_for_start(&name);
                if crate::mathml::local(&name) == "math" && !self_close {
                    let el = read_math(&mut lx, name, attrs);
                    if !b.in_head()
                        && let Some((linear, display)) = crate::mathml::read(&el)
                    {
                        b.start_para().inlines.push(Inline::Equation { linear, display });
                    }
                    continue;
                }
                if matches!(name.as_str(), "script" | "style" | "noscript" | "template" | "textarea") && !self_close {
                    let from = lx.pos;
                    let to = lx.skip_past(&format!("</{name}"));
                    if name == "style" {
                        parse_sheet(s.get(from..to).unwrap_or(""), &mut b.sheet);
                    }
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

/// A `<math>` element (#816): its tokens up to the matching end tag, as a tree. Nesting and size
/// are bounded; an unclosed element ends with the input.
fn read_math(lx: &mut Lexer, name: String, attrs: Vec<(String, String)>) -> crate::mathml::El {
    use crate::mathml::{El, Item, MAX_NODES, local};
    let mut stack: Vec<El> = vec![El { name, attrs, children: Vec::new() }];
    let mut count = 0usize;
    let close = |stack: &mut Vec<El>| {
        if stack.len() > 1
            && let Some(done) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            parent.children.push(Item::El(done));
        }
    };
    while let Some(t) = lx.next() {
        count += 1;
        if count > MAX_NODES * 4 {
            // Over the cap: drop the rest of this element so its tokens aren't read as HTML text.
            skip_math(lx, t);
            break;
        }
        match t {
            Tok::Text(t) => {
                if let Some(top) = stack.last_mut()
                    && !t.is_empty()
                {
                    top.children.push(Item::Text(t));
                }
            }
            Tok::Start { name, attrs, self_close } => {
                let el = El { name, attrs, children: Vec::new() };
                if self_close || stack.len() > wordcraft_doc::math::MAX_DEPTH {
                    if let Some(top) = stack.last_mut() {
                        top.children.push(Item::El(el));
                    }
                } else {
                    stack.push(el);
                }
            }
            Tok::End(name) => {
                let name = local(&name).to_string();
                // Close up to the matching open element; a stray end tag is ignored.
                if let Some(k) = stack.iter().rposition(|e| local(&e.name) == name) {
                    if k == 0 {
                        break;
                    }
                    while stack.len() > k {
                        close(&mut stack);
                    }
                }
            }
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().unwrap_or_default()
}

/// Skip tokens (starting with `first`) up to and including the `</math>` that closes the element
/// being read; nested `<math>` elements are counted. An unclosed element ends with the input.
fn skip_math(lx: &mut Lexer, first: Tok) {
    use crate::mathml::local;
    let mut depth = 0usize;
    let mut t = Some(first);
    while let Some(tok) = t {
        match tok {
            Tok::Start { name, self_close: false, .. } if local(&name) == "math" => depth = depth.saturating_add(1),
            Tok::End(name) if local(&name) == "math" => {
                if depth == 0 {
                    return;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        t = lx.next();
    }
}

/// Parse HTML into a document.
pub fn import(s: &str) -> Document {
    model::to_doc(&parse(s))
}

/// [`import`], loading pictures that aren't `data:` URIs through `images`.
pub fn import_with(s: &str, images: ImageLoader) -> Document {
    model::to_doc(&parse_with(s, images))
}

/// A web page opened as a document, as Word opens one (a file, or a document's HTML
/// `w:altChunk`): its text is Times New Roman 12 pt on single lines, headings bold and black at
/// the sizes browsers give them, and a `p` or heading whose CSS leaves a margin out (or writes it
/// invalidly) gets HTML's automatic spacing there; a margin a span states replaces the one its
/// paragraph states (both as Word's import does). [`import`] reads a pasted fragment, which takes
/// the document's own formatting instead (but its CSS still applies).
pub fn import_page(s: &str, images: ImageLoader) -> Document {
    model::to_doc(&parse_in(s, images, true))
}

/// [`import_page`] onto `page` (its size and margins), which tables size to: HTML gives no page
/// size, so the caller picks it (the region's paper, #1381).
pub fn import_on(s: &str, images: ImageLoader, page: &wordcraft_doc::SectionProps) -> Document {
    model::to_doc_in(&parse_in(s, images, true), page.clone())
}

/// The font of a web page's text when its CSS names none.
pub const PAGE_FONT: &str = "Times New Roman";
/// Heading sizes on a web page, `h1`–`h6`, points.
const PAGE_HEADING_SIZES: [f32; 6] = [24.0, 18.0, 13.5, 12.0, 10.0, 7.5];

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
    for i in inlines.iter() {
        match i {
            Inline::Text(t, f) => {
                let mut open = String::new();
                let mut close: Vec<&str> = Vec::new();
                // A script or other non-web target is written as plain text (`is_safe_href`).
                if let Some(l) = f.link.as_deref().filter(|l| wordcraft_doc::link::is_safe_href(l)) {
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
            // No picture to show: the browser shows the alt text in its place.
            Inline::Figure(alt) => o.push_str(&format!("<img alt=\"{}\">", esc(alt))),
            // MathML, which browsers, Word and LibreOffice show as an equation (#816).
            Inline::Equation { linear, display } => o.push_str(&crate::mathml::linear_to_mathml(linear, *display)),
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
            let mut css = vec!["padding:4px 6px".to_string(), "vertical-align:top".to_string()];
            if !t.borderless {
                css.insert(0, "border:1px solid #999".to_string());
            }
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

/// `doc`'s content as an HTML fragment (no head or style sheet): what a copy puts on the system
/// clipboard beside its text, equations as MathML (#816).
pub fn export_fragment(doc: &Document) -> String {
    let flow = model::from_doc(doc);
    let mut out = String::from("<meta charset=\"utf-8\">");
    blocks_html(&flow.blocks, &mut out, 0);
    out
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

    fn table_style(html: &str) -> Option<String> {
        let doc = crate::model::to_doc(&parse(html));
        doc.body.iter().find_map(|b| match &**b {
            wordcraft_doc::Block::Table(t) => Some(t.props.style.clone().unwrap_or_default()),
            _ => None,
        })
    }

    #[test]
    fn borderless_tables_get_no_grid_style() {
        assert_eq!(table_style("<table><tr><td>a<td>b</table>").as_deref(), Some(""));
        assert_eq!(table_style("<table style='border:none'><tr><td style='border:0'>a</table>").as_deref(), Some(""));
        assert_eq!(table_style("<table border=0><tr><td style='border: 0px none'>a</table>").as_deref(), Some(""));
        assert_eq!(table_style("<table border=1><tr><td>a</table>").as_deref(), Some("TableGrid"));
        assert_eq!(table_style("<table style='border:1px solid #000'><tr><td>a</table>").as_deref(), Some("TableGrid"));
        assert_eq!(table_style("<table><tr><td style='border:1px solid #999'>a</table>").as_deref(), Some("TableGrid"));
    }

    #[test]
    fn borderless_tables_export_without_cell_borders() {
        // #438: a table without grid style and borders writes no cell borders and reopens borderless.
        for (src, grid) in [
            ("<table style='border:none'><tr><td>A<td>B</table>", false),
            ("<table><tr><td>A<td>B</table>", false),
            ("<table border=1><tr><td>A<td>B</table>", true),
        ] {
            let out = export(&crate::model::to_doc(&parse(src)));
            assert_eq!(out.contains("border:1px"), grid, "{src}: {out}");
            assert_eq!(table_style(&out).as_deref(), Some(if grid { "TableGrid" } else { "" }), "{src}");
            assert!(out.contains(">A</td>") && out.contains(">B</td>"), "{out}");
        }
    }

    #[test]
    fn css_preformatted_paragraphs_keep_edge_spaces() {
        // #439: `white-space:pre` keeps leading and trailing spaces like `<pre>`; normal
        // paragraphs still collapse.
        let f = parse(
            "<p style=\"white-space:pre\">  Alpha  Beta  </p><p style=\"white-space:pre-wrap\">\n  Gamma  \n</p><pre>  Alpha  </pre><p>  Alpha  Beta  </p>",
        );
        assert_eq!(texts(&f), vec!["NormalNone:  Alpha  Beta  ", "NormalNone:  Gamma  ", "CodeNone:  Alpha  ", "NormalNone:Alpha Beta"]);
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
    fn omitted_head_keeps_body_text() {
        let f = parse("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>T</title>\n<body><p>Hello world</p><p>Second</p></body></html>");
        assert_eq!(f.meta.title, "T");
        assert_eq!(texts(&f), vec!["NormalNone:Hello world", "NormalNone:Second"]);

        for body in [
            "<p>Hello world</p>",
            "<div>Hello world</div>",
            "<span>Hello world</span>",
            "<a href='https://example.test'>Hello world</a>",
            "<custom>Hello world</custom>",
            "<textarea>ignored</textarea>Hello world",
        ] {
            let f = parse(&format!(
                "<html><head><title>T &amp; U</title><meta name='author' content='Ann'><style>p{{}}</style><script>ignored</script>{body}",
            ));
            assert_eq!(f.meta.title, "T & U");
            assert_eq!(f.meta.author, "Ann");
            assert_eq!(texts(&f), vec!["NormalNone:Hello world"], "{body}");
        }
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

    /// The paragraphs of every cell of the first table, row by row.
    fn cell_paras(f: &Flow) -> Vec<Vec<Vec<Para>>> {
        let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
        t.rows
            .iter()
            .map(|r| {
                r.iter().map(|c| c.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.clone()) } else { None }).collect()).collect()
            })
            .collect()
    }

    fn all_bold(p: &Para) -> bool {
        !p.inlines.is_empty() && p.inlines.iter().all(|i| matches!(i, Inline::Text(_, f) if f.bold))
    }

    fn any_bold(p: &Para) -> bool {
        p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) if f.bold))
    }

    #[test]
    fn header_cells_are_bold_unless_their_css_says_otherwise() {
        // #1381: a <th> is bold, as in browsers and Word; `font-weight:normal` on it or inside it wins.
        let f = parse(
            "<table><thead><tr><th>Head</th><th style='font-weight:normal'>Plain</th><th><span style='font-weight:400'>Inner</span></th></tr></thead><tbody><tr><td>a<td>b<td>c</tbody></table>",
        );
        let rows = cell_paras(&f);
        assert!(all_bold(&rows[0][0][0]), "{:?}", rows[0][0]);
        assert!(!any_bold(&rows[0][1][0]), "{:?}", rows[0][1]);
        assert!(!any_bold(&rows[0][2][0]), "{:?}", rows[0][2]);
        assert!(!any_bold(&rows[1][0][0]), "td stays regular");
    }

    #[test]
    fn cell_text_align_reaches_the_cell_paragraphs() {
        // #1381: `text-align` on a td/th, or inherited from its tr/thead/tbody/table, aligns the
        // cell's paragraphs; a paragraph's own text-align inside the cell wins.
        let f = parse(
            "<table style='text-align:center'><tbody style='text-align:justify'><tr style='text-align:right'>\
             <td style='text-align:right;'>r</td><td>inherited</td><td style='text-align:right'><p style='text-align:center'>own</p></td>\
             <td align=right>attr</td></tr><tr><td>body</td></tr></tbody><tr><td>table</td></tr></table>",
        );
        let rows = cell_paras(&f);
        assert_eq!(rows[0][0][0].align, Some(Align::Right), "td style");
        assert_eq!(rows[0][1][0].align, Some(Align::Right), "from tr");
        assert_eq!(rows[0][2][0].align, Some(Align::Center), "own paragraph wins");
        assert_eq!(rows[0][3][0].align, Some(Align::Right), "align attribute");
        assert_eq!(rows[1][0][0].align, Some(Align::Justify), "from tbody");
        assert_eq!(rows[2][0][0].align, Some(Align::Center), "from table");
        // `align` on a table places the table; it doesn't align the text in its cells.
        let f = parse("<table align=center><tr><td>x</td></tr></table>");
        assert_eq!(cell_paras(&f)[0][0][0].align, None);
    }

    #[test]
    fn reporter_sample_header_bold_and_right_aligned_cells() {
        // #1381: the reporter's table.
        let f = parse(
            "<table border=\"1\" style=\"width:100%;border-collapse:collapse;\">\n<thead><tr style=\"background-color:#e8f0fa;\"><th>Indicatore</th><th>Valore atteso</th></tr></thead>\n<tbody>\n<tr><td>Audit</td><td style=\"text-align:right;\">4</td></tr>\n</tbody></table>",
        );
        let rows = cell_paras(&f);
        assert!(all_bold(&rows[0][0][0]) && all_bold(&rows[0][1][0]), "{:?}", rows[0]);
        assert_eq!(rows[1][1][0].align, Some(Align::Right));
        assert_eq!(rows[1][0][0].align, None);
    }

    #[test]
    fn entities() {
        assert_eq!(unescape("a &lt;b&gt; &#65;&#x42; &bogus; &"), "a <b> AB &bogus; &");
    }

    fn paras(f: &Flow) -> Vec<&Para> {
        f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
    }
    fn fmt_of(p: &Para) -> Fmt {
        p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap_or_default()
    }

    #[test]
    fn style_sheet_rules_apply_in_cascade_order() {
        let f = parse(
            "<style><!-- p { color: #00ff00; font-size: 10pt } .big { font-size: 20pt } p.big { font-weight: bold } \
             /* a comment { } */ @media print { p { color: #ff0000 } } div p, li:hover { font-style: italic } .000003 { text-align: center } --></style>\
             <p>plain</p><p class='big x'>big</p><p class=big style='font-size: 8pt'>inline wins</p><p class=000003>digits</p>",
        );
        let ps = paras(&f);
        let (plain, big, inline, digits) = (fmt_of(ps[0]), fmt_of(ps[1]), fmt_of(ps[2]), fmt_of(ps[3]));
        assert_eq!((plain.color, plain.size, plain.bold, plain.italic), (Some(wordcraft_doc::Rgb(0, 255, 0)), Some(10.0), false, false));
        assert_eq!((big.size, big.bold), (Some(20.0), true));
        assert_eq!(inline.size, Some(8.0));
        assert_eq!(ps[3].align, Some(Align::Center));
        assert_eq!(digits.size, Some(10.0));
    }

    #[test]
    fn paragraph_css_states_spacing_indents_and_line_height() {
        let f = parse(
            "<style>p.a { margin: 6pt 1in 3pt; text-indent: -18pt; line-height: 150% }</style>\
             <p class=a>a</p><p style='margin-top: 14,4px; margin-bottom: auto; line-height: 20pt'>b</p><p>c</p>",
        );
        let ps = paras(&f);
        assert_eq!(
            ps[0].spacing,
            Spacing {
                before: Some(6.0),
                after: Some(3.0),
                left: Some(72.0),
                right: Some(72.0),
                first: Some(-18.0),
                line: Some(LineSpacing::Multiple(1.5)),
                ..Spacing::default()
            }
        );
        // Values that don't parse (a comma decimal, `auto`) state nothing.
        assert_eq!(ps[1].spacing, Spacing { line: Some(LineSpacing::Exactly(20.0)), ..Spacing::default() });
        assert_eq!(ps[2].spacing, Spacing::default());
    }

    #[test]
    fn a_page_opens_as_word_opens_one() {
        let html = "<style>p.k { margin-bottom: 13px } span.z { margin: 0in }</style>\
                    <p>first</p><h1>Head</h1><p class=k><span class=z>kept</span></p><p class=k>own</p><div>bare</div><p>&nbsp;</p>";
        let f = parse_in(html, &|_| None, true);
        let ps = paras(&f);
        // Times New Roman 12 pt on single lines; headings bold and black at a browser's sizes.
        let first = fmt_of(ps[0]);
        assert_eq!((first.font.as_deref(), first.size), (Some(PAGE_FONT), Some(12.0)));
        assert_eq!(ps[0].spacing.line, Some(LineSpacing::Multiple(1.0)));
        let head = fmt_of(ps[1]);
        assert_eq!((head.bold, head.size, head.color), (true, Some(24.0), Some(wordcraft_doc::Rgb(0, 0, 0))));
        // Automatic spacing where a p or heading leaves its margins out (layout drops it at the
        // top of the page and of a cell).
        let auto = |p: &Para| (p.spacing.before_auto, p.spacing.after_auto, p.spacing.before, p.spacing.after);
        assert_eq!(auto(ps[0]), (true, true, None, None));
        assert_eq!(auto(ps[1]), (true, true, None, None));
        // A span's margin replaces the one its paragraph states; the one it leaves out stays automatic.
        assert_eq!(auto(ps[2]), (true, false, None, Some(0.0)));
        assert_eq!(auto(ps[3]), (true, false, None, Some(9.75)));
        // Other blocks get no automatic spacing.
        assert_eq!(auto(ps[4]), (false, false, None, None));
        // The document says so, and the empty paragraph's mark keeps its size.
        let d = model::to_doc(&f);
        let props = |i: usize| match &*d.body[i] {
            wordcraft_doc::Block::Para(p) => p.clone(),
            b => panic!("{b:?}"),
        };
        assert_eq!((props(0).props.space_before_auto, props(0).props.space_after_auto), (Some(true), Some(true)));
        assert_eq!((props(2).props.space_after_auto, props(2).props.space_after), (None, Some(0.0)));
        assert_eq!(props(5).mark.size, Some(12.0));
        // `<p>&nbsp;</p>` keeps the size of its space for its line.
        assert!(ps[5].inlines.is_empty());
        assert_eq!(ps[5].mark.as_ref().and_then(|m| m.size), Some(12.0));
        // A fragment (a paste) keeps the document's own formatting.
        let pasted = parse(html);
        assert_eq!(fmt_of(paras(&pasted)[0]).font, None);
        assert_eq!(paras(&pasted)[0].spacing, Spacing::default());
    }

    #[test]
    fn cells_give_their_columns_widths() {
        let f = parse(
            "<style>td.n { width: 42px }</style><table border=1><tr><td colspan=3>t</td></tr><tr><td class=n>1</td><td width=200>a</td><td style='width: 2in'>b</td></tr></table>",
        );
        let FBlock::Table(t) = &f.blocks[0] else { panic!("{:?}", f.blocks) };
        assert_eq!(t.widths, [31.5, 150.0, 144.0]);
        // A column without a width leaves them all to the layout.
        let f = parse("<table><tr><td width=100>a</td><td>b</td></tr></table>");
        let FBlock::Table(t) = &f.blocks[0] else { panic!() };
        assert!(t.widths.is_empty());
    }

    #[test]
    fn cells_take_their_css_borders() {
        use wordcraft_doc::props::{Border, BorderStyle};
        let f = parse(
            "<style>td.b { border-top: solid windowtext 1px; border-bottom: 2pt double #ff0000; border-left: none }</style><table><tr><td class=b>a</td><td>b</td></tr></table>",
        );
        let FBlock::Table(t) = &f.blocks[0] else { panic!() };
        let b = t.rows[0][0].borders.expect("borders");
        assert_eq!(b.top, Some(Border::single(0.75)));
        assert_eq!(b.bottom, Some(Border { style: BorderStyle::Double, width: 2.0, color: Some(wordcraft_doc::Rgb(255, 0, 0)), space: 0.0 }));
        assert_eq!(b.left.map(|l| l.style), Some(BorderStyle::None));
        assert_eq!(b.right, None);
        assert_eq!(t.rows[0][1].borders, None);
    }

    #[test]
    fn style_sheet_edge_cases() {
        let f = parse_in(
            "<style>@charset \"utf-8\"; @import url(a.css); p.a { color: #0000ff } body { margin: 20px } p.b { margin: 10 5% medium }</style>\
             <body><p class=a>a</p>bare<p class=b>b</p></body>",
            &|_| None,
            true,
        );
        let ps = paras(&f);
        // A rule right after statement at-rules still applies.
        assert_eq!(fmt_of(ps[0]).color, Some(wordcraft_doc::Rgb(0, 0, 255)));
        // The body's margins aren't the paragraphs' indents.
        assert_eq!((ps[1].spacing.left, ps[1].spacing.before), (None, None));
        // Bare numbers, percentages and keywords state no margin: automatic spacing stays.
        assert_eq!((ps[2].spacing.before_auto, ps[2].spacing.left), (true, None));
        // The `style` attribute wins even over a sheet with more declarations than are read.
        let many = (0..600).map(|i| format!("x-{i}: 1")).collect::<Vec<_>>().join(";");
        let f = parse(&format!("<style>p {{ {many}; color: #ff0000 }}</style><p style='color: #00ff00'>c</p>"));
        assert_eq!(fmt_of(paras(&f)[0]).color, Some(wordcraft_doc::Rgb(0, 255, 0)));
    }

    #[test]
    fn hostile_style_sheets_never_panic() {
        // One declaration block shared by many selectors isn't copied for each.
        let selectors = vec!["p"; 20_000].join(",");
        let huge = format!("<style>{selectors} {{ color: red; {} }}</style><p>x</p>", "x: y;".repeat(200_000));
        let _ = parse_in(&huge, &|_| None, true);
        let many = ".a { color: red } ".repeat(20_000);
        for css in ["{", "}", "p {", "@media { p { x } ", "p { color: red", "/* open", ".{}", "p..a{}", "\u{0}.é{x:y}", many.as_str()] {
            let _ = parse_in(&format!("<style>{css}</style><p class='a é'>x</p><td style='width: 1e40px'>y</td>"), &|_| None, true);
        }
    }
}
