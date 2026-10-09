//! Equations: insert (empty, from the gallery, linear format or LaTeX), edit in place (type,
//! structures, symbols, caret movement, delete, build-up), convert between built-up and linear
//! form, inline/display, justification and equation numbers.

use serde_json::{Value, json};
use wordcraft_doc::math::{Arg, MNode, Math, MathJc, parse_linear, to_linear};
use wordcraft_doc::math_edit::{self as me, MathPos, TypeStyle};
use wordcraft_doc::math_latex::{parse_latex, to_latex};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{InlineObject, Pos};

use super::{delete_selection, sel_result, split_para};
use crate::{CmdError, CmdResult, CommandSpec, MathEdit, Selection, Session, p};

fn editing(s: &Session) -> Option<&'static str> {
    if s.math.is_none() { Some("not editing an equation") } else { None }
}

fn near_equation(s: &Session) -> Option<&'static str> {
    if s.math.is_some() || equation_at_caret(s).is_some() { None } else { Some("no equation at the caret") }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.equation", "Equation", "Insert › Symbols", insert)
            .key("Alt+=")
            .params(r#"{"linear"?: string, "latex"?: string, "builtin"?: "quadratic|binomial|fourier|pythagoras|area|expansion|taylor|trig1|trig2|euler|gaussian|normal|bayes|derivative", "display"?: bool, "edit"?: bool (default true)}"#),
        CommandSpec::new("equation.edit", "Edit Equation", "Equation", edit)
            .pure()
            .when(near_equation)
            .params(r#"{"pos"?: {"path": [[node, child]…], "off": n}}"#),
        CommandSpec::new("equation.exit", "Leave Equation", "Equation", exit).pure().when(editing).params(r#"{"before"?: bool}"#),
        CommandSpec::new("equation.type", "Type in Equation", "Equation", type_text).when(editing).params(r#"{"text": string}"#),
        CommandSpec::new("equation.insertStructure", "Insert Structure", "Equation › Structures", insert_structure)
            .params(r#"{"id"?: string (see equation.gallery), "linear"?: string, "latex"?: string}"#),
        CommandSpec::new("equation.insertSymbol", "Insert Symbol", "Equation › Symbols", insert_symbol).params(r#"{"char": string}"#),
        CommandSpec::new("equation.move", "Move in Equation", "Equation", move_caret)
            .pure()
            .when(editing)
            .params(r#"{"dir": "left|right|up|down|home|end|next|prev"}"#),
        CommandSpec::new("equation.backspace", "Delete Back in Equation", "Equation", |s, _| delete(s, true)).when(editing),
        CommandSpec::new("equation.delete", "Delete in Equation", "Equation", |s, _| delete(s, false)).when(editing),
        CommandSpec::new("equation.enter", "Enter in Equation", "Equation", enter).when(editing),
        CommandSpec::new("equation.convert", "Convert", "Equation › Conversions", convert)
            .when(near_equation_or_all)
            .params(r#"{"to": "professional|linear", "all"?: bool}"#),
        CommandSpec::new("equation.inputFormat", "Input Format", "Equation › Conversions", |s, v| {
            if let Some(f) = p::str(v, "format") {
                s.math_latex = f.eq_ignore_ascii_case("latex");
            }
            Ok(json!({"format": if s.math_latex { "latex" } else { "unicode" }}))
        })
        .pure()
        .params(r#"{"format": "unicode|latex"}"#),
        CommandSpec::new("equation.normalText", "Normal Text", "Equation › Tools", |s, v| {
            s.math_normal_text = p::bool(v, "value").unwrap_or(!s.math_normal_text);
            Ok(json!({"normalText": s.math_normal_text}))
        })
        .pure()
        .params(r#"{"value"?: bool}"#),
        CommandSpec::new("equation.display", "Change to Display", "Equation", set_display).when(near_equation).params(r#"{"value"?: bool}"#),
        CommandSpec::new("equation.justify", "Justification", "Equation", justify)
            .when(near_equation)
            .params(r#"{"jc": "centerGroup|center|left|right"}"#),
        CommandSpec::new("equation.number", "Equation Number", "Equation", number).when(near_equation).params(r#"{"value"?: bool}"#),
        CommandSpec::new("equation.set", "Set Equation", "Equation", set).when(near_equation).params(r#"{"linear"?: string, "latex"?: string}"#),
        CommandSpec::new("equation.get", "Equation Info", "Equation", get).pure().when(near_equation),
        CommandSpec::new("equation.gallery", "Equation Gallery", "Equation › Structures", gallery).pure(),
        CommandSpec::new("equation.structureActions", "Structure Actions", "Equation", structure_actions).pure().when(editing),
        CommandSpec::new("equation.structure", "Change Structure", "Equation", structure)
            .when(editing)
            .params(r#"{"action": string (see equation.structureActions), "level"?: n}"#),
    ]
}

fn near_equation_or_all(_: &Session) -> Option<&'static str> {
    None
}

/// The equation the caret is at: on its U+FFFC or just after it (or the one being edited).
fn equation_at_caret(s: &Session) -> Option<Pos> {
    if let Some(m) = &s.math {
        return Some(m.at.clone());
    }
    let f = &s.sel.focus;
    let para = s.doc.para_at(f)?;
    let is_eq = |off: usize| matches!(para.object_at(off), Some(InlineObject::Equation { .. }));
    if is_eq(f.off) {
        return Some(f.clone());
    }
    let before = f.off.checked_sub(OBJ.len_utf8())?;
    if is_eq(before) {
        return Some(Pos { off: before, ..f.clone() });
    }
    // A selection holding exactly one equation.
    let (a, b) = s.sel.ordered();
    if a.path == b.path && b.off == a.off + OBJ.len_utf8() && is_eq(a.off) {
        return Some(a);
    }
    None
}

/// Run `f` on the equation at `at`; keeps its linear text in step and drops the source OMML
/// (it no longer matches).
fn with_eq<R>(s: &mut Session, at: &Pos, f: impl FnOnce(&mut Math, &mut bool) -> R) -> Result<R, CmdError> {
    let para = s.doc.para_mut(at.story, &at.path)?;
    let Some(InlineObject::Equation { linear, display, math }) = para.object_at_mut(at.off) else {
        return Err(CmdError::Failed("no equation there".into()));
    };
    if math.nodes.is_empty() && !linear.is_empty() && !math.linear {
        math.nodes = parse_linear(linear);
    }
    let r = f(math, display);
    *linear = if math.linear { plain(&math.nodes) } else { to_linear(&math.nodes) };
    math.omml.clear();
    para.touch();
    Ok(r)
}

/// All the text of an equation's runs (its linear form while shown linear).
fn plain(nodes: &[MNode]) -> String {
    let mut s = String::new();
    for n in nodes {
        if let MNode::Run(r) = n {
            s.push_str(&r.text);
        }
    }
    s
}

fn result(s: &Session) -> CmdResult {
    let Some(m) = &s.math else { return sel_result(s) };
    let info = s.doc.para_at(&m.at).and_then(|p| match p.object_at(m.at.off) {
        Some(InlineObject::Equation { linear, display, .. }) => Some((linear.clone(), *display)),
        _ => None,
    });
    let (linear, display) = info.unwrap_or_default();
    Ok(json!({"editing": true, "linear": linear, "display": display, "pos": serde_json::to_value(&m.pos).unwrap_or(Value::Null)}))
}

/// Nodes from `linear`, `latex` or a gallery `id` / `builtin` parameter.
fn nodes_from(v: &Value) -> Option<Arg> {
    if let Some(id) = p::str(v, "id") {
        return crate::math_gallery::template(id).map(|t| t.nodes());
    }
    if let Some(b) = p::str(v, "builtin") {
        return crate::math_gallery::BUILT_INS.iter().find(|(id, _, _)| *id == b).map(|(_, _, lin)| parse_linear(lin));
    }
    if let Some(l) = p::str(v, "latex") {
        return Some(parse_latex(l));
    }
    p::str(v, "linear").map(parse_linear)
}

fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let selected = if s.sel.is_collapsed() { String::new() } else { s.selected_text() };
    let nodes = match nodes_from(v) {
        Some(n) => n,
        None if p::str(v, "id").is_some() || p::str(v, "builtin").is_some() => return Err(CmdError::Params("unknown equation".into())),
        // Selected text becomes the equation (Word converts it).
        None => parse_linear(selected.trim()),
    };
    insert_nodes_as_equation(s, nodes, v)
}

fn insert_nodes_as_equation(s: &mut Session, nodes: Arg, v: &Value) -> CmdResult {
    s.math = None;
    let props = s.typing_props();
    let at = delete_selection(s)?;
    // On an empty line it's a display equation, in text an inline one.
    let empty_line = s.doc.para_at(&at).is_none_or(|p| p.text.trim().is_empty());
    let display = p::bool(v, "display").unwrap_or(empty_line);
    let linear = to_linear(&nodes);
    let caret = match me::next_placeholder(&nodes, &MathPos::default(), false) {
        Some(p) if !nodes.is_empty() => p,
        _ => MathPos::new(Vec::new(), me::units(&nodes)),
    };
    let obj = InlineObject::Equation { linear, display, math: Math { nodes, ..Default::default() } };
    let end = s.doc.insert_object(&at, obj, &props)?;
    if p::bool(v, "edit").unwrap_or(true) {
        s.sel = Selection::caret(at.clone());
        s.math = Some(MathEdit { at, pos: caret });
    } else {
        s.sel = Selection::caret(end);
    }
    result(s)
}

fn edit(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let nodes = match s.doc.para_at(&at).and_then(|p| p.object_at(at.off)) {
        Some(InlineObject::Equation { math, linear, .. }) => {
            if math.nodes.is_empty() && !math.linear {
                parse_linear(linear)
            } else {
                math.nodes.clone()
            }
        }
        _ => return Err(CmdError::Failed("no equation there".into())),
    };
    let pos =
        v.get("pos").and_then(|p| serde_json::from_value::<MathPos>(p.clone()).ok()).unwrap_or_else(|| MathPos::new(Vec::new(), me::units(&nodes)));
    s.sel = Selection::caret(at.clone());
    s.math = Some(MathEdit { pos: me::clamp(&nodes, &pos), at });
    result(s)
}

fn exit(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(m) = s.math.take() {
        let before = p::bool(v, "before").unwrap_or(false);
        let off = if before { m.at.off } else { m.at.off + OBJ.len_utf8() };
        s.sel = Selection::caret(Pos { off, ..m.at });
    }
    sel_result(s)
}

fn current(s: &Session) -> Result<MathEdit, CmdError> {
    s.math.clone().ok_or_else(|| CmdError::Disabled("not editing an equation".into()))
}

fn type_text(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?.to_string();
    let m = current(s)?;
    let latex = s.math_latex;
    let style = TypeStyle { nor: s.math_normal_text };
    let mut pos = m.pos.clone();
    with_eq(s, &m.at, |math, _| {
        if math.linear {
            // Linear form is plain text: no build-up.
            pos = me::insert_text(&mut math.nodes, &pos, &text, TypeStyle::default());
            set_literal(&mut math.nodes);
            return;
        }
        for c in text.chars().take(10_000) {
            let rel = matches!(wordcraft_doc::math::math_class(c), wordcraft_doc::math::MClass::Rel);
            if !style.nor && (c == ' ' || c == '\t' || rel) {
                let mut changed = false;
                if let Some(p) = me::autocorrect(&mut math.nodes, &pos) {
                    pos = p;
                    changed = true;
                }
                if let Some(p) = me::build_up(&mut math.nodes, &pos, latex) {
                    pos = p;
                    changed = true;
                }
                if c == ' ' && changed {
                    continue;
                }
            }
            pos = me::insert_text(&mut math.nodes, &pos, &c.to_string(), style);
        }
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    result(s)
}

/// Runs of a linear-form equation are literal text.
fn set_literal(nodes: &mut Arg) {
    for n in nodes.iter_mut() {
        if let MNode::Run(r) = n {
            r.lit = true;
        }
    }
}

fn insert_structure(s: &mut Session, v: &Value) -> CmdResult {
    let nodes = nodes_from(v).ok_or_else(|| CmdError::Params("give an id, linear or latex".into()))?;
    let Some(m) = s.math.clone() else {
        // Outside an equation: a new equation holding the structure.
        return insert_nodes_as_equation(s, nodes, &json!({}));
    };
    let mut pos = m.pos.clone();
    with_eq(s, &m.at, |math, _| {
        if math.linear {
            pos = me::insert_text(&mut math.nodes, &pos, &to_linear(&nodes), TypeStyle::default());
            set_literal(&mut math.nodes);
        } else {
            pos = me::insert_nodes(&mut math.nodes, &pos, nodes);
        }
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    result(s)
}

fn insert_symbol(s: &mut Session, v: &Value) -> CmdResult {
    let c = p::req_str(v, "char")?.to_string();
    if c.is_empty() {
        return Err(CmdError::Params("empty char".into()));
    }
    let Some(m) = s.math.clone() else {
        return insert_nodes_as_equation(s, parse_linear(&c), &json!({}));
    };
    let mut pos = m.pos.clone();
    let style = TypeStyle { nor: s.math_normal_text };
    with_eq(s, &m.at, |math, _| {
        pos = me::insert_text(&mut math.nodes, &pos, &c, style);
        if math.linear {
            set_literal(&mut math.nodes);
        }
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    result(s)
}

fn nodes_of(s: &Session, at: &Pos) -> Option<Arg> {
    match s.doc.para_at(at)?.object_at(at.off)? {
        InlineObject::Equation { math, linear, .. } => {
            Some(if math.nodes.is_empty() && !math.linear { parse_linear(linear) } else { math.nodes.clone() })
        }
        _ => None,
    }
}

fn move_caret(s: &mut Session, v: &Value) -> CmdResult {
    let m = current(s)?;
    // Leaving a slot finishes what was typed there (AutoCorrect names, build-up).
    let latex = s.math_latex;
    let mut pos = m.pos.clone();
    let mut changed = false;
    if !s.math_normal_text {
        with_eq(s, &m.at, |math, _| {
            if math.linear {
                return;
            }
            if let Some(p) = me::autocorrect(&mut math.nodes, &pos) {
                pos = p;
                changed = true;
            }
            if let Some(p) = me::build_up(&mut math.nodes, &pos, latex) {
                pos = p;
                changed = true;
            }
        })?;
    }
    if changed && let Some(mm) = s.math.as_mut() {
        mm.pos = pos;
    }
    let m = current(s)?;
    let nodes = nodes_of(s, &m.at).ok_or_else(|| CmdError::Failed("no equation there".into()))?;
    let dir = p::str(v, "dir").unwrap_or("right");
    let next = match dir {
        "left" => me::move_left(&nodes, &m.pos),
        "right" => me::move_right(&nodes, &m.pos),
        "up" => Some(me::move_vertical(&nodes, &m.pos, true).unwrap_or(m.pos.clone())),
        "down" => Some(me::move_vertical(&nodes, &m.pos, false).unwrap_or(m.pos.clone())),
        "home" => Some(me::home_end(&nodes, &m.pos, false)),
        "end" => Some(me::home_end(&nodes, &m.pos, true)),
        "next" | "prev" => Some(me::next_placeholder(&nodes, &m.pos, dir == "prev").unwrap_or(m.pos.clone())),
        other => return Err(CmdError::Params(format!("unknown direction {other}"))),
    };
    match next {
        Some(p) => {
            if let Some(m) = s.math.as_mut() {
                m.pos = p;
            }
            result(s)
        }
        // Past either end: the caret leaves the equation.
        None => exit(s, &json!({"before": dir == "left"})),
    }
}

fn delete(s: &mut Session, back: bool) -> CmdResult {
    let m = current(s)?;
    let nodes = nodes_of(s, &m.at).ok_or_else(|| CmdError::Failed("no equation there".into()))?;
    // An empty equation goes away entirely.
    if nodes.is_empty() {
        s.math = None;
        s.sel = Selection { anchor: m.at.clone(), focus: Pos { off: m.at.off + OBJ.len_utf8(), ..m.at.clone() } };
        let at = delete_selection(s)?;
        s.sel = Selection::caret(at);
        return sel_result(s);
    }
    let mut pos = m.pos.clone();
    with_eq(s, &m.at, |math, _| {
        pos = if back { me::delete_back(&mut math.nodes, &pos) } else { me::delete_forward(&mut math.nodes, &pos) };
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    result(s)
}

fn enter(s: &mut Session, _: &Value) -> CmdResult {
    let m = current(s)?;
    let latex = s.math_latex;
    let mut pos = m.pos.clone();
    let mut changed = false;
    with_eq(s, &m.at, |math, _| {
        if math.linear {
            return;
        }
        if let Some(p) = me::autocorrect(&mut math.nodes, &pos) {
            pos = p;
            changed = true;
        }
        if let Some(p) = me::build_up(&mut math.nodes, &pos, latex) {
            pos = p;
            changed = true;
        }
        if me::number_equation(&mut math.nodes) {
            pos = MathPos::new(Vec::new(), me::units(&math.nodes));
            changed = true;
        }
    })?;
    if changed {
        if let Some(m) = s.math.as_mut() {
            m.pos = pos;
        }
        return result(s);
    }
    // Nothing to build: Enter ends the equation and starts a new paragraph.
    s.math = None;
    let after = Pos { off: m.at.off + OBJ.len_utf8(), ..m.at };
    let new = split_para(s, &after)?;
    s.sel = Selection::caret(new);
    sel_result(s)
}

/// Built-up ↔ linear form for one equation's nodes.
fn convert_math(math: &mut Math, professional: bool, latex: bool) {
    if professional {
        if math.linear {
            let text = plain(&math.nodes);
            math.nodes = if latex { parse_latex(&text) } else { parse_linear(&text) };
            math.linear = false;
        }
    } else if !math.linear {
        let text = if latex { to_latex(&math.nodes) } else { to_linear(&math.nodes) };
        math.nodes = vec![MNode::Run(wordcraft_doc::math::MRun { text, lit: true, ..Default::default() })];
        math.linear = true;
    }
}

fn convert(s: &mut Session, v: &Value) -> CmdResult {
    let professional = p::str(v, "to").map(|t| !t.eq_ignore_ascii_case("linear")).unwrap_or(true);
    let latex = s.math_latex;
    if p::bool(v, "all").unwrap_or(false) {
        let mut n = 0;
        let stories: Vec<wordcraft_doc::StoryRef> =
            std::iter::once(wordcraft_doc::StoryRef::Body).chain(s.doc.parts.keys().map(|k| wordcraft_doc::StoryRef::Part(*k))).collect();
        for story in stories {
            for path in s.doc.para_paths(story) {
                let offs: Vec<usize> = s
                    .doc
                    .para(story, &path)
                    .map(|p| p.object_offsets().into_iter().filter(|o| matches!(p.object_at(*o), Some(InlineObject::Equation { .. }))).collect())
                    .unwrap_or_default();
                for off in offs {
                    let at = Pos { story, path: path.clone(), off };
                    with_eq(s, &at, |math, _| convert_math(math, professional, latex))?;
                    n += 1;
                }
            }
        }
        if let Some(m) = s.math.as_mut() {
            m.pos = MathPos::default();
        }
        s.clamp_selection();
        return Ok(json!({"converted": n}));
    }
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let mut end = 0;
    with_eq(s, &at, |math, _| {
        convert_math(math, professional, latex);
        end = me::units(&math.nodes);
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = MathPos::new(Vec::new(), end);
    }
    result(s)
}

fn set_display(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    with_eq(s, &at, |_, display| *display = p::bool(v, "value").unwrap_or(!*display))?;
    result(s)
}

fn justify(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let jc = match p::str(v, "jc").unwrap_or("centerGroup") {
        "left" => MathJc::Left,
        "right" => MathJc::Right,
        "center" => MathJc::Center,
        _ => MathJc::CenterGroup,
    };
    with_eq(s, &at, |math, display| {
        math.jc = jc;
        *display = true;
    })?;
    result(s)
}

/// Add (or remove) an automatic equation number: `#` with nothing after it.
fn number(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let mut pos_reset = false;
    with_eq(s, &at, |math, display| {
        let numbered = matches!(math.nodes.as_slice(), [MNode::EqArr { rows }] if rows.iter().any(|r| wordcraft_doc::math::has_number_mark(r)));
        let want = p::bool(v, "value").unwrap_or(!numbered);
        if want && !numbered {
            let mut row = std::mem::take(&mut math.nodes);
            row.push(MNode::Run(wordcraft_doc::math::MRun::new("#")));
            math.nodes = vec![MNode::EqArr { rows: vec![row] }];
            *display = true;
            pos_reset = true;
        } else if !want && numbered {
            if let [MNode::EqArr { rows }] = math.nodes.as_mut_slice()
                && let Some(row) = rows.first_mut()
            {
                // Drop everything from the `#` on.
                let mut out = Vec::new();
                for n in std::mem::take(row) {
                    if let MNode::Run(mut r) = n {
                        if let Some(k) = r.text.find('#') {
                            r.text.truncate(k);
                            if !r.text.is_empty() {
                                out.push(MNode::Run(r));
                            }
                            break;
                        }
                        out.push(MNode::Run(r));
                    } else {
                        out.push(n);
                    }
                }
                math.nodes = out;
            }
            pos_reset = true;
        }
    })?;
    if pos_reset && let Some(m) = s.math.as_mut() {
        m.pos = MathPos::default();
    }
    s.clamp_selection();
    result(s)
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let nodes = nodes_from(v).ok_or_else(|| CmdError::Params("give linear or latex".into()))?;
    let end = me::units(&nodes);
    with_eq(s, &at, |math, _| {
        math.nodes = nodes;
        math.linear = false;
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = MathPos::new(Vec::new(), end);
    }
    result(s)
}

fn get(s: &mut Session, _: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let Some(InlineObject::Equation { linear, display, math }) = s.doc.para_at(&at).and_then(|p| p.object_at(at.off)) else {
        return Err(CmdError::Failed("no equation there".into()));
    };
    let nodes = if math.nodes.is_empty() && !math.linear { parse_linear(linear) } else { math.nodes.clone() };
    Ok(json!({
        "linear": linear,
        "latex": to_latex(&nodes),
        "display": display,
        "linearForm": math.linear,
        "editing": s.math.is_some(),
        "pos": s.math.as_ref().map(|m| serde_json::to_value(&m.pos).unwrap_or(Value::Null)),
        "at": super::pos_json(&at),
    }))
}

fn structure_actions(s: &mut Session, _: &Value) -> CmdResult {
    let m = current(s)?;
    let nodes = nodes_of(s, &m.at).ok_or_else(|| CmdError::Failed("no equation there".into()))?;
    let acts: Vec<Value> =
        me::structure_actions(&nodes, &m.pos).into_iter().map(|(l, a, label)| json!({"level": l, "action": a, "label": label})).collect();
    Ok(json!({"actions": acts}))
}

fn structure(s: &mut Session, v: &Value) -> CmdResult {
    let m = current(s)?;
    let action = p::req_str(v, "action")?.to_string();
    let nodes = nodes_of(s, &m.at).ok_or_else(|| CmdError::Failed("no equation there".into()))?;
    let level = match p::u64(v, "level") {
        Some(l) => l as usize,
        None => me::structure_actions(&nodes, &m.pos)
            .into_iter()
            .find(|(_, a, _)| *a == action)
            .map(|(l, _, _)| l)
            .ok_or_else(|| CmdError::Params(format!("{action} doesn't apply here")))?,
    };
    let mut pos = None;
    with_eq(s, &m.at, |math, _| pos = me::apply_structure(&mut math.nodes, &m.pos, level, &action))?;
    let pos = pos.ok_or_else(|| CmdError::Params(format!("{action} doesn't apply here")))?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    result(s)
}

fn gallery(_: &mut Session, _: &Value) -> CmdResult {
    let structures: Vec<Value> = crate::math_gallery::STRUCTURES
        .iter()
        .map(|g| {
            json!({
                "id": g.id,
                "label": g.label.replace('\n', " "),
                "sections": g.sections.iter().map(|(name, ts)| json!({
                    "name": name,
                    "templates": ts.iter().map(|t| json!({"id": t.id, "label": t.label, "linear": t.linear})).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let symbols: Vec<Value> = crate::math_gallery::SYMBOL_SETS.iter().map(|(n, chars)| json!({"name": n, "chars": chars})).collect();
    let builtins: Vec<Value> = crate::math_gallery::BUILT_INS.iter().map(|(id, l, lin)| json!({"id": id, "label": l, "linear": lin})).collect();
    Ok(json!({"structures": structures, "symbols": symbols, "builtins": builtins}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use wordcraft_doc::Document;

    fn session() -> Session {
        let mut s = Session::new(Document::new());
        s.run("document.setText", &json!({"text": ""})).unwrap();
        s
    }

    fn linear(s: &Session) -> String {
        s.math.as_ref().and_then(|m| s.doc.para_at(&m.at).and_then(|p| p.object_at(m.at.off))).map(|o| o.plain_text().to_string()).unwrap_or_default()
    }

    #[test]
    fn type_an_equation_like_word() {
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        assert!(s.math.is_some(), "editing the new equation");
        // An empty line gives a display equation.
        assert!(matches!(s.doc.para_at(&s.math.as_ref().unwrap().at).unwrap().object_at(0), Some(InlineObject::Equation { display: true, .. })));
        for t in ["x", "=", "(-b\\pm", " ", "\\sqrt", " ", "(b^2-4ac))/2a", " "] {
            s.run("equation.type", &json!({"text": t})).unwrap();
        }
        assert_eq!(linear(&s), "x=(-b±√(b^2-4ac))/2a");
        let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
        assert!(matches!(nodes.get(1), Some(MNode::Frac { .. })), "{nodes:#?}");
        // One undo step for the typing.
        s.run("equation.exit", &json!({})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        assert!(linear(&s).is_empty());
    }

    #[test]
    fn structures_symbols_and_navigation() {
        let mut s = session();
        s.run("equation.insertStructure", &json!({"id": "frac.stacked"})).unwrap();
        s.run("equation.type", &json!({"text": "1"})).unwrap();
        s.run("equation.move", &json!({"dir": "down"})).unwrap();
        s.run("equation.insertSymbol", &json!({"char": "π"})).unwrap();
        assert_eq!(linear(&s), "1/π");
        s.run("equation.move", &json!({"dir": "right"})).unwrap();
        s.run("equation.insertStructure", &json!({"id": "nary.sumLimits"})).unwrap();
        s.run("equation.type", &json!({"text": "k=1"})).unwrap();
        s.run("equation.move", &json!({"dir": "next"})).unwrap();
        s.run("equation.type", &json!({"text": "n"})).unwrap();
        s.run("equation.move", &json!({"dir": "next"})).unwrap();
        s.run("equation.type", &json!({"text": "k"})).unwrap();
        assert_eq!(linear(&s), "1/π∑_(k=1)^n▒k");
        let r = s.run("equation.get", &json!({})).unwrap();
        assert!(r["latex"].as_str().unwrap().contains("\\sum"));
        // Moving right past the end leaves the equation.
        for _ in 0..20 {
            if s.math.is_none() {
                break;
            }
            let _ = s.run("equation.move", &json!({"dir": "right"}));
        }
        assert!(s.math.is_none());
    }

    #[test]
    fn numbering_conversion_and_display() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "E=mc^2"})).unwrap();
        s.run("equation.type", &json!({"text": "#(1)"})).unwrap();
        s.run("equation.enter", &json!({})).unwrap();
        assert_eq!(linear(&s), "E=mc^2#(1)");
        s.run("equation.number", &json!({"value": false})).unwrap();
        assert_eq!(linear(&s), "E=mc^2");
        s.run("equation.number", &json!({})).unwrap();
        assert_eq!(linear(&s), "E=mc^2#");
        s.run("equation.convert", &json!({"to": "linear"})).unwrap();
        s.run("equation.convert", &json!({"to": "professional"})).unwrap();
        assert_eq!(linear(&s), "E=mc^2#");
        s.run("equation.display", &json!({"value": false})).unwrap();
        let r = s.run("equation.get", &json!({})).unwrap();
        assert_eq!(r["display"], false);
        // LaTeX input.
        s.run("equation.set", &json!({"latex": "\\frac{a}{b}"})).unwrap();
        assert_eq!(linear(&s), "a/b");
        // Backspacing an equation empty, then once more, removes it.
        let mut n = 0;
        while s.math.is_some() && n < 50 {
            s.run("equation.backspace", &json!({})).unwrap();
            n += 1;
        }
        assert!(s.math.is_none());
        assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().objects.len(), 0);
    }

    #[test]
    fn gallery_lists_everything() {
        let mut s = session();
        let g = s.run("equation.gallery", &json!({})).unwrap();
        assert_eq!(g["structures"].as_array().unwrap().len(), 11);
        assert!(g["symbols"].as_array().unwrap().len() >= 8);
        for b in crate::math_gallery::BUILT_INS {
            let mut s = session();
            s.run("insert.equation", &json!({"builtin": b.0, "edit": false})).unwrap();
        }
    }
}
