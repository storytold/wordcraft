//! Word's floating mini toolbar: the bar over a non-collapsed text selection with the most-used
//! formatting controls. The framework's selection task bar places it (above the selection, flipped
//! and clamped inside the document area); this module decides when it shows and what it holds.

use craft::behavior::{EditEvent, Preview};
use craft::keymap::CommandRef;
use craft::pages::ui::PageBody;
use craft::ui::buttons::{Button, ControlPlace, GroupLevel, ToggleButton};
use craft::ui::canvas_controls::TaskBar;
use craft::ui::contract::{Bind, Entry, Gap, Hover, Props, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::ComboBox;
use craft::ui::runtime::env::{Label, with_env};
use egui::{Rect, Ui, pos2};
use serde_json::{Value, json};

use crate::WordApp;
use crate::colors::{Remember, Target};

/// Distance from the selection's start (its left edge, or right in a right-to-left paragraph) at which the bar is centred.
const ANCHOR_INSET: f32 = 4.0;
/// Least height of a selection rectangle (screen points) that counts as visible for anchoring.
const MIN_VISIBLE_H: f32 = 1.0;

/// The part of the selection the bar anchors to: the topmost visible line of the selection inside
/// `clip` (the scroll viewport), cut down to its visible portion. `None` when no usable part of the
/// selection is on screen. Rectangles that only touch the clip's edge, or are not finite, are ignored.
/// For `rtl` the result spans every run on that line (see below); otherwise it is the first such
/// rectangle. Pure geometry, unit tested without a window.
pub fn visible_anchor(selection_rects: impl IntoIterator<Item = Rect>, clip: Rect, rtl: bool) -> Option<Rect> {
    if !clip.is_finite() || !clip.is_positive() {
        return None;
    }
    let mut first: Option<Rect> = None;
    let mut visible_rects = Vec::new();
    for sr in selection_rects {
        if !sr.is_finite() || !sr.intersects(clip) {
            continue;
        }
        let visible = sr.intersect(clip);
        // A sliver on the clip's edge is not a visible anchor (and could be zero-sized).
        if visible.width() <= 0.0 || visible.height() < MIN_VISIBLE_H {
            continue;
        }
        if first.is_none_or(|f| visible.min.y < f.min.y) {
            first = Some(visible);
        }
        visible_rects.push(visible);
    }
    let top = first?;
    if !rtl {
        return Some(top);
    }
    // A bidirectional line is emitted as several visual runs, left to right: span the whole line, so
    // its right edge (the start of a right-to-left paragraph) is not that of the first run.
    Some(visible_rects.into_iter().filter(|r| (top.min.y..=top.max.y).contains(&r.center().y)).fold(top, |u, r| u.union(r)))
}

/// The x the bar centres at over `anchor`: its start edge, less the inset, mirrored for right-to-left.
fn start_x(anchor: Rect, rtl: bool) -> f32 {
    if rtl { anchor.max.x - ANCHOR_INSET } else { anchor.min.x + ANCHOR_INSET }
}

/// Whether the paragraph where the selection starts is right-to-left.
pub(crate) fn starts_rtl(app: &WordApp) -> bool {
    let start = app.session.sel.ordered().0;
    app.session.doc.para_at(&start).is_some_and(|p| app.session.doc.styles.resolve_para(&p.props).bidi)
}

/// Whether the bar may be shown for the current app state.
pub fn visible(app: &WordApp) -> bool {
    // Reading Mode permits text selection and copying, but never formatting controls.
    if app.session.view.read_mode {
        return false;
    }
    // Never over a modal dialog or the Backstage.
    if app.modal_open() || app.ui.backstage {
        return false;
    }
    // Never while the right-click context menu is open: it owns the pointer and would double up.
    if app.canvas.context_menu_open {
        return false;
    }
    // Never while a drag-select is in progress, in the text or a table's selection areas (Word
    // shows it on mouse-up), nor while a table is being moved.
    if app.canvas.drag_selecting() || crate::table_mouse::dragging(app) {
        return false;
    }
    // Never while keytips are showing: the ribbon owns the letters, and the bar's buttons reuse
    // the ribbon's widgets, so they would record duplicate badges.
    if app.keytips_active {
        return false;
    }
    // Nor for whole table cells: pinned above them, it would cover the table's column selection
    // areas, insert buttons and move handle.
    if app.session.cell_range().is_some() {
        return false;
    }
    // Nor once a click outside the document put it away for this selection (`TaskBarOutput::closed`).
    if app.canvas.mini_dismissed.as_ref() == Some(&app.session.sel) {
        return false;
    }
    // Never for a collapsed caret.
    !app.session.sel.is_collapsed()
}

/// Shows the bar over the selection's anchor (`CanvasState::mini_anchor`, computed in this pass
/// from this pass's selection rects). It runs inside the page view's body, because the task bar
/// maps its anchor through the canvas view of its environment, and stays inside the view's rect.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, body: &PageBody<'_>) {
    // A new selection brings the bar back, even one over the same text.
    if app.canvas.mini_dismissed.as_ref().is_some_and(|d| *d != app.session.sel) {
        app.canvas.mini_dismissed = None;
    }
    if !app.ui.mini_toolbar || !visible(app) {
        app.canvas.list_preview = None;
        if app.canvas.command_previewed.is_some() {
            ui.ctx().request_repaint();
        }
        return;
    }
    let Some(anchor) = app.canvas.mini_anchor else { return };
    // The bar centres over the selection's start rather than drifting to the centre of long selections (#7447);
    // a right-to-left paragraph at the start puts it at the right edge (#7464).
    let start_x = start_x(anchor, starts_rtl(app));
    let anchor_box = Rect::from_min_max(pos2(start_x, anchor.min.y), pos2(start_x, anchor.max.y));
    let points: Vec<_> = [anchor_box.left_top(), anchor_box.right_top(), anchor_box.right_bottom(), anchor_box.left_bottom()]
        .into_iter()
        .map(|corner| body.view.to_doc(corner))
        .collect();
    let shown = TaskBar::selection(tl!("Formatting").to_string(), &points).show(ui, body.view.rect, |ui, _cx| members(app, ui, true));
    if shown.output.closed {
        app.canvas.mini_dismissed = Some(app.session.sel.clone());
    }
    // The page was drawn before the bar this frame: another frame shows or clears the preview.
    if app.canvas.command_preview != app.canvas.command_previewed {
        ui.ctx().request_repaint();
    }
}

/// A text context menu offers formatting at a caret too. Object and equation menus keep their
/// own controls; the standalone selection toolbar remains hidden while this popup is open.
pub fn context_menu_visible(app: &WordApp) -> bool {
    app.ui.mini_toolbar
        && !app.session.view.read_mode
        && app.session.math.is_none()
        && app.session.cell_range().is_none()
        && !crate::ribbon::has_picture_selected(&app.session)
        && wordcraft_engine::cmd::objects::object_selection(&app.session).is_none()
}

/// The bar's controls in two rows, like Word's: font, size, grow and shrink; then B I U S, the
/// highlight and font colour, and the four alignments. With `preview` the page shows a control's
/// command while the pointer rests on it; the context menu's copy of the bar (which calls this
/// from `canvas/context_menu.rs`) previews nothing.
pub(crate) fn members(app: &mut WordApp, ui: &mut Ui, preview: bool) {
    let st: Value = wordcraft_engine::cmd::format::state(&app.session);
    let flag = |k: &str| match st.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty() && s != "none",
        _ => false,
    };
    // As on the ribbon, the buttons show alignment as seen on the page (#3579).
    let align = app
        .session
        .doc
        .para_at(&app.session.sel.focus)
        .map(|_| {
            let r = wordcraft_engine::cmd::para::selection_para(&app.session);
            r.align.visual(r.bidi)
        })
        .unwrap_or_default();

    // A list's preview, started in an earlier frame, holds until its list ends it.
    if preview && let Some(held) = app.canvas.list_preview.clone() {
        app.canvas.command_preview = Some(held);
    }
    with_env(
        ui,
        |overrides| {
            overrides.values.set(GroupLevel::Compact);
            overrides.values.set(ControlPlace::Ribbon);
        },
        |ui| {
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    let mut family = st.get("font").and_then(Value::as_str).unwrap_or("").to_string();
                    let picked = crate::widgets::font_box::font_box(ui, app, tl!("Font").to_string(), &mut family);
                    follow_list(app, preview, &picked.preview, |f| Some(("format.font".to_string(), json!({"name": f}))));
                    if let Some(f) = &picked.picked {
                        let _ = app.run("format.font", json!({"name": f}));
                    }

                    let size = st.get("size").and_then(Value::as_f64).map(number_text).unwrap_or_default();
                    let sizes: Vec<Entry<String>> = wordcraft_engine::cmd::format::SIZES
                        .iter()
                        .map(|s| Entry::new(number_text(f64::from(*s)), number_text(f64::from(*s))))
                        .collect();
                    let mut text = size.clone();
                    let combo = ComboBox::new(tl!("Font Size").to_string(), Bind::bound(&mut text), &sizes).show(ui);
                    follow_list(app, preview, &combo.preview, |v| {
                        v.trim().parse::<f64>().ok().map(|x| ("format.size".to_string(), json!({"size": x})))
                    });
                    // Only a pick or Enter applies the size: typing alone is an edit of the box's draft.
                    let committed =
                        combo.edit.submitted || combo.edit.events.iter().any(|e| matches!(e, EditEvent::Set { .. } | EditEvent::Commit { .. }));
                    if committed
                        && text != size
                        && let Ok(x) = text.trim().parse::<f64>()
                    {
                        let _ = app.run("format.size", json!({"size": x}));
                    }

                    for (id, icon, tip) in
                        [("format.growFont", "grow", tl!("Increase Font Size")), ("format.shrinkFont", "shrink", tl!("Decrease Font Size"))]
                    {
                        let b = Button::command(CommandRef::new(id)).icon(crate::icons::id(icon)).tooltip(tip.to_string()).show(ui);
                        preview_on_hover(app, b.hover, b.response.enabled(), id, json!({}), preview);
                    }
                });

                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    for (id, key, icon) in [
                        ("format.bold", "bold", "bold"),
                        ("format.italic", "italic", "italic"),
                        ("format.underline", "underline", "underline"),
                        ("format.strikethrough", "strike", "strike"),
                    ] {
                        toggle(ui, app, id, icon, flag(key), preview);
                    }
                    let _ = Divider::vertical().show(ui);

                    let hl = crate::colors::highlight_split(ui, app, tl!("Text Highlight Color").to_string());
                    follow_list(app, preview, &hl.preview, |c| {
                        wordcraft_doc::props::Highlight::ALL
                            .iter()
                            .find(|h| h.rgb().as_ref() == Some(c))
                            .map(|h| ("format.highlight".to_string(), json!({"color": h.ooxml()})))
                    });
                    // PLAN: assumed ColorPick::hover (ribbon-w11, S-L413)
                    let color = app.canvas.last_highlight.clone();
                    preview_on_hover(app, hl.hover, hl.response.enabled(), "format.highlight", json!({"color": color}), preview);
                    if hl.picked.is_some() {
                        app.canvas.focus_page(ui.ctx());
                    }
                    let fc = crate::colors::font_color_split(
                        ui,
                        app,
                        tl!("Font Color").to_string(),
                        Target::cmd("format.color").remember(Remember::FontColor),
                    );
                    follow_list(app, preview, &fc.preview, |c| Some(("format.color".to_string(), json!({"color": c.hex()}))));
                    // PLAN: assumed ColorPick::hover (ribbon-w11, S-L413)
                    let color = app.canvas.last_font_color.clone();
                    preview_on_hover(app, fc.hover, fc.response.enabled(), "format.color", json!({"color": color}), preview);
                    if fc.picked.is_some() {
                        app.canvas.focus_page(ui.ctx());
                    }
                    let _ = Divider::vertical().show(ui);

                    use wordcraft_doc::Align as A;
                    for (id, icon, a) in [
                        ("para.alignLeft", "alignLeft", A::Left),
                        ("para.alignCenter", "alignCenter", A::Center),
                        ("para.alignRight", "alignRight", A::Right),
                        ("para.justify", "justify", A::Justify),
                    ] {
                        toggle(ui, app, id, icon, align == a, preview);
                    }
                });
            });
        },
    );
}

/// Keeps a bar list's preview: the framework reports `Shows` once, when the pointer reaches an
/// option, and `Ended` once, when it leaves the rows, the list closes or a row is picked. The page
/// shows the command `command` makes of the option until then (`CanvasState::list_preview`).
fn follow_list<T>(app: &mut WordApp, preview: bool, event: &Preview<T>, command: impl FnOnce(&T) -> Option<(String, Value)>) {
    match event {
        Preview::Shows(v) if preview => app.canvas.list_preview = command(v),
        Preview::Ended => app.canvas.list_preview = None,
        _ => {}
    }
}

/// An icon toggle running command `id` and drawn `on`; while hovered it previews the command. The
/// command carries no checked state, so the value is bound here and only draws the button.
fn toggle(ui: &mut Ui, app: &mut WordApp, id: &str, icon: &'static str, on: bool, preview: bool) {
    let r = <ToggleButton as Props>::command(ToggleButton::new(Label::Empty, Bind::given(on)), CommandRef::new(id))
        .icon(crate::icons::id(icon))
        .icon_only()
        .show(ui);
    preview_on_hover(app, r.hover, r.response.enabled(), id, json!({}), preview);
}

/// Previews command `id` with `params` while its control is hovered and not clicked. After a
/// click the control's `Hover::clicked` holds until the pointer leaves, so a toggle does not
/// preview its other state under the pointer and look as if it flashed.
fn preview_on_hover(app: &mut WordApp, hover: Hover, enabled: bool, id: &str, params: Value, preview: bool) {
    if preview && enabled && hover.over && !hover.clicked {
        app.canvas.command_preview = Some((id.to_string(), params));
    }
}

/// A size as the box shows it: whole numbers without a fraction.
fn number_text(s: f64) -> String {
    if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }
}

#[cfg(test)]
mod tests;
