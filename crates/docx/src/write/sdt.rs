//! Content controls → `w:sdt` (ECMA-376 §17.5.2; check boxes and repeating sections as the
//! [MS-DOCX] `w14`/`w15` extensions).
//!
//! Controls are markers in the model ([`wordcraft_doc::control`]); the file wants them as
//! elements around runs (inline) or around whole paragraphs and tables (block level). [`plan`]
//! decides, per block list, which controls are written where so that the XML always nests.

use std::collections::HashSet;

use wordcraft_doc::Blocks;
use wordcraft_doc::control::{ContentControl, ControlEvent, ControlKind, control_events, pair_events};
use wordcraft_doc::para::OBJ;

use crate::xml::W;

/// Where a block list's controls are written.
#[derive(Default)]
pub struct Plan<'a> {
    /// Controls opening before block `i` (outermost first).
    pub opens: Vec<Vec<&'a ContentControl>>,
    /// Controls closing after block `i`.
    pub closes: Vec<u32>,
    /// Paragraph markers written inline, as (block, byte offset).
    pub inline: HashSet<(usize, usize)>,
}

impl Plan<'_> {
    /// Whether a block-level control is around block `i`.
    pub fn covers(&self, i: usize) -> bool {
        let mut open = 0i64;
        for k in 0..=i {
            open += self.opens.get(k).map(|v| v.len() as i64).unwrap_or(0);
            if k < i {
                open -= i64::from(self.closes.get(k).copied().unwrap_or(0));
            }
        }
        open > 0
    }
    /// The byte offsets of paragraph `i`'s markers that are written inline.
    pub fn inline_in(&self, i: usize) -> Vec<usize> {
        let mut v: Vec<usize> = self.inline.iter().filter(|(b, _)| *b == i).map(|(_, o)| *o).collect();
        v.sort_unstable();
        v
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Inline,
    Block,
}

/// Plan the controls of a block list (not looking into cells: they have their own lists).
/// Markers without a partner are not written.
pub fn plan(bl: &Blocks) -> Plan<'_> {
    let n = bl.len();
    let mut out = Plan { opens: vec![Vec::new(); n], closes: vec![0; n], inline: HashSet::new() };
    let events = control_events(bl);
    if events.is_empty() {
        return out;
    }
    let (pairs, _) = pair_events(&events);
    // Ranges in the order they open (parents before children).
    let mut ranges: Vec<(usize, usize)> = pairs.iter().map(|(o, c, _)| (*o, *c)).collect();
    ranges.sort_unstable();
    // Parent of each range: the innermost one opened before it and closed after it.
    let mut parent: Vec<Option<usize>> = vec![None; ranges.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (k, (o, _)) in ranges.iter().enumerate() {
        while let Some(&top) = stack.last() {
            if ranges.get(top).is_some_and(|(_, tc)| tc < o) {
                stack.pop();
            } else {
                break;
            }
        }
        if let Some(slot) = parent.get_mut(k) {
            *slot = stack.last().copied();
        }
        stack.push(k);
    }
    let para = |b: usize| bl.get(b).and_then(|x| x.as_para());
    // Only control starts before `off` in the paragraph / only control ends after it.
    let at_start = |b: usize, off: usize| {
        para(b).is_some_and(|p| {
            p.text.get(..off).is_some_and(|t| t.chars().all(|c| c == OBJ))
                && (0..off).step_by(OBJ.len_utf8()).all(|o| matches!(p.object_at(o), Some(wordcraft_doc::InlineObject::ControlStart { .. })))
        })
    };
    let at_end = |b: usize, off: usize| {
        para(b).is_some_and(|p| {
            let from = off + OBJ.len_utf8();
            p.text.get(from..).is_some_and(|t| t.chars().all(|c| c == OBJ))
                && (from..p.len()).step_by(OBJ.len_utf8()).all(|o| matches!(p.object_at(o), Some(wordcraft_doc::InlineObject::ControlEnd)))
        })
    };
    let mut mode = vec![Mode::Inline; ranges.len()];
    let mut span: Vec<(usize, usize)> = vec![(0, 0); ranges.len()];
    for (k, (o, c)) in ranges.iter().enumerate() {
        let (Some(ControlEvent::Open { block: sb, off: so, control }), Some(e)) = (events.get(*o), events.get(*c)) else { continue };
        let (eb, eo) = (e.block(), e.off());
        let m = match (so, eo) {
            (Some(so), Some(eo)) if *sb == eb => {
                let parent_inline = parent.get(k).copied().flatten().and_then(|p| mode.get(p)).is_some_and(|m| *m == Mode::Inline);
                if !parent_inline && control.block && at_start(*sb, *so) && at_end(eb, eo) { Mode::Block } else { Mode::Inline }
            }
            _ => Mode::Block,
        };
        if let Some(slot) = mode.get_mut(k) {
            *slot = m;
        }
        if let Some(slot) = span.get_mut(k) {
            *slot = (*sb, eb);
        }
    }
    // Block-level controls nest by block: one that opens in the block another (not its
    // ancestor) closes after starts with the next block instead.
    for k in 0..ranges.len() {
        if mode.get(k) != Some(&Mode::Block) {
            continue;
        }
        let Some(&(o, _)) = ranges.get(k) else { continue };
        let Some(&(sb, eb)) = span.get(k) else { continue };
        let clash = ranges
            .iter()
            .enumerate()
            .any(|(q, (_, qc))| q != k && *qc < o && mode.get(q) == Some(&Mode::Block) && span.get(q).is_some_and(|s| s.1 == sb));
        if clash {
            let to = (sb + 1).min(eb);
            if let Some(s) = span.get_mut(k) {
                s.0 = to;
            }
        }
    }
    for (k, (o, c)) in ranges.iter().enumerate() {
        let Some(ControlEvent::Open { control, .. }) = events.get(*o) else { continue };
        match mode.get(k) {
            Some(Mode::Block) => {
                let Some(&(sb, eb)) = span.get(k) else { continue };
                if let Some(v) = out.opens.get_mut(sb) {
                    v.push(control);
                }
                if let Some(x) = out.closes.get_mut(eb) {
                    *x += 1;
                }
            }
            _ => {
                for e in [events.get(*o), events.get(*c)].into_iter().flatten() {
                    if let Some(off) = e.off() {
                        out.inline.insert((e.block(), off));
                    }
                }
            }
        }
    }
    out
}

fn on(w: &mut W, name: &str, v: bool) {
    if v {
        w.empty(name, &[]);
    }
}

/// `w:sdtPr` (and `w:sdtEndPr`), then open `w:sdtContent`.
pub fn open(w: &mut W, c: &ContentControl) {
    w.open("w:sdt", &[]);
    w.open("w:sdtPr", &[]);
    w.raw(&c.rpr_xml);
    if !c.title.is_empty() {
        w.val("w:alias", &c.title);
    }
    if !c.tag.is_empty() {
        w.val("w:tag", &c.tag);
    }
    if let Some(id) = c.id {
        w.val("w:id", &id.to_string());
    }
    if let Some(l) = c.lock.ooxml() {
        w.val("w:lock", l);
    }
    if !c.placeholder.is_empty() {
        w.open("w:placeholder", &[]);
        w.val("w:docPart", &c.placeholder);
        w.close("w:placeholder");
    }
    on(w, "w:temporary", c.temporary);
    on(w, "w:showingPlcHdr", c.showing_placeholder);
    // Kept elements of the schema's sequence before the type, then the type, then the rest.
    let early = |x: &&String| {
        ["<w:dataBinding", "<w:label", "<w:tabIndex"].iter().any(|n| x.strip_prefix(n).is_some_and(|rest| rest.starts_with([' ', '/', '>'])))
    };
    for x in c.extra.iter().filter(early) {
        w.raw(x);
    }
    kind(w, &c.kind);
    for x in c.extra.iter().filter(|x| !early(x)) {
        w.raw(x);
    }
    w.close("w:sdtPr");
    w.raw(&c.end_pr_xml);
    w.open("w:sdtContent", &[]);
}

/// Close `w:sdtContent` and `w:sdt`.
pub fn close(w: &mut W) {
    w.raw("</w:sdtContent></w:sdt>");
}

fn kind(w: &mut W, k: &ControlKind) {
    match k {
        ControlKind::RichText => {}
        ControlKind::Text { multi_line } => {
            if *multi_line {
                w.empty("w:text", &[("w:multiLine", "1")]);
            } else {
                w.empty("w:text", &[]);
            }
        }
        ControlKind::CheckBox { checked, checked_char, checked_font, unchecked_char, unchecked_font } => {
            w.open("w14:checkbox", &[]);
            w.empty("w14:checked", &[("w14:val", if *checked { "1" } else { "0" })]);
            for (tag, ch, font) in [("w14:checkedState", checked_char, checked_font), ("w14:uncheckedState", unchecked_char, unchecked_font)] {
                let hex = format!("{:04X}", u32::from(*ch));
                if font.is_empty() {
                    w.empty(tag, &[("w14:val", &hex)]);
                } else {
                    w.empty(tag, &[("w14:val", &hex), ("w14:font", font)]);
                }
            }
            w.close("w14:checkbox");
        }
        ControlKind::ComboBox { items, last_value } | ControlKind::DropDown { items, last_value } => {
            let tag = if matches!(k, ControlKind::ComboBox { .. }) { "w:comboBox" } else { "w:dropDownList" };
            if last_value.is_empty() {
                w.open(tag, &[]);
            } else {
                w.open(tag, &[("w:lastValue", last_value)]);
            }
            for i in items {
                w.empty("w:listItem", &[("w:displayText", &i.display), ("w:value", i.value())]);
            }
            w.close(tag);
        }
        ControlKind::Date { full_date, format, lid, calendar, store_as } => {
            if full_date.is_empty() {
                w.open("w:date", &[]);
            } else {
                w.open("w:date", &[("w:fullDate", full_date)]);
            }
            for (tag, v) in [("w:dateFormat", format), ("w:lid", lid), ("w:storeMappedDataAs", store_as), ("w:calendar", calendar)] {
                if !v.is_empty() {
                    w.val(tag, v);
                }
            }
            w.close("w:date");
        }
        ControlKind::Picture => w.empty("w:picture", &[]),
        ControlKind::Gallery { list, gallery, category, unique } => {
            let tag = if *list { "w:docPartList" } else { "w:docPartObj" };
            w.open(tag, &[]);
            if !gallery.is_empty() {
                w.val("w:docPartGallery", gallery);
            }
            if !category.is_empty() {
                w.val("w:docPartCategory", category);
            }
            on(w, "w:docPartUnique", *unique);
            w.close(tag);
        }
        ControlKind::RepeatingSection { title, no_insert_delete } => {
            w.open("w15:repeatingSection", &[]);
            if !title.is_empty() {
                w.val("w15:sectionTitle", title);
            }
            on(w, "w15:doNotAllowInsertDeleteSection", *no_insert_delete);
            w.close("w15:repeatingSection");
        }
        ControlKind::RepeatingSectionItem => w.empty("w15:repeatingSectionItem", &[]),
    }
}
