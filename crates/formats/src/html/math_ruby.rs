use super::builder::block_kind;
use super::lexer::{Lexer, Tok};

/// A `<math>` element (#816): its tokens up to the matching end tag, as a tree. Nesting and size
/// are bounded; an unclosed element ends with the input.
pub(super) fn read_math(lx: &mut Lexer, name: String, attrs: Vec<(String, String)>) -> crate::mathml::El {
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

/// A `<ruby>` element's base text and reading (#1681).
#[derive(Default)]
pub(super) struct RubyText {
    pub(super) base: String,
    pub(super) ruby: String,
}

/// Which part of a `<ruby>` element the text being read belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RubyPart {
    /// The text being annotated: an `<rb>` child, or the text outside `<rt>`/`<rp>`.
    Base,
    /// The reading: an `<rt>` child.
    Ruby,
    /// `<rp>` fallback parentheses, which are not text: browsers show them only where ruby isn't
    /// supported.
    Fallback,
}

/// A block-level tag ends an open `<ruby>` implicitly, as it does in a browser.
fn ends_ruby(t: &Tok) -> bool {
    match t {
        Tok::Start { name, .. } | Tok::End(name) => block_kind(name).is_some(),
        Tok::Text(_) => false,
    }
}

/// Read a `<ruby>` element's content: its base text from `<rb>` children (or the text outside
/// them), its reading from `<rt>` children, and `<rp>` fallback parentheses dropped. Tolerant,
/// like the rest of the reader: tags inside are ignored, an unclosed element ends at the next
/// block-level tag (handed back to the caller) or with the input, and the tokens read are capped.
pub(super) fn read_ruby(lx: &mut Lexer) -> RubyText {
    /// Most tokens one `<ruby>` element is read from: a missing end tag must not scan the rest of
    /// a hostile file here.
    const MAX_TOKENS: usize = 4096;
    let mut out = RubyText::default();
    let mut part = RubyPart::Base;
    let mut nested = 0usize;
    let mut count = 0usize;
    while count < MAX_TOKENS {
        count += 1;
        let start = lx.pos;
        let Some(t) = lx.next() else { break };
        if nested == 0 && ends_ruby(&t) {
            // The boundary ends the element: give the token back to the main reader.
            lx.pos = start;
            break;
        }
        match t {
            Tok::Text(t) => match part {
                RubyPart::Base => out.base.push_str(&t),
                RubyPart::Ruby => out.ruby.push_str(&t),
                RubyPart::Fallback => {}
            },
            // The self-closing slash is ignored on HTML elements (`<rt/>` opens an `rt`), as in a
            // browser's parser.
            Tok::Start { name, self_close, .. } => match name.as_str() {
                "rb" => part = RubyPart::Base,
                "rt" => part = RubyPart::Ruby,
                "rp" => part = RubyPart::Fallback,
                "ruby" if !self_close => nested += 1,
                _ => {}
            },
            Tok::End(name) => match name.as_str() {
                "rb" | "rt" | "rp" => part = RubyPart::Base,
                "ruby" if nested > 0 => nested -= 1,
                "ruby" => break,
                _ => {}
            },
        }
    }
    out
}
