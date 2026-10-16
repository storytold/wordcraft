//! The equation editor: equaSHion's [`EquationEditor`] widget over the equation on the page,
//! with no box: typing reads as typing in the text.
//!
//! It opens wherever editing an equation starts (`Session::math` becomes set): Alt+=,
//! Insert › Equation, a click or double-click on an equation, the Equation tab's galleries
//! outside an equation, an agent's `insert.equation`. The equation on the page is laid out by
//! equaSHion (see `wordcraft-layout`), so the widget, which draws nothing but the caret,
//! selection and active place, sits exactly on it. Typing and the Equation tab change the
//! document as they go, through the engine's own commands (`equation.set`), folded into one undo
//! step, so Undo, Track Changes and save work as for any other edit:
//! - Enter, Esc, a click elsewhere on the page or Close Equation leave the equation, keeping
//!   the edit, as leaving an equation does in Word; a new equation never filled goes away again.
//!
//! While the editor is open, the Equation tab's structure, symbol and ready-made galleries insert
//! into the widget rather than into the document, and the tab's options (display, justification,
//! number, conversion) first apply the pending edit. The widget never touches the shared egui
//! context's fonts, visuals or style; it draws with its own bundled fonts.

use equashion_egui::{Edge, EquationEditor};
use serde_json::{Value, json};
use wordcraft_doc::math::{Arg, Math, parse_linear};
use wordcraft_doc::{InlineObject, Pos};
use wordcraft_engine::Selection;
use wordcraft_engine::equashion_bridge as bridge;

use crate::WordApp;
use crate::theme::Tokens;

/// Undo labels of the commands that insert a new equation (`insert.equation`, and the
/// Equation tab's structure and symbol galleries used outside an equation).
const INSERT_STEPS: &[&str] = &["Equation", "Insert Structure", "Insert Symbol"];

/// Commands that change the edited equation in the document: the pending edit goes in first.
const APPLY_FIRST: &[&str] = &["equation.display", "equation.justify", "equation.number", "equation.convert", "equation.structure"];

/// The editor while it is open.
pub(crate) struct Open {
    widget: EquationEditor,
    /// The equation's place in the document.
    at: Pos,
    /// The equation as last loaded from (or applied to) the document, to notice changes made
    /// elsewhere (an agent, a ribbon option).
    loaded: Equation,
    /// The equation was inserted right before the editor opened.
    fresh: bool,
    /// An edit has been applied to the document since the editor opened.
    applied: bool,
    /// Undo depth when the editor opened (or last applied).
    depth: usize,
    /// Why the last apply failed.
    error: Option<String>,
}

/// An equation object's parts: linear text, display flag, tree.
type Equation = (String, bool, Math);

/// The equation editor's state in [`WordApp`].
#[derive(Default)]
pub struct EquationHost {
    pub(crate) open: Option<Open>,
    /// Undo depth at the end of the last frame.
    seen_depth: usize,
    /// The editor is about to open because an arrow key crossed into the equation: the caret
    /// starts at this edge, and that same key press must not also move it.
    entering: Option<Edge>,
}

impl EquationHost {
    /// The editor window is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The open editor's widget.
    pub fn widget(&self) -> Option<&EquationEditor> {
        self.open.as_ref().map(|o| &o.widget)
    }

    /// The open editor's widget, mutable.
    pub fn widget_mut(&mut self) -> Option<&mut EquationEditor> {
        self.open.as_mut().map(|o| &mut o.widget)
    }
}

/// Left or Right with the caret directly after or before an inline equation (nothing selected)
/// goes into the equation, at that end, instead of stepping over it.
fn enter_by_arrow(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let left = match id {
        "caret.left" => true,
        "caret.right" => false,
        _ => return None,
    };
    if params.get("extend").and_then(Value::as_bool).unwrap_or(false) || app.session.sel.anchor != app.session.sel.focus {
        return None;
    }
    let f = app.session.sel.focus.clone();
    let para = app.session.doc.para_at(&f)?;
    let is_eq = |off: usize| matches!(para.object_at(off), Some(InlineObject::Equation { .. }));
    let hit = if left { f.off.checked_sub(wordcraft_doc::para::OBJ.len_utf8()).is_some_and(is_eq) } else { is_eq(f.off) };
    if !hit {
        return None;
    }
    // Entering from the right end of the text puts the caret at the equation's right end.
    app.equation.entering = Some(if left { Edge::Right } else { Edge::Left });
    Some(app.execute("equation.edit", json!({})))
}

/// The equation object at `at`.
fn equation_at(app: &WordApp, at: &Pos) -> Option<Equation> {
    match app.session.doc.para_at(at)?.object_at(at.off)? {
        InlineObject::Equation { linear, display, math } => Some((linear.clone(), *display, math.clone())),
        _ => None,
    }
}

fn load(widget: &mut EquationEditor, eq: &Equation) {
    widget.set_math(bridge::to_editor(&eq.0, eq.1, &eq.2));
}

/// Open the editor on the equation at `at`; `fresh` when it was inserted just now.
fn open(app: &mut WordApp, at: Pos, fresh: bool) {
    let Some(loaded) = equation_at(app, &at) else { return };
    let depth = app.session.undo_depth();
    // Arrow keys at either end of the equation hand the caret back to the text around it.
    let mut widget = EquationEditor::new().inline(true).overlay(true).exit_at_edges(true).id_salt("wordcraft-equation-editor");
    load(&mut widget, &loaded);
    if let Some(edge) = app.equation.entering {
        widget.caret_to(edge);
    }
    app.equation.open = Some(Open { widget, at, loaded, fresh, applied: false, depth, error: None });
}

/// Apply the widget's equation to the document (`equation.set`, and `equation.display` when
/// the display toggle changed), as one undo step, joined to the insert when the equation is new.
fn apply(app: &mut WordApp, o: &mut Open) -> Result<(), String> {
    let Some(before) = equation_at(app, &o.at) else { return Err(tl!("The equation is no longer in the document.").to_string()) };
    o.widget.editor_mut().flush();
    let m = o.widget.math().clone();
    let display = m.para;
    // Point the equation commands at this equation, even when the caret has moved on.
    let here = app.session.math.as_ref().is_some_and(|e| e.at == o.at);
    let saved = (!here).then(|| (app.session.math.take(), app.session.sel.clone()));
    if !here {
        app.session.sel = Selection::caret(o.at.clone());
    }
    // One undo step for the whole edit: a new equation joins the insert that made it, and each
    // later apply joins the one before it (when nothing else was done in between).
    if (o.fresh || o.applied) && app.session.undo_depth() == o.depth {
        app.session.join_next_undo();
    }
    let r = app.execute("equation.set", json!({"omml": bridge::editor_omml(&m)}));
    if r.is_ok() && display != before.1 {
        app.session.join_next_undo();
        let _ = app.execute("equation.display", json!({"value": display}));
    }
    if let Some((math, sel)) = saved {
        app.session.math = math;
        app.session.sel = sel;
    }
    r?;
    o.widget.mark_clean();
    o.applied = true;
    o.depth = app.session.undo_depth();
    o.loaded = equation_at(app, &o.at).unwrap_or(before);
    o.error = None;
    Ok(())
}

/// The new, never filled equation the editor opened on: its insertion is still the last undo step.
fn untouched_new(app: &WordApp, o: &Open) -> bool {
    o.fresh && app.session.undo_depth() == o.depth && equation_at(app, &o.at).is_some_and(|e| e.0.is_empty() && e.2.nodes.is_empty())
}

/// Drop the edit: a new empty equation goes away again, otherwise the caret leaves it.
fn cancel(app: &mut WordApp, o: Open) {
    if untouched_new(app, &o) {
        let _ = app.execute("edit.undo", json!({}));
    } else if app.session.math.as_ref().is_some_and(|e| e.at == o.at) {
        let _ = app.execute("equation.exit", json!({}));
    }
    app.canvas.want_focus = true;
}

/// Enter / Insert / Update: apply and leave the equation. The editor stays open with the
/// reason when that fails.
fn commit(app: &mut WordApp, o: Open) -> Option<Open> {
    commit_to(app, o, false)
}

/// [`commit`], leaving the caret before the equation instead of after it when `before`.
fn commit_to(app: &mut WordApp, mut o: Open, before: bool) -> Option<Open> {
    if o.widget.editor().is_blank() && untouched_new(app, &o) {
        cancel(app, o);
        return None;
    }
    if let Err(e) = apply(app, &mut o) {
        o.error = Some(e);
        return Some(o);
    }
    if app.session.math.as_ref().is_some_and(|e| e.at == o.at) {
        let _ = app.execute("equation.exit", json!({"before": before}));
    }
    app.canvas.want_focus = true;
    None
}

/// Close without asking, keeping a pending edit (the equation was left some other way).
fn keep_and_close(app: &mut WordApp, mut o: Open) {
    if o.widget.is_dirty() {
        let _ = apply(app, &mut o);
    }
}

/// Follow the session: open on the equation being edited, keep the edit and close when the
/// editing ends some other way, reload when the document's copy changes and nothing is pending.
fn sync(app: &mut WordApp) {
    // The equation is gone (Undo, deleted elsewhere): nothing to apply it to.
    if let Some(o) = &app.equation.open
        && equation_at(app, &o.at).is_none()
    {
        app.equation.open = None;
    }
    let editing = app.session.math.as_ref().map(|m| m.at.clone());
    // An equation was inserted since the last frame (and is the one being edited now).
    let fresh = app.session.undo_depth() > app.equation.seen_depth && app.session.undo_label().is_some_and(|l| INSERT_STEPS.contains(&l));
    match (editing, app.equation.open.take()) {
        (Some(at), None) => open(app, at, fresh),
        (Some(at), Some(o)) if o.at != at => {
            keep_and_close(app, o);
            open(app, at, fresh);
        }
        (None, Some(o)) => {
            keep_and_close(app, o);
            app.canvas.want_focus = true;
        }
        (_, o) => app.equation.open = o,
    }
    if let Some(mut o) = app.equation.open.take() {
        if let Some(now) = equation_at(app, &o.at)
            && now != o.loaded
            && !o.widget.is_dirty()
        {
            load(&mut o.widget, &now);
            o.loaded = now;
        }
        app.equation.open = Some(o);
    }
}

/// Draw the editor over the equation (after everything that may start or end editing in this
/// frame).
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    sync(app);
    if let Some(mut o) = app.equation.open.take() {
        // The page took the keyboard (a click on the equation, a ribbon button): typing still
        // belongs to the equation (once it is on screen to be typed into).
        if app.canvas.focused && app.canvas.equation_screen.is_some() {
            let id = o.widget.canvas_id();
            ctx.memory_mut(|m| m.request_focus(id));
        }
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut leave = esc;
        let mut changed = false;
        let mut exit: Option<Edge> = None;
        // The arrow press that brought the caret into the equation is spent.
        if app.equation.entering.take().is_some() {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
            });
        }
        if let Some((x, baseline, pt)) = app.canvas.equation_screen {
            o.widget.set_font_size(f64::from(pt).clamp(4.0, 1200.0));
            // Laid out in the style the page draws it in (inline: text style).
            if let Some(eq) = equation_at(app, &o.at) {
                o.widget.set_display_style(eq.1);
            }
            let pad = EquationEditor::inline_pad();
            let base = o.widget.inline_baseline().unwrap_or((pad.y + 0.8 * pt).max(1.0));
            let t = Tokens::get(ctx);
            egui::Area::new(egui::Id::new("wordcraft-equation-in-place"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(x - pad.x, baseline - base))
                .show(ctx, |ui| {
                    ui.visuals_mut().selection.stroke.color = t.accent;
                    let r = o.widget.show(ui);
                    changed = r.changed;
                    leave |= r.committed || r.cancelled;
                    exit = r.exit;
                });
            ctx.request_repaint();
        }
        if changed && let Err(e) = apply(app, &mut o) {
            o.error = Some(e);
        }
        if let Some(edge) = exit.filter(|_| !leave) {
            // An arrow at the end of the top row: keep the edit and carry on in the text, on the
            // side the arrow pointed to.
            app.equation.open = commit_to(app, o, edge == Edge::Left);
        } else if leave {
            app.equation.open = commit(app, o);
        } else {
            app.equation.open = Some(o);
        }
    }
    app.equation.seen_depth = app.session.undo_depth();
}

/// The upstream nodes of an Equation-tab insertion (`id`, `linear`, `latex`, `omml`).
fn gallery_nodes(params: &Value) -> Option<Arg> {
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    if let Some(id) = s("id") {
        return wordcraft_engine::math_gallery::template(id).map(|t| t.nodes());
    }
    if let Some(b) = s("builtin") {
        return wordcraft_engine::math_gallery::BUILT_INS.iter().find(|(id, _, _)| *id == b).map(|(_, _, lin)| parse_linear(lin));
    }
    if let Some(x) = s("omml") {
        return bridge::omml_equation(x).map(|(m, _)| m.nodes);
    }
    if let Some(l) = s("latex") {
        return Some(wordcraft_doc::math_latex::parse_latex(l));
    }
    s("linear").map(parse_linear)
}

/// A user command while the editor is open: the Equation tab's galleries insert into the widget,
/// and its options apply the pending edit first. `Some` when the command was handled here.
pub(crate) fn intercept(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if app.equation.open.is_none() {
        return enter_by_arrow(app, id, params);
    }
    let into_widget = matches!(id, "equation.insertStructure" | "equation.insertSymbol")
        || (id == "insert.equation" && params.get("builtin").and_then(Value::as_str).is_some());
    if into_widget {
        let mut o = app.equation.open.take()?;
        if id == "equation.insertSymbol" {
            let text = params.get("char").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                app.equation.open = Some(o);
                return Some(Err("empty char".into()));
            }
            for c in text.chars().take(64) {
                o.widget.editor_mut().insert_symbol(c);
            }
        } else {
            let Some(nodes) = gallery_nodes(params) else {
                app.equation.open = Some(o);
                return Some(Err("unknown equation".into()));
            };
            let tpl = bridge::to_editor("", false, &Math { nodes, ..Default::default() });
            o.widget.editor_mut().insert_template(&tpl);
        }
        o.widget.sync_linear();
        // Into the document at once, like typing.
        if let Err(e) = apply(app, &mut o) {
            o.error = Some(e);
        }
        let linear = o.widget.linear();
        app.equation.open = Some(o);
        return Some(Ok(json!({"equationEditor": true, "linear": linear})));
    }
    if APPLY_FIRST.contains(&id)
        && let Some(mut o) = app.equation.open.take()
    {
        let r = if o.widget.is_dirty() { apply(app, &mut o) } else { Ok(()) };
        app.equation.open = Some(o);
        if let Err(e) = r {
            return Some(Err(e));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use egui::vec2;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::math::MNode;
    use wordcraft_engine::Session;

    use super::*;
    use crate::Services;

    fn harness() -> Harness<'static, WordApp> {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        h
    }

    fn steps(h: &mut Harness<'static, WordApp>, n: usize) {
        for _ in 0..n {
            h.step();
        }
    }

    fn equations(app: &WordApp) -> Vec<Equation> {
        let s = &app.session;
        let mut out = Vec::new();
        for path in s.doc.para_paths(wordcraft_doc::StoryRef::Body) {
            if let Some(p) = s.doc.para(wordcraft_doc::StoryRef::Body, &path) {
                for off in p.object_offsets() {
                    if let Some(InlineObject::Equation { linear, display, math }) = p.object_at(off) {
                        out.push((linear.clone(), *display, math.clone()));
                    }
                }
            }
        }
        out
    }

    fn type_text(h: &mut Harness<'static, WordApp>, t: &str) {
        h.event(egui::Event::Text(t.into()));
        steps(h, 2);
    }

    /// Right at the end of the equation goes on in the text after it; Left from there goes back
    /// in at the right end; Left at the start of the equation goes on in the text before it.
    #[test]
    fn arrows_cross_the_equation_boundary_both_ways() {
        let mut h = harness();
        h.state_mut().run("text.insert", json!({"text": "ab"})).unwrap();
        // (Not Alt+=: it arms the canvas to swallow the "=" a real keyboard then types.)
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.state().equation.is_open());
        type_text(&mut h, "x");
        let at = h.state().equation.open.as_ref().map(|o| o.at.clone()).expect("open");
        // Right at the end of the equation: out, caret after it, typing is plain text.
        h.key_press(egui::Key::ArrowRight);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open(), "Right at the end leaves the equation");
        assert_eq!(h.state().session.sel.focus.off, at.off + wordcraft_doc::para::OBJ.len_utf8(), "caret right after it");
        type_text(&mut h, "c");
        assert_eq!(equations(h.state()).len(), 1);
        assert_eq!(h.state().session.doc.para_at(&at).map(|p| p.text.clone()).unwrap_or_default(), format!("ab{}c", wordcraft_doc::para::OBJ));
        // Left from just after the equation is where text meets it: step Left over "c", then
        // Left again goes into the equation, at its right end.
        h.key_press(egui::Key::ArrowLeft);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open(), "Left from after the text only steps over it");
        h.key_press(egui::Key::ArrowLeft);
        steps(&mut h, 3);
        assert!(h.state().equation.is_open(), "Left next to the equation goes into it");
        assert_eq!(h.state().equation.widget().map(|w| w.linear()), Some("x".to_string()));
        type_text(&mut h, "y");
        assert_eq!(h.state().equation.widget().map(|w| w.linear()), Some("xy".to_string()), "entered at the right end");
        // Left through the equation and out at its start: caret before it.
        for _ in 0..3 {
            h.key_press(egui::Key::ArrowLeft);
            steps(&mut h, 2);
        }
        assert!(!h.state().equation.is_open(), "Left at the start leaves the equation");
        assert_eq!(h.state().session.sel.focus.off, at.off, "caret right before it");
    }

    /// Text typed after leaving an equation with the arrow looks like the text around it.
    #[test]
    fn text_after_the_equation_has_the_surrounding_formatting() {
        let mut h = harness();
        h.state_mut().run("format.bold", json!({})).unwrap();
        h.state_mut().run("text.insert", json!({"text": "ab"})).unwrap();
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        type_text(&mut h, "x");
        let at = h.state().equation.open.as_ref().map(|o| o.at.clone()).expect("open");
        h.key_press(egui::Key::ArrowRight);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open());
        type_text(&mut h, "c");
        let para = h.state().session.doc.para_at(&at).expect("paragraph").clone();
        let off_c = para.text.find('c').expect("the typed c");
        let off_a = para.text.find('a').expect("the a");
        assert_eq!(para.props_at(off_c).bold, para.props_at(off_a).bold, "c is formatted like ab");
        assert_eq!(para.props_at(off_c).bold, Some(true), "and both are bold");
    }

    #[test]
    fn alt_equals_opens_the_widget_and_enter_inserts_in_one_undo_step() {
        let mut h = harness();
        assert!(h.state().canvas.focused, "the page has the keyboard");
        h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Equals);
        steps(&mut h, 3);
        assert!(h.state().equation.is_open(), "the editor opened on the new equation");
        // "Insert" is also the ribbon tab's name now, so only the dialog's Cancel tells a window apart.
        assert!(h.query_by_label("Cancel").is_none(), "no window");
        let canvas = h.state().equation.widget().map(|w| w.canvas_id());
        assert!(canvas.is_some_and(|c| h.ctx.memory(|m| m.has_focus(c))), "typing goes to the widget");
        let depth = h.state().session.undo_depth();
        type_text(&mut h, "x^2");
        assert_eq!(h.state().equation.widget().map(|w| w.linear()), Some("x^2".to_string()));
        // Typed into the equation itself: the document has it as it goes.
        assert!(matches!(equations(h.state())[0].2.nodes.as_slice(), [MNode::Script { .. }]), "{:?}", equations(h.state())[0].2.nodes);
        h.key_press(egui::Key::Enter);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open(), "Enter closes the editor");
        assert!(h.state().session.math.is_none(), "and leaves the equation");
        let eqs = equations(h.state());
        assert_eq!(eqs.len(), 1);
        assert!(matches!(eqs[0].2.nodes.as_slice(), [MNode::Script { .. }]), "{:?}", eqs[0].2.nodes);
        assert_eq!(h.state().session.undo_depth(), depth, "insert and edit are one undo step");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        steps(&mut h, 2);
        assert!(equations(h.state()).is_empty(), "one Undo takes the whole equation back");
    }

    #[test]
    fn esc_keeps_the_edit_and_a_new_empty_equation_goes_away() {
        let mut h = harness();
        h.state_mut().run("insert.equation", json!({"linear": "a/b", "edit": false})).unwrap();
        steps(&mut h, 2);
        // Re-edit the existing equation, as a click on it does.
        let at = h.state().session.sel.focus.clone();
        h.state_mut().session.sel = Selection::caret(Pos { off: at.off.saturating_sub(wordcraft_doc::para::OBJ.len_utf8()), ..at });
        h.state_mut().run("equation.edit", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.state().equation.is_open());
        assert!(h.query_by_label("Update").is_none(), "no window");
        let depth = h.state().session.undo_depth();
        type_text(&mut h, "+c");
        assert_eq!(equations(h.state())[0].0, "a/b+c", "typed into the equation on the page");
        type_text(&mut h, "+d");
        h.key_press(egui::Key::Escape);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open(), "Esc leaves the equation");
        assert_eq!(equations(h.state())[0].0, "a/b+c+d", "and keeps the edit");
        assert_eq!(h.state().session.undo_depth(), depth + 1, "the whole edit is one undo step");

        // A new equation closed untouched leaves nothing behind.
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        assert_eq!(equations(h.state()).len(), 2);
        h.key_press(egui::Key::Escape);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open());
        assert_eq!(equations(h.state()).len(), 1, "the empty new equation is gone");
    }

    #[test]
    fn equation_tab_galleries_insert_into_the_widget() {
        let mut h = harness();
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        h.state_mut().run("equation.insertStructure", json!({"id": "frac.stacked"})).unwrap();
        h.state_mut().run("equation.insertSymbol", json!({"char": "π"})).unwrap();
        steps(&mut h, 2);
        let tree = h.state().equation.widget().map(|w| w.math().clone()).unwrap_or_default();
        assert!(matches!(tree.body.first().map(|n| n.bare()), Some(equashion_egui::equashion_math::Node::Frac { .. })), "{tree:?}");
        // And into the document at once.
        assert!(matches!(equations(h.state())[0].2.nodes.first(), Some(MNode::Frac { .. })));
        h.key_press(egui::Key::Enter);
        steps(&mut h, 3);
        assert!(matches!(equations(h.state())[0].2.nodes.first(), Some(MNode::Frac { .. })));
    }

    #[test]
    fn leaving_the_equation_keeps_the_edit_and_options_apply_it_first() {
        let mut h = harness();
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        type_text(&mut h, "y");
        // An Equation tab option applies the pending edit before changing the equation.
        h.state_mut().run("equation.display", json!({"value": false})).unwrap();
        steps(&mut h, 2);
        let e = &equations(h.state())[0];
        assert_eq!((e.0.as_str(), e.1), ("y", false));
        type_text(&mut h, "+1");
        // Close Equation (or a click elsewhere) ends the editing: the edit stays.
        h.state_mut().run("equation.exit", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open());
        assert_eq!(equations(h.state())[0].0, "y+1");
    }
}
