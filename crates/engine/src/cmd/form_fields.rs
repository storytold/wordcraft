//! Filling in legacy form fields (`FORMTEXT`, `FORMCHECKBOX`, `FORMDROPDOWN`; Developer › Legacy
//! Tools), the fields older Word forms are built from (#3603). Each is one
//! [`InlineObject::Field`]: its `w:ffData` (`form`) defines it and holds a check box's state and a
//! drop-down's choice, its `result` holds a text field's text. Filling one in changes only those,
//! so it works while Restrict Editing's Filling in forms is enforced
//! ([`crate::cmd::protect`] treats an enabled legacy field like a fillable content control).
//!
//! Observed in Word: in a protected form, typing replaces a text field's text, clicking or Space
//! ticks or clears a check box, and a drop-down offers its entries. A text field takes at most its
//! `w:maxLength` characters; one cleared of its text shows five en spaces, as an unfilled field
//! does. A field whose `w:enabled` is off can't be filled in, nor can one Word computes (a
//! calculated, current date or current time text field), nor one inside a content control that
//! locks its contents. Filling a field in isn't tracked, so it's refused while Track Changes is on.

use serde_json::{Value, json};
use wordcraft_doc::legacy_form::{self, LegacyForm};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, Pos, StoryRef};

use super::{fields, parse_pos, pos_json};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

const LOC: &str = "Developer › Controls";

/// What an unfilled text field shows: five en spaces.
pub const BLANK: &str = "\u{2002}\u{2002}\u{2002}\u{2002}\u{2002}";

/// Longest text a field takes when its `w:maxLength` sets no limit (Word's own cap is 255 for
/// some types; this only bounds what a command can write).
const MAX_TEXT: usize = 32_767;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("developer.formFields", "Legacy Form Fields", LOC, list)
            .params(r#"{} → [{"pos", "kind": "text|checkBox|dropDown", "name", "value", "entries"?, "maxLength"?, "enabled"}] (every legacy form field, in document order)"#)
            .pure(),
        CommandSpec::new("developer.fillFormField", "Fill In Legacy Form Field", LOC, fill).params(
            r#"{"field"?: name (w:name), "at"?: pos (else the field at the selection), "text"?: string (a text field), "checked"?: bool (a check box; omitted, it toggles), "index"?: int | "entry"?: string (a drop-down)} → the field"#,
        ),
    ]
}

/// A legacy form field in a story.
struct FormField {
    at: Pos,
    form: LegacyForm,
    ff_data: String,
    result: String,
}

impl FormField {
    fn json(&self, doc: &Document) -> Value {
        let mut v = json!({
            "pos": pos_json(&self.at),
            "name": self.form.name(),
            "fillable": fillable_at(doc, &self.at, &self.ff_data),
        });
        match &self.form {
            LegacyForm::Text { max_len, .. } => {
                v["kind"] = json!("text");
                v["value"] = json!(if self.result == BLANK { "" } else { self.result.as_str() });
                if let Some(n) = max_len {
                    v["maxLength"] = json!(n);
                }
            }
            LegacyForm::CheckBox { checked, .. } => {
                v["kind"] = json!("checkBox");
                v["value"] = json!(checked);
            }
            LegacyForm::DropDown { entries, .. } => {
                v["kind"] = json!("dropDown");
                v["value"] = json!(self.form.shown_text().unwrap_or(""));
                v["entries"] = json!(entries);
            }
        }
        v
    }
}

/// Every legacy form field of a story, in document order.
fn story_fields(doc: &Document, story: StoryRef) -> Vec<FormField> {
    let mut out = Vec::new();
    for path in doc.para_paths(story) {
        let Some(para) = doc.para(story, &path) else { continue };
        if !para.objects.iter().any(|o| matches!(o, InlineObject::Field { form, .. } if !form.is_empty())) {
            continue;
        }
        for off in para.object_offsets() {
            let Some(o @ InlineObject::Field { form: ff_data, result, .. }) = para.object_at(off) else { continue };
            if let Some(form) = o.legacy_form() {
                out.push(FormField { at: Pos::new(story, path.clone(), off), form, ff_data: ff_data.clone(), result: result.clone() });
            }
        }
    }
    out
}

/// Does `o`'s own definition let it be filled in: a legacy form field, enabled, and not one Word
/// computes? Where it sits matters too ([`fillable_at`]); forms protection's walk
/// ([`crate::cmd::protect`]) knows that from the controls it is inside.
pub fn fillable_field(o: &InlineObject) -> bool {
    matches!(o, InlineObject::Field { form, .. } if legacy_form::fillable(form)) && o.legacy_form().is_some()
}

/// Can the legacy form field at `at` be filled in: its definition allows it, and no content
/// control around it locks its contents?
fn fillable_at(doc: &Document, at: &Pos, ff_data: &str) -> bool {
    legacy_form::fillable(ff_data) && doc.control_at(at).is_none_or(|r| r.control.lock.can_edit())
}

/// Does the document hold a legacy form field that can be filled in? Stops at the first.
pub fn any_fillable(doc: &Document) -> bool {
    doc.control_stories().into_iter().any(|st| {
        doc.para_paths(st).into_iter().any(|path| {
            doc.para(st, &path).is_some_and(|p| {
                p.object_offsets().into_iter().any(|off| {
                    p.object_at(off).is_some_and(|o| {
                        fillable_field(o) && doc.control_at(&Pos::new(st, path.clone(), off)).is_none_or(|r| r.control.lock.can_edit())
                    })
                })
            })
        })
    })
}

/// The legacy form fields that can be filled in, as spans (the field object), in story order:
/// what forms protection counts and what Find Next Region and Tab step through.
pub fn fillable_spans(doc: &Document) -> Vec<(Pos, Pos)> {
    doc.control_stories()
        .into_iter()
        .flat_map(|st| story_fields(doc, st))
        .filter(|f| fillable_at(doc, &f.at, &f.ff_data))
        .map(|f| {
            let mut end = f.at.clone();
            end.off += OBJ.len_utf8();
            (f.at, end)
        })
        .collect()
}

/// The legacy form field a command names: by `field` (its `w:name`, the selection's story first),
/// by `at`, or the one at the selection.
fn target(s: &Session, v: &Value) -> Result<FormField, CmdError> {
    if let Some(name) = p::str(v, "field") {
        let story = s.sel.focus.story;
        let order = std::iter::once(story).chain(s.doc.control_stories().into_iter().filter(|st| *st != story));
        for st in order {
            if let Some(f) = story_fields(&s.doc, st).into_iter().find(|f| f.form.name() == name) {
                return Ok(f);
            }
        }
        return Err(CmdError::Failed(format!("no legacy form field is called {name:?}")));
    }
    let at = match v.get("at") {
        Some(a) => Some(parse_pos(a).ok_or_else(|| CmdError::Params("`at` is a position".into()))?),
        None => None,
    };
    let wanted: Vec<Pos> = match at {
        Some(a) => vec![a],
        // The first legacy form field the selection holds (it may hold other fields too).
        None => fields::selected(s).into_iter().filter(|f| f.end.is_none()).map(|f| f.at).collect(),
    };
    let story = wanted.first().map_or(s.sel.focus.story, |a| a.story);
    story_fields(&s.doc, story).into_iter().find(|f| wanted.contains(&f.at)).ok_or_else(not_on_field)
}

fn not_on_field() -> CmdError {
    CmdError::Disabled("the selection isn't on a legacy form field".into())
}

fn list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(Value::Array(s.doc.control_stories().into_iter().flat_map(|st| story_fields(&s.doc, st)).map(|f| f.json(&s.doc)).collect()))
}

/// The text a text field takes: one line (line and paragraph breaks become spaces), no control
/// characters or object placeholders, at most `max_len` characters; empty shows as unfilled.
fn field_text(text: &str, max_len: Option<u32>) -> String {
    let limit = max_len.map_or(MAX_TEXT, |n| usize::try_from(n).unwrap_or(MAX_TEXT).min(MAX_TEXT));
    let clean: String = text
        .chars()
        .map(|c| if matches!(c, '\n' | '\r' | '\u{000B}' | '\u{2028}' | '\u{2029}') { ' ' } else { c })
        .filter(|c| (!c.is_control() || *c == '\t') && *c != OBJ)
        .take(limit)
        .collect();
    if clean.is_empty() { BLANK.to_string() } else { clean }
}

fn fill(s: &mut Session, v: &Value) -> CmdResult {
    let f = target(s, v)?;
    let name = f.form.name();
    if !legacy_form::enabled(&f.ff_data) {
        return Err(CmdError::Disabled(format!("the form field {name:?} can't be filled in (it isn't enabled)")));
    }
    if legacy_form::computed(&f.ff_data) {
        return Err(CmdError::Disabled(format!("the form field {name:?} is calculated or dated by Word, not typed")));
    }
    if !fillable_at(&s.doc, &f.at, &f.ff_data) {
        return Err(CmdError::Disabled(format!("the form field {name:?} is in a content control whose contents are locked")));
    }
    if s.doc.settings.track_changes {
        return Err(CmdError::Disabled("filling in a legacy form field isn't tracked yet: turn off Track Changes first".into()));
    }
    let (ff_data, result) = match &f.form {
        LegacyForm::Text { max_len, .. } => {
            let Some(text) = p::str(v, "text") else { return Err(CmdError::Params("a text field takes `text`".into())) };
            (f.ff_data.clone(), field_text(text, *max_len))
        }
        LegacyForm::CheckBox { checked, .. } => {
            let now = p::bool(v, "checked").unwrap_or(!checked);
            (
                legacy_form::with_checked(&f.ff_data, now).ok_or_else(|| CmdError::Failed("the check box's definition can't be read".into()))?,
                f.result.clone(),
            )
        }
        LegacyForm::DropDown { entries, .. } => {
            let index = if let Some(i) = p::u64(v, "index") {
                usize::try_from(i).ok().filter(|i| *i < entries.len())
            } else if let Some(e) = p::str(v, "entry") {
                entries.iter().position(|x| x == e)
            } else {
                return Err(CmdError::Params("a drop-down takes `index` or `entry`".into()));
            };
            let Some(index) = index else { return Err(CmdError::Params(format!("no such entry; the list has {}", entries.len()))) };
            let picked = entries.get(index).cloned().unwrap_or_default();
            (
                legacy_form::with_selected(&f.ff_data, index).ok_or_else(|| CmdError::Failed("the drop-down's definition can't be read".into()))?,
                picked,
            )
        }
    };
    let para = s.doc.para_mut(f.at.story, &f.at.path)?;
    if let Some(InlineObject::Field { form, result: r, dirty, .. }) = para.object_at_mut(f.at.off) {
        *form = ff_data;
        *r = result;
        *dirty = false;
    }
    para.touch();
    story_fields(&s.doc, f.at.story)
        .into_iter()
        .find(|x| x.at == f.at)
        .map(|x| x.json(&s.doc))
        .ok_or_else(|| CmdError::Failed("the field went missing".into()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wordcraft_doc::props::CharProps;

    pub(crate) const TEXT: &str =
        r#"<w:ffData><w:name w:val="Name"/><w:enabled/><w:calcOnExit w:val="0"/><w:textInput><w:maxLength w:val="10"/></w:textInput></w:ffData>"#;
    pub(crate) const BOX: &str = r#"<w:ffData><w:name w:val="Agree"/><w:enabled/><w:calcOnExit w:val="0"/><w:checkBox><w:sizeAuto/><w:default w:val="0"/></w:checkBox></w:ffData>"#;
    pub(crate) const LIST: &str = r#"<w:ffData><w:name w:val="Court"/><w:enabled/><w:calcOnExit w:val="0"/><w:ddList><w:listEntry w:val="Sydney"/><w:listEntry w:val="Adelaide"/></w:ddList></w:ffData>"#;

    fn field(instr: &str, form: &str, result: &str) -> InlineObject {
        InlineObject::Field { instr: instr.into(), result: result.into(), locked: false, dirty: false, code: false, form: form.into() }
    }

    /// "Name: ⌂ Agree: ⌂ Court: ⌂" with a text field, a check box and a drop-down.
    pub(crate) fn form_session() -> Session {
        let mut s = Session::new(Document::from_text("Name: \nAgree: \nCourt: \nLocked end."));
        let props = CharProps::default();
        let paths = s.doc.para_paths(StoryRef::Body);
        for (path, (instr, form, result)) in paths.iter().zip([("FORMTEXT", TEXT, BLANK), ("FORMCHECKBOX", BOX, ""), ("FORMDROPDOWN", LIST, "")]) {
            let end = s.doc.para(StoryRef::Body, path).unwrap().text.len();
            s.doc.insert_object(&Pos::new(StoryRef::Body, path.clone(), end), field(instr, form, result), &props).unwrap();
        }
        s
    }

    #[test]
    fn lists_and_fills_each_kind() {
        let mut s = form_session();
        let all = s.run("developer.formFields", &json!({})).unwrap();
        assert_eq!(all.as_array().map(Vec::len), Some(3));
        assert_eq!(all[0]["kind"], "text");
        assert_eq!(all[0]["value"], "", "an unfilled field reads as empty");
        assert_eq!(all[0]["maxLength"], 10);
        assert_eq!(all[1]["value"], false);
        assert_eq!(all[2]["entries"], json!(["Sydney", "Adelaide"]));

        // Text, cut to its maximum length, one line.
        let r = s.run("developer.fillFormField", &json!({"field": "Name", "text": "Ada\nLovelace-Byron"})).unwrap();
        assert_eq!(r["value"], "Ada Lovela");
        assert!(s.doc.shown_text(StoryRef::Body).contains("Name: Ada Lovela"));
        // Cleared, it shows as unfilled again.
        s.run("developer.fillFormField", &json!({"field": "Name", "text": ""})).unwrap();
        assert!(s.doc.shown_text(StoryRef::Body).contains(&format!("Name: {BLANK}")));

        // A check box toggles, or takes a value.
        assert_eq!(s.run("developer.fillFormField", &json!({"field": "Agree"})).unwrap()["value"], true);
        assert_eq!(s.run("developer.fillFormField", &json!({"field": "Agree"})).unwrap()["value"], false);
        assert_eq!(s.run("developer.fillFormField", &json!({"field": "Agree", "checked": true})).unwrap()["value"], true);

        // A drop-down, by entry or index; an entry it hasn't is refused.
        assert_eq!(s.run("developer.fillFormField", &json!({"field": "Court", "entry": "Adelaide"})).unwrap()["value"], "Adelaide");
        assert_eq!(s.run("developer.fillFormField", &json!({"field": "Court", "index": 0})).unwrap()["value"], "Sydney");
        assert!(s.run("developer.fillFormField", &json!({"field": "Court", "index": 2})).is_err());
        assert!(s.run("developer.fillFormField", &json!({"field": "Court", "entry": "Perth"})).is_err());
        assert!(s.run("developer.fillFormField", &json!({"field": "Court"})).is_err());

        // Wrong value for the kind, or no such field.
        assert!(s.run("developer.fillFormField", &json!({"field": "Name"})).is_err());
        assert!(s.run("developer.fillFormField", &json!({"field": "Nobody", "text": "x"})).is_err());
    }

    #[test]
    fn the_field_at_the_selection_or_a_position() {
        let mut s = form_session();
        let fields = story_fields(&s.doc, StoryRef::Body);
        // The caret just before the check box.
        s.sel = crate::Selection::caret(fields[1].at.clone());
        assert_eq!(s.run("developer.fillFormField", &json!({})).unwrap()["name"], "Agree");
        let at = pos_json(&fields[0].at);
        assert_eq!(s.run("developer.fillFormField", &json!({"at": at, "text": "Bo"})).unwrap()["value"], "Bo");
        // Away from any field: refused.
        s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
        assert!(s.run("developer.fillFormField", &json!({"checked": true})).is_err());
    }

    #[test]
    fn a_disabled_field_is_not_filled_in() {
        let mut s = form_session();
        let f = story_fields(&s.doc, StoryRef::Body).remove(1);
        let para = s.doc.para_mut(StoryRef::Body, &f.at.path).unwrap();
        if let Some(InlineObject::Field { form, .. }) = para.object_at_mut(f.at.off) {
            *form = form.replace("<w:enabled/>", r#"<w:enabled w:val="0"/>"#);
        }
        assert!(s.run("developer.fillFormField", &json!({"field": "Agree"})).is_err());
        assert_eq!(fillable_spans(&s.doc).len(), 2);
        assert_eq!(s.run("developer.formFields", &json!({})).unwrap()[1]["fillable"], false);
    }

    /// Word computes a calculated, current date or current time text field; nobody types it.
    #[test]
    fn a_computed_field_is_not_filled_in() {
        let mut s = form_session();
        let f = story_fields(&s.doc, StoryRef::Body).remove(0);
        let para = s.doc.para_mut(StoryRef::Body, &f.at.path).unwrap();
        if let Some(InlineObject::Field { form, .. }) = para.object_at_mut(f.at.off) {
            *form = form.replace("<w:textInput>", r#"<w:textInput><w:type w:val="calculated"/>"#);
        }
        let e = s.run("developer.fillFormField", &json!({"field": "Name", "text": "999"})).unwrap_err().to_string();
        assert!(e.contains("calculated"), "{e}");
        assert_eq!(fillable_spans(&s.doc).len(), 2);
    }

    /// Filling a field in isn't a tracked change yet, so it isn't done behind the reviewer's back.
    #[test]
    fn track_changes_refuses_filling_in() {
        let mut s = form_session();
        s.doc.settings.track_changes = true;
        assert!(s.run("developer.fillFormField", &json!({"field": "Agree"})).is_err());
        s.doc.settings.track_changes = false;
        assert!(s.run("developer.fillFormField", &json!({"field": "Agree"})).is_ok());
    }

    /// A selection holding another field first still finds the form field after it.
    #[test]
    fn the_selection_s_form_field_not_its_first_field() {
        let mut s = form_session();
        let f = story_fields(&s.doc, StoryRef::Body).remove(0);
        let date =
            InlineObject::Field { instr: "DATE".into(), result: "1/1/2026".into(), locked: false, dirty: false, code: false, form: String::new() };
        let start = Pos::new(StoryRef::Body, f.at.path.clone(), 0);
        s.doc.insert_object(&start, date, &CharProps::default()).unwrap();
        let mut end = f.at.clone();
        end.off += 2 * OBJ.len_utf8();
        s.sel = crate::Selection { anchor: start, focus: end };
        assert_eq!(s.run("developer.fillFormField", &json!({"text": "Ada"})).unwrap()["name"], "Name");
    }

    /// Filled-in values are saved where Word keeps them and read back the same; the rest of each
    /// field's `w:ffData` is kept as it was, and so is forms protection.
    #[test]
    fn filled_fields_survive_save() {
        let mut s = form_session();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        s.run("developer.fillFormField", &json!({"field": "Name", "text": "Ada & Bo"})).unwrap();
        s.run("developer.fillFormField", &json!({"field": "Agree", "checked": true})).unwrap();
        s.run("developer.fillFormField", &json!({"field": "Court", "index": 1})).unwrap();
        let bytes = crate::io::save_bytes("f.docx", &s.doc).unwrap();
        let mut t = Session::new(crate::io::open_bytes("f.docx", &bytes).unwrap());
        assert_eq!(t.doc.settings.protection.as_deref(), Some("forms"));
        let all = t.run("developer.formFields", &json!({})).unwrap();
        assert_eq!((all[0]["value"].clone(), all[1]["value"].clone(), all[2]["value"].clone()), (json!("Ada & Bo"), json!(true), json!("Adelaide")));
        let fields = story_fields(&t.doc, StoryRef::Body);
        assert_eq!(legacy_form::definition(&fields[1].ff_data), BOX);
        assert_eq!(legacy_form::definition(&fields[2].ff_data), LIST);
        assert_eq!(fields[0].ff_data, TEXT);
        // And it can be filled in again after reopening.
        t.run("developer.fillFormField", &json!({"field": "Agree"})).unwrap();
        assert_eq!(t.run("developer.formFields", &json!({})).unwrap()[1]["value"], false);
    }

    #[test]
    fn field_text_is_one_clean_line() {
        assert_eq!(field_text("a\r\nb\u{0}c\u{FFFC}d", None), "a  bcd");
        assert_eq!(field_text("abcdef", Some(3)), "abc");
        assert_eq!(field_text("", Some(3)), BLANK);
        assert_eq!(field_text("ééé", Some(2)), "éé", "characters, not bytes");
    }
}
