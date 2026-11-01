//! AutoFormat As You Type's "Format beginning of list item like the one before it": Enter at the
//! end of a list item gives the next item's lead-in, the text typed up to its first punctuation
//! character, the character formatting that the whole lead-in of the item before shares.

use std::sync::LazyLock;

use serde_json::Map;
use wordcraft_doc::props::CharProps;

use super::{CmdError, NumRef, OBJ, Pos, Session, Value, type_text_effect};
use crate::cmd::InsertEffect;

/// Unicode punctuation (general category P): where a lead-in ends.
static PUNCTUATION: LazyLock<Option<regex::Regex>> = LazyLock::new(|| regex::Regex::new(r"\p{P}").ok());

/// Most runs of the previous item's lead-in compared; a longer lead-in is not repeated.
const MAX_RUNS: usize = 256;

/// Character properties the lead-in never passes on: links, revision marks, proofing, the
/// symbol font and text direction stay as typing in the new item has them.
const KEPT: &[&str] = &["link", "ins", "del", "fmtChange", "symFont", "lang", "langBidi", "langEa", "noProof", "rtl", "hidden", "webHidden"];

/// A new list item whose lead-in is being typed with the previous item's lead-in formatting.
#[derive(Clone, Debug)]
pub(crate) struct LeadIn {
    document: u64,
    /// Where the caret is after the lead-in typed so far.
    at: Pos,
    /// The list and level of the new item.
    numbering: NumRef,
    /// The formatting typing uses until the first punctuation character.
    lead: CharProps,
    /// The formatting typing would have used without the option: what follows the punctuation.
    body: CharProps,
    /// The formatting picked for typing before the lead-in started, if any.
    before: Option<CharProps>,
}

/// The finished list item Enter splits, kept until the new item exists.
pub(super) struct Candidate {
    numbering: NumRef,
    runs: Vec<CharProps>,
}

fn is_punctuation(c: char) -> bool {
    let mut buf = [0u8; 4];
    PUNCTUATION.as_ref().is_some_and(|re| re.is_match(c.encode_utf8(&mut buf)))
}

/// Is the option on? Not while the document limits formatting to its styles: the lead-in's
/// formatting is direct formatting.
fn option_on(s: &Session) -> bool {
    let ac = &s.prefs.autocorrect;
    ac.enabled && ac.format_list_item_start && !(crate::cmd::protect::enforced(&s.doc) && s.doc.settings.protect_formatting)
}

/// The lead-in being typed at the caret, while typing still uses its formatting and the item
/// is still in the same list at the same level. Moving the caret, picking other formatting,
/// taking the item out of its list, turning the option off or limiting formatting ends it.
fn active(s: &Session) -> Option<&LeadIn> {
    let l = s.lead_in.as_ref()?;
    let numbered = || s.doc.para_at(&l.at).is_some_and(|p| s.doc.styles.resolve_para(&p.props).numbering == Some(l.numbering));
    (option_on(s)
        && l.document == s.document_id()
        && s.sel.is_collapsed()
        && s.sel.focus == l.at
        && s.pending.as_ref() == Some(&l.lead)
        && numbered())
    .then_some(l)
}

/// Is a lead-in being typed at the caret?
pub(super) fn is_active(s: &Session) -> bool {
    active(s).is_some()
}

/// After AutoCorrect rewrote the text before the caret during a lead-in: the lead-in goes on at
/// the caret, or ends when the paragraph now has a punctuation character before the caret.
pub(super) fn follow(s: &mut Session) {
    let f = s.sel.focus.clone();
    let punctuated = s.doc.para_at(&f).and_then(|p| p.text.get(..f.off)).is_some_and(|t| t.chars().any(is_punctuation));
    let Some(l) = s.lead_in.as_mut().filter(|l| l.at.story == f.story && l.at.path == f.path) else { return };
    l.at = f;
    if punctuated {
        finish(s);
    }
}

/// After Undo or Redo put back a lead-in with the text and selection of its step: the lead-in goes
/// on at the restored caret while that is collapsed in the lead-in's paragraph, typing still uses
/// its formatting and no punctuation character comes before the caret. Otherwise the next typing
/// ends it.
pub(crate) fn resync(s: &mut Session) {
    let f = s.sel.focus.clone();
    let punctuated = s.doc.para_at(&f).and_then(|p| p.text.get(..f.off)).is_none_or(|t| t.chars().any(is_punctuation));
    let document = s.document_id();
    let collapsed = s.sel.is_collapsed();
    let pending = s.pending.clone();
    if let Some(l) = s.lead_in.as_mut().filter(|l| {
        l.document == document && collapsed && !punctuated && l.at.story == f.story && l.at.path == f.path && pending.as_ref() == Some(&l.lead)
    }) {
        l.at = f;
    }
}

/// Put back the lead-in of a window's view. When the document changed since the view was taken,
/// the lead-in ends: typing in that window goes on with the formatting it would have had without
/// the option.
pub(crate) fn put_back(s: &mut Session, lead_in: Option<LeadIn>, current: bool) {
    match lead_in {
        Some(l) if !current => {
            drop_lead(s, l);
            s.lead_in = None;
        }
        l => s.lead_in = l,
    }
}

/// A lead-in that no longer applies at the caret: when typing still uses its formatting,
/// typing goes back to the formatting picked before the lead-in started.
fn drop_lead(s: &mut Session, l: LeadIn) {
    if s.pending.as_ref() == Some(&l.lead) {
        s.pending = l.before;
    }
}

/// End a lead-in: typing goes on with the formatting it would have had without the option.
pub(crate) fn finish(s: &mut Session) {
    if let Some(body) = active(s).map(|l| l.body.clone()) {
        s.pending = Some(body);
    } else if let Some(l) = s.lead_in.take() {
        drop_lead(s, l);
    }
    s.lead_in = None;
}

/// The list item at `at` when Enter at its end starts a new item whose lead-in can follow it:
/// the option is on, the item has text and the caret is at its end. Its lead-in runs from its
/// first character that is not white space to the character before its first punctuation
/// character (all of its text when it has none); deleted and hidden text and objects are left out.
pub(super) fn candidate(s: &Session, at: &Pos) -> Option<Candidate> {
    if !option_on(s) || !s.sel.is_collapsed() {
        return None;
    }
    let para = s.doc.para_at(at)?;
    let numbering = s.doc.styles.resolve_para(&para.props).numbering.filter(|n| n.num != 0)?;
    if para.is_empty() || at.off != para.len() {
        return None;
    }
    let mut runs: Vec<CharProps> = Vec::new();
    'runs: for (range, props) in para.run_ranges() {
        if props.del.is_some() || props.hidden == Some(true) {
            continue;
        }
        let mut used = false;
        for c in para.text.get(range)?.chars() {
            if is_punctuation(c) {
                if used {
                    runs.push(props.unlinked());
                }
                break 'runs;
            }
            if c == OBJ || (c.is_whitespace() && runs.is_empty() && !used) {
                continue;
            }
            used = true;
        }
        if used {
            runs.push(props.unlinked());
            if runs.len() > MAX_RUNS {
                return None;
            }
        }
    }
    (!runs.is_empty()).then_some(Candidate { numbering, runs })
}

/// `base` with every property the runs share, set or unset alike, except [`KEPT`] ones.
fn shared(runs: &[CharProps], base: &CharProps) -> Option<CharProps> {
    let object = |c: &CharProps| match serde_json::to_value(c) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    };
    let maps: Vec<Map<String, Value>> = runs.iter().map(object).collect::<Option<_>>()?;
    let first = maps.first()?;
    let mut out = object(base)?;
    let keys: std::collections::BTreeSet<String> = maps.iter().flat_map(|m| m.keys().cloned()).chain(out.keys().cloned()).collect();
    for k in keys.iter().filter(|k| !KEPT.contains(&k.as_str())) {
        let value = first.get(k);
        if maps.iter().all(|m| m.get(k) == value) {
            match value {
                Some(v) => out.insert(k.clone(), v.clone()),
                None => out.remove(k),
            };
        }
    }
    serde_json::from_value(Value::Object(out)).ok()
}

/// After Enter made the new item at the caret: when it is an empty item of the same list and
/// level and the lead-in's shared formatting differs from what typing would use, typing uses it.
pub(super) fn start(s: &mut Session, candidate: Option<Candidate>) {
    s.lead_in = None;
    let Some(c) = candidate else { return };
    let at = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&at) else { return };
    if !para.is_empty() || s.doc.styles.resolve_para(&para.props).numbering != Some(c.numbering) {
        return;
    }
    let body = s.typing_props();
    let Some(lead) = shared(&c.runs, &body).filter(|l| *l != body) else { return };
    let before = s.pending.replace(lead.clone());
    s.lead_in = Some(LeadIn { document: s.document_id(), at, numbering: c.numbering, lead, body, before });
}

/// Type `text`, all of it in the lead-in's formatting, and keep the lead-in at the caret.
fn type_lead(s: &mut Session, text: &str) -> Result<InsertEffect, CmdError> {
    let effect = type_text_effect(s, text)?;
    let at = s.sel.focus.clone();
    if let Some(l) = s.lead_in.as_mut() {
        l.at = at;
    }
    Ok(effect)
}

/// Type `text` like [`type_text_effect`]; during a lead-in, the text up to and including its first
/// punctuation character (or up to a paragraph break) takes the lead-in's formatting and the rest
/// the formatting typing would use without the option.
pub(crate) fn type_text_lead(s: &mut Session, text: &str) -> Result<InsertEffect, CmdError> {
    if active(s).is_none() {
        finish(s);
        return type_text_effect(s, text);
    }
    let end = text.char_indices().find_map(|(i, c)| match c {
        '\r' | '\u{2029}' => Some(i),
        c if is_punctuation(c) => Some(i + c.len_utf8()),
        _ => None,
    });
    let Some((head, rest)) = end.and_then(|i| Some((text.get(..i)?, text.get(i..)?))) else { return type_lead(s, text) };
    let mut effect = if head.is_empty() { InsertEffect::default() } else { type_lead(s, head)? };
    finish(s);
    if !rest.is_empty() {
        let more = type_text_effect(s, rest)?;
        effect.content_changed |= more.content_changed;
        effect.state_changed |= more.state_changed;
    }
    Ok(effect)
}
