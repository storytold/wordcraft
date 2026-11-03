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
mod tests;
