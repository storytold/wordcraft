use super::builder::{Builder, ImageLoader, parse_sheet};
use super::lexer::{Lexer, Tok};
use super::math_ruby::{read_math, read_ruby};
use super::{Document, Flow, Inline, model};

/// Parse HTML text into the flow model.
pub fn parse(s: &str) -> Flow {
    parse_with(s, &|_| None)
}

/// [`parse`], loading pictures that aren't `data:` URIs through `images`.
pub fn parse_with(s: &str, images: ImageLoader) -> Flow {
    parse_in(s, images, false)
}

pub(super) fn parse_in(s: &str, images: ImageLoader, page: bool) -> Flow {
    // HTML normalizes input newlines before tokenization, not after decoding character references.
    let normalized = if s.contains('\r') {
        let mut out = String::with_capacity(s.len());
        let mut chunks = s.split('\r');
        out.push_str(chunks.next().unwrap_or_default());
        for chunk in chunks {
            out.push('\n');
            out.push_str(chunk.strip_prefix('\n').unwrap_or(chunk));
        }
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(s)
    };
    // The lexer's offsets (a style element's text is cut out by them) are into the normalized text.
    let s: &str = &normalized;
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
                if name == "ruby" && !self_close {
                    let r = read_ruby(&mut lx);
                    if !b.in_head() {
                        b.push_ruby(&r.base, &r.ruby);
                    }
                    continue;
                }
                if matches!(name.as_str(), "script" | "style" | "noscript" | "template" | "textarea") && !self_close {
                    let from = lx.pos;
                    let to = lx.skip_past_end_tag(&name);
                    if name == "style" {
                        parse_sheet(s.get(from..to).unwrap_or(""), &mut b.sheet);
                    }
                    continue;
                }
                b.start(&name, &attrs, self_close);
            }
            Tok::End(name) => b.end(&name),
        }
    }
    b.finish()
}

/// Collapse white space inside ruby markup as body text does (HTML's white space only: an
/// ideographic space is text), without the space the element's indentation leaves at the edges.
pub(super) fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000C}') {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out.trim_matches(' ').to_string()
}

/// Parse HTML into a document.
pub fn import(s: &str) -> Document {
    model::begin_import();
    model::to_doc(&parse(s))
}

/// [`import`], loading pictures that aren't `data:` URIs through `images`.
pub fn import_with(s: &str, images: ImageLoader) -> Document {
    model::begin_import();
    model::to_doc(&parse_with(s, images))
}

/// A web page opened as a document, as Word opens one (a file, or a document's HTML
/// `w:altChunk`): its text is Times New Roman 12 pt on single lines, headings bold and black at
/// the sizes browsers give them, and a `p` or heading whose CSS leaves a margin out (or writes it
/// invalidly) gets HTML's automatic spacing there; a margin a span states replaces the one its
/// paragraph states (both as Word's import does). [`import`] reads a pasted fragment, which takes
/// the document's own formatting instead (but its CSS still applies).
pub fn import_page(s: &str, images: ImageLoader) -> Document {
    model::begin_import();
    model::to_doc(&parse_in(s, images, true))
}

/// [`import_page`] onto `page` (its size and margins), which tables size to: HTML gives no page
/// size, so the caller picks it (the region's paper, #1381).
pub fn import_on(s: &str, images: ImageLoader, page: &wordcraft_doc::SectionProps) -> Document {
    model::begin_import();
    model::to_doc_in(&parse_in(s, images, true), page.clone())
}

/// The font of a web page's text when its CSS names none.
pub const PAGE_FONT: &str = "Times New Roman";
/// Heading sizes on a web page, `h1`–`h6`, points.
pub(super) const PAGE_HEADING_SIZES: [f32; 6] = [24.0, 18.0, 13.5, 12.0, 10.0, 7.5];
