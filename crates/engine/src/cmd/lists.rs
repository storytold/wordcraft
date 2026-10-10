//! List definitions: Define New Multilevel List (`list.define`) and reading one back
//! (`list.get`). A definition is Word's abstract numbering: nine levels, each with its number
//! format, style, start, restart rule, linked paragraph style, alignment, indents, what follows
//! the number and the number's font.

use serde_json::{Value, json};
use wordcraft_doc::StoryRef;
use wordcraft_doc::numbering::{AbstractNum, Level, LevelSuffix, ListKind, Num, levels_for};
use wordcraft_doc::props::{Align, NumRef, Rgb, TextColor};
use wordcraft_doc::section::NumFormat;

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The most levels a list has (Word's nine).
pub const LEVELS: usize = 9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("list.define", "Define New Multilevel List", "Home › Paragraph › Multilevel List", define).params(
            r#"{"levels": [{"format"?: "decimal|lowerLetter|upperLetter|lowerRoman|upperRoman|decimalZero|ordinal|bullet|none", "text"?: "%1.%2." (%n = level n's number; the bullet character for bullet), "start"?: n, "restartAfter"?: 0-8 (0 = never; omit = the level above), "style"?: paragraph style linked to the level | null, "align"?: "left|center|right", "alignedAt"?: pt, "indentAt"?: pt, "follow"?: "tab|space|nothing", "tabAt"?: pt | null, "font"?: string | null, "bold"?: bool, "italic"?: bool, "color"?: "RRGGBB" | null, "legal"?: bool}, …] (index = level, up to 9; fields left out keep the base list's), "num"?: list id to change in place (default: a new list), "apply"?: bool (default true: number the selected paragraphs), "name"?: string}"#,
        ),
        CommandSpec::new("list.get", "List Definition", "Home › Paragraph › Multilevel List", get)
            .params(r#"{"num"?: list id (default: the caret's list)} → {"num", "levels": [as list.define]}"#)
            .pure(),
    ]
}

/// A level as `list.define` takes it.
pub fn level_json(l: &Level) -> Value {
    let color = match l.chr.color {
        Some(TextColor::Rgb(c)) => json!(c.hex()),
        _ => Value::Null,
    };
    json!({
        "format": l.format.ooxml(),
        "text": l.text,
        "start": l.start,
        "restartAfter": if l.restart { l.restart_after.map(Value::from).unwrap_or(Value::Null) } else { json!(0) },
        "style": l.style,
        "align": match l.align {
            Align::Center => "center",
            Align::Right => "right",
            _ => "left",
        },
        "alignedAt": l.indent - l.hanging,
        "indentAt": l.indent,
        "follow": match l.suffix {
            LevelSuffix::Tab => "tab",
            LevelSuffix::Space => "space",
            LevelSuffix::Nothing => "nothing",
        },
        "tabAt": l.tab,
        "font": l.chr.font,
        "bold": l.chr.bold.unwrap_or(false),
        "italic": l.chr.italic.unwrap_or(false),
        "color": color,
        "legal": l.legal,
    })
}

/// `base` changed by the fields `v` gives.
pub fn level_from(mut l: Level, i: usize, v: &Value) -> Level {
    let pt = |k: &str| p::f32(v, k).map(|x| x.clamp(-1584.0, 1584.0));
    if let Some(f) = p::str(v, "format") {
        l.format = NumFormat::from_ooxml(f);
    }
    if let Some(t) = p::str(v, "text") {
        l.text = t.chars().take(64).collect();
    }
    if let Some(s) = p::u64(v, "start") {
        l.start = s.min(32_767) as u32;
    }
    match v.get("restartAfter") {
        Some(Value::Null) => {
            l.restart = true;
            l.restart_after = None;
        }
        Some(r) => {
            if let Some(k) = r.as_u64() {
                l.restart = k != 0;
                // Restarting after the level just above is the default.
                l.restart_after = Some(k.min(8) as u8).filter(|k| *k != 0 && *k as usize != i);
            }
        }
        None => {}
    }
    match v.get("style") {
        Some(Value::Null) => l.style = None,
        Some(Value::String(s)) => l.style = Some(s.chars().take(253).collect()).filter(|s: &String| !s.is_empty()),
        _ => {}
    }
    if let Some(a) = p::str(v, "align") {
        l.align = match a {
            "center" => Align::Center,
            "right" => Align::Right,
            _ => Align::Left,
        };
    }
    let aligned = pt("alignedAt").unwrap_or(l.indent - l.hanging);
    if let Some(ind) = pt("indentAt") {
        l.indent = ind;
    }
    l.hanging = l.indent - aligned;
    if let Some(f) = p::str(v, "follow") {
        l.suffix = match f {
            "space" => LevelSuffix::Space,
            "nothing" => LevelSuffix::Nothing,
            _ => LevelSuffix::Tab,
        };
    }
    match v.get("tabAt") {
        Some(Value::Null) => l.tab = None,
        Some(_) => l.tab = pt("tabAt").or(l.tab),
        None => {}
    }
    match v.get("font") {
        Some(Value::Null) => l.chr.font = None,
        Some(Value::String(s)) => l.chr.font = Some(s.chars().take(64).collect()).filter(|s: &String| !s.trim().is_empty()),
        _ => {}
    }
    if let Some(b) = p::bool(v, "bold") {
        l.chr.bold = Some(b);
    }
    if let Some(b) = p::bool(v, "italic") {
        l.chr.italic = Some(b);
    }
    match v.get("color") {
        Some(Value::Null) => l.chr.color = None,
        Some(Value::String(s)) => l.chr.color = Rgb::parse(s).map(TextColor::Rgb).or(l.chr.color),
        _ => {}
    }
    if let Some(b) = p::bool(v, "legal") {
        l.legal = b;
    }
    l
}

/// The caret's list, if it is in one.
pub fn caret_list(s: &Session) -> Option<u32> {
    let para = s.doc.para_at(&s.sel.focus)?;
    para.props.numbering.or_else(|| s.doc.styles.resolve_para(&para.props).numbering).filter(|n| n.num != 0).map(|n| n.num)
}

/// The nine levels list `num` shows (its own level definitions over its abstract list's).
pub fn levels_of(s: &Session, num: u32) -> Option<Vec<Level>> {
    s.doc.numbering.num(num)?;
    Some((0..LEVELS).map(|i| s.doc.numbering.level(num, i as u8).cloned().unwrap_or_else(|| default_level(i))).collect())
}

/// Level `i` of a new multilevel list: 1. a. i.
fn default_level(i: usize) -> Level {
    levels_for(ListKind::Numbered).into_iter().nth(i).unwrap_or_default()
}

fn get(s: &mut Session, v: &Value) -> CmdResult {
    let num =
        p::u64(v, "num").map(|n| n.min(u32::MAX as u64) as u32).or_else(|| caret_list(s)).ok_or_else(|| CmdError::Failed("not in a list".into()))?;
    let levels = levels_of(s, num).ok_or_else(|| CmdError::Params("no such list".into()))?;
    Ok(json!({"num": num, "levels": levels.iter().map(level_json).collect::<Vec<_>>()}))
}

fn define(s: &mut Session, v: &Value) -> CmdResult {
    let given = v.get("levels").and_then(Value::as_array).ok_or_else(|| CmdError::Params("`levels` (array) is required".into()))?;
    let target = p::u64(v, "num").map(|n| n.min(u32::MAX as u64) as u32);
    let base = match target {
        Some(n) => levels_of(s, n).ok_or_else(|| CmdError::Params("no such list".into()))?,
        None => (0..LEVELS).map(default_level).collect(),
    };
    let levels: Vec<Level> = base
        .into_iter()
        .enumerate()
        .map(|(i, l)| match given.get(i) {
            Some(lv) if lv.is_object() => level_from(l, i, lv),
            _ => l,
        })
        .collect();
    let name = p::str(v, "name").map(|n| n.chars().take(64).collect::<String>()).filter(|n| !n.trim().is_empty());
    let num = match target {
        Some(n) => {
            // Change the list in place: every list sharing its definition follows.
            let aid = s.doc.numbering.num(n).map(|x| x.abstract_id).ok_or_else(|| CmdError::Params("no such list".into()))?;
            if let Some(a) = s.doc.numbering.abstracts.iter_mut().find(|a| a.id == aid) {
                a.levels = levels.clone();
                if name.is_some() {
                    a.name = name;
                }
            }
            if let Some(x) = s.doc.numbering.nums.iter_mut().find(|x| x.id == n) {
                x.level_overrides.clear();
            }
            let sharing: Vec<u32> = s.doc.numbering.nums.iter().filter(|x| x.abstract_id == aid).map(|x| x.id).collect();
            touch_lists(s, &sharing)?;
            n
        }
        None => {
            let aid = s.doc.numbering.abstracts.iter().map(|a| a.id.saturating_add(1)).max().unwrap_or(0);
            s.doc.numbering.abstracts.push(AbstractNum { id: aid, name, levels: levels.clone() });
            let nid = s.doc.numbering.nums.iter().map(|n| n.id.saturating_add(1)).max().unwrap_or(1).max(1);
            s.doc.numbering.nums.push(Num { id: nid, abstract_id: aid, ..Default::default() });
            nid
        }
    };
    // Levels linked to a paragraph style number every paragraph in that style.
    let mut linked = Vec::new();
    for (i, l) in levels.iter().enumerate() {
        let Some(id) = l.style.as_deref().map(|st| s.doc.styles.find(st).map(|x| x.id.clone()).unwrap_or_else(|| st.to_string())) else { continue };
        if let Some(st) = s.doc.styles.get_mut(&id) {
            st.para.numbering = Some(NumRef { num, level: i as u8 });
            linked.push(id);
        }
    }
    if !linked.is_empty() {
        touch_styles(s, &linked)?;
    }
    if p::bool(v, "apply").unwrap_or(true) {
        super::para::fmt(s, &|pp| {
            // Paragraphs already in a list keep their level.
            let level = pp.numbering.filter(|n| n.num != 0).map(|n| n.level).unwrap_or(0);
            pp.numbering = Some(NumRef { num, level });
            pp.indent_left = None;
            pp.indent_first = None;
            if pp.style.is_none() || pp.style.as_deref() == Some("Normal") {
                pp.style = Some("ListParagraph".into());
            }
        })?;
    }
    let mut r = sel_result(s)?;
    r["num"] = json!(num);
    Ok(r)
}

/// Lay out again every paragraph in one of `nums`.
fn touch_lists(s: &mut Session, nums: &[u32]) -> Result<(), CmdError> {
    touch_where(s, &|p| p.props.numbering.is_some_and(|n| nums.contains(&n.num)))
}

/// Lay out again every paragraph in one of `styles`.
fn touch_styles(s: &mut Session, styles: &[String]) -> Result<(), CmdError> {
    touch_where(s, &|p| p.props.style.as_ref().is_some_and(|st| styles.contains(st)))
}

/// Lay out again every paragraph, in every story, that `f` picks.
fn touch_where(s: &mut Session, f: &dyn Fn(&wordcraft_doc::Paragraph) -> bool) -> Result<(), CmdError> {
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for story in stories {
        for path in s.doc.para_paths(story) {
            if s.doc.para(story, &path).is_some_and(f) {
                s.doc.para_mut(story, &path)?.touch();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_layout::Placed;

    use crate::Session;

    fn labels(s: &mut Session) -> Vec<String> {
        let l = s.layout();
        l.pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| if let Placed::Lines { para, l0: 0, .. } = i { para.label.as_ref().map(|lb| lb.text.clone()) } else { None })
            .collect()
    }

    /// #328: a custom three-level list numbers paragraphs 1. / 1.1. / a) and survives a save to
    /// .docx and back with every level setting.
    #[test]
    fn a_defined_multilevel_list_numbers_and_round_trips_through_docx() {
        let mut s = Session::new(Document::new());
        for (i, t) in ["One", "Two", "Three"].iter().enumerate() {
            if i > 0 {
                s.run("text.newParagraph", &json!({})).unwrap();
            }
            s.run("text.insert", &json!({"text": t})).unwrap();
        }
        s.run("select.all", &json!({})).unwrap();
        let levels = json!([
            {"format": "decimal", "text": "%1.", "start": 1, "alignedAt": 0, "indentAt": 18, "follow": "tab", "tabAt": 24, "font": "Arial", "bold": true},
            {"format": "decimal", "text": "%1.%2.", "alignedAt": 18, "indentAt": 45, "follow": "space", "restartAfter": 1},
            {"format": "lowerLetter", "text": "%3)", "start": 1, "align": "right", "alignedAt": 54, "indentAt": 72, "follow": "nothing", "restartAfter": 0, "style": "Heading3"},
        ]);
        let r = s.run("list.define", &json!({"levels": levels, "name": "Spec"})).unwrap();
        let num = r["num"].as_u64().unwrap() as u32;
        // Second and third paragraphs one and two levels down.
        let paths = s.doc.para_paths(wordcraft_doc::StoryRef::Body);
        for (k, lvl) in [(1usize, 1u8), (2, 2)] {
            let p = s.doc.para_mut(wordcraft_doc::StoryRef::Body, &paths[k]).unwrap();
            p.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: lvl });
            p.touch();
        }
        s.relayout();
        assert_eq!(labels(&mut s), ["1.", "1.1.", "a)"]);

        let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        let mut b = Session::new(back);
        assert_eq!(labels(&mut b), ["1.", "1.1.", "a)"]);
        let got = b.run("list.get", &json!({"num": num})).unwrap();
        let want = s.run("list.get", &json!({"num": num})).unwrap();
        assert_eq!(got["levels"], want["levels"], "every level setting round-trips");
        let l = &got["levels"];
        assert_eq!((l[0]["tabAt"].as_f64(), l[0]["font"].as_str(), l[0]["bold"].as_bool()), (Some(24.0), Some("Arial"), Some(true)));
        assert_eq!((l[1]["follow"].as_str(), l[1]["alignedAt"].as_f64(), l[1]["indentAt"].as_f64()), (Some("space"), Some(18.0), Some(45.0)));
        assert_eq!((l[2]["restartAfter"].as_u64(), l[2]["align"].as_str(), l[2]["style"].as_str()), (Some(0), Some("right"), Some("Heading3")));
        assert_eq!(l[2]["follow"], "nothing");
        // Programmatic calls without levels are an error, not a dialog.
        assert!(s.run("list.define", &json!({})).is_err());
    }
}
