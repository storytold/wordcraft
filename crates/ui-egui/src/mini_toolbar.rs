//! Word's floating mini toolbar: the small translucent bar over a non-collapsed text selection
//! with the most-used formatting controls. It mirrors Word's trigger (selection, not caret),
//! position (above, clamped to the viewport, flipping below when tight) and dismissal rules.

use egui::{Align, CornerRadius, Layout, Order, Rect, Sense, Vec2, pos2};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{WordApp, widgets};

/// Vertical gap between the bar and the selection's first line.
const VGAP: f32 = 8.0;
/// Least height of a selection rectangle (screen points) that counts as visible for anchoring.
const MIN_VISIBLE_H: f32 = 1.0;
/// Distance from the selection's left edge at which the bar is centred.
const ANCHOR_INSET: f32 = 4.0;
/// Padding inside the bar.
const PAD_X: f32 = 4.0;
const PAD_Y: f32 = 3.0;
/// Height of one row of controls (the ribbon's 22 pt buttons plus a little air), and the gap
/// between the two rows.
const ROW_H: f32 = 24.0;
const ROW_GAP: f32 = 2.0;
/// Horizontal gap between neighbouring controls.
const SPACING: f32 = 1.0;
/// The ribbon's widget metrics: a small icon button, a split button (icon part plus its 11 pt
/// arrow, two allocations), a group divider and the size box.
const BTN_W: f32 = 24.0;
const SPLIT_W: f32 = 24.0 + SPACING + 11.0;
const SEP_W: f32 = 9.0;
const COMBO_SIZE_W: f32 = 52.0;
/// Gap between the size box and the grow/shrink buttons.
const GAP: f32 = 3.0;
/// Row 2, like Word's: B I U S | highlight, font colour | the four alignments. Fourteen
/// allocations, so thirteen gaps; the two inside the split buttons are counted in `SPLIT_W`.
const ROW2_W: f32 = 8.0 * BTN_W + 2.0 * SPLIT_W + 2.0 * SEP_W + 11.0 * SPACING;
/// Row 1: font, size, grow, shrink. The font box takes what row 2 leaves, so both rows end
/// together and the box starts at the bar's left edge.
const COMBO_FONT_W: f32 = ROW2_W - (COMBO_SIZE_W + GAP + 2.0 * BTN_W + 3.0 * SPACING);

/// The bar's size: fixed, so it never jitters as the selection changes, and wide enough for every
/// control of both rows (#535: one row of a fixed width cut off everything after Align Left).
pub const BAR_W: f32 = PAD_X * 2.0 + ROW2_W;
pub const BAR_H: f32 = PAD_Y * 2.0 + ROW_H * 2.0 + ROW_GAP;

/// The part of the selection the bar anchors to: the topmost selection rectangle that is actually
/// visible inside `clip` (the scroll viewport), cut down to its visible portion. `None` when no
/// usable part of the selection is on screen. Rectangles that only touch the clip's edge, or are
/// not finite, are ignored. Pure geometry, unit tested without a window.
pub fn visible_anchor(selection_rects: impl IntoIterator<Item = Rect>, clip: Rect) -> Option<Rect> {
    if !clip.is_finite() || !clip.is_positive() {
        return None;
    }
    let mut first: Option<Rect> = None;
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
    }
    first
}

/// Where the bar sits for a selection line and viewport; `None` for degenerate inputs (NaN
/// rects must never reach egui). Pure geometry, unit tested without a window.
pub fn anchor_rect(selection_top: f32, selection_left: f32, viewport: Rect) -> Option<Rect> {
    if !selection_top.is_finite() || !selection_left.is_finite() || !viewport.is_finite() || !viewport.is_positive() {
        return None;
    }
    // Horizontal: centre on the selection's start (a few points in), clamped inside the viewport.
    let cx = (selection_left + ANCHOR_INSET).clamp(viewport.left(), viewport.right());
    let x = (cx - BAR_W / 2.0).clamp(viewport.left(), (viewport.right() - BAR_W).max(viewport.left()));
    // Vertical: prefer above, then below. Check both viewport edges: a selection can be
    // partially scrolled out of view, so merely checking `above >= viewport.top()` is not enough.
    let above = selection_top - VGAP - BAR_H;
    let below = selection_top + VGAP;
    let above_fits = above >= viewport.top() && above + BAR_H <= viewport.bottom();
    let below_fits = below >= viewport.top() && below + BAR_H <= viewport.bottom();
    let y = if above_fits {
        above
    } else if below_fits {
        below
    } else {
        // Neither side has room (or the supplied anchor is outside the viewport). Keep the bar
        // inside the visible area whenever the viewport can accommodate its height.
        let desired = if above < viewport.top() { below } else { above };
        desired.clamp(viewport.top(), (viewport.bottom() - BAR_H).max(viewport.top()))
    };
    Some(Rect::from_min_size(pos2(x, y), Vec2::new(BAR_W, BAR_H)))
}

/// Where the bar and the controls in it were drawn last frame (`[bar, controls]`), kept in the
/// context's memory under this id so tests can check that no control spills out of the bar.
pub fn shown_id() -> egui::Id {
    egui::Id::unique("wc_mini_toolbar").with("shown")
}

/// Whether the bar may be shown for the current app state.
pub fn visible(app: &WordApp) -> bool {
    // Reading Mode permits text selection and copying, but never formatting controls.
    if app.session.view.read_mode {
        return false;
    }
    // Never over a modal dialog or the Backstage.
    if app.dialog.is_some() || app.colors.is_some() || app.ui.backstage {
        return false;
    }
    // Never while the right-click context menu is open: it owns the pointer and would double up.
    if app.canvas.context_menu_open {
        return false;
    }
    // Never while a drag-select is in progress, in the text or a table's selection areas (Word
    // shows it on mouse-up), nor while a table is being moved.
    if app.canvas.drag_selecting() || app.canvas.table_drag.is_some() {
        return false;
    }
    // Never while keytips are showing: the ribbon owns the letters, and the bar's buttons reuse
    // the ribbon's widgets, so they would record duplicate badges.
    if app.ui.keytips != crate::keytips::Phase::Off {
        return false;
    }
    // Nor for whole table cells: pinned above them, it would cover the table's column selection
    // areas, insert buttons and move handle.
    if app.session.cell_range().is_some() {
        return false;
    }
    // Nor once a click outside the document put it away for this selection (`track_dismissal`).
    if app.canvas.mini_dismissed.as_ref() == Some(&app.session.sel) {
        return false;
    }
    // Never for a collapsed caret.
    !app.session.sel.is_collapsed()
}

/// Puts the bar away when the user turns to the ribbon (or a pane, the rulers, the status bar):
/// a press outside the document hides it until the selection changes, so it never covers the
/// ribbon's drop-downs. Runs every frame; `viewport` is the document area.
pub fn track_dismissal(app: &mut WordApp, ctx: &egui::Context, viewport: Rect) {
    if app.canvas.mini_dismissed.as_ref().is_some_and(|d| *d != app.session.sel) {
        app.canvas.mini_dismissed = None;
    }
    let Some(p) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None }) else { return };
    // A press on a popup (the bar's own lists, or a ribbon list already open) neither hides the
    // bar nor brings it back: hiding it would close its own list before the click lands.
    if ctx.layer_id_at(p).is_some_and(|l| l.order >= Order::Foreground) {
        return;
    }
    app.canvas.mini_dismissed = if viewport.contains(p) {
        // A press in the document starts a new selection, which gets the bar again even when it
        // covers the same text.
        None
    } else {
        Some(app.session.sel.clone())
    };
}

/// Show the bar: `selection` is the screen rect of the selection's first line, `viewport` the
/// canvas area it must stay inside.
pub fn show(app: &mut WordApp, ctx: &egui::Context, selection: Rect, viewport: Rect) {
    if !visible(app) {
        return;
    }
    let Some(bar) = anchor_rect(selection.min.y, selection.min.x, viewport) else { return };
    // The bar belongs to the page: as the selection scrolls under the ribbon (or past the status
    // bar) it slides under it with the text, and goes once none of it shows.
    let shown = bar.intersect(viewport);
    if !shown.is_positive() {
        return;
    }
    let t = Tokens::get(ctx);
    // The bar sits on the Middle layer: above the document (Background), below every popup and
    // menu (Foreground: the ribbon's drop-downs, the context menu, its own lists) and tooltip, so
    // it never covers one. It is pinned to the selection, not movable, and egui stops a click at
    // the topmost layer, so a click on the bar never reaches the document underneath. Clipped to
    // the document area (`constrain_to` without `constrain`, which would push it inside), the part
    // past its edge takes no clicks either, so the ribbon and the rulers under it keep them.
    let area = egui::Area::new(egui::Id::unique("wc_mini_toolbar")).order(Order::Middle).interactable(true);
    let area = area.constrain_to(viewport).constrain(false).fixed_pos(bar.min);
    area.show(ctx, |ui| {
        ui.set_clip_rect(shown);
        let painter = ui.painter();
        painter.rect_filled(bar.translate(Vec2::new(0.0, 2.0)), CornerRadius::same(6), t.page_shadow);
        painter.rect_filled(bar, CornerRadius::same(6), t.menu);
        painter.rect_stroke(bar, CornerRadius::same(6), egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
        let inner = bar.shrink2(Vec2::new(PAD_X, PAD_Y));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        child.spacing_mut().item_spacing = Vec2::new(SPACING, ROW_GAP);
        content(app, &mut child, &t);
        let used = child.min_rect();
        ctx.data_mut(|d| d.insert_temp(shown_id(), [bar, used]));
    });
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

/// The existing two-row toolbar in the context menu's popup, above its command panel. Sharing
/// the popup lets its font/size/colour lists stay open without dismissing the parent menu.
pub fn show_in_context_menu(app: &mut WordApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(4, 3)).show(ui, |ui| {
        ui.push_id("context_mini_toolbar", |ui| {
            // Menu styling removes input borders; use the same controls as the standalone bar.
            ui.set_style(ui.ctx().global_style());
            ui.set_width(ROW2_W);
            ui.spacing_mut().item_spacing = Vec2::new(SPACING, ROW_GAP);
            content(app, ui, &t);
            if app.dialog.is_some() || app.colors.is_some() {
                ui.close();
            }
        });
    });
}

/// The controls in two rows like Word's mini toolbar, reusing the ribbon's widgets so the bar
/// matches the ribbon exactly.
fn content(app: &mut WordApp, ui: &mut egui::Ui, t: &Tokens) {
    let st: Value = app.session.run("format.state", &json!({})).unwrap_or_default();
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    // As on the ribbon, the buttons show alignment as seen on the page (#3579).
    let align = app
        .session
        .doc
        .para_at(&app.session.sel.focus)
        .map(|p| {
            let r = app.session.doc.styles.resolve_para(&p.props);
            r.align.visual(r.bidi)
        })
        .unwrap_or_default();
    let row = |ui: &mut egui::Ui, app: &mut WordApp, add: &dyn Fn(&mut egui::Ui, &mut WordApp)| {
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), ROW_H), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_height(ROW_H);
            add(ui, app);
        });
    };

    row(ui, app, &|ui, app| {
        let font = st.get("font").and_then(Value::as_str).unwrap_or("").to_string();
        let font = app.previews.font_shown(&font);
        let fams = app.previews.families();
        let recent: Vec<String> = app.ui.recent_fonts.iter().map(|f| app.previews.font_shown(f)).collect();
        let prev = app.previews.font_preview_fn();
        if let Some(f) = widgets::font_combo_with_recent(ui, "mini_font", COMBO_FONT_W, &font, &fams, &recent, Some(&*prev)) {
            let _ = if f == crate::widgets::ADD_FONT {
                app.run("font.add", json!({}))
            } else if f == crate::widgets::LOAD_SYSTEM_FONTS {
                app.run("font.loadSystem", json!({}))
            } else {
                let f = app.previews.font_stored(&f);
                app.run("format.font", json!({"name": f}))
            };
        }
        let size =
            st.get("size").and_then(Value::as_f64).map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }).unwrap_or_default();
        let sizes: Vec<String> =
            wordcraft_engine::cmd::format::SIZES.iter().map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }).collect();
        if let Some(v) = widgets::combo_hint(ui, "mini_size", COMBO_SIZE_W, &size, &sizes, None, tl!("Font Size"))
            && let Ok(x) = v.trim().parse::<f64>()
        {
            let _ = app.run("format.size", json!({"size": x}));
        }
        ui.add_space(GAP);
        widgets::small(ui, app, "grow", None, "Increase Font Size", crate::widgets::ButtonAction::new("format.growFont", json!({}), false));
        widgets::small(ui, app, "shrink", None, "Decrease Font Size", crate::widgets::ButtonAction::new("format.shrinkFont", json!({}), false));
    });

    row(ui, app, &|ui, app| {
        widgets::small(ui, app, "bold", None, "Bold", crate::widgets::ButtonAction::toggle("format.bold", json!({}), flag("bold")));
        widgets::small(ui, app, "italic", None, "Italic", crate::widgets::ButtonAction::toggle("format.italic", json!({}), flag("italic")));
        widgets::small(
            ui,
            app,
            "underline",
            None,
            "Underline",
            crate::widgets::ButtonAction::toggle("format.underline", json!({}), flag("underline")),
        );
        widgets::small(
            ui,
            app,
            "strike",
            None,
            "Strikethrough",
            crate::widgets::ButtonAction::toggle("format.strikethrough", json!({}), flag("strike")),
        );
        sep(ui, t);

        let hl = app.canvas.last_highlight.clone();
        let (icon, sw, color) = highlight_button_appearance(&hl);
        widgets::split(
            ui,
            app,
            icon,
            "Text Highlight Color",
            crate::widgets::ButtonAction::new("format.highlight", json!({"color": color}), false),
            sw,
            |ui, app| {
                highlight_menu(ui, app);
            },
        );
        let fc = app.canvas.last_font_color.clone();
        let sw = wordcraft_doc::Rgb::parse(&fc).map(crate::theme::c32);
        widgets::split(
            ui,
            app,
            "fontcolor",
            "Font Color",
            crate::widgets::ButtonAction::new("format.color", json!({"color": fc}), false),
            sw,
            |ui, app| {
                font_color_menu(ui, app);
            },
        );
        sep(ui, t);

        use wordcraft_doc::Align as A;
        widgets::small(ui, app, "alignLeft", None, "Align Left", crate::widgets::ButtonAction::toggle("para.alignLeft", json!({}), align == A::Left));
        widgets::small(
            ui,
            app,
            "alignCenter",
            None,
            "Center",
            crate::widgets::ButtonAction::toggle("para.alignCenter", json!({}), align == A::Center),
        );
        widgets::small(
            ui,
            app,
            "alignRight",
            None,
            "Align Right",
            crate::widgets::ButtonAction::toggle("para.alignRight", json!({}), align == A::Right),
        );
        widgets::small(ui, app, "justify", None, "Justify", crate::widgets::ButtonAction::toggle("para.justify", json!({}), align == A::Justify));
    });
}

/// A hairline divider between the bar's groups.
fn sep(ui: &mut egui::Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(SEP_W, 20.0), Sense::hover());
    let p = ui.painter();
    p.vline(r.center().x, r.y_range().shrink(4.0), egui::Stroke::new(1.0, t.border));
}

/// The split-button icon and colour bar for the remembered highlight choice.
pub(crate) fn highlight_button_appearance(name: &str) -> (&'static str, Option<egui::Color32>, &'static str) {
    let highlight = wordcraft_doc::props::Highlight::from_ooxml(name);
    let icon = if highlight == wordcraft_doc::props::Highlight::None { "highlightNoColor" } else { "highlight" };
    (icon, highlight.rgb().map(crate::theme::c32), highlight.ooxml())
}

/// Highlight colour grid and No Color, shared by the ribbon and the mini toolbar. The pick,
/// No Color too, becomes the split button's colour, as in Word: after No Color a click on the
/// button removes highlighting (#2105).
pub(crate) fn highlight_menu(ui: &mut egui::Ui, app: &mut WordApp) {
    egui::Grid::new("mini_hl").spacing(Vec2::new(3.0, 3.0)).show(ui, |ui| {
        for (i, h) in wordcraft_doc::props::Highlight::ALL.iter().enumerate().skip(1) {
            let Some(c) = h.rgb() else { continue };
            let (r, resp) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::click());
            ui.painter().rect_filled(r, 2.0, egui::Color32::from_rgb(c.0, c.1, c.2));
            if resp.on_hover_text(h.name()).clicked() {
                app.canvas.last_highlight = h.ooxml().to_string();
                widgets::run_from_ribbon(app, "format.highlight", json!({"color": h.ooxml()}));
                ui.close();
            }
            if i % 5 == 0 {
                ui.end_row();
            }
        }
    });
    let enabled = widgets::enabled(app, "format.highlight");
    if ui.add_enabled(enabled, egui::Button::new(tl!("No Color")).min_size(Vec2::new(200.0, 0.0))).clicked() {
        app.canvas.last_highlight = "none".into();
        widgets::run_from_ribbon(app, "format.highlight", json!({"color": "none"}));
        ui.close();
    }
}

/// Font colour: Automatic, the colour grid, Recent Colors and More Colors… (#606).
fn font_color_menu(ui: &mut egui::Ui, app: &mut WordApp) {
    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
        if ui.selectable_label(false, "Automatic").clicked() {
            widgets::run_from_ribbon(app, "format.color", json!({"color": "auto"}));
            ui.close();
            return;
        }
        let theme = app.session.doc.settings.theme_colors.clone();
        let target = crate::colors::Target::cmd("format.color").remember(crate::colors::Remember::FontColor);
        crate::colors::menu(ui, app, &theme, target);
    });
}

#[cfg(test)]
mod tests;
