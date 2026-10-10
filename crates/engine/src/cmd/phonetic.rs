//! Home › Font › Phonetic Guide: ruby text (furigana, pinyin…) over base text, kept as one
//! [`InlineObject::Ruby`] in the paragraph.

use serde_json::{Value, json};
use wordcraft_doc::para::{OBJ, RubyAlign, ruby_offset, ruby_raise};
use wordcraft_doc::{InlineObject, Pos};

use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Longest base or ruby text, characters.
const MAX_CHARS: usize = 255;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("format.phonetic", "Phonetic Guide", "Home › Font", phonetic).params(
            r#"{"ruby"?: string | null, "align"?: "center|distributeLetter|distributeSpace|left|right|rightVertical", "size"?: number, "offset"?: number}"#,
        ),
    ]
}

/// Sets the selected text's ruby (one group over the whole selection, or the phonetic guide the
/// selection is or the caret touches); `null` or `""` removes it. Without `ruby`, opens the
/// dialog and returns the selection's phonetic guide (`base`, `ruby`, `align`, `size`, `offset`).
fn phonetic(s: &mut Session, v: &Value) -> CmdResult {
    let Some(rv) = v.get("ruby") else {
        s.ui_requests.push(json!({"open": "phonetic"}));
        return Ok(state(s));
    };
    let text: String = match rv {
        Value::Null => String::new(),
        Value::String(t) => t.chars().filter(|c| !c.is_control() && *c != OBJ).take(MAX_CHARS).collect(),
        _ => return Err(CmdError::Params("`ruby` must be a string or null".into())),
    };
    let text = text.trim().to_string();
    let align = match p::str(v, "align") {
        Some(a) => Some(RubyAlign::from_ooxml(a).ok_or_else(|| CmdError::Params(format!("unknown alignment `{a}`")))?),
        None => None,
    };
    let size = p::f32(v, "size").map(|x| x.clamp(1.0, 1584.0));
    let offset = p::f32(v, "offset").map(|x| x.clamp(-1584.0, 1584.0));
    if let Some(at) = ruby_at(s) {
        let end = Pos { off: at.off + OBJ.len_utf8(), ..at.clone() };
        let bsz = base_size(s, &at);
        let para = s.doc.para_mut(at.story, &at.path)?;
        let props = para.props_of_char(at.off).clone();
        match para.object_at_mut(at.off) {
            Some(InlineObject::Ruby { base, .. }) if text.is_empty() => {
                // Remove: the base text stays, in the ruby's formatting.
                let base = base.clone();
                s.doc.delete_range(&at, &end)?;
                let stop = s.doc.insert_text(&at, &base, &props)?;
                s.sel = Selection { anchor: at, focus: stop };
                return Ok(state(s));
            }
            Some(InlineObject::Ruby { ruby, align: al, size: sz, raise, base_size, .. }) => {
                let bsz = if *base_size > 0.0 { *base_size } else { bsz };
                let keep = ruby_offset(*raise, bsz, *sz);
                *ruby = text;
                *al = align.unwrap_or(*al);
                *sz = size.unwrap_or(*sz);
                *raise = ruby_raise(offset.unwrap_or(keep), bsz, *sz).clamp(0.0, 1584.0);
                *base_size = bsz;
            }
            _ => return Err(CmdError::Failed("no phonetic guide at the selection".into())),
        }
        para.touch();
        s.sel = Selection { anchor: at, focus: end };
        return Ok(state(s));
    }
    if text.is_empty() {
        return Err(CmdError::Failed("no phonetic guide at the selection".into()));
    }
    let (a, b) = s.sel.ordered();
    if a == b || a.story != b.story || a.path != b.path {
        return Err(CmdError::Failed("select the text, within one paragraph, to set a phonetic guide over".into()));
    }
    let para = s.doc.para_at(&a).ok_or_else(|| CmdError::Failed("no paragraph at the selection".into()))?;
    let base = para.text.get(a.off..b.off).unwrap_or("").to_string();
    if base.is_empty() || base.chars().any(|c| c == OBJ || c.is_control()) {
        return Err(CmdError::Failed("a phonetic guide goes over plain text (no objects, tabs or breaks)".into()));
    }
    if base.chars().count() > MAX_CHARS {
        return Err(CmdError::Failed(format!("a phonetic guide covers at most {MAX_CHARS} characters")));
    }
    let props = para.props_of_char(a.off).clone();
    let bsz = base_size(s, &a);
    let size = size.unwrap_or_else(|| default_size(bsz));
    let raise = ruby_raise(offset.unwrap_or(0.0), bsz, size).clamp(0.0, 1584.0);
    let obj = InlineObject::Ruby {
        base,
        ruby: text,
        align: align.unwrap_or_default(),
        size,
        raise,
        base_size: bsz,
        lang: String::new(),
        props: Default::default(),
    };
    s.doc.delete_range(&a, &b)?;
    let end = s.doc.insert_object(&a, obj, &props)?;
    s.sel = Selection { anchor: a, focus: end };
    Ok(state(s))
}

/// The phonetic guide the selection is (exactly one ruby object) or the caret is next to.
fn ruby_at(s: &Session) -> Option<Pos> {
    let (a, b) = s.sel.ordered();
    let para = s.doc.para_at(&a)?;
    let ruby = |off: usize| matches!(para.object_at(off), Some(InlineObject::Ruby { .. })).then(|| Pos { off, ..a.clone() });
    let w = OBJ.len_utf8();
    if a == b {
        return ruby(a.off).or_else(|| a.off.checked_sub(w).and_then(ruby));
    }
    (a.story == b.story && a.path == b.path && b.off == a.off.saturating_add(w)).then(|| ruby(a.off)).flatten()
}

/// The resolved font size of the character at `at`, points.
fn base_size(s: &Session, at: &Pos) -> f32 {
    let Some(para) = s.doc.para_at(at) else { return 10.5 };
    let size = s.doc.styles.resolve_char(para.props.style.as_deref(), para.props_of_char(at.off)).size;
    if size.is_finite() { size.clamp(1.0, 1584.0) } else { 10.5 }
}

/// Ruby half the base size, rounded down to a half point (10.5 pt text gets 5 pt ruby).
fn default_size(base: f32) -> f32 {
    (base.floor() / 2.0).max(1.0)
}

/// What the Phonetic Guide shows for the selection: its ruby, or the selected text with defaults.
pub fn state(s: &Session) -> Value {
    if let Some(at) = ruby_at(s)
        && let Some(InlineObject::Ruby { base, ruby, align, size, raise, base_size: bs, .. }) = s.doc.para_at(&at).and_then(|p| p.object_at(at.off))
    {
        let bs = if *bs > 0.0 { *bs } else { base_size(s, &at) };
        // `+ 0.0` turns a rounded -0 into 0.
        let offset = (ruby_offset(*raise, bs, *size) * 10.0).round() / 10.0 + 0.0;
        return json!({"base": base, "ruby": ruby, "align": align.ooxml(), "size": size, "offset": offset, "existing": true});
    }
    let (a, b) = s.sel.ordered();
    let base = if a.story == b.story && a.path == b.path { s.selected_text() } else { String::new() };
    let base: String = base.chars().filter(|c| !c.is_control() && *c != OBJ).take(MAX_CHARS).collect();
    json!({"base": base, "ruby": "", "align": "center", "size": default_size(base_size(s, &a)), "offset": 0.0, "existing": false})
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::{Document, InlineObject, Paragraph, StoryRef};

    use crate::Session;

    fn session(text: &str) -> Session {
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(Paragraph::with_text(text, Default::default()))];
        Session::new(d)
    }

    fn para(s: &Session) -> Paragraph {
        s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).cloned().unwrap()
    }

    #[test]
    fn phonetic_guide_applies_edits_removes_and_undoes() {
        let mut s = session("I read 漢字 daily.");
        s.run("select.text", &json!({"text": "漢字"})).unwrap();
        let r = s.run("format.phonetic", &json!({"ruby": "かんじ", "align": "distributeSpace"})).unwrap();
        assert_eq!((r["base"].as_str(), r["ruby"].as_str(), r["existing"].as_bool()), (Some("漢字"), Some("かんじ"), Some(true)));
        let p = para(&s);
        assert_eq!(p.plain_text(), "I read 漢字 daily.");
        let Some(InlineObject::Ruby { ruby, size, align, .. }) = p.objects.first() else { panic!("{:?}", p.objects) };
        assert_eq!((ruby.as_str(), *size, align.ooxml()), ("かんじ", 6.0, "distributeSpace"));
        // The selection is the ruby: editing it keeps the base, changes the reading and offset.
        let r = s.run("format.phonetic", &json!({"ruby": "カンジ", "offset": 2})).unwrap();
        assert_eq!((r["ruby"].as_str(), r["offset"].as_f64()), (Some("カンジ"), Some(2.0)));
        assert_eq!(para(&s).objects.len(), 1);
        // Remove: the base text is plain text again, selected.
        s.run("format.phonetic", &json!({"ruby": null})).unwrap();
        assert!(para(&s).objects.is_empty());
        assert_eq!(s.selected_text(), "漢字");
        // Undo brings the edited ruby back, then the first one, then the plain text.
        s.run("edit.undo", &json!({})).unwrap();
        assert!(matches!(para(&s).objects.first(), Some(InlineObject::Ruby { ruby, .. }) if ruby == "カンジ"));
        s.run("edit.undo", &json!({})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        assert!(para(&s).objects.is_empty());
        assert_eq!(para(&s).plain_text(), "I read 漢字 daily.");
        // Without `ruby` it reports the selection (and asks the UI for the dialog).
        s.run("select.text", &json!({"text": "read"})).unwrap();
        let r = s.run("format.phonetic", &json!({})).unwrap();
        assert_eq!((r["base"].as_str(), r["existing"].as_bool()), (Some("read"), Some(false)));
        assert!(s.run("format.phonetic", &json!({"ruby": "x", "align": "sideways"})).is_err());
    }
}
