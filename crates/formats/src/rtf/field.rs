//! Page fields kept as fields (#6222). The reader writes a field's result (`\fldrslt`) into the
//! paragraph like any text, which for a page number is the number Word last stored. When the
//! group of a field the layout works out from where it lands closes (`PAGE`, `NUMPAGES`,
//! `SECTIONPAGES`, `SECTION`, `PAGEREF`), a result that is plain text in one formatting, in the
//! paragraph the field started in, is replaced by an [`Inline::Field`] holding the instruction
//! (`\fldinst`) and that result, as for a DOCX field. Any other field, a result over several
//! paragraphs or formats, or one holding pictures or other objects stays what the reader made of
//! it: a field's result is drawn as one piece that does not wrap, so a long `MERGEFIELD` result
//! in a narrow table cell would overflow it. Only `import` keeps fields: `parse`, the flow
//! reader, reads their results as text.

use super::reader::Reader;
use super::state::Dest;
use super::{Fmt, Inline};

/// The open fields read so far; deeper nesting keeps its results as text.
const MAX_OPEN: usize = 32;

/// A field being read in the body text.
pub(super) struct OpenField {
    /// The depth of the `{\field` group.
    depth: usize,
    /// Its instruction, in [`Reader::fields`].
    index: usize,
    /// The paragraph it started in ([`Reader::para_seq`]).
    para: u64,
    /// The paragraph's inlines when it started, and the length of the last one when that was
    /// text: a result in the same formatting runs on in that text.
    start: usize,
    tail_from: Option<usize>,
    /// Its result group was met in the text (not inside another field's instruction).
    result: bool,
    locked: bool,
}

impl Reader {
    /// `\field`, after its instruction slot is made: note where its result starts.
    pub(super) fn open_field(&mut self) {
        // Hidden text drops the result: the field stays hidden with it.
        if !self.marks_on || self.st.dest != Dest::Body || self.st.hidden || self.open_fields.len() >= MAX_OPEN {
            return;
        }
        let Some(index) = self.st.field else { return };
        let tail_from = match self.para.inlines.last() {
            Some(Inline::Text(t, _)) => Some(t.len()),
            _ => None,
        };
        let (depth, para, start) = (self.depth(), self.para_seq, self.para.inlines.len());
        self.open_fields.push(OpenField { depth, index, para, start, tail_from, result: false, locked: false });
    }

    /// `\fldrslt` and `\fldlock`, read in a field's group.
    pub(super) fn field_word(&mut self, w: &str) {
        let (depth, field, body) = (self.depth(), self.st.field, self.st.dest == Dest::Body);
        let Some(f) = self.open_fields.last_mut() else { return };
        match w {
            "fldrslt" if body && field == Some(f.index) && depth == f.depth + 1 => f.result = true,
            "fldlock" if depth == f.depth => f.locked = true,
            _ => {}
        }
    }

    /// `\chpgn`, the current page number: a `PAGE` field.
    pub(super) fn page_number_symbol(&mut self) {
        if self.marks_on && self.st.dest == Dest::Body && !self.st.hidden {
            let fmt = self.st.fmt.clone();
            self.para.inlines.push(Inline::Field { instr: "PAGE".into(), result: String::new(), fmt, locked: false });
            self.para_has = true;
        }
    }

    /// A group closes: if it is an open field's, make the field.
    pub(super) fn close_field_group(&mut self) {
        if self.open_fields.last().is_none_or(|f| f.depth != self.depth()) {
            return;
        }
        let Some(f) = self.open_fields.pop() else { return };
        if !f.result || f.para != self.para_seq {
            return;
        }
        let instr = self.fields.get(f.index).map_or("", |s| s.trim());
        if !page_dependent(instr) {
            return;
        }
        let Some((result, fmt)) = result_text(&self.para.inlines, &f, &self.st.fmt) else { return };
        let field = Inline::Field { instr: instr.to_string(), result, fmt, locked: f.locked };
        let inlines = &mut self.para.inlines;
        inlines.truncate(f.start);
        if let (Some(n), Some(Inline::Text(t, _))) = (f.tail_from, inlines.last_mut()) {
            t.truncate(n);
        }
        inlines.push(field);
        self.para_has = true;
    }
}

/// A field whose result the layout works out from the page it lands on. `STYLEREF` is not one
/// here: the layout looks its style up by name, and a running head whose style the RTF names
/// differently would show an error instead of Word's text.
fn page_dependent(instr: &str) -> bool {
    let name = instr.split_whitespace().next().unwrap_or("");
    ["PAGE", "NUMPAGES", "SECTIONPAGES", "SECTION", "PAGEREF"].iter().any(|n| name.eq_ignore_ascii_case(n))
}

/// The text the field's result added to the paragraph and its formatting, when that is plain
/// text in one formatting with no tracked change; `None` otherwise. An empty result takes the
/// field group's formatting.
fn result_text(inlines: &[Inline], f: &OpenField, group: &Fmt) -> Option<(String, Fmt)> {
    if inlines.len() < f.start {
        return None;
    }
    let mut runs: Vec<(&str, &Fmt)> = Vec::new();
    if let Some(n) = f.tail_from {
        let Some(Inline::Text(t, fmt)) = f.start.checked_sub(1).and_then(|i| inlines.get(i)) else { return None };
        let tail = t.get(n..)?;
        if !tail.is_empty() {
            runs.push((tail, fmt));
        }
    }
    for i in &inlines[f.start..] {
        match i {
            Inline::Text(t, fmt) => runs.push((t, fmt)),
            _ => return None,
        }
    }
    match runs.as_slice() {
        [] => Some((String::new(), group.clone())),
        [(t, fmt)] if fmt.ins.is_none() && fmt.del.is_none() && fmt.fmt_change.is_none() => Some(((*t).to_string(), (*fmt).clone())),
        _ => None,
    }
}
