//! Word's field markup in HTML (#5581): `<span style='mso-element:field-begin'>`, the field code,
//! `field-separator`, the result it last showed, `field-end`, inside `<!--[if supportFields]>`
//! conditional comments. Better Notes for Zotero writes a whole citation in one comment; Word's
//! own pages put the result between two comments, as ordinary HTML. Either way the field becomes
//! a [`Inline::FieldStart`] … [`Inline::FieldEnd`] around its result, so the result keeps its
//! formatting and the code (`ADDIN ZOTERO_ITEM CSL_CITATION {…}`) reaches the document.

use super::Inline;
use super::builder::{Builder, decls};
use super::import::skip_raw_text;
use super::lexer::{Lexer, Tok};

/// Most fields open at once (nested); deeper ones are read as their result.
const MAX_OPEN: usize = 64;
/// Longest field code kept, in bytes (a citation's code carries its item data).
const MAX_CODE: usize = 1 << 20;
/// Most tokens one `supportFields` comment is read as.
const MAX_TOKENS: usize = 1 << 20;
/// Most `supportFields` openers read inside one comment (each one unclosed).
const MAX_NESTED: usize = 16;

/// Where an open field is.
enum State {
    /// Its code is being read.
    Code(String),
    /// Its result is being read: the content goes on as body content.
    Result,
    /// It began inside another field's code (a nested field there): its text is that code's.
    InCode,
    /// Past [`MAX_OPEN`]: its marks are ignored.
    Ignored,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Begin,
    Separator,
    End,
}

/// The fields open in the page being read.
#[derive(Default)]
pub(super) struct Fields {
    open: Vec<State>,
    /// Inside a `supportFields` comment, the only place a code's text can be: browsers show what
    /// is outside one, so there it is the page's text.
    in_comment: bool,
}

/// The field mark a start tag is (`mso-element:field-begin|field-separator|field-end` in its
/// style), if any.
fn mark(attrs: &[(String, String)]) -> Option<Mark> {
    let style = attrs.iter().find(|(k, _)| k == "style").map(|(_, v)| v.as_str())?;
    let (_, v) = decls(style).find(|(k, _)| k == "mso-element")?;
    match v.trim().to_ascii_lowercase().as_str() {
        "field-begin" => Some(Mark::Begin),
        "field-separator" => Some(Mark::Separator),
        "field-end" => Some(Mark::End),
        _ => None,
    }
}

/// Whether `text` is white space only.
fn is_blank(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii_whitespace())
}

/// A field code as Word reads it from HTML: white space collapsed, trimmed.
fn clean_code(code: &str) -> String {
    code.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

impl Fields {
    /// Take `t` when it is field markup (a mark, code text, a `supportFields` comment); hand
    /// anything else back for the page reader.
    pub(super) fn take(&mut self, t: Tok, lx: &mut Lexer, b: &mut Builder) -> Option<Tok> {
        match t {
            Tok::Text(text) => {
                if self.in_comment {
                    if let Some(code) = self.code() {
                        if code.len().saturating_add(text.len()) <= MAX_CODE {
                            code.push_str(&text);
                        }
                        return None;
                    }
                } else if self.code().is_some() {
                    // White space between the comments a code spans; any other text is the
                    // page's, so the code it follows was never going to go on.
                    if is_blank(&text) {
                        return None;
                    }
                    self.abandon_code(b);
                }
                Some(Tok::Text(text))
            }
            Tok::Start { name, attrs, self_close } => {
                let Some(m) = mark(&attrs) else { return Some(Tok::Start { name, attrs, self_close }) };
                if !b.in_head() {
                    self.mark(m, b);
                }
                // An empty mark's end tag must not close an enclosing span. An element with
                // content (or a void one, which has no end tag) stays in the page.
                if self_close || lx.skip_end_tag_here(&name) { None } else { Some(Tok::Start { name, attrs, self_close }) }
            }
            Tok::SupportFields(markup) => {
                self.in_comment = true;
                self.comment(&markup, b);
                self.in_comment = false;
                None
            }
            t => Some(t),
        }
    }

    /// Read the markup of a `supportFields` comment: marks, code and (Better Notes) the result.
    ///
    /// A `<!--[if supportFields]>` opened inside it (which can only be unclosed: comments don't
    /// nest) is read on from where it stands, in a loop rather than by recursion, so hostile
    /// markup can't exhaust the stack; past [`MAX_NESTED`] such openers the rest is dropped.
    fn comment(&mut self, markup: &str, b: &mut Builder) {
        let mut current = markup.to_owned();
        let mut count = 0usize;
        for _ in 0..=MAX_NESTED {
            match self.comment_part(&current, b, &mut count) {
                Some(next) => current = next,
                None => return,
            }
        }
    }

    /// Read one comment's markup, counting tokens into `count`; the markup of a `supportFields`
    /// comment opened inside it, when there is one, is handed back to be read next.
    fn comment_part(&mut self, markup: &str, b: &mut Builder, count: &mut usize) -> Option<String> {
        let mut lx = Lexer::new(markup);
        // White space the comment's source lays out around a mark (`\n<span …field-separator…>\n`)
        // is not the result's: it is dropped after a mark and held until content follows.
        let mut after_mark = true;
        let mut held = String::new();
        while *count < MAX_TOKENS
            && let Some(t) = lx.next()
        {
            *count += 1;
            if let Tok::SupportFields(next) = t {
                return Some(next);
            }
            let is_mark = matches!(&t, Tok::Start { attrs, .. } if mark(attrs).is_some());
            // A space with no line break in a result is the result's, not the source's layout.
            if let Tok::Text(text) = &t
                && self.code().is_none()
                && is_blank(text)
                && (text.contains(['\n', '\r']) || !matches!(self.open.last(), Some(State::Result)))
            {
                if !after_mark && held.len() < 64 {
                    held.push_str(text);
                }
                continue;
            }
            if is_mark {
                held.clear();
                after_mark = true;
            } else {
                if !held.is_empty() {
                    b.text(&held);
                    held.clear();
                }
                after_mark = false;
            }
            match self.take(t, &mut lx, b) {
                Some(Tok::Text(t)) => b.text(&t),
                Some(Tok::Start { name, attrs, self_close }) => {
                    b.close_head_for_start(&name);
                    if !skip_raw_text(&name, self_close, &mut lx, b) {
                        b.start(&name, &attrs, self_close);
                    }
                }
                Some(Tok::End(name)) => b.end(&name),
                _ => {}
            }
        }
        None
    }

    /// The code being read, when text belongs to one (the outermost field still in its code).
    fn code(&mut self) -> Option<&mut String> {
        self.open.iter_mut().find_map(|s| match s {
            State::Code(c) => Some(c),
            _ => None,
        })
    }

    /// Close the fields from the outermost one still in its code: page text came instead of the
    /// rest of it.
    fn abandon_code(&mut self, b: &mut Builder) {
        let Some(at) = self.open.iter().position(|s| matches!(s, State::Code(_))) else { return };
        for s in self.open.split_off(at).into_iter().rev() {
            close(s, b);
        }
    }

    /// Act on field mark `m`: open a field, start its result, or close it.
    fn mark(&mut self, m: Mark, b: &mut Builder) {
        match m {
            Mark::Begin => {
                let in_code = self.open.iter().any(|s| matches!(s, State::Code(_) | State::InCode));
                let state = if self.open.len() >= MAX_OPEN {
                    State::Ignored
                } else if in_code {
                    State::InCode
                } else {
                    State::Code(String::new())
                };
                self.open.push(state);
            }
            Mark::Separator => {
                if let Some(top) = self.open.last_mut()
                    && let State::Code(code) = top
                {
                    let instr = clean_code(code);
                    // A field without a code is no field: its result stays as plain content.
                    if instr.is_empty() {
                        *top = State::Ignored;
                    } else {
                        *top = State::Result;
                        b.start_para().inlines.push(Inline::FieldStart(instr));
                    }
                }
            }
            Mark::End => {
                if let Some(s) = self.open.pop() {
                    close(s, b);
                }
            }
        }
    }

    /// Close the fields the page leaves open, at the end of its last paragraph (a new one only
    /// when the page ends in a table or has none).
    pub(super) fn finish(&mut self, flow: &mut super::Flow) {
        let mut tail = Vec::new();
        while let Some(s) = self.open.pop() {
            match s {
                State::Code(code) if !clean_code(&code).is_empty() => {
                    tail.push(Inline::FieldStart(clean_code(&code)));
                    tail.push(Inline::FieldEnd);
                }
                State::Result => tail.push(Inline::FieldEnd),
                State::Code(_) | State::InCode | State::Ignored => {}
            }
        }
        if tail.is_empty() {
            return;
        }
        if let Some(super::FBlock::Para(p)) = flow.blocks.last_mut() {
            p.inlines.extend(tail);
        } else {
            flow.blocks.push(super::FBlock::Para(super::Para { inlines: tail, ..Default::default() }));
        }
    }
}

/// Close an open field at the builder's position: a field still in its code gets an empty
/// result; fields nested in a code or past [`MAX_OPEN`] leave nothing.
fn close(s: State, b: &mut Builder) {
    match s {
        // No separator: a field with an empty result.
        State::Code(code) if !clean_code(&code).is_empty() => {
            let inlines = &mut b.start_para().inlines;
            inlines.push(Inline::FieldStart(clean_code(&code)));
            inlines.push(Inline::FieldEnd);
        }
        State::Result => b.start_para().inlines.push(Inline::FieldEnd),
        State::Code(_) | State::InCode | State::Ignored => {}
    }
}
