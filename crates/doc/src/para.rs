//! Paragraphs: text with formatting runs and inline objects.
//!
//! A paragraph's text is one `String`. Formatting is a list of [`Run`]s whose byte lengths sum to
//! the text length. Inline objects (pictures, fields, note references, bookmarks, comment
//! anchors…) sit in the text as U+FFFC; the k-th U+FFFC is `objects[k]`. Special characters:
//! `\t` tab, `\n` manual line break, U+000C page break, U+000E column break.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::props::{CharProps, ParaProps};
use crate::section::SectionProps;
use crate::{DocError, Result};

mod float;
mod inline_kinds;
mod inline_object;
mod run;

pub use float::{AltText, Anchor, Float, FloatAlign, MAX_ALT_TEXT, Wrap, WrapText, cap_alt};
pub use inline_kinds::{NoteKind, PTabAlign, PTabBase, RubyAlign, ShapeKind, ruby_offset, ruby_raise};
pub use inline_object::{CanvasStyle, GroupChild, InlineObject, MAX_GROUP_CHILDREN};
pub use run::{COLUMN_BREAK, LINE_BREAK, NB_HYPHEN, NBSP, OBJ, PAGE_BREAK, Run, SOFT_HYPHEN, next_rev};

/// A paragraph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paragraph {
    pub text: String,
    pub runs: Vec<Run>,
    #[serde(default, skip_serializing_if = "ParaProps::is_empty")]
    pub props: ParaProps,
    /// Formatting of the paragraph mark (used when typing into an empty paragraph).
    #[serde(default, skip_serializing_if = "CharProps::is_empty")]
    pub mark: CharProps,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<InlineObject>,
    /// Set when this paragraph ends a section (OOXML keeps `w:sectPr` on the last paragraph).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<Box<SectionProps>>,
    /// Change counter for caches (not part of equality).
    #[serde(skip, default = "next_rev")]
    pub rev: u64,
}

impl PartialEq for Paragraph {
    fn eq(&self, o: &Self) -> bool {
        self.text == o.text
            && self.runs == o.runs
            && self.props == o.props
            && self.mark == o.mark
            && self.objects == o.objects
            && self.section == o.section
    }
}

impl Default for Paragraph {
    fn default() -> Self {
        Paragraph::new()
    }
}

impl Paragraph {
    pub fn new() -> Self {
        Paragraph {
            text: String::new(),
            runs: Vec::new(),
            props: ParaProps::default(),
            mark: CharProps::default(),
            objects: Vec::new(),
            section: None,
            rev: next_rev(),
        }
    }
    /// A paragraph of plain text in one run.
    pub fn with_text(text: &str, props: CharProps) -> Self {
        let mut p = Paragraph::new();
        let clean: String = text.chars().filter(|c| *c != OBJ).collect();
        if !clean.is_empty() {
            p.runs.push(Run { len: clean.len(), props: props.clone() });
        }
        p.text = clean;
        p.mark = props;
        p
    }
    pub fn styled(mut self, style: &str) -> Self {
        self.props.style = Some(style.to_string());
        self
    }
    pub fn touch(&mut self) {
        self.rev = next_rev();
    }
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    /// The text with objects removed or replaced by their text (fields → result).
    pub fn plain_text(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut k = 0;
        for c in self.text.chars() {
            if c == OBJ {
                if let Some(o) = self.objects.get(k) {
                    out.push_str(o.plain_text());
                }
                k += 1;
            } else {
                out.push(c);
            }
        }
        out
    }
    /// [`Self::plain_text`] as it reads on screen: characters in runs set in a symbol font
    /// (Symbol, Wingdings…) become their standard Unicode equivalents
    /// ([`crate::symbol_fonts`]). For copying as text and plain-text export; the stored text is
    /// unchanged.
    pub fn shown_text(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut k = 0;
        let mut runs = self.run_ranges().peekable();
        for (i, c) in self.text.char_indices() {
            while runs.peek().is_some_and(|(r, _)| r.end <= i) {
                runs.next();
            }
            if c == OBJ {
                if let Some(o) = self.objects.get(k) {
                    out.push_str(o.plain_text());
                }
                k += 1;
                continue;
            }
            let font = runs.peek().filter(|(r, _)| r.contains(&i)).and_then(|(_, p)| p.shown_font());
            out.push(font.and_then(|f| crate::symbol_fonts::symbol_char(f, c)).unwrap_or(c));
        }
        out
    }
    /// [`Self::plain_text`] without tracked deletions: the text as it reads with every change accepted.
    pub fn final_text(&self) -> String {
        self.text_without(&self.deleted_ranges())
    }
    /// The byte ranges of the tracked deletions.
    pub fn deleted_ranges(&self) -> Vec<std::ops::Range<usize>> {
        self.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| r).collect()
    }
    /// [`Self::plain_text`] without the text in the byte ranges `dropped` (in any order, overlapping
    /// or not).
    pub fn text_without(&self, dropped: &[std::ops::Range<usize>]) -> String {
        let mut sorted = dropped.to_vec();
        sorted.sort_unstable_by_key(|r| r.start);
        let mut next = sorted.iter().peekable();
        let mut out = String::with_capacity(self.text.len());
        let mut k = 0;
        for (i, c) in self.text.char_indices() {
            // By start: once the ranges that end by `i` are skipped, `i` is dropped if the next has begun.
            while next.next_if(|r| r.end <= i).is_some() {}
            let keep = next.peek().is_none_or(|r| r.start > i);
            if c == OBJ {
                if keep && let Some(o) = self.objects.get(k) {
                    out.push_str(o.plain_text());
                }
                k += 1;
            } else if keep {
                out.push(c);
            }
        }
        out
    }

    fn check(&self, off: usize) -> Result<()> {
        if off > self.text.len() || !self.text.is_char_boundary(off) {
            return Err(DocError::BadOffset(off));
        }
        Ok(())
    }

    /// Clamp `off` to the text and back to a char boundary.
    pub fn clamp(&self, off: usize) -> usize {
        let mut o = off.min(self.text.len());
        while o > 0 && !self.text.is_char_boundary(o) {
            o -= 1;
        }
        o
    }

    /// Index of the run containing byte `off` and the offset where that run starts. At a run
    /// boundary the earlier run wins (typing continues the formatting to the left).
    fn run_at(&self, off: usize) -> Option<(usize, usize)> {
        let mut start = 0;
        for (i, r) in self.runs.iter().enumerate() {
            let end = start + r.len;
            if off < end || (off == end && off > start) {
                return Some((i, start));
            }
            start = end;
        }
        None
    }

    /// Formatting in effect at `off` (what typing there would get): the run to the left, or the
    /// first run at offset 0, or the paragraph mark when empty.
    pub fn props_at(&self, off: usize) -> &CharProps {
        if off == 0 {
            return self.runs.first().map(|r| &r.props).unwrap_or(&self.mark);
        }
        self.run_at(off).and_then(|(i, _)| self.runs.get(i)).map(|r| &r.props).unwrap_or(&self.mark)
    }

    /// Formatting of the character starting at `off`.
    pub fn props_of_char(&self, off: usize) -> &CharProps {
        let mut start = 0;
        for r in &self.runs {
            if off < start + r.len {
                return &r.props;
            }
            start += r.len;
        }
        &self.mark
    }

    /// Runs as (byte range, props).
    pub fn run_ranges(&self) -> impl Iterator<Item = (std::ops::Range<usize>, &CharProps)> {
        let mut s = 0;
        self.runs.iter().map(move |r| {
            let a = s;
            s += r.len;
            (a..s, &r.props)
        })
    }

    /// Number of objects in `text[..off]`.
    fn objects_before(&self, off: usize) -> usize {
        self.text.get(..off).map(|t| t.chars().filter(|c| *c == OBJ).count()).unwrap_or(0)
    }

    /// The object at byte `off` (which must hold U+FFFC).
    pub fn object_at(&self, off: usize) -> Option<&InlineObject> {
        if !self.text.get(off..)?.starts_with(OBJ) {
            return None;
        }
        self.objects.get(self.objects_before(off))
    }
    pub fn object_at_mut(&mut self, off: usize) -> Option<&mut InlineObject> {
        if !self.text.get(off..)?.starts_with(OBJ) {
            return None;
        }
        let k = self.objects_before(off);
        self.objects.get_mut(k)
    }
    /// Byte offsets of all objects, in order.
    pub fn object_offsets(&self) -> Vec<usize> {
        self.text.char_indices().filter(|(_, c)| *c == OBJ).map(|(i, _)| i).collect()
    }
    /// (byte offset, object) for every object, in text order, in one pass. Prefer it to
    /// [`Paragraph::object_offsets`] + [`Paragraph::object_at`], which counts the objects before
    /// each offset again, so a loop over a paragraph's objects is quadratic (#3614).
    pub fn objects_with_offsets(&self) -> impl Iterator<Item = (usize, &InlineObject)> + '_ {
        self.text.char_indices().filter(|(_, c)| *c == OBJ).map(|(i, _)| i).zip(self.objects.iter())
    }

    /// Insert plain text (U+FFFC is dropped) at `off` with `props`. Returns the inserted length.
    pub fn insert_text(&mut self, off: usize, s: &str, props: &CharProps) -> Result<usize> {
        self.check(off)?;
        let clean: std::borrow::Cow<str> = if s.contains(OBJ) { s.chars().filter(|c| *c != OBJ).collect::<String>().into() } else { s.into() };
        if clean.is_empty() {
            return Ok(0);
        }
        self.text.insert_str(off, &clean);
        self.insert_run(off, clean.len(), props.clone());
        self.touch();
        Ok(clean.len())
    }

    /// Insert an object at `off`.
    pub fn insert_object(&mut self, off: usize, obj: InlineObject, props: &CharProps) -> Result<()> {
        self.check(off)?;
        let k = self.objects_before(off);
        self.text.insert(off, OBJ);
        self.objects.insert(k.min(self.objects.len()), obj);
        self.insert_run(off, OBJ.len_utf8(), props.clone());
        self.touch();
        Ok(())
    }

    fn insert_run(&mut self, off: usize, len: usize, props: CharProps) {
        // Split the run at `off`, insert, merge neighbours.
        let mut start = 0;
        let mut idx = self.runs.len();
        for (i, r) in self.runs.iter().enumerate() {
            if off <= start {
                idx = i;
                break;
            }
            if off < start + r.len {
                // split
                let left = off - start;
                let right = r.len - left;
                let p = r.props.clone();
                if let Some(r) = self.runs.get_mut(i) {
                    r.len = left;
                }
                self.runs.insert(i + 1, Run { len: right, props: p });
                idx = i + 1;
                break;
            }
            start += r.len;
        }
        self.runs.insert(idx.min(self.runs.len()), Run { len, props });
        self.normalize();
    }

    /// Merge adjacent equal runs and drop empty ones.
    pub fn normalize(&mut self) {
        // Runs must end on character boundaries and cover the text exactly (never expected, but
        // files lie): clamp each end to the text, push it past a split character, pad the tail.
        let n = self.text.len();
        let mut end = 0usize;
        let mut out: Vec<Run> = Vec::with_capacity(self.runs.len());
        for r in self.runs.drain(..) {
            let mut e = end.saturating_add(r.len).min(n);
            while !self.text.is_char_boundary(e) {
                e += 1;
            }
            if e == end {
                continue;
            }
            match out.last_mut() {
                Some(l) if l.props == r.props => l.len += e - end,
                _ => out.push(Run { len: e - end, props: r.props }),
            }
            end = e;
        }
        if end < n {
            match out.last_mut() {
                Some(l) => l.len += n - end,
                None => out.push(Run { len: n, props: self.mark.clone() }),
            }
        }
        self.runs = out;
    }

    /// Delete `a..b` (bytes). Objects inside are removed.
    pub fn delete(&mut self, a: usize, b: usize) -> Result<()> {
        let (a, b) = (a.min(b), a.max(b));
        self.check(a)?;
        self.check(b)?;
        if a == b {
            return Ok(());
        }
        if self.runs.is_empty() || a == 0 {
            // Keep the formatting of what's deleted for the mark when emptying.
            if b == self.text.len() {
                // Formatting only: the mark is a tracked change of its own or it is not, whatever
                // revision the deleted text was part of.
                let revisions = (self.mark.ins, self.mark.del, self.mark.fmt_change.clone());
                let mut mark = self.props_of_char(a).clone();
                (mark.ins, mark.del, mark.fmt_change) = revisions;
                self.mark = mark;
            }
        }
        let k0 = self.objects_before(a);
        let k1 = self.objects_before(b);
        if k1 > k0 && k1 <= self.objects.len() {
            self.objects.drain(k0..k1);
        }
        self.text.replace_range(a..b, "");
        let mut start = 0;
        for r in &mut self.runs {
            let (rs, re) = (start, start + r.len);
            start = re;
            let ov = re.min(b).saturating_sub(rs.max(a));
            r.len -= ov.min(r.len);
        }
        self.normalize();
        self.touch();
        Ok(())
    }

    /// Delete `a..b` except the one-character objects at the offsets in `keep` (control
    /// markers a deletion leaves in place).
    pub fn delete_except(&mut self, a: usize, b: usize, keep: &[usize]) -> Result<()> {
        let (a, b) = (a.min(b), a.max(b));
        let mut keep: Vec<usize> =
            keep.iter().copied().filter(|k| *k >= a && k + OBJ.len_utf8() <= b && self.text.get(*k..).is_some_and(|t| t.starts_with(OBJ))).collect();
        if keep.is_empty() {
            return self.delete(a, b);
        }
        keep.sort_unstable();
        keep.dedup();
        // Pieces between the kept objects, last first so earlier offsets stay valid.
        let mut pieces = Vec::with_capacity(keep.len() + 1);
        let mut from = a;
        for k in &keep {
            pieces.push((from, *k));
            from = k + OBJ.len_utf8();
        }
        pieces.push((from, b));
        for (x, y) in pieces.into_iter().rev() {
            if y > x {
                self.delete(x, y)?;
            }
        }
        Ok(())
    }

    /// Apply `f` to the formatting of `a..b`.
    pub fn format(&mut self, a: usize, b: usize, f: &dyn Fn(&mut CharProps)) -> Result<()> {
        let (a, b) = (a.min(b), a.max(b));
        self.check(a)?;
        self.check(b)?;
        if a == b {
            return Ok(());
        }
        let mut out = Vec::with_capacity(self.runs.len() + 2);
        let mut start = 0;
        for r in self.runs.drain(..) {
            let (rs, re) = (start, start + r.len);
            start = re;
            let cut = [rs, a.clamp(rs, re), b.clamp(rs, re), re];
            for w in cut.windows(2) {
                let (x, y) = (w[0], w[1]);
                if y <= x {
                    continue;
                }
                let mut p = r.props.clone();
                if x >= a && y <= b {
                    f(&mut p);
                }
                out.push(Run { len: y - x, props: p });
            }
        }
        self.runs = out;
        self.normalize();
        self.touch();
        Ok(())
    }

    /// Split at `off`: `self` keeps `..off`, the returned paragraph gets `off..` with the same
    /// paragraph properties (the section break, if any, moves to the second half).
    pub fn split_off(&mut self, off: usize) -> Result<Paragraph> {
        self.check(off)?;
        let k = self.objects_before(off);
        let tail_text = self.text.split_off(off);
        let tail_objs = if k <= self.objects.len() { self.objects.split_off(k) } else { Vec::new() };
        let mut tail_runs = Vec::new();
        let mut start = 0;
        let mut keep = Vec::new();
        for r in self.runs.drain(..) {
            let (rs, re) = (start, start + r.len);
            start = re;
            if re <= off {
                keep.push(r);
            } else if rs >= off {
                tail_runs.push(r);
            } else {
                keep.push(Run { len: off - rs, props: r.props.clone() });
                tail_runs.push(Run { len: re - off, props: r.props });
            }
        }
        self.runs = keep;
        // The new paragraph's mark takes the formatting at the split, but never a hyperlink.
        let carry = self.props_at(off).unlinked();
        let mut tail = Paragraph {
            text: tail_text,
            runs: tail_runs,
            props: self.props.clone(),
            mark: if self.text.is_empty() && off == 0 { self.mark.clone() } else { carry },
            objects: tail_objs,
            section: self.section.take(),
            rev: next_rev(),
        };
        if tail.text.is_empty() {
            tail.mark = self.props_at(off).unlinked();
        }
        self.normalize();
        tail.normalize();
        self.touch();
        Ok(tail)
    }

    /// Append `other`'s content (its paragraph properties are dropped; its section break kept).
    pub fn append(&mut self, other: Paragraph) {
        let Paragraph { text, runs, objects, section, .. } = other;
        self.text.push_str(&text);
        self.runs.extend(runs);
        self.objects.extend(objects);
        if section.is_some() {
            self.section = section;
        }
        self.normalize();
        self.touch();
    }

    /// Previous caret stop before `off` (0 at start): a grapheme boundary that doesn't split a
    /// complex-script syllable ([`crate::cluster`]).
    pub fn prev_boundary(&self, off: usize) -> usize {
        crate::cluster::prev_cluster(&self.text, self.clamp(off))
    }
    /// Next caret stop after `off` (len at end), see [`Self::prev_boundary`].
    pub fn next_boundary(&self, off: usize) -> usize {
        crate::cluster::next_cluster(&self.text, self.clamp(off))
    }
    /// Where Backspace at `off` deletes from: one Arabic combining mark or one character in the
    /// Brahmic scripts (Myanmar, Khmer, Devanagari, Thai…), otherwise the previous caret stop.
    pub fn backspace_from(&self, off: usize) -> usize {
        let off = self.clamp(off);
        match self.text.get(..off).and_then(|t| t.char_indices().next_back()) {
            Some((i, c)) if crate::cluster::backspaces_by_char(c) => i,
            _ => self.prev_boundary(off),
        }
    }
    /// Start of the word at or before `off` (Ctrl+Left).
    /// Thai and Lao have no spaces between words; ICU4X dictionary segmentation is used there.
    pub fn word_start(&self, off: usize) -> usize {
        let off = self.clamp(off);
        if self.text.get(..off).and_then(|t| t.chars().next_back()).is_some_and(crate::cluster::is_thai_lao) {
            return crate::cluster::thai_word_start(&self.text, off);
        }
        let Some(before) = self.text.get(..off) else { return 0 };
        let mut idx = off;
        let mut seen_word = false;
        for (i, c) in before.char_indices().rev() {
            // Thai/Lao chars are alphanumeric but belong to their own script; treat the script
            // transition as a word boundary so the Latin scan doesn't walk into Thai/Lao runs.
            let w = crate::bidi::is_word_char(c) && !crate::cluster::is_thai_lao(c);
            if w {
                seen_word = true;
                idx = i;
            } else if seen_word {
                break;
            } else if crate::cluster::is_thai_lao(c) {
                // Reached a Thai/Lao run before finding any Latin word (e.g. cursor on a space
                // between Thai and Latin).  Delegate to Thai segmentation at the run's right edge.
                return crate::cluster::thai_word_start(&self.text, i + c.len_utf8());
            } else {
                idx = i;
            }
        }
        idx
    }
    /// End of the word after `off`, including trailing spaces (Ctrl+Right, like Word).
    /// Thai and Lao have no spaces between words; ICU4X dictionary segmentation is used there.
    pub fn word_end(&self, off: usize) -> usize {
        let off = self.clamp(off);
        if self.text.get(off..).and_then(|t| t.chars().next()).is_some_and(crate::cluster::is_thai_lao) {
            return crate::cluster::thai_word_end(&self.text, off);
        }
        let Some(after) = self.text.get(off..) else { return self.text.len() };
        let it = after.char_indices().peekable();
        let mut end = self.text.len();
        // Thai/Lao is alphanumeric but forms its own script; it must not be treated as a Latin
        // word char — the script transition is a word boundary.
        let is_latin_word = |c: char| crate::bidi::is_word_char(c) && !crate::cluster::is_thai_lao(c);
        let first_word = after.chars().next().is_some_and(|c| is_latin_word(c) && c != '\'');
        let mut in_space = false;
        for (i, c) in it {
            let w = is_latin_word(c);
            if c == ' ' || c == NBSP {
                in_space = true;
                continue;
            }
            if in_space || (first_word && !w) || (!first_word && i > 0) {
                end = off + i;
                break;
            }
        }
        end
    }
    /// The word around `off` (double-click): (start, end) without trailing space.
    /// Thai and Lao have no spaces between words; ICU4X dictionary segmentation is used there.
    pub fn word_at(&self, off: usize) -> (usize, usize) {
        let off = self.clamp(off);
        let at_off = self.text.get(off..).and_then(|t| t.chars().next());
        let before_off = self.text.get(..off).and_then(|t| t.chars().next_back());
        // Use ICU4X Thai/Lao segmentation when the character AT the cursor is Thai/Lao, or when
        // the cursor sits on a space or end-of-text and the preceding character is Thai/Lao.
        // Never route to thai_word_at when the cursor is on a Latin/other script character (e.g.
        // the first letter of "test" in "ไทยtest" must select the Latin word, not the Thai one).
        let use_thai = at_off.is_some_and(crate::cluster::is_thai_lao)
            || (at_off.map_or(true, |c| matches!(c, ' ' | '\u{00A0}')) && before_off.is_some_and(crate::cluster::is_thai_lao));
        if use_thai {
            return crate::cluster::thai_word_at(&self.text, off);
        }
        // Thai/Lao chars satisfy is_alphanumeric, so exclude them from the Latin scan — they act
        // as a script boundary (the Thai path above handles them).
        let is_w = |c: char| crate::bidi::is_word_char(c) && !crate::cluster::is_thai_lao(c);
        let mut a = off;
        for (i, c) in self.text.get(..off).unwrap_or("").char_indices().rev() {
            if !is_w(c) {
                break;
            }
            a = i;
        }
        let mut b = off;
        for (i, c) in self.text.get(off..).unwrap_or("").char_indices() {
            if !is_w(c) {
                if i == 0 && a == off {
                    b = off + c.len_utf8();
                }
                break;
            }
            b = off + i + c.len_utf8();
        }
        // Word also selects the trailing spaces.
        let mut e = b;
        for (i, c) in self.text.get(b..).unwrap_or("").char_indices() {
            if c != ' ' {
                break;
            }
            e = b + i + 1;
        }
        (a, e)
    }
    /// Sentence around `off` (Ctrl+click): ends after `.`, `!`, `?` and following spaces.
    pub fn sentence_at(&self, off: usize) -> (usize, usize) {
        let off = self.clamp(off);
        let t = &self.text;
        let mut a = 0;
        let mut prev_end = false;
        for (i, c) in t.char_indices() {
            if i > off {
                break;
            }
            if prev_end && c != ' ' {
                a = i;
            }
            if matches!(c, '.' | '!' | '?') {
                prev_end = true;
            } else if c != ' ' {
                prev_end = false;
            }
        }
        let mut b = t.len();
        // A caret on the space right after a sentence's end belongs to that sentence; a terminator
        // directly followed by text ("Hello.World") doesn't end the sentence the caret is in.
        let mut ended = t.get(..off).unwrap_or("").ends_with(['.', '!', '?']) && t.get(off..).unwrap_or("").starts_with(' ');
        for (i, c) in t.get(off..).unwrap_or("").char_indices() {
            if ended && c != ' ' {
                b = off + i;
                break;
            }
            if matches!(c, '.' | '!' | '?') {
                ended = true;
            }
        }
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_at_sentence_start() {
        let p = Paragraph::with_text("Hello. World", CharProps::default());
        assert_eq!(p.sentence_at(0), (0, 7));
        assert_eq!(p.sentence_at(6), (0, 7));
        assert_eq!(p.sentence_at(7), (7, 12));
        assert_eq!(p.sentence_at(8), (7, 12));
        assert_eq!(p.sentence_at(99), (7, 12));
        let p = Paragraph::with_text("Hello.World", CharProps::default());
        assert_eq!(p.sentence_at(6), (6, 11));
        assert_eq!(p.sentence_at(8), (6, 11));
    }

    fn bold() -> CharProps {
        CharProps { bold: Some(true), ..Default::default() }
    }

    #[test]
    fn text_without_takes_ranges_in_any_order() {
        // Multi-byte chars at range edges, an object, and ranges unsorted, overlapping, nested and empty.
        let mut p = Paragraph::with_text("añb€c", CharProps::default());
        p.insert_object(
            3,
            InlineObject::Field { instr: "PAGE".into(), result: "7".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
        let text = p.text.clone();
        let naive = |dropped: &[std::ops::Range<usize>]| {
            let mut out = String::new();
            let mut k = 0;
            for (i, c) in text.char_indices() {
                let keep = !dropped.iter().any(|r| r.contains(&i));
                if c == OBJ {
                    if keep && let Some(o) = p.objects.get(k) {
                        out.push_str(o.plain_text());
                    }
                    k += 1;
                } else if keep {
                    out.push(c);
                }
            }
            out
        };
        let n = text.len();
        let cases: Vec<Vec<std::ops::Range<usize>>> =
            vec![vec![], vec![1..3], vec![5..n, 0..1], vec![0..4, 2..3, 3..6], vec![2..9, 1..2, 4..4, 0..n], vec![6..6, 9..n, 0..0], vec![3..n + 10]];
        for dropped in cases {
            assert_eq!(p.text_without(&dropped), naive(&dropped), "{dropped:?} of {text:?}");
        }
        assert_eq!(p.text_without(std::slice::from_ref(&(1..3))), "a7b€c");
    }

    #[test]
    fn insert_and_runs() {
        let mut p = Paragraph::with_text("Hello world", CharProps::default());
        p.insert_text(5, ",", &CharProps::default()).unwrap();
        assert_eq!(p.text, "Hello, world");
        assert_eq!(p.runs.len(), 1);
        p.insert_text(0, "Oh ", &bold()).unwrap();
        assert_eq!(p.runs.len(), 2);
        assert_eq!(p.runs[0].len, 3);
        assert!(p.insert_text(100, "x", &bold()).is_err());
    }

    #[test]
    fn insert_rejects_mid_char() {
        let mut p = Paragraph::with_text("é", CharProps::default());
        assert!(p.insert_text(1, "x", &bold()).is_err());
    }

    #[test]
    fn format_splits_runs() {
        let mut p = Paragraph::with_text("abcdef", CharProps::default());
        p.format(2, 4, &|c| c.bold = Some(true)).unwrap();
        assert_eq!(p.runs.iter().map(|r| r.len).collect::<Vec<_>>(), vec![2, 2, 2]);
        assert_eq!(p.props_of_char(2).bold, Some(true));
        p.format(0, 6, &|c| c.bold = Some(true)).unwrap();
        assert_eq!(p.runs.len(), 1);
    }

    #[test]
    fn delete_merges() {
        let mut p = Paragraph::with_text("abcdef", CharProps::default());
        p.format(2, 4, &|c| c.bold = Some(true)).unwrap();
        p.delete(2, 4).unwrap();
        assert_eq!(p.text, "abef");
        assert_eq!(p.runs.len(), 1);
        p.delete(0, 4).unwrap();
        assert!(p.runs.is_empty());
    }

    #[test]
    fn objects_track_text() {
        let mut p = Paragraph::with_text("ab", CharProps::default());
        p.insert_object(
            1,
            InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
        p.insert_object(0, InlineObject::BookmarkStart { name: "x".into() }, &CharProps::default()).unwrap();
        assert_eq!(p.objects.len(), 2);
        assert!(matches!(p.object_at(0), Some(InlineObject::BookmarkStart { .. })));
        let off = p.object_offsets()[1];
        assert!(matches!(p.object_at(off), Some(InlineObject::Field { .. })));
        // The one-pass walk agrees with offsets + object_at.
        let one_pass: Vec<(usize, &InlineObject)> = p.objects_with_offsets().collect();
        let each: Vec<(usize, &InlineObject)> = p.object_offsets().into_iter().filter_map(|o| Some((o, p.object_at(o)?))).collect();
        assert_eq!(one_pass, each);
        assert_eq!(one_pass.len(), 2);
        assert_eq!(p.plain_text(), "a1b");
        p.delete(0, 3).unwrap();
        assert_eq!(p.objects.len(), 1);
        let tail = p.split_off(p.object_offsets()[0]).unwrap();
        assert!(p.objects.is_empty());
        assert_eq!(tail.objects.len(), 1);
    }

    #[test]
    fn split_and_append() {
        let mut p = Paragraph::with_text("Hello world", bold());
        let t = p.split_off(5).unwrap();
        assert_eq!(p.text, "Hello");
        assert_eq!(t.text, " world");
        assert_eq!(t.runs[0].props.bold, Some(true));
        p.append(t);
        assert_eq!(p.text, "Hello world");
        assert_eq!(p.runs.len(), 1);
        let t = p.split_off(p.len()).unwrap();
        assert!(t.is_empty());
        assert_eq!(t.mark.bold, Some(true));
    }

    #[test]
    fn emptying_a_paragraph_keeps_the_formatting_but_not_the_revisions_of_its_text() {
        let mut props = bold();
        props.ins = Some(3);
        props.del = Some(4);
        let mut p = Paragraph::with_text("Hello", props);
        p.mark = bold();
        p.delete(0, 5).unwrap();
        assert!(p.is_empty());
        assert_eq!(p.mark.bold, Some(true));
        assert_eq!((p.mark.ins, p.mark.del), (None, None));
        // A mark that is itself a tracked change keeps its own revision.
        let mut p = Paragraph::with_text("Hello", bold());
        p.mark.del = Some(7);
        p.delete(0, 5).unwrap();
        assert_eq!((p.mark.ins, p.mark.del), (None, Some(7)));
    }

    #[test]
    fn words() {
        let p = Paragraph::with_text("The quick  brown fox.", CharProps::default());
        assert_eq!(p.word_end(0), 4);
        assert_eq!(p.word_end(4), 11);
        assert_eq!(p.word_start(10), 4);
        assert_eq!(p.word_start(11), 4);
        assert_eq!(p.word_at(5), (4, 11));
        assert_eq!(p.sentence_at(3), (0, 21));
        assert_eq!(p.next_boundary(0), 1);
        assert_eq!(p.prev_boundary(0), 0);
        // Mixed Thai/Latin (no spaces): script transitions are word boundaries; ICU4X word
        // boundaries within a Thai run are also respected.
        //
        // Byte layout: "ไทย"=9B(0) "test"=4B(9) "ลาว"=9B(13) "test"=4B(22)
        //              "ไม่"=9B(26) "รวม"=9B(35) "Khmer"=5B(44) → total 49B.
        // "ไม่รวม" is one Thai run (26..44); ICU4X splits it into ไม่(26..35)/รวม(35..44).
        let mixed = Paragraph::with_text("ไทยtestลาวtestไม่รวมKhmer", CharProps::default());
        assert_eq!(mixed.text.len(), 49);
        // Ctrl+Right (word_end)
        assert_eq!(mixed.word_end(0),  9,  "ไทย→");
        assert_eq!(mixed.word_end(9),  13, "first test→");
        assert_eq!(mixed.word_end(13), 22, "ลาว→");
        assert_eq!(mixed.word_end(22), 26, "second test→");
        assert_eq!(mixed.word_end(26), 35, "ไม่→ (ICU4X intra-run boundary)");
        assert_eq!(mixed.word_end(35), 44, "รวม→");
        assert_eq!(mixed.word_end(44), 49, "Khmer→end");
        // Ctrl+Left (word_start)
        assert_eq!(mixed.word_start(9),  0,  "←ไทย");
        assert_eq!(mixed.word_start(13), 9,  "←first test");
        assert_eq!(mixed.word_start(22), 13, "←ลาว");
        assert_eq!(mixed.word_start(26), 22, "←second test");
        assert_eq!(mixed.word_start(35), 26, "←ไม่");
        assert_eq!(mixed.word_start(44), 35, "←รวม");
        assert_eq!(mixed.word_start(49), 44, "←Khmer");
        // Double-click (word_at)
        assert_eq!(mixed.word_at(3),  (0,  9),  "inside ไทย");
        assert_eq!(mixed.word_at(9),  (9,  13), "on 't' after ไทย → Latin");
        assert_eq!(mixed.word_at(13), (13, 22), "on 'ล'");
        assert_eq!(mixed.word_at(16), (13, 22), "inside ลาว");
        assert_eq!(mixed.word_at(26), (26, 35), "on 'ไ' of ไม่");
        assert_eq!(mixed.word_at(35), (26, 35), "boundary inside Thai run → left word ไม่");
        assert_eq!(mixed.word_at(38), (35, 44), "inside รวม");
        assert_eq!(mixed.word_at(44), (44, 49), "on 'K' → Khmer");
        // Ctrl+Left across a space into a Thai run: "อนุรักษ์ภาษาไทย abc"
        // "อนุรักษ์"=24B(0) "ภาษา"=12B(24) "ไทย"=9B(36) " "=1B(45) "abc"=3B(46) → 49B
        let spaced = Paragraph::with_text("อนุรักษ์ภาษาไทย abc", CharProps::default());
        assert_eq!(spaced.text.len(), 49);
        assert_eq!(spaced.word_start(46), 36, "Ctrl+Left from 'a' crosses space → start of ไทย");
        assert_eq!(spaced.word_start(47), 46, "Ctrl+Left from inside 'abc' → start of abc");
        // NBSP after Thai routes word_at to thai_word_at.
        let nbsp = Paragraph::with_text("ไทย\u{00A0}abc", CharProps::default());
        // "ไทย"=9B, NBSP=2B(9..11), "abc"=3B(11..14)
        assert_eq!(nbsp.word_at(9), (0, 9), "cursor on NBSP after Thai → selects Thai word");
    }

    proptest::proptest! {
        #[test]
        fn edits_keep_runs_consistent(ops in proptest::collection::vec((0u8..4, 0usize..40, 0usize..40, "[a-zé ]{0,5}"), 0..40)) {
            let mut p = Paragraph::with_text("seed text", CharProps::default());
            for (k, a, b, s) in ops {
                let a = p.clamp(a);
                let b = p.clamp(b);
                let _ = match k {
                    0 => p.insert_text(a, &s, &bold()).map(|_| ()),
                    1 => p.delete(a, b),
                    2 => p.format(a, b, &|c| c.italic = Some(true)),
                    _ => p.split_off(a).map(|t| p.append(t)),
                };
                let total: usize = p.runs.iter().map(|r| r.len).sum();
                proptest::prop_assert_eq!(total, p.text.len());
                proptest::prop_assert!(p.runs.iter().all(|r| r.len > 0));
                proptest::prop_assert_eq!(p.text.matches(OBJ).count(), p.objects.len());
            }
        }
    }
}
