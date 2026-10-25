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
use crate::{CmdError, CmdResult, CommandSpec, ImeEditOutcome, MathEdit, Selection, Session, p};

fn editing(s: &Session) -> Option<&'static str> {
    if s.math.is_none() { Some("not editing an equation") } else { None }
}

fn near_equation(s: &Session) -> Option<&'static str> {
    if s.math.is_some() || equation_at_caret(s).is_some() { None } else { Some("no equation at the caret") }
}

/// Builds the equation command table: insert, edit and exit, typing, structures, symbols, caret
/// movement, deletion (including bounded surrounding deletion), enter, conversion, input format,
/// normal text, display, justification, numbering, set and get, gallery and structure actions.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.equation", "Equation", "Insert › Symbols", insert)
            .key("Alt+=")
            .params(r#"{"linear"?: string, "latex"?: string, "builtin"?: "pythagoras|area|quadratic|binomial|expansion|euler|derivative|taylor|gaussian|fourier|trig1|trig2|normal|bayes", "display"?: bool, "edit"?: bool (default true)}"#),
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
        CommandSpec::new("equation.deleteSurrounding", "Delete Around Caret in Equation", "Equation", delete_surrounding)
            .when(editing)
            .params(r#"{"before": n, "after": n}"#),
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

/// What one equation edit changed. `content_changed` gates an IME receipt; `state_changed`
/// covers style, representation and lazy-normalization changes that must poison a mixed frame.
#[derive(Clone, Copy, Debug, Default)]
struct EquationEffect {
    content_changed: bool,
    state_changed: bool,
}

impl EquationEffect {
    /// Merges another edit's effect flags, so a change on either side marks the merged effect.
    fn merge(&mut self, other: Self) {
        self.content_changed |= other.content_changed;
        self.state_changed |= other.state_changed;
    }
}

/// Records the edit as a classified IME outcome; any content change also counts as a state
/// change, and `transformed` marks automatic rewrites.
fn apply_effect(s: &mut Session, effect: EquationEffect, transformed: bool) {
    s.set_ime_edit_outcome(ImeEditOutcome {
        classified: true,
        content_changed: effect.content_changed,
        state_changed: effect.state_changed || effect.content_changed,
        transformed,
        selection_changed: false,
    });
}

/// Records a classified IME outcome with no content or state change, distinguishing a real
/// no-op from a command that never classified its edit.
fn classified_noop(s: &mut Session) {
    s.set_ime_edit_outcome(ImeEditOutcome { classified: true, ..ImeEditOutcome::default() });
}

/// Run `f` on the equation at `at`; keeps its linear text in step and drops the source OMML
/// (it no longer matches).
///
/// The logical before-state is captured *after* lazy `nodes` materialization: an imported
/// equation that only gets normalized is a state change, never an IME content change.
fn with_eq<R>(s: &mut Session, at: &Pos, f: impl FnOnce(&mut Math, &mut bool) -> R) -> Result<(R, EquationEffect), CmdError> {
    let para = s.doc.para_mut(at.story, &at.path)?;
    let Some(InlineObject::Equation { linear, display, math }) = para.object_at_mut(at.off) else {
        return Err(CmdError::Failed("no equation there".into()));
    };
    let normalized = math.nodes.is_empty() && !linear.is_empty() && !math.linear;
    if normalized {
        math.nodes = parse_linear(linear);
    }
    let nodes_before = math.nodes.clone();
    let linear_mode_before = math.linear;
    let jc_before = math.jc;
    let display_before = *display;
    let cached_linear_before = linear.clone();
    let omml_before_nonempty = !math.omml.is_empty();
    let r = f(math, display);
    let new_linear = if math.linear { plain(&math.nodes) } else { to_linear(&math.nodes) };
    let content_changed = math.nodes != nodes_before || math.linear != linear_mode_before;
    let state_changed = normalized
        || content_changed
        || math.jc != jc_before
        || *display != display_before
        || omml_before_nonempty
        || new_linear != cached_linear_before;
    *linear = new_linear;
    math.omml.clear();
    para.touch();
    Ok((r, EquationEffect { content_changed, state_changed }))
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

/// Clears any editing session, deletes the selection, creates an equation object (display on an
/// empty line unless the request overrides it), optionally opens editing at the first
/// placeholder, and reports a content change.
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
        let Some(session) = crate::next_math_session() else {
            return Err(CmdError::Failed("no equation editing session available".into()));
        };
        s.sel = Selection::caret(at.clone());
        s.math = Some(MathEdit { at, pos: caret, session });
    } else {
        s.sel = Selection::caret(end);
    }
    // Creating an equation inserts visible content, even when its tree is initially empty.
    s.set_ime_edit_outcome(ImeEditOutcome {
        classified: true,
        content_changed: true,
        state_changed: true,
        transformed: false,
        selection_changed: false,
    });
    result(s)
}

/// Starts editing the equation at the caret, materializing lazy nodes and clamping the requested
/// position, and returns the equation and caret.
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
    let Some(session) = crate::next_math_session() else {
        return Err(CmdError::Failed("no equation editing session available".into()));
    };
    s.sel = Selection::caret(at.clone());
    s.math = Some(MathEdit { pos: me::clamp(&nodes, &pos), at, session });
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

/// Caps typing at `MAX_EQUATION_TYPE_CHARS`; empty text is a classified no-op. Linear form
/// inserts literal runs, while built-up form applies AutoCorrect and build-up before spaces and
/// relational characters.
fn type_text(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?.to_string();
    let m = current(s)?;
    if text.is_empty() {
        // No normalization, no linear-cache rewrite, no revision: an empty commit is a no-op.
        classified_noop(s);
        return result(s);
    }
    let latex = s.math_latex;
    let style = TypeStyle { nor: s.math_normal_text };
    let mut pos = m.pos.clone();
    let mut transformed = false;
    let (_, effect) = with_eq(s, &m.at, |math, _| {
        if math.linear {
            // Linear form is plain text: no build-up.
            let text: String = text.chars().take(crate::MAX_EQUATION_TYPE_CHARS).collect();
            pos = me::insert_text(&mut math.nodes, &pos, &text, TypeStyle::default());
            set_literal(&mut math.nodes);
            return;
        }
        for c in text.chars().take(crate::MAX_EQUATION_TYPE_CHARS) {
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
                if changed {
                    transformed = true;
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
    apply_effect(s, effect, transformed);
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

/// Inserts a gallery, linear or LaTeX structure: a new equation when none is being edited,
/// literal text in linear form, or nodes at the caret in built-up form.
fn insert_structure(s: &mut Session, v: &Value) -> CmdResult {
    let nodes = nodes_from(v).ok_or_else(|| CmdError::Params("give an id, linear or latex".into()))?;
    let Some(m) = s.math.clone() else {
        // Outside an equation: a new equation holding the structure.
        return insert_nodes_as_equation(s, nodes, &json!({}));
    };
    let mut pos = m.pos.clone();
    let (_, effect) = with_eq(s, &m.at, |math, _| {
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
    apply_effect(s, effect, false);
    result(s)
}

/// Rejects an empty character; outside editing creates an equation, otherwise inserts the
/// character at the math caret, marked literal in linear form.
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
    let (_, effect) = with_eq(s, &m.at, |math, _| {
        pos = me::insert_text(&mut math.nodes, &pos, &c, style);
        if math.linear {
            set_literal(&mut math.nodes);
        }
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    apply_effect(s, effect, false);
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

/// Validates the direction, commits an AutoCorrect or build-up triggered by leaving a slot as one
/// undoable edit, moves or clamps the math caret, and exits past either end.
fn move_caret(s: &mut Session, v: &Value) -> CmdResult {
    let m = current(s)?;
    let dir = p::str(v, "dir").unwrap_or("right");
    if !matches!(dir, "left" | "right" | "up" | "down" | "home" | "end" | "next" | "prev") {
        return Err(CmdError::Params(format!("unknown direction {dir}")));
    }
    // Leaving a slot finishes what was typed there (AutoCorrect names, build-up). That is worked
    // out on a local copy: plain navigation must leave the equation (and its source OMML)
    // untouched, and a real change is a document edit — undoable and marking the document dirty
    // (#446, #447). Any failure happens before the single commit, so no rollback is needed.
    let shown_linear = matches!(
        s.doc.para_at(&m.at).and_then(|p| p.object_at(m.at.off)),
        Some(InlineObject::Equation { math, .. }) if math.linear
    );
    let mut nodes = nodes_of(s, &m.at).ok_or_else(|| CmdError::Failed("no equation there".into()))?;
    let mut pos = m.pos.clone();
    if !s.math_normal_text && !shown_linear {
        let latex = s.math_latex;
        let before = nodes.clone();
        let mut candidate = nodes.clone();
        let mut candidate_pos = pos.clone();
        let mut changed = false;
        if let Some(p) = me::autocorrect(&mut candidate, &candidate_pos) {
            candidate_pos = p;
            changed = true;
        }
        if let Some(p) = me::build_up(&mut candidate, &candidate_pos, latex) {
            candidate_pos = p;
            changed = true;
        }
        pos = candidate_pos;
        if changed && candidate != before {
            // `equation.move` is a pure command, so record the edit here.
            s.checkpoint("Build Up");
            let (_, effect) = with_eq(s, &m.at, |math, _| math.nodes = candidate.clone())?;
            apply_effect(s, effect, true);
            // `equation.move` is pure for ordinary navigation, so `Session::run` does not touch
            // the document for us. A build-up is a real edit: mark it dirty and refresh layout.
            s.touch();
            nodes = candidate;
        }
    }
    let next = match dir {
        "left" => me::move_left(&nodes, &pos),
        "right" => me::move_right(&nodes, &pos),
        "up" => Some(me::move_vertical(&nodes, &pos, true).unwrap_or(pos.clone())),
        "down" => Some(me::move_vertical(&nodes, &pos, false).unwrap_or(pos.clone())),
        "home" => Some(me::home_end(&nodes, &pos, false)),
        "end" => Some(me::home_end(&nodes, &pos, true)),
        "next" | "prev" => Some(me::next_placeholder(&nodes, &pos, dir == "prev").unwrap_or(pos.clone())),
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
/// With the caret in an empty equation, removes the whole equation as a content change;
/// otherwise deletes backward or forward, updates the caret and records the effect.
fn delete(s: &mut Session, back: bool) -> CmdResult {
    let m = current(s)?;
    let nodes = nodes_of(s, &m.at).ok_or_else(|| CmdError::Failed("no equation there".into()))?;
    // An empty equation goes away entirely: removing the object is a content change.
    if nodes.is_empty() {
        s.math = None;
        s.sel = Selection { anchor: m.at.clone(), focus: Pos { off: m.at.off + OBJ.len_utf8(), ..m.at.clone() } };
        let at = delete_selection(s)?;
        s.sel = Selection::caret(at);
        s.set_ime_edit_outcome(ImeEditOutcome {
            classified: true,
            content_changed: true,
            state_changed: true,
            transformed: false,
            selection_changed: false,
        });
        return sel_result(s);
    }
    let mut pos = m.pos.clone();
    let (_, effect) = with_eq(s, &m.at, |math, _| {
        pos = if back { me::delete_back(&mut math.nodes, &pos) } else { me::delete_forward(&mut math.nodes, &pos) };
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    apply_effect(s, effect, false);
    result(s)
}

/// Delete a bounded number of equation units on each side of the caret in one transaction.
///
/// The requested steps are simulated on a local tree and caret. Any step that cannot make
/// progress refuses the whole request before the document is touched; the commit (if any) is a
/// single command and a single undo step.
fn delete_surrounding(s: &mut Session, v: &Value) -> CmdResult {
    let m = current(s)?;
    let before = p::u64(v, "before").and_then(|n| usize::try_from(n).ok()).unwrap_or(usize::MAX);
    let after = p::u64(v, "after").and_then(|n| usize::try_from(n).ok()).unwrap_or(usize::MAX);
    if before.saturating_add(after) == 0 {
        return Err(CmdError::Params("nothing to delete".into()));
    }
    if before > crate::MAX_EQUATION_DELETE_UNITS || after > crate::MAX_EQUATION_DELETE_UNITS {
        return Err(CmdError::Params(format!("at most {} units on each side", crate::MAX_EQUATION_DELETE_UNITS)));
    }
    let mut nodes = nodes_of(s, &m.at).ok_or_else(|| CmdError::Failed("no equation there".into()))?;
    let mut pos = m.pos.clone();
    for _ in 0..before {
        let prev_nodes = nodes.clone();
        let prev_pos = pos.clone();
        pos = me::delete_back(&mut nodes, &pos);
        if nodes == prev_nodes {
            // Walking into the next argument is not a deletion. A requested unit that only
            // moves the caret must refuse the whole request, or a later step can apply against
            // a tree the platform's surrounding snapshot did not describe.
            return Err(CmdError::Failed(if pos == prev_pos {
                "the equation cannot delete that many units".into()
            } else {
                "the equation delete made no progress".into()
            }));
        }
    }
    for _ in 0..after {
        let prev_nodes = nodes.clone();
        let prev_pos = pos.clone();
        pos = me::delete_forward(&mut nodes, &pos);
        if nodes == prev_nodes {
            return Err(CmdError::Failed(if pos == prev_pos {
                "the equation cannot delete that many units".into()
            } else {
                "the equation delete made no progress".into()
            }));
        }
    }
    let (_, effect) = with_eq(s, &m.at, |math, _| math.nodes = nodes)?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    apply_effect(s, effect, false);
    Ok(json!({"before": before, "after": after, "changed": true}))
}
/// In built-up form runs AutoCorrect, build-up and equation numbering; when nothing changes,
/// leaves the equation and splits a new paragraph.
fn enter(s: &mut Session, _: &Value) -> CmdResult {
    let m = current(s)?;
    let latex = s.math_latex;
    let mut pos = m.pos.clone();
    let mut changed = false;
    let (_, effect) = with_eq(s, &m.at, |math, _| {
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
        apply_effect(s, effect, true);
        return result(s);
    }
    // Nothing to build: Enter ends the equation and starts a new paragraph.
    s.math = None;
    let after = Pos { off: m.at.off + OBJ.len_utf8(), ..m.at };
    let new = split_para(s, &after)?;
    s.sel = Selection::caret(new);
    s.set_ime_edit_outcome(ImeEditOutcome {
        classified: true,
        content_changed: true,
        state_changed: true,
        transformed: false,
        selection_changed: false,
    });
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

/// Converts one equation or all equations between built-up and linear form, honoring LaTeX mode,
/// merging every effect and resetting the math caret.
fn convert(s: &mut Session, v: &Value) -> CmdResult {
    let professional = p::str(v, "to").map(|t| !t.eq_ignore_ascii_case("linear")).unwrap_or(true);
    let latex = s.math_latex;
    if p::bool(v, "all").unwrap_or(false) {
        let mut n = 0;
        let mut effect = EquationEffect::default();
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
                    let (_, one) = with_eq(s, &at, |math, _| convert_math(math, professional, latex))?;
                    effect.merge(one);
                    n += 1;
                }
            }
        }
        if let Some(m) = s.math.as_mut() {
            m.pos = MathPos::default();
        }
        s.clamp_selection();
        apply_effect(s, effect, false);
        return Ok(json!({"converted": n}));
    }
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let mut end = 0;
    let (_, effect) = with_eq(s, &at, |math, _| {
        convert_math(math, professional, latex);
        end = me::units(&math.nodes);
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = MathPos::new(Vec::new(), end);
    }
    apply_effect(s, effect, false);
    result(s)
}

/// Toggles or sets the equation's inline or display mode.
fn set_display(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let (_, effect) = with_eq(s, &at, |_, display| *display = p::bool(v, "value").unwrap_or(!*display))?;
    apply_effect(s, effect, false);
    result(s)
}

/// Maps `jc` to the math justification, with unknown values falling back to center-group, and
/// forces display mode.
fn justify(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let jc = match p::str(v, "jc").unwrap_or("centerGroup") {
        "left" => MathJc::Left,
        "right" => MathJc::Right,
        "center" => MathJc::Center,
        _ => MathJc::CenterGroup,
    };
    let (_, effect) = with_eq(s, &at, |math, display| {
        math.jc = jc;
        *display = true;
    })?;
    apply_effect(s, effect, false);
    result(s)
}

/// Add (or remove) an automatic equation number: `#` with nothing after it.
fn number(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let mut pos_reset = false;
    let (_, effect) = with_eq(s, &at, |math, display| {
        let numbered = matches!(math.nodes.as_slice(), [MNode::EqArr { rows }] if rows.iter().any(|r| wordcraft_doc::math::has_number_mark(r)));
        let want = p::bool(v, "value").unwrap_or(!numbered);
        if want && !numbered {
            let mut row = std::mem::take(&mut math.nodes);
            row.push(MNode::Run(wordcraft_doc::math::MRun::new("#")));
            math.nodes = vec![MNode::EqArr { rows: vec![row] }];
            *display = true;
            pos_reset = true;
        } else if !want && numbered {
            // Every row loses its number; a single row stands alone again, several stay an
            // array (#444).
            if let [MNode::EqArr { rows }] = math.nodes.as_mut_slice() {
                let mut rows: Vec<Arg> = std::mem::take(rows).into_iter().map(without_number).collect();
                math.nodes = if rows.len() == 1 { rows.pop().unwrap_or_default() } else { vec![MNode::EqArr { rows }] };
            }
            pos_reset = true;
        }
    })?;
    if pos_reset && let Some(m) = s.math.as_mut() {
        m.pos = MathPos::default();
    }
    s.clamp_selection();
    apply_effect(s, effect, false);
    result(s)
}

/// An equation-array row without its number: everything from the `#` on goes.
fn without_number(row: Arg) -> Arg {
    let mut out = Vec::new();
    for n in row {
        match n {
            MNode::Run(mut r) if !r.lit && r.text.contains('#') => {
                if let Some(k) = r.text.find('#') {
                    r.text.truncate(k);
                }
                if !r.text.is_empty() {
                    out.push(MNode::Run(r));
                }
                break;
            }
            n => out.push(n),
        }
    }
    out
}

/// Replaces the equation's nodes from linear or LaTeX input, forces built-up form, and puts the
/// caret at the new end.
fn set(s: &mut Session, v: &Value) -> CmdResult {
    let at = equation_at_caret(s).ok_or_else(|| CmdError::Disabled("no equation at the caret".into()))?;
    let nodes = nodes_from(v).ok_or_else(|| CmdError::Params("give linear or latex".into()))?;
    let end = me::units(&nodes);
    let (_, effect) = with_eq(s, &at, |math, _| {
        math.nodes = nodes;
        math.linear = false;
    })?;
    if let Some(m) = s.math.as_mut() {
        m.pos = MathPos::new(Vec::new(), end);
    }
    apply_effect(s, effect, false);
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

/// Applies a named structure action at an explicit or inferred level, refusing an action that
/// does not apply at the caret.
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
    let (_, effect) = with_eq(s, &m.at, |math, _| pos = me::apply_structure(&mut math.nodes, &m.pos, level, &action))?;
    let pos = pos.ok_or_else(|| CmdError::Params(format!("{action} doesn't apply here")))?;
    if let Some(m) = s.math.as_mut() {
        m.pos = pos;
    }
    apply_effect(s, effect, false);
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

    /// A session on a DOCX whose only paragraph is `<m:oMath>{math}</m:oMath>`.
    fn imported(math: &str) -> Session {
        use std::io::Write;
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let m = "http://schemas.openxmlformats.org/officeDocument/2006/math";
        let parts = [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            ("word/document.xml", format!(r#"<w:document xmlns:w="{w}" xmlns:m="{m}"><w:body><w:p><m:oMath>{math}</m:oMath></w:p><w:sectPr/></w:body></w:document>"#)),
        ];
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, xml) in parts {
            zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zw.write_all(xml.as_bytes()).unwrap();
        }
        let bytes = zw.finish().unwrap().into_inner();
        let mut s = Session::new(wordcraft_docx::read(&bytes).unwrap());
        s.run("equation.edit", &json!({})).unwrap();
        s
    }

    fn document_xml(s: &Session) -> String {
        use std::io::Read;
        let bytes = wordcraft_docx::write(&s.doc).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
        xml
    }

    #[test]
    fn removing_numbers_keeps_every_row() {
        // #444: a numbered two-row array lost its second row when its numbers were removed.
        for numbered in ["█(a#(1)@b#(2))", "█(a@b#(2))"] {
            let mut s = session();
            s.run("insert.equation", &json!({"linear": numbered})).unwrap();
            s.run("equation.number", &json!({"value": false})).unwrap();
            assert_eq!(linear(&s), "█(a@b)", "{numbered}");
            let mut back = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap());
            assert_eq!(back.run("equation.get", &json!({})).unwrap()["linear"], "█(a@b)", "{numbered} after save and reopen");
            s.run("edit.undo", &json!({})).unwrap();
            assert_eq!(linear(&s), numbered, "Undo restores the numbers");
        }
        // A single numbered row stands alone again.
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "a#(1)"})).unwrap();
        s.run("equation.number", &json!({"value": false})).unwrap();
        assert_eq!(linear(&s), "a");
    }

    #[test]
    fn colour_survives_save_and_reopen() {
        // #1467: a coloured equation came back black after saving and reopening.
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "(a+b)/c"})).unwrap();
        s.run("equation.exit", &json!({})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        s.run("format.color", &json!({"color": "C00000"})).unwrap();
        assert!(document_xml(&s).contains(r#"<w:color w:val="C00000"/></w:rPr><m:t"#), "the math runs carry the colour");
        let mut back = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap());
        back.run("equation.edit", &json!({})).unwrap();
        let at = back.math.as_ref().unwrap().at.clone();
        let p = back.doc.para_at(&at).unwrap();
        let (_, props) = p.run_ranges().find(|(r, _)| r.contains(&at.off)).unwrap();
        assert_eq!(props.color, Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(0xC0, 0, 0))), "the equation's colour is back");
        let Some(InlineObject::Equation { math, .. }) = p.object_at(at.off) else { panic!() };
        let mut nodes = math.nodes.clone();
        let mut runs = 0;
        wordcraft_doc::math_edit::each_run_mut(&mut nodes, &mut |r| {
            runs += 1;
            assert_eq!(r.color, None, "the colour is the equation's, not stuck on each run");
        });
        assert!(runs > 0);
        // Automatic again: no colour written.
        back.run("select.all", &json!({})).unwrap();
        back.run("format.color", &json!({"color": "auto"})).unwrap();
        assert!(!document_xml(&back).contains("C00000"));
    }

    #[test]
    fn imported_colour_keeps_source_omml_until_recoloured() {
        // #1467: lifting a Word equation's colour must not drop the OMML the model can't hold.
        let r = r#"<w:rPr><w:color w:val="FF0000"/></w:rPr>"#;
        let ctrl = r#"<m:ctrlPr><w:rPr><w:color w:val="00FF00"/></w:rPr></m:ctrlPr>"#;
        let mut s =
            imported(&format!("<m:f><m:fPr>{ctrl}</m:fPr><m:num><m:r>{r}<m:t>a</m:t></m:r></m:num><m:den><m:r>{r}<m:t>b</m:t></m:r></m:den></m:f>"));
        assert!(document_xml(&s).contains("00FF00"), "untouched: source OMML written back");
        s.run("select.all", &json!({})).unwrap();
        s.run("format.color", &json!({"color": "0000FF"})).unwrap();
        let xml = document_xml(&s);
        assert!(xml.contains("0000FF") && !xml.contains("FF0000"), "recoloured: runs rewritten");
    }

    #[test]
    fn navigation_keeps_imported_omml() {
        // #446: a Left move without any change dropped the fraction's imported control colour.
        let red = r#"<m:f><m:fPr><m:ctrlPr><w:rPr><w:color w:val="FF0000"/></w:rPr></m:ctrlPr></m:fPr><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>"#;
        let mut s = imported(red);
        s.run("equation.move", &json!({"dir": "left"})).unwrap();
        assert!(document_xml(&s).contains(r#"w:val="FF0000""#), "the control colour survives navigation");
        // A genuine edit still writes the new content.
        s.run("equation.type", &json!({"text": "x"})).unwrap();
        assert_eq!(linear(&s), "a/bx");
        assert!(document_xml(&s).contains(">bx<"));
    }

    /// A Left move that builds up `a/b` makes the document dirty, advances layout and IME revisions,
    /// reports a classified transformed change, and Undo restores the literal run; plain navigation
    /// leaves no undo step.
    #[test]
    fn navigation_build_up_is_an_undoable_edit() {
        // #447: building `a/b` up on a Left move left the document clean and Undo did nothing.
        let mut s = imported("<m:r><m:t>a/b</m:t></m:r>");
        assert!(!s.dirty);
        let layout_rev = s.rev();
        let ime_rev = revision(&mut s);
        s.run("equation.move", &json!({"dir": "left"})).unwrap();
        // Every command replaces the last outcome; sample the edit before the read-only query.
        let outcome = s.ime_edit_outcome();
        assert_eq!(s.run("equation.get", &json!({})).unwrap()["latex"], "\\frac{a}{b}");
        assert!(s.dirty, "build-up marks the document dirty");
        assert!(s.rev() > layout_rev, "build-up refreshes the layout revision");
        assert!(outcome.classified && outcome.content_changed && outcome.state_changed && outcome.transformed, "{outcome:?}");
        assert!(revision(&mut s) > ime_rev, "build-up advances the unavailable IME revision");
        s.run("edit.undo", &json!({})).unwrap();
        s.run("equation.edit", &json!({})).unwrap();
        assert_eq!(s.run("equation.get", &json!({})).unwrap()["latex"], "a/b", "Undo restores the plain run");
        // Plain navigation is no edit.
        let mut s = imported("<m:r><m:t>abc</m:t></m:r>");
        s.run("equation.move", &json!({"dir": "left"})).unwrap();
        assert!(!s.dirty);
        assert_eq!(s.undo_depth(), 0);
    }

    #[test]
    fn latex_text_keeps_its_spaces() {
        // #448: `\text{for all x}` came out as `forallx`.
        for (tex, want) in [
            ("\\text{for all x}", "\\text{for all x}"),
            ("\\text{a b}", "\\text{a b}"),
            ("\\text{ab}", "\\text{ab}"),
            ("x + y", "x+y"),
            ("x+y", "x+y"),
        ] {
            let mut s = session();
            s.run("insert.equation", &json!({"latex": tex})).unwrap();
            assert_eq!(s.run("equation.get", &json!({})).unwrap()["latex"], want, "{tex}");
        }
    }

    #[test]
    fn typed_multi_letter_scripts_stay_whole() {
        // #1609, Word for Mac 16.113.4: `V_dd` + Space leaves "dd" entirely in the subscript.
        for typed in ["V_dd", "V_(dd)"] {
            let mut s = session();
            s.run("insert.equation", &json!({})).unwrap();
            s.run("equation.type", &json!({"text": typed})).unwrap();
            s.run("equation.type", &json!({"text": " "})).unwrap();
            let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
            assert_eq!(nodes.len(), 1, "{typed}: {nodes:#?}");
            assert!(
                matches!(&nodes[0], MNode::Script { sub, .. } if matches!(sub.as_slice(), [MNode::Run(r)] if r.text == "dd")),
                "{typed}: {nodes:#?}"
            );
        }
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        s.run("equation.type", &json!({"text": "x^ab "})).unwrap();
        let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
        assert!(matches!(&nodes[0], MNode::Script { sup, .. } if matches!(sup.as_slice(), [MNode::Run(r)] if r.text == "ab")), "{nodes:#?}");
    }

    #[test]
    fn a_built_script_keeps_the_letter_typed_after_it() {
        // #1609 review: `x_i y ` built x_{iy}: the letter after a script is its own item.
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        s.run("equation.type", &json!({"text": "x_i y "})).unwrap();
        let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
        assert_eq!(nodes.len(), 2, "{nodes:#?}");
        assert!(matches!(&nodes[0], MNode::Script { sub, .. } if matches!(sub.as_slice(), [MNode::Run(r)] if r.text == "i")), "{nodes:#?}");
        assert!(matches!(&nodes[1], MNode::Run(r) if r.text.trim() == "y"), "{nodes:#?}");
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        s.run("equation.type", &json!({"text": "a_i b_j "})).unwrap();
        let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
        let subs: Vec<&str> = nodes
            .iter()
            .filter_map(|n| match n {
                MNode::Script { sub, .. } => match sub.as_slice() {
                    [MNode::Run(r)] => Some(r.text.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(subs, ["i", "j"], "{nodes:#?}");
        assert!(nodes.iter().all(|n| matches!(n, MNode::Script { .. }) || matches!(n, MNode::Run(r) if r.text.trim().is_empty())), "{nodes:#?}");
    }

    /// Returns the session's current IME state revision, allocating a new one when the caret state changed.
    fn revision(s: &mut Session) -> u64 {
        s.ime_state_revision().expect("an IME state revision")
    }

    /// Clears the stored node tree of the equation currently being edited, leaving its linear text
    /// and lazy state in place.
    fn clear_nodes(s: &mut Session) {
        let at = s.math.as_ref().expect("editing").at.clone();
        if let Some(InlineObject::Equation { math, .. }) = s.doc.para_mut(at.story, &at.path).expect("paragraph").object_at_mut(at.off) {
            math.nodes.clear();
        }
    }

    /// Backspace at the start of an equation is classified with no content or state change,
    /// allocates no revision and leaves the linear text intact.
    #[test]
    fn equation_backspace_at_start_is_a_classified_noop() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "x+y"})).unwrap();
        s.run("equation.move", &json!({"dir": "home"})).unwrap();
        let before = revision(&mut s);
        s.run("equation.backspace", &json!({})).unwrap();
        let outcome = s.ime_edit_outcome();
        assert!(outcome.classified);
        assert!(!outcome.content_changed && !outcome.state_changed, "{outcome:?}");
        assert_eq!(revision(&mut s), before, "a no-op allocates no revision");
        assert_eq!(linear(&s), "x+y");
    }

    /// Materializing an imported equation's lazy nodes is a state change only: the outcome is
    /// classified, content is unchanged, and the revision advances.
    #[test]
    fn lazy_normalization_is_state_not_content() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "x+1"})).unwrap();
        clear_nodes(&mut s);
        // Editing parses the local tree but leaves the stored equation lazy.
        s.run("equation.edit", &json!({})).unwrap();
        s.run("equation.move", &json!({"dir": "home"})).unwrap();
        let before = revision(&mut s);
        s.run("equation.backspace", &json!({})).unwrap();
        let outcome = s.ime_edit_outcome();
        assert!(outcome.classified);
        assert!(!outcome.content_changed, "the normalized tree was already logical content");
        assert!(outcome.state_changed, "materializing the cached nodes is a state change");
        assert!(revision(&mut s) > before, "state changes advance the revision");
    }

    /// Empty typing is a classified no-op in built-up and linear form, without a new revision.
    #[test]
    fn empty_equation_type_is_a_classified_noop_in_both_modes() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "x"})).unwrap();
        let before = revision(&mut s);
        s.run("equation.type", &json!({"text": ""})).unwrap();
        assert!(s.ime_edit_outcome().classified);
        assert!(!s.ime_edit_outcome().content_changed && !s.ime_edit_outcome().state_changed);
        assert_eq!(revision(&mut s), before, "built-up empty type is a no-op");
        s.run("equation.convert", &json!({"to": "linear"})).unwrap();
        let before = revision(&mut s);
        s.run("equation.type", &json!({"text": ""})).unwrap();
        assert!(!s.ime_edit_outcome().content_changed && !s.ime_edit_outcome().state_changed);
        assert_eq!(revision(&mut s), before, "linear empty type is a no-op");
    }

    /// Typing `a/b ` or `\alpha ` reports both a content change and a transformed automatic rewrite.
    #[test]
    fn automatic_equation_rewrites_are_transformed() {
        for text in ["a/b ", "\\alpha "] {
            let mut s = session();
            s.run("insert.equation", &json!({})).unwrap();
            s.run("equation.type", &json!({"text": text})).unwrap();
            let outcome = s.ime_edit_outcome();
            assert!(outcome.content_changed, "{text}: {outcome:?}");
            assert!(outcome.transformed, "{text}: {outcome:?}");
        }
    }

    /// Justification and display changes advance the revision and report a state, not content,
    /// change.
    #[test]
    fn equation_style_changes_are_state_changes_without_content() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "x"})).unwrap();
        let before = revision(&mut s);
        s.run("equation.justify", &json!({"jc": "left"})).unwrap();
        let outcome = s.ime_edit_outcome();
        assert!(outcome.classified && outcome.state_changed && !outcome.content_changed, "{outcome:?}");
        assert!(revision(&mut s) > before);
        let before = revision(&mut s);
        s.run("equation.display", &json!({"value": false})).unwrap();
        assert!(s.ime_edit_outcome().state_changed && !s.ime_edit_outcome().content_changed);
        assert!(revision(&mut s) > before);
    }

    /// Converting every equation reports one classified effect that merges their content and state
    /// changes.
    #[test]
    fn convert_all_merges_every_equations_effect() {
        let mut s = session();
        s.run("document.setText", &json!({"text": "\r"})).unwrap();
        s.run("insert.equation", &json!({"linear": "a/b"})).unwrap();
        s.run("equation.convert", &json!({"to": "linear"})).unwrap();
        s.run("equation.exit", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.equation", &json!({"linear": "c/d"})).unwrap();
        s.run("equation.exit", &json!({})).unwrap();
        s.run("equation.convert", &json!({"to": "professional", "all": true})).unwrap();
        let outcome = s.ime_edit_outcome();
        assert!(outcome.classified && outcome.content_changed && outcome.state_changed, "{outcome:?}");
    }

    /// A bounded deletion around the caret removes the requested units in one undo step and reports
    /// a content change.
    #[test]
    fn equation_delete_surrounding_is_one_transaction_and_one_undo_step() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "abcd"})).unwrap();
        let depth = s.undo_depth();
        s.run("equation.deleteSurrounding", &json!({"before": 2, "after": 0})).unwrap();
        assert_eq!(linear(&s), "ab");
        assert_eq!(s.undo_depth(), depth + 1, "one undo step, not one per unit");
        assert!(s.ime_edit_outcome().classified && s.ime_edit_outcome().content_changed);
    }

    /// An impossible bounded deletion applies no prefix, adds no undo step, keeps the session and
    /// caret, and leaves no classified outcome.
    #[test]
    fn equation_delete_surrounding_refuses_partial_progress() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "ab"})).unwrap();
        let session_token = s.math.as_ref().unwrap().session;
        let at = s.math.as_ref().unwrap().at.clone();
        let depth = s.undo_depth();
        let err = s.run("equation.deleteSurrounding", &json!({"before": 3, "after": 0})).unwrap_err();
        assert!(err.to_string().contains("cannot delete"), "{err}");
        assert_eq!(linear(&s), "ab", "the whole request is refused, no prefix applied");
        assert_eq!(s.undo_depth(), depth, "no undo step for a refused transaction");
        let m = s.math.as_ref().expect("still editing");
        assert_eq!(m.session, session_token);
        assert_eq!(m.at, at);
        assert!(!s.ime_edit_outcome().classified, "a refusal has no outcome");
        assert_eq!(
            s.last_command.as_ref().map(|(id, _)| id.as_str()),
            Some("equation.deleteSurrounding"),
            "only the pre-dispatch Repeat record remains"
        );
    }

    /// Zero or oversized counts and deletions from an empty equation are refused without mutation;
    /// exactly deleting all text leaves the empty equation object.
    #[test]
    fn equation_delete_surrounding_bounds_and_empty_equations() {
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        assert!(s.run("equation.deleteSurrounding", &json!({"before": 0, "after": 0})).is_err(), "zero counts are refused");
        assert!(s.run("equation.deleteSurrounding", &json!({"before": 1, "after": 0})).is_err(), "an empty equation refuses");
        assert!(s.math.is_some(), "the empty equation object stays");
        s.run("equation.type", &json!({"text": "a"})).unwrap();
        let depth = s.undo_depth();
        assert!(s.run("equation.deleteSurrounding", &json!({"before": 65, "after": 0})).is_err());
        assert!(s.run("equation.deleteSurrounding", &json!({"before": 0, "after": 65})).is_err());
        assert_eq!(linear(&s), "a");
        assert_eq!(s.undo_depth(), depth);
        s.run("equation.deleteSurrounding", &json!({"before": 1, "after": 0})).unwrap();
        assert_eq!(linear(&s), "", "exactly deleting the last unit leaves the equation object");
        assert!(s.math.is_some());
    }

    /// A forward deletion that would only step into another argument is refused whole, leaving the
    /// text and undo depth unchanged (the caret is not asserted here).
    #[test]
    fn equation_delete_surrounding_refuses_a_caret_only_step() {
        let mut s = session();
        s.run("insert.equation", &json!({"latex": "\\frac{a}{b}"})).unwrap();
        // At the end of the numerator a forward delete only steps into the denominator: the
        // tree does not change, so a request that would report progress is refused whole.
        s.math.as_mut().expect("editing").pos = MathPos { path: vec![(0, 0)], off: 1 };
        let before = linear(&s);
        let depth = s.undo_depth();
        let err = s.run("equation.deleteSurrounding", &json!({"before": 0, "after": 1})).unwrap_err();
        assert!(err.to_string().contains("no progress"), "{err}");
        assert_eq!(linear(&s), before);
        assert_eq!(s.undo_depth(), depth);
    }

    /// A deletion that really erases on one side cannot buy a caret-only step on the other:
    /// the mixed request is refused whole, so a following IME commit cannot land in the
    /// argument the platform never asked about.
    #[test]
    fn equation_delete_surrounding_refuses_a_mixed_caret_only_step() {
        let mut s = session();
        s.run("insert.equation", &json!({"latex": "\\frac{a}{b}"})).unwrap();
        // At the end of the numerator. `before: 1` erases `a`, but `after: 1` would only step
        // into the denominator (the report's mixed-direction reproducer).
        s.math.as_mut().expect("editing").pos = MathPos { path: vec![(0, 0)], off: 1 };
        let before = linear(&s);
        let pos = s.math.as_ref().expect("editing").pos.clone();
        let depth = s.undo_depth();
        let err = s.run("equation.deleteSurrounding", &json!({"before": 1, "after": 1})).unwrap_err();
        assert!(err.to_string().contains("no progress"), "{err}");
        assert_eq!(linear(&s), before, "the successful-looking prefix is not applied");
        assert_eq!(s.undo_depth(), depth, "no undo step for a refused mixed request");
        assert_eq!(s.math.as_ref().expect("editing").pos, pos, "the caret stays where the request found it");
    }

    /// Typing more than the cap inserts exactly `MAX_EQUATION_TYPE_CHARS` characters in both linear
    /// and built-up form.
    #[test]
    fn equation_type_caps_linear_and_built_up_input() {
        for linear_mode in [false, true] {
            let mut s = session();
            s.run("insert.equation", &json!({})).unwrap();
            if linear_mode {
                s.run("equation.convert", &json!({"to": "linear"})).unwrap();
            }
            let text = "a".repeat(crate::MAX_EQUATION_TYPE_CHARS + 5);
            s.run("equation.type", &json!({"text": text})).unwrap();
            assert_eq!(linear(&s).chars().count(), crate::MAX_EQUATION_TYPE_CHARS, "linear={linear_mode}");
        }
    }

    /// Deleting one character forward removes it and moves the math caret to the start of the
    /// remaining text.
    #[test]
    fn equation_delete_surrounding_moves_the_caret_for_a_one_step_delete() {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": "abc"})).unwrap();
        s.run("equation.move", &json!({"dir": "home"})).unwrap();
        s.run("equation.deleteSurrounding", &json!({"after": 1, "before": 0})).unwrap();
        assert_eq!(linear(&s), "bc");
        assert_eq!(s.math.as_ref().unwrap().pos, MathPos::new(Vec::new(), 0));
    }
}
