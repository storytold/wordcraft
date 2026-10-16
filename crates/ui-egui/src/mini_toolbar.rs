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

/// Paste Options after a paste (#820): Keep Source, Merge Formatting, Keep Text Only. An
/// Floating Paste Options widget (#820) appearing at the end of the pasted text: choosing an
/// option undoes the paste and redoes it in that form; if re-pasting fails, edit.redo restores the
/// original paste so content isn't lost. Anything else dismisses the widget (see `after_command`).
pub fn paste_options(app: &mut WordApp, ctx: &egui::Context) {
    let Some(at) = app.canvas.paste_options else { return };
    if app.dialog.is_some() || app.ui.backstage {
        return;
    }
    egui::Area::new(egui::Id::new("wc_paste_options")).order(Order::Tooltip).interactable(true).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (label, id) in [
                    ("Keep Source Formatting", "edit.paste"),
                    ("Match Destination Formatting", "edit.pasteMerge"),
                    ("Keep Text Only", "edit.pasteText"),
                ] {
                    if ui.small_button(crate::i18n::t(label)).clicked() && app.run("edit.undo", json!({})).is_ok() && app.run(id, json!({})).is_err()
                    {
                        let _ = app.run("edit.redo", json!({}));
                    }
                }
            });
        });
    });
}

/// Where the bar and the controls in it were drawn last frame (`[bar, controls]`), kept in the
/// context's memory under this id so tests can check that no control spills out of the bar.
pub fn shown_id() -> egui::Id {
    egui::Id::new("wc_mini_toolbar").with("shown")
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
    let area = egui::Area::new(egui::Id::new("wc_mini_toolbar")).order(Order::Middle).interactable(true);
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

/// The controls in two rows like Word's mini toolbar, reusing the ribbon's widgets so the bar
/// matches the ribbon exactly.
fn content(app: &mut WordApp, ui: &mut egui::Ui, t: &Tokens) {
    let st: Value = app.session.run("format.state", &json!({})).unwrap_or_default();
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    let align = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props).align).unwrap_or_default();
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
        let prev = app.previews.font_preview_fn();
        if let Some(f) = widgets::font_combo(ui, "mini_font", COMBO_FONT_W, &font, &fams, Some(&*prev)) {
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
        if let Some(v) = widgets::combo(ui, "mini_size", COMBO_SIZE_W, &size, &sizes, None)
            && let Ok(x) = v.trim().parse::<f64>()
        {
            let _ = app.run("format.size", json!({"size": x}));
        }
        ui.add_space(GAP);
        widgets::small(ui, app, "grow", None, "Increase Font Size", "format.growFont", json!({}), false);
        widgets::small(ui, app, "shrink", None, "Decrease Font Size", "format.shrinkFont", json!({}), false);
    });

    row(ui, app, &|ui, app| {
        widgets::small(ui, app, "bold", None, "Bold", "format.bold", json!({}), flag("bold"));
        widgets::small(ui, app, "italic", None, "Italic", "format.italic", json!({}), flag("italic"));
        widgets::small(ui, app, "underline", None, "Underline", "format.underline", json!({}), flag("underline"));
        widgets::small(ui, app, "strike", None, "Strikethrough", "format.strikethrough", json!({}), flag("strike"));
        sep(ui, t);

        let hl = app.canvas.last_highlight.clone();
        widgets::split(ui, app, "highlight", "Text Highlight Color", "format.highlight", json!({"color": hl}), false, None, |ui, app| {
            highlight_menu(ui, app);
        });
        let fc = app.canvas.last_font_color.clone();
        let sw = wordcraft_doc::Rgb::parse(&fc).map(crate::theme::c32);
        widgets::split(ui, app, "fontcolor", "Font Color", "format.color", json!({"color": fc}), false, sw, |ui, app| {
            font_color_menu(ui, app);
        });
        sep(ui, t);

        use wordcraft_doc::Align as A;
        widgets::small(ui, app, "alignLeft", None, "Align Left", "para.alignLeft", json!({}), align == A::Left);
        widgets::small(ui, app, "alignCenter", None, "Center", "para.alignCenter", json!({}), align == A::Center);
        widgets::small(ui, app, "alignRight", None, "Align Right", "para.alignRight", json!({}), align == A::Right);
        widgets::small(ui, app, "justify", None, "Justify", "para.justify", json!({}), align == A::Justify);
    });
}

/// A hairline divider between the bar's groups.
fn sep(ui: &mut egui::Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(SEP_W, 20.0), Sense::hover());
    let p = ui.painter();
    p.vline(r.center().x, r.y_range().shrink(4.0), egui::Stroke::new(1.0, t.border));
}

/// Highlight colour grid, matching the ribbon's.
fn highlight_menu(ui: &mut egui::Ui, app: &mut WordApp) {
    egui::Grid::new("mini_hl").spacing(Vec2::new(3.0, 3.0)).show(ui, |ui| {
        for (i, h) in wordcraft_doc::props::Highlight::ALL.iter().enumerate().skip(1) {
            let Some(c) = h.rgb() else { continue };
            let (r, resp) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::click());
            ui.painter().rect_filled(r, 2.0, egui::Color32::from_rgb(c.0, c.1, c.2));
            if resp.on_hover_text(h.name()).clicked() {
                app.canvas.last_highlight = h.ooxml().to_string();
                let _ = app.run("format.highlight", json!({"color": h.ooxml()}));
                ui.close();
            }
            if i % 5 == 0 {
                ui.end_row();
            }
        }
    });
    if ui.selectable_label(false, "No Color").clicked() {
        let _ = app.run("format.highlight", json!({"color": "none"}));
        ui.close();
    }
}

/// Font colour: Automatic, the colour grid, Recent Colors and More Colors… (#606).
fn font_color_menu(ui: &mut egui::Ui, app: &mut WordApp) {
    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
        if ui.selectable_label(false, "Automatic").clicked() {
            let _ = app.run("format.color", json!({"color": "auto"}));
            ui.close();
            return;
        }
        let theme = app.session.doc.settings.theme_colors.clone();
        let target = crate::colors::Target::cmd("format.color").remember(crate::colors::Remember::FontColor);
        crate::colors::menu(ui, app, &theme, target);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), Vec2::new(1200.0, 700.0))
    }

    #[test]
    fn reading_mode_hides_formatting_even_with_an_existing_selection() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("text.insert", json!({"text": "Selectable text"})).unwrap();
        app.run("select.all", json!({})).unwrap();
        assert!(!app.session.sel.is_collapsed());
        assert!(visible(&app), "selection shows the formatting bar in editing mode");

        app.run("view.readMode", json!({})).unwrap();
        assert!(!app.session.sel.is_collapsed(), "entering Reading Mode must preserve the selection");
        assert!(!visible(&app), "formatting bar must not be accessible in Reading Mode");

        app.run("view.printLayout", json!({})).unwrap();
        assert!(visible(&app), "returning to editing restores the formatting bar");
    }

    #[test]
    fn sits_above_the_selection_when_there_is_room() {
        let left = vp().center().x;
        let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
        assert_eq!(r.size(), Vec2::new(BAR_W, BAR_H));
        assert!((r.max.y - (400.0 - VGAP)).abs() < 1e-3, "bar bottom should clear the selection by VGAP");
        assert!(r.max.y < 400.0, "the bar must be above the selection");
        assert!((r.center().x - (left + ANCHOR_INSET)).abs() < 1e-3, "bar should be centred on the selection's left edge");
    }

    #[test]
    fn centred_unless_the_viewport_forces_a_shift() {
        let left = vp().min.x + 20.0;
        let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
        assert!((r.min.x - vp().min.x).abs() < 1e-3, "the bar must be pushed onto the viewport's left edge");
        let right = vp().max.x - 20.0;
        let r = anchor_rect(400.0, right, vp()).unwrap_or(Rect::EVERYTHING);
        assert!((r.max.x - vp().max.x).abs() < 1e-3, "the bar must be pushed onto the viewport's right edge");
    }

    #[test]
    fn flips_below_when_there_is_no_room_above() {
        let r = anchor_rect(60.0, 300.0, vp()).unwrap_or(Rect::EVERYTHING);
        assert!((r.min.y - (60.0 + VGAP)).abs() < 1e-3, "bar top should clear the selection by VGAP below");
        assert!(r.min.y > 60.0, "the bar must be below the selection");
    }

    #[test]
    fn never_leaves_the_viewport() {
        for left in [0.0, 130.0, 400.0, 2000.0, -500.0] {
            let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
            assert!(r.min.x >= vp().min.x - 1e-3, "left {left}: bar left {} out of viewport", r.min.x);
            assert!(r.max.x <= vp().max.x + 1e-3, "left {left}: bar right {} out of viewport", r.max.x);
        }
    }

    #[test]
    fn vertical_position_stays_inside_viewport_for_offscreen_anchor() {
        let viewport = vp();
        for selection_top in [viewport.top() - 500.0, viewport.bottom() + 500.0] {
            let r = anchor_rect(selection_top, viewport.center().x, viewport).unwrap_or(Rect::EVERYTHING);
            assert!(r.min.y >= viewport.top() - 1e-3, "bar top {} is above viewport {}", r.min.y, viewport.top());
            assert!(r.max.y <= viewport.bottom() + 1e-3, "bar bottom {} exceeds viewport {}", r.max.y, viewport.bottom());
        }
    }

    #[test]
    fn anchor_skips_rects_scrolled_offscreen_and_clips_partial_ones() {
        let clip = vp();
        // Page 1's line is above the viewport, page 2's line is inside it.
        let above = Rect::from_min_size(pos2(100.0, clip.top() - 300.0), Vec2::new(200.0, 20.0));
        let inside = Rect::from_min_size(pos2(120.0, clip.top() + 50.0), Vec2::new(200.0, 20.0));
        assert_eq!(visible_anchor([above, inside], clip), Some(inside));
        // Partly scrolled off the top: anchored to the visible part, never above the viewport.
        let partial = Rect::from_min_size(pos2(100.0, clip.top() - 10.0), Vec2::new(200.0, 20.0));
        let a = visible_anchor([partial], clip).unwrap();
        assert!((a.min.y - clip.top()).abs() < 1e-3 && (a.height() - 10.0).abs() < 1e-3);
        // Entirely off either end, or only touching the edge: nothing to anchor to.
        let below = Rect::from_min_size(pos2(100.0, clip.bottom() + 5.0), Vec2::new(200.0, 20.0));
        let touching = Rect::from_min_size(pos2(100.0, clip.top() - 20.0), Vec2::new(200.0, 20.0));
        assert_eq!(visible_anchor([above, below, touching], clip), None);
        assert_eq!(visible_anchor([], clip), None);
        // Input order does not matter: the topmost visible rectangle wins.
        let lower = Rect::from_min_size(pos2(100.0, clip.top() + 200.0), Vec2::new(200.0, 20.0));
        assert_eq!(visible_anchor([lower, inside], clip), Some(inside));
        // Degenerate input is rejected, not propagated.
        let nan = Rect::from_min_size(pos2(f32::NAN, 0.0), Vec2::new(10.0, 10.0));
        assert_eq!(visible_anchor([nan], clip), None);
    }

    #[test]
    fn bar_for_a_partly_hidden_selection_stays_clear_of_the_top_edge() {
        let clip = vp();
        let partial = Rect::from_min_size(pos2(100.0, clip.top() - 40.0), Vec2::new(200.0, 20.0 + 30.0));
        let a = visible_anchor([partial], clip).unwrap();
        let bar = anchor_rect(a.min.y, a.min.x, clip).unwrap();
        assert!(bar.min.y >= clip.top() - 1e-3 && bar.max.y <= clip.bottom() + 1e-3);
    }

    #[test]
    fn degenerate_inputs_are_rejected_not_crashed() {
        assert!(anchor_rect(f32::NAN, 10.0, vp()).is_none());
        assert!(anchor_rect(10.0, f32::NAN, vp()).is_none());
        assert!(anchor_rect(f32::INFINITY, 10.0, vp()).is_none());
        assert!(anchor_rect(10.0, f32::NEG_INFINITY, vp()).is_none());
        assert!(anchor_rect(10.0, 10.0, Rect::EVERYTHING).is_none());
        assert!(anchor_rect(10.0, 10.0, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(0.0, 100.0))).is_none());
    }

    #[test]
    fn a_viewport_narrower_than_the_bar_still_yields_a_rect() {
        let r = anchor_rect(10.0, 5.0, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(20.0, 20.0)));
        let r = r.unwrap_or(Rect::EVERYTHING);
        assert!(r.width() > 0.0 && r.height() > 0.0, "a tiny viewport must still place a positive-size bar");
        assert!(r.min.x.is_finite() && r.min.y.is_finite());
    }

    /// #535: the bar was one fixed-width row that cut off everything after Align Left. Draw it
    /// over a real selection: every control fits inside the bar, and the bar is no wider than its
    /// controls (both rows end together).
    #[test]
    fn every_control_fits_inside_the_bar() {
        use wordcraft_engine::Session;
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "The quick brown fox jumps over the lazy dog"})).unwrap();
        a.run("select.text", json!({"text": "brown fox"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let [bar, used] = h.ctx.data(|d| d.get_temp::<[Rect; 2]>(shown_id())).expect("the bar is shown over a selection");
        let viewport = h.state().canvas.viewport.expect("the canvas exposes its actual scroll viewport");
        assert!(viewport.contains_rect(bar), "toolbar {:?} must stay inside canvas viewport {:?}", bar, viewport);
        let inner = bar.shrink2(Vec2::new(PAD_X, PAD_Y));
        assert!(
            inner.expand(0.1).contains_rect(used),
            "controls {:?} spill out of the bar {:?}",
            [used.min.x, used.min.y, used.max.x, used.max.y],
            [inner.min.x, inner.min.y, inner.max.x, inner.max.y]
        );
        assert!((used.width() - inner.width()).abs() < 0.5, "the bar ({}) should be as wide as its controls ({})", inner.width(), used.width());
        assert!(used.min.x - inner.min.x < 0.5, "the font box starts at the bar's left edge");
    }

    /// The whole window over a document with `pick` selected, run until the bar is up.
    fn bar_over(pick: &str) -> egui_kittest::Harness<'static, WordApp> {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "The quick brown fox jumps over the lazy dog"})).unwrap();
        a.run("select.text", json!({"text": pick})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        steps(&mut h);
        h
    }

    fn steps(h: &mut egui_kittest::Harness<'static, WordApp>) {
        for _ in 0..6 {
            h.step();
        }
    }

    /// The bar's layer, when it was drawn last frame.
    fn bar_layer(h: &egui_kittest::Harness<'static, WordApp>) -> Option<egui::LayerId> {
        h.ctx.memory(|m| m.areas().visible_layer_ids().into_iter().find(|l| l.id == egui::Id::new("wc_mini_toolbar")))
    }

    fn click_at(h: &mut egui_kittest::Harness<'static, WordApp>, at: egui::Pos2) {
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        }
        steps(h);
    }

    fn format_state(h: &mut egui_kittest::Harness<'static, WordApp>) -> Value {
        h.state_mut().session.run("format.state", &json!({})).unwrap()
    }

    /// The bar was drawn on the Tooltip layer, over every drop-down: the ribbon's font size list
    /// opened under it. Popups and menus are on Foreground; the bar must sit below them.
    #[test]
    fn the_bar_draws_below_popups() {
        let h = bar_over("brown fox");
        let layer = bar_layer(&h).expect("the bar is shown over a selection");
        assert!(layer.order < Order::Foreground, "the bar is on {:?}, over the ribbon's drop-downs", layer.order);
    }

    /// Turning to the ribbon puts the bar away (it covered the ribbon's drop-downs); it comes back
    /// for the next selection, even one over the same text.
    #[test]
    fn a_click_on_the_ribbon_puts_the_bar_away_until_the_selection_changes() {
        use egui_kittest::kittest::Queryable;
        let mut h = bar_over("brown fox");
        assert!(visible(h.state()) && bar_layer(&h).is_some(), "the bar is up over the selection");
        // The ribbon's Bold is the topmost one; the bar has its own lower down.
        let ribbon_bold = |h: &egui_kittest::Harness<'static, WordApp>| {
            h.get_all_by_label("Bold").map(|n| n.rect().center()).min_by(|a, b| a.y.total_cmp(&b.y)).expect("the ribbon has Bold")
        };
        let at = ribbon_bold(&h);
        click_at(&mut h, at);
        assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)), "the click ran the ribbon's Bold");
        assert!(!visible(h.state()), "a click on the ribbon puts the bar away");
        assert!(bar_layer(&h).is_none(), "the bar is no longer drawn");

        h.state_mut().run("select.text", json!({"text": "lazy"})).unwrap();
        steps(&mut h);
        assert!(visible(h.state()) && bar_layer(&h).is_some(), "a new selection brings the bar back");

        let at = ribbon_bold(&h);
        click_at(&mut h, at);
        assert!(bar_layer(&h).is_none());
        h.state_mut().run("select.text", json!({"text": "quick"})).unwrap();
        steps(&mut h);
        h.state_mut().run("select.text", json!({"text": "lazy"})).unwrap();
        steps(&mut h);
        assert!(bar_layer(&h).is_some(), "selecting the same text again brings the bar back");
    }

    /// The ribbon's size list hangs over the page: picking from it is no press in the document, so
    /// the bar stays away (it would cover the list again otherwise).
    #[test]
    fn picking_from_a_ribbon_list_over_the_page_keeps_the_bar_away() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut h = bar_over("brown fox");
        let size = format_state(&mut h).get("size").and_then(Value::as_f64).unwrap_or_default();
        let size = if size.fract() == 0.0 { format!("{size:.0}") } else { size.to_string() };
        // The ribbon's size box is the topmost one showing the size; the bar has its own.
        let size_box = h.get_all_by_role(Role::TextInput).filter(|n| n.value().as_deref() == Some(size.as_str())).map(|n| n.rect());
        let size_box = size_box.min_by(|a, b| a.min.y.total_cmp(&b.min.y)).expect("the ribbon has a size box");
        click_at(&mut h, pos2(size_box.max.x + 8.0, size_box.center().y));
        assert!(bar_layer(&h).is_none(), "opening the ribbon's list puts the bar away");
        let page = h.state().canvas.canvas_rect.expect("the document is shown");
        let item = h.get_by_label("28").rect().center();
        assert!(page.contains(item), "the list's 28 ({item:?}) should hang over the page ({page:?}) for this test");
        click_at(&mut h, item);
        assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(28.0), "the ribbon's size list applied 28 pt");
        assert!(!visible(h.state()) && bar_layer(&h).is_none(), "the bar stays away after a pick from the ribbon's list");
    }

    /// Scrolled until the selection went under the ribbon, the bar went with it and drew over the
    /// ribbon and the ruler. It stays in the document area: the part past its edge neither shows
    /// nor takes the pointer.
    #[test]
    // #1069 lets the bar slide under the ribbon with a selection scrolled past the document's
    // top; #1501, landed first, anchors it to the visible part of the selection only and drops it
    // once none shows. Which one Word does is the owner's call; until then this one is off.
    #[ignore = "conflicts with #1501's visible-selection anchor; see the comment above"]
    fn the_bar_scrolls_under_the_ribbon_not_over_it() {
        let mut h = bar_over("The quick");
        let page = h.state().canvas.canvas_rect.expect("the document is shown");
        // Scroll the selection's line to 45 points above the document area: the bar, below the
        // line now, has its first row past the edge, over the ruler, and its second showing.
        let target = page.min.y - 45.0;
        let line = h.state().canvas.mini_anchor.expect("the selection is on screen");
        h.event(egui::Event::PointerMoved(page.center()));
        h.step();
        let delta = egui::vec2(0.0, target - line.min.y);
        h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, phase: egui::TouchPhase::Start, modifiers: Default::default() });
        h.step();
        h.state_mut().canvas.wheel = Default::default();
        steps(&mut h);
        let line = h.state().canvas.mini_anchor.expect("the selection is still laid out");
        assert!((line.min.y - target).abs() < 1.0, "the line scrolled to {}, not {target}", line.min.y);
        let [bar, _] = h.ctx.data(|d| d.get_temp::<[Rect; 2]>(shown_id())).expect("the bar still shows in part");
        assert!(bar.min.y < page.min.y && bar.max.y > page.min.y, "the bar {bar:?} should straddle the document's top {}", page.min.y);

        let drawn: Vec<Rect> =
            h.output().shapes.iter().filter(|s| matches!(&s.shape, egui::epaint::Shape::Rect(r) if r.rect == bar)).map(|s| s.clip_rect).collect();
        assert!(!drawn.is_empty(), "the bar's background is painted");
        for clip in drawn {
            assert!(page.expand(0.5).contains_rect(clip), "the bar paints outside the document area {page:?}, over the ribbon: {clip:?}");
        }

        // The ruler's left-indent marker sits under the hidden part, at the line's start and the
        // ruler's foot (`canvas::rulers`); dragging it still moves the indent.
        let marker = pos2(line.min.x, page.min.y - 6.0);
        assert!(bar.contains(marker), "the bar {bar:?} should cover the left-indent marker {marker:?}");
        h.event(egui::Event::PointerMoved(marker));
        h.event(egui::Event::PointerButton { pos: marker, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        for dx in [6.0, 18.0, 36.0] {
            h.event(egui::Event::PointerMoved(marker + egui::vec2(dx, 0.0)));
            h.step();
        }
        let end = marker + egui::vec2(36.0, 0.0);
        h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        steps(&mut h);
        let s = &h.state().session;
        let indent = s.doc.para_at(&s.sel.focus).map(|p| s.doc.styles.resolve_para(&p.props).indent_left).unwrap_or_default();
        assert!(indent > 0.0, "dragging the ruler's left-indent marker under the bar's hidden part left the indent at {indent}");
    }

    /// Picking from one of the bar's own lists (popups below it) keeps the bar up.
    #[test]
    fn picking_from_the_bars_own_list_keeps_it() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut h = bar_over("brown fox");
        let [bar, _] = h.ctx.data(|d| d.get_temp::<[Rect; 2]>(shown_id())).expect("the bar is shown over a selection");
        // Its two boxes are font and size; the size box's arrow is just right of the rightmost.
        let size_box =
            h.get_all_by_role(Role::TextInput).map(|n| n.rect()).filter(|r| bar.contains_rect(*r)).max_by(|a, b| a.min.x.total_cmp(&b.min.x));
        let size_box = size_box.expect("the bar has a size box");
        click_at(&mut h, pos2(size_box.max.x + 8.0, size_box.center().y));
        let twenty = h.get_by_label("20").rect().center();
        click_at(&mut h, twenty);
        assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(20.0), "the bar's size list applied 20 pt");
        assert!(visible(h.state()) && bar_layer(&h).is_some(), "the bar stays up after its own list is used");
    }
}
