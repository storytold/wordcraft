//! Insert › SmartArt (a picker of layouts by category, with previews drawn in code) and the
//! SmartArt Text Pane (the selected graphic's items as an editable outline). Each change runs one
//! command (`insert.smartArt`, `smartArt.items`, `smartArt.addShape`…), so agents get the same
//! result without the UI. The picker is shown through [`crate::dialogs_insert::InsertDialog`];
//! the pane through [`crate::panes`].

use egui::{Color32, Painter, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::smart_art::{MAX_ITEMS, SmartArtColors, SmartArtLayout, SmartArtSpec};

use crate::WordApp;
use crate::theme::{Tokens, c32, regular, semibold};
use crate::widgets;

/// The picker's categories (`All` first), in the order shown.
pub const CATEGORIES: [&str; 8] = ["All", "List", "Process", "Cycle", "Hierarchy", "Relationship", "Matrix", "Pyramid"];

/// Insert › SmartArt: the category shown and the layout picked.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartArtForm {
    pub category: String,
    pub layout: SmartArtLayout,
}

impl SmartArtForm {
    pub fn new(_app: &WordApp) -> SmartArtForm {
        SmartArtForm { category: "All".into(), layout: SmartArtLayout::default() }
    }

    /// The layouts the current category shows.
    pub fn shown(&self) -> Vec<SmartArtLayout> {
        SmartArtLayout::ALL.into_iter().filter(|l| self.category == "All" || l.category() == self.category).collect()
    }
}

/// Theme accent `n` (1–6) of the document.
pub fn accent(theme: &[Rgb], n: usize) -> Color32 {
    theme.get(3 + n.clamp(1, 6)).map(|c| c32(*c)).unwrap_or(Color32::from_rgb(0x15, 0x60, 0x82))
}

/// A small picture of `layout` in `r`, in the colours `colors` takes from `theme`; connector
/// lines in `line`.
pub fn preview(p: &Painter, r: Rect, layout: SmartArtLayout, colors: SmartArtColors, theme: &[Rgb], line: Color32) {
    let r = r.shrink(r.width().min(r.height()) * 0.08);
    let s = r.width().min(r.height());
    let c = r.center();
    let col = |i: usize| accent(theme, colors.accent(i));
    // Unit-square coordinates (of the square in the middle of `r`) to screen.
    let at = |x: f32, y: f32| pos2(c.x + (x - 0.5) * s, c.y + (y - 0.5) * s);
    let block = |x0: f32, y0: f32, x1: f32, y1: f32, k: usize, round: f32| p.rect_filled(Rect::from_min_max(at(x0, y0), at(x1, y1)), round, col(k));
    let thin = Stroke::new(1.0, line);
    match layout {
        SmartArtLayout::BasicList => {
            for (k, (x, y)) in [(0.0, 0.18), (0.35, 0.18), (0.7, 0.18), (0.175, 0.55), (0.525, 0.55)].into_iter().enumerate() {
                block(x, y, x + 0.3, y + 0.27, k, 0.0);
            }
        }
        SmartArtLayout::Process => {
            for k in 0..3 {
                let x = k as f32 * 0.37;
                block(x, 0.36, x + 0.26, 0.64, k, 3.0);
                if k < 2 {
                    let ax = x + 0.285;
                    p.add(egui::Shape::convex_polygon(
                        vec![at(ax, 0.45), at(ax + 0.06, 0.5), at(ax, 0.55)],
                        col(k).gamma_multiply(0.6),
                        Stroke::NONE,
                    ));
                }
            }
        }
        SmartArtLayout::Cycle => {
            p.circle_stroke(c, s * 0.34, thin);
            for k in 0..5 {
                let a = (-90.0 + 72.0 * k as f32).to_radians();
                p.circle_filled(c + vec2(a.cos(), a.sin()) * s * 0.34, s * 0.12, col(k));
            }
        }
        SmartArtLayout::Hierarchy => {
            p.line_segment([at(0.5, 0.3), at(0.5, 0.45)], thin);
            p.line_segment([at(0.17, 0.45), at(0.83, 0.45)], thin);
            for x in [0.17, 0.5, 0.83] {
                p.line_segment([at(x, 0.45), at(x, 0.6)], thin);
            }
            block(0.33, 0.08, 0.67, 0.3, 0, 3.0);
            for (k, x) in [0.02, 0.35, 0.68].into_iter().enumerate() {
                block(x, 0.6, x + 0.3, 0.82, k + 1, 3.0);
            }
        }
        SmartArtLayout::Pyramid => {
            // Half the pyramid's width at height `y` (apex at 0.05, base at 0.95).
            let hw = |y: f32| 0.45 * (y - 0.05) / 0.9;
            for (k, (y0, y1)) in [(0.05, 0.37), (0.37, 0.66), (0.66, 0.95)].into_iter().enumerate() {
                let pts = if k == 0 {
                    vec![at(0.5, y0), at(0.5 + hw(y1), y1), at(0.5 - hw(y1), y1)]
                } else {
                    vec![at(0.5 - hw(y0), y0), at(0.5 + hw(y0), y0), at(0.5 + hw(y1), y1), at(0.5 - hw(y1), y1)]
                };
                p.add(egui::Shape::convex_polygon(pts, col(k), Stroke::new(1.0, line)));
            }
        }
        SmartArtLayout::Radial => {
            let spokes: Vec<egui::Pos2> = (0..4)
                .map(|k| {
                    let a = (-90.0 + 90.0 * k as f32).to_radians();
                    c + vec2(a.cos(), a.sin()) * s * 0.36
                })
                .collect();
            for q in &spokes {
                p.line_segment([c, *q], thin);
            }
            p.circle_filled(c, s * 0.17, col(0));
            for (k, q) in spokes.into_iter().enumerate() {
                p.circle_filled(q, s * 0.12, col(k + 1));
            }
        }
        SmartArtLayout::Matrix => {
            for k in 0..4 {
                let (x, y) = ((k % 2) as f32 * 0.5 + 0.03, (k / 2) as f32 * 0.5 + 0.03);
                block(x, y, x + 0.44, y + 0.44, k, 3.0);
            }
        }
        SmartArtLayout::Venn => {
            for k in 0..3 {
                let a = (-90.0 + 120.0 * k as f32).to_radians();
                let q = c + vec2(a.cos(), a.sin()) * s * 0.15;
                p.circle_filled(q, s * 0.3, col(k).gamma_multiply(0.5));
                p.circle_stroke(q, s * 0.3, Stroke::new(1.0, col(k)));
            }
        }
    }
}

/// A colour variant's name in the interface language.
pub fn colors_label(c: SmartArtColors) -> String {
    match c {
        SmartArtColors::Colorful => tl!("Colorful").to_string(),
        _ => crate::i18n::fmt(tl!("Accent {n}"), &[("n", &c.accent(0).to_string())]),
    }
}

/// Swatches of a colour variant in `r`: the accents its shapes take.
pub fn swatches(p: &Painter, r: Rect, colors: SmartArtColors, theme: &[Rgb]) {
    let n = if colors == SmartArtColors::Colorful { 5 } else { 3 };
    let w = r.width() / n as f32;
    for k in 0..n {
        let cell = Rect::from_min_size(pos2(r.left() + k as f32 * w, r.top()), vec2(w - 2.0, r.height()));
        p.rect_filled(cell, 2.0, accent(theme, colors.accent(k)));
    }
}

/// Swatches in `r` of the colours the items of `g` have now: their own or the style's.
pub fn custom_swatches(p: &Painter, r: Rect, g: &SmartArtSpec, theme: &[Rgb]) {
    let shown: Vec<Color32> = (0..g.items.len())
        .filter_map(|i| match g.items.get(i)?.color {
            Some(c) => Some(c32(c)),
            None => Some(accent(theme, g.style_accent(i)?)),
        })
        .take(5)
        .collect();
    let w = r.width() / shown.len().max(1) as f32;
    for (k, c) in shown.into_iter().enumerate() {
        p.rect_filled(Rect::from_min_size(pos2(r.left() + k as f32 * w, r.top()), vec2(w - 2.0, r.height())), 2.0, c);
    }
}

/// Height of the picker's three columns, points: three rows of 96 pt tiles and the gaps between them.
const PICKER_HEIGHT: f32 = 3.0 * 96.0 + 2.0 * 8.0;

/// The picker's columns side by side. A separator in a row takes all the height it is offered,
/// which for a dialog is the whole window, so the row is offered only [`PICKER_HEIGHT`].
fn picker_row(ui: &mut Ui, columns: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        ui.set_max_height(PICKER_HEIGHT);
        crate::rtl::horizontal_top(ui, columns);
    });
}

/// The picker; returns true to close.
pub fn picker_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SmartArtForm) -> bool {
    let t = Tokens::get(ui.ctx());
    let theme = app.session.doc.settings.theme_colors.clone();
    let mut chosen = None;
    picker_row(ui, |ui| {
        crate::rtl::vertical(ui, |ui| {
            ui.set_width(120.0);
            for cat in CATEGORIES {
                if ui.selectable_label(f.category == cat, tl!(cat)).clicked() {
                    f.category = cat.to_string();
                    if !f.shown().contains(&f.layout)
                        && let Some(first) = f.shown().first()
                    {
                        f.layout = *first;
                    }
                }
            }
        });
        ui.separator();
        crate::rtl::vertical(ui, |ui| {
            ui.set_width(3.0 * 96.0 + 16.0);
            ui.set_min_height(2.0 * 96.0 + 8.0);
            let shown = f.shown();
            crate::rtl::Table::new("smart_art_layouts").columns(0).spacing(vec2(8.0, 8.0)).show(ui, |table| {
                for layouts in shown.chunks(3) {
                    table.row(|r| {
                        for layout in layouts.iter().copied() {
                            r.cell(|ui| {
                                let (rect, resp) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::click());
                                let on = f.layout == layout;
                                let fill = if on {
                                    t.checked
                                } else if resp.hovered() {
                                    t.hover
                                } else {
                                    t.input
                                };
                                ui.painter().rect(rect, 4.0, fill, Stroke::new(1.0, if on { t.accent } else { t.border }), egui::StrokeKind::Inside);
                                preview(
                                    ui.painter(),
                                    Rect::from_min_size(rect.min + vec2(14.0, 4.0), vec2(68.0, 62.0)),
                                    layout,
                                    SmartArtColors::default(),
                                    &theme,
                                    t.text_dim,
                                );
                                // A label wider than the tile (a long translation) breaks at its middle space.
                                let label = tl!(layout.label());
                                let width = |text: &str| ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), regular(10.5), t.text).size().x);
                                let lines = if width(label) <= rect.width() - 6.0 {
                                    vec![label.to_string()]
                                } else {
                                    let split = label
                                        .match_indices(' ')
                                        .map(|(i, _)| (label[..i].to_string(), label[i + 1..].to_string()))
                                        .min_by(|a, b| width(&a.0).max(width(&a.1)).total_cmp(&width(&b.0).max(width(&b.1))));
                                    split.map_or_else(|| vec![label.to_string()], |(a, b)| vec![a, b])
                                };
                                for (n, line) in lines.iter().rev().enumerate() {
                                    ui.painter().text(
                                        pos2(rect.center().x, rect.bottom() - 11.0 - 12.5 * n as f32),
                                        egui::Align2::CENTER_CENTER,
                                        line,
                                        regular(10.5),
                                        t.text,
                                    );
                                }
                                let resp = resp.on_hover_text(label);
                                if resp.clicked() {
                                    f.layout = layout;
                                }
                                if resp.double_clicked() {
                                    chosen = Some(layout);
                                }
                            });
                        }
                    });
                }
            });
        });
        ui.separator();
        crate::rtl::vertical(ui, |ui| {
            ui.set_width(200.0);
            let (rect, _) = ui.allocate_exact_size(vec2(200.0, 150.0), Sense::hover());
            ui.painter().rect(rect, 4.0, t.input, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            preview(ui.painter(), rect.shrink(10.0), f.layout, SmartArtColors::default(), &theme, t.text_dim);
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl!(f.layout.label())).font(semibold(13.0)));
            ui.add(egui::Label::new(tl!(f.layout.description())).wrap());
        });
    });
    let (ok, cancel) = crate::dialogs_insert::buttons(ui, "OK", "Cancel", true);
    let Some(layout) = chosen.or(ok.then_some(f.layout)) else { return cancel };
    app.run("insert.smartArt", json!({"layout": layout.id()})).is_ok() || cancel
}

/// The `smartArt.items` params for `g` with item `i`'s text replaced by `text`.
pub fn items_with(g: &SmartArtSpec, i: usize, text: &str) -> Value {
    let items: Vec<Value> = g
        .items
        .iter()
        .enumerate()
        .map(|(j, it)| json!({"text": if j == i { text } else { it.text.as_str() }, "level": it.level, "color": it.color.map(|c| c.hex())}))
        .collect();
    json!({"items": items})
}

/// The `smartArt.items` params for `g` without item `i` (the items under it move up a level).
pub fn items_without(g: &SmartArtSpec, i: usize) -> Value {
    let under = g.descendants(i);
    let items: Vec<Value> = g
        .items
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(j, it)| {
            json!({"text": it.text, "level": if under.contains(&j) { it.level.saturating_sub(1) } else { it.level }, "color": it.color.map(|c| c.hex())})
        })
        .collect();
    json!({"items": items})
}

/// The Text Pane button on the left edge of the selected graphic's frame, as in Word, while
/// the pane is closed.
pub fn frame_button(app: &mut WordApp, ctx: &egui::Context, layout: &wordcraft_layout::DocLayout, pages: &[Rect], scale: f32, clip: Rect) {
    // The pane and the picked item go with the graphic: the pane opens again only on request.
    if wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_none() {
        app.session.view.smart_art_pane = false;
        app.smart_art_item = None;
        app.smart_art_typing = None;
        return;
    }
    if app.session.view.smart_art_pane {
        return;
    }
    let Some((hit, _)) = crate::objects::active(app, layout) else { return };
    let Some(page) = pages.get(hit.page) else { return };
    let frame = crate::frame::to_screen(*page, scale, hit.rect);
    let at = pos2(frame.min.x - 28.0, frame.center().y - 14.0);
    if !clip.contains(at) {
        return;
    }
    let t = Tokens::get(ctx);
    let area =
        egui::Area::new(egui::Id::unique("smart_art_frame_button")).order(egui::Order::Foreground).interactable(!app.modal_open()).fixed_pos(at);
    area.show(ctx, |ui| {
        egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(4).inner_margin(2).show(ui, |ui| {
            if crate::widgets::icon_button(ui, "textPane", tl!("Text Pane")).clicked() {
                let _ = app.run("smartArt.textPane", json!({"value": true}));
            }
        });
    });
}

/// The item of the selected SmartArt graphic whose shape is under screen point `at`.
fn item_under(app: &WordApp, layout: &wordcraft_layout::DocLayout, pages: &[Rect], scale: f32, at: egui::Pos2) -> Option<usize> {
    let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session)?;
    let (hit, _) = crate::objects::active(app, layout)?;
    let frame = crate::frame::to_screen(*pages.get(hit.page)?, scale, hit.rect);
    if !(frame.width() > 0.0 && frame.height() > 0.0) {
        return None;
    }
    // Into the graphic's own points: turned back and unflipped.
    let p = crate::frame::turn(at, frame.center(), -hit.spin.deg);
    let (mut u, mut v) = ((p.x - frame.min.x) / frame.width(), (p.y - frame.min.y) / frame.height());
    if hit.spin.flip_h {
        u = 1.0 - u;
    }
    if hit.spin.flip_v {
        v = 1.0 - v;
    }
    wordcraft_engine::cmd::smart_art::item_at_point(&g, hit.rect.w, hit.rect.h, u * hit.rect.w, v * hit.rect.h)
}

/// A click on the selected SmartArt graphic picks the item whose shape is under the pointer (none
/// on its background), as clicking a shape in Word does: typing then goes into that item.
pub fn pick_item(app: &mut WordApp, layout: &wordcraft_layout::DocLayout, pages: &[Rect], scale: f32, at: egui::Pos2) {
    if wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_none() {
        return;
    }
    let item = item_under(app, layout, pages, scale, at);
    if item != app.smart_art_item {
        app.smart_art_typing = None;
    }
    app.smart_art_item = item;
}

/// The picked item's shape, outlined inside the selected graphic's frame.
pub fn paint_item(app: &WordApp, painter: &Painter, t: &Tokens, layout: &wordcraft_layout::DocLayout, pages: &[Rect], scale: f32) {
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return };
    let Some(i) = app.smart_art_item else { return };
    let Some((hit, _)) = crate::objects::active(app, layout) else { return };
    let Some(page) = pages.get(hit.page) else { return };
    let Some([x, y, w, h]) = wordcraft_engine::cmd::smart_art::item_box(&g, hit.rect.w, hit.rect.h, i) else { return };
    let frame = crate::frame::to_screen(*page, scale, hit.rect);
    let corner = |u: f32, v: f32| {
        let u = if hit.spin.flip_h { hit.rect.w - u } else { u };
        let v = if hit.spin.flip_v { hit.rect.h - v } else { v };
        crate::frame::turn(pos2(frame.min.x + u * scale, frame.min.y + v * scale), frame.center(), hit.spin.deg)
    };
    let pts = vec![corner(x, y), corner(x + w, y), corner(x + w, y + h), corner(x, y + h)];
    painter.add(egui::Shape::closed_line(pts, Stroke::new(1.25, t.accent)));
}

/// With an item of the selected graphic picked (clicked, or typed in): Backspace erases its last
/// character, since typing goes at its end, and Escape goes back to the whole graphic (Delete
/// still removes the graphic). False for other keys, or when no item is picked.
pub fn key(app: &mut WordApp, key: egui::Key, m: egui::Modifiers) -> bool {
    if !m.is_none() || !matches!(key, egui::Key::Backspace | egui::Key::Escape) {
        return false;
    }
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return false };
    let Some(i) = app.smart_art_item.filter(|i| *i < g.items.len()) else { return false };
    if key == egui::Key::Escape {
        app.smart_art_item = None;
        app.smart_art_typing = None;
        return true;
    }
    let mut text = g.items.get(i).map_or(String::new(), |it| it.text.clone());
    if text.is_empty() {
        return true;
    }
    // As in body text: a whole grapheme (an emoji sequence, a letter with its combining marks),
    // or one character in the scripts whose Backspace removes a sign at a time.
    let from = match text.chars().next_back() {
        Some(c) if wordcraft_doc::cluster::backspaces_by_char(c) => text.len() - c.len_utf8(),
        _ => wordcraft_doc::cluster::prev_cluster(&text, text.len()),
    };
    text.truncate(from);
    if app.smart_art_typing == Some(i) {
        app.session.join_next_undo();
    }
    if app.run("smartArt.items", items_with(&g, i, &text)).is_ok() {
        app.smart_art_typing = Some(i);
    }
    true
}

/// Text typed with a SmartArt graphic selected goes at the end of its current item (the first,
/// before any is picked), and the Text Pane opens, as Word does; it never replaces the graphic.
///
/// What typing into a selected SmartArt graphic did.
///
/// A selected graphic always consumes the input, even when it has no item to edit: the typing
/// must not fall through into the body. Only the absence of a SmartArt target lets the body type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SmartArtTypeResult {
    /// No SmartArt graphic is selected: the caller may type into the body.
    NotHandled,
    /// A selected graphic consumed the input without editing the document.
    ConsumedNoChange,
    /// A SmartArt item really changed.
    Changed,
}

/// Appends typing to the selected SmartArt item, joining an active typing undo, and returns
/// whether the input was unhandled, consumed without a change, or changed an item.
pub(crate) fn type_into(app: &mut WordApp, typed: &str) -> SmartArtTypeResult {
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else {
        return SmartArtTypeResult::NotHandled;
    };
    let Some(i) = app.smart_art_item.filter(|i| *i < g.items.len()).or((!g.items.is_empty()).then_some(0)) else {
        // Selected but empty: consume, never insert into the body, never claim an edit.
        return SmartArtTypeResult::ConsumedNoChange;
    };
    let text = g.items.get(i).map_or(String::new(), |it| it.text.clone()) + typed;
    if app.smart_art_typing == Some(i) {
        app.session.join_next_undo();
    }
    if app.run("smartArt.items", items_with(&g, i, &text)).is_err() {
        // A selected graphic that refuses the edit still consumes the event; an IME composition
        // must start afresh rather than be applied to the body.
        app.canvas.ime_interrupt_now = true;
        return SmartArtTypeResult::ConsumedNoChange;
    }
    app.smart_art_item = Some(i);
    app.smart_art_typing = Some(i);
    let _ = app.run("smartArt.textPane", json!({"value": true}));
    SmartArtTypeResult::Changed
}

/// Side of an item's colour button, points.
const SWATCH: f32 = 18.0;

/// Id of item `i`'s colour button.
fn color_button_id(i: usize) -> egui::Id {
    egui::Id::unique(("smart_art_color_button", i))
}

/// The button at the right of item `i`'s row: its shape's colour (the style's until changed).
/// A click offers the colour grid; the style's colour is offered back.
fn color_button(app: &mut WordApp, ui: &mut Ui, g: &SmartArtSpec, i: usize) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::hover());
    let resp = ui.interact(rect, color_button_id(i), Sense::click());
    let label = g
        .items
        .get(i)
        .filter(|it| !it.text.is_empty())
        .map_or_else(|| format!("{} ({})", tl!("Shape color"), i + 1), |it| format!("{}: {}", tl!("Shape color"), it.text));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label.as_str()));
    let Some(accent_n) = g.style_accent(i) else { return };
    let theme = app.session.doc.settings.theme_colors.clone();
    let own = g.items.get(i).and_then(|it| it.color);
    let fill = own.map_or_else(|| accent(&theme, accent_n), c32);
    ui.painter().rect_filled(rect, 3.0, fill);
    let line = if resp.hovered() { t.accent } else { t.border_strong };
    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, line), egui::StrokeKind::Inside);
    let resp = resp.on_hover_text(tl!("Shape color"));
    egui::Popup::menu(&resp).show(|ui| {
        let mut pick = widgets::color_grid(ui, &theme).map(|hex| json!({"index": i, "color": hex}));
        ui.add_space(2.0);
        ui.separator();
        if ui.add_enabled(own.is_some(), egui::Button::new(tl!("Style Color")).frame(false)).clicked() {
            pick = Some(json!({"index": i}));
        }
        if let Some(params) = pick {
            app.smart_art_typing = None;
            let _ = app.run("smartArt.itemColor", params);
            ui.close();
        }
    });
}

/// The Text Pane: a row per item, indented by level. Typing changes the graphic as you go (one
/// undo step per item typed in); Enter adds an item after the current one; Tab demotes the
/// focused item and Shift+Tab promotes it, as Word's Text Pane does.
pub fn text_pane(app: &mut WordApp, ui: &mut Ui) {
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return };
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("One line per item. Enter adds an item.")).color(t.text_dim));
    ui.add_space(4.0);
    let focus_id = egui::Id::unique("smart_art_focus");
    let want_focus: Option<usize> = ui.data_mut(|d| d.remove_temp(focus_id));
    let mut typed = false;
    app.smart_art_row_focused = false;
    egui::ScrollArea::vertical().id_salt("smart_art_items").max_height((ui.available_height() - 40.0).max(60.0)).show(ui, |ui| {
        for (i, it) in g.items.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(f32::from(it.level) * 16.0);
                ui.label(egui::RichText::new("•").color(t.text_dim));
                let mut text = it.text.clone();
                let item_name = format!("{} {}", tl!("Text Pane"), i + 1);
                let width = (ui.available_width() - SWATCH - ui.spacing().item_spacing.x).max(40.0);
                let r = ui.add(egui::TextEdit::singleline(&mut text).id_salt(("smart_art_item", i)).desired_width(width)).accessible_name(item_name);
                color_button(app, ui, &g, i);
                if want_focus == Some(i) {
                    r.request_focus();
                }
                if r.has_focus() || r.gained_focus() {
                    app.smart_art_item = Some(i);
                    app.smart_art_row_focused = true;
                }
                if r.changed() {
                    typed = true;
                    if app.smart_art_typing == Some(i) {
                        app.session.join_next_undo();
                    }
                    if app.run("smartArt.items", items_with(&g, i, &text)).is_ok() {
                        app.smart_art_typing = Some(i);
                    }
                }
                if r.lost_focus() && ui.input(|inp| inp.key_pressed(egui::Key::Enter)) && g.items.len() < MAX_ITEMS {
                    app.smart_art_typing = None;
                    if app.run("smartArt.addShape", json!({"after": i})).is_ok() {
                        // The new item comes after this one and the items under it.
                        let next = g.descendants(i).end;
                        app.smart_art_item = Some(next);
                        ui.data_mut(|d| d.insert_temp(focus_id, next));
                    }
                }
            });
        }
    });
    if !typed && !ui.ctx().memory(|m| m.focused().is_some()) {
        app.smart_art_typing = None;
    }
    ui.separator();
    let current = app.smart_art_item.filter(|i| *i < g.items.len());
    ui.horizontal_wrapped(|ui| {
        let at = current.map_or(json!({}), |i| json!({"after": i}));
        if ui.add_enabled(g.items.len() < MAX_ITEMS, egui::Button::new(tl!("Add"))).clicked() {
            app.smart_art_typing = None;
            let _ = app.run("smartArt.addShape", at);
        }
        let idx = current.map_or(json!({}), |i| json!({"index": i}));
        if ui.button(tl!("Promote")).clicked() {
            app.smart_art_typing = None;
            let _ = app.run("smartArt.promote", idx.clone());
        }
        if ui.button(tl!("Demote")).clicked() {
            app.smart_art_typing = None;
            let _ = app.run("smartArt.demote", idx);
        }
        if ui.add_enabled(current.is_some() && g.items.len() > 1, egui::Button::new(tl!("Remove"))).clicked()
            && let Some(i) = current
        {
            app.smart_art_typing = None;
            if app.run("smartArt.items", items_without(&g, i)).is_ok() {
                app.smart_art_item = i.checked_sub(1);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_buttons_expose_item_names_and_empty_row_indices() {
        use egui_kittest::kittest::Queryable;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let mut spec = SmartArtSpec::sample(SmartArtLayout::Process);
        spec.items[0].text = "Named".into();
        spec.items[1].text.clear();
        let mut h = egui_kittest::Harness::builder().build_ui(move |ui| {
            color_button(&mut app, ui, &spec, 0);
            color_button(&mut app, ui, &spec, 1);
        });
        h.run_steps(2);
        h.get_by_label("Shape color: Named");
        h.get_by_label("Shape color (2)");
    }

    #[test]
    fn removing_an_item_lifts_the_items_under_it() {
        let g = SmartArtSpec::sample(SmartArtLayout::Hierarchy);
        let v = items_without(&g, 1);
        let levels: Vec<u64> = v["items"].as_array().unwrap().iter().map(|i| i["level"].as_u64().unwrap()).collect();
        assert_eq!(levels, [0, 1, 1, 2, 2]);
        assert_eq!(items_with(&g, 0, "Boss")["items"][0]["text"], "Boss");
    }

    /// The colour button opens a grid: a swatch sets the item's own colour and Style Color clears it.
    #[test]
    fn the_color_button_sets_and_clears_an_items_colour() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.smartArt", json!({"layout": "process"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        let mut n = 0u32;
        let mut frame = |ctx: &egui::Context, events: Vec<egui::Event>, a: &mut WordApp| {
            n += 1;
            let input = egui::RawInput { time: Some(f64::from(n) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
            ctx.run_ui(input, |ui| text_pane(a, ui)).drop_without_applying_deltas();
        };
        let click = |ctx: &egui::Context, pos: egui::Pos2, a: &mut WordApp, frame: &mut dyn FnMut(&egui::Context, Vec<egui::Event>, &mut WordApp)| {
            let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(ctx, vec![egui::Event::PointerMoved(pos), button(true)], a);
            frame(ctx, vec![button(false)], a);
            frame(ctx, vec![], a);
        };
        let colour =
            |a: &WordApp| wordcraft_engine::cmd::smart_art::selected_smart_art(&a.session).and_then(|g| g.items.get(1).and_then(|i| i.color));
        frame(&ctx, vec![], &mut a);
        let button = ctx.read_response(color_button_id(1)).expect("item 1 has a colour button");
        let popup = egui::Popup::default_response_id(&button);
        click(&ctx, button.rect.center(), &mut a, &mut frame);
        assert!(egui::Popup::is_id_open(&ctx, popup), "the button opens the colour grid");
        let area = ctx.memory(|m| m.area_rect(popup)).expect("the grid is drawn");
        // The first theme swatch: under the "Theme Colors" label at the popup's top left.
        click(&ctx, area.min + vec2(14.0, 38.0), &mut a, &mut frame);
        assert!(colour(&a).is_some(), "a swatch gives the item its own colour");
        let button = ctx.read_response(color_button_id(1)).unwrap();
        click(&ctx, button.rect.center(), &mut a, &mut frame);
        let area = ctx.memory(|m| m.area_rect(popup)).expect("the grid is drawn again");
        click(&ctx, pos2(area.min.x + 30.0, area.max.y - 14.0), &mut a, &mut frame);
        assert_eq!(colour(&a), None, "Style Color clears it");
    }

    /// #2467 review: a graphic turned or mirrored (from a file: SmartArt can't be turned here)
    /// still picks the item whose shape is under the pointer. The point is worked out on its own:
    /// the item's centre in the graphic, mirrored, then turned about the frame's centre.
    #[test]
    fn a_turned_or_flipped_graphic_picks_the_item_under_the_pointer() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.smartArt", json!({"layout": "hierarchy"})).unwrap();
        for (deg, flip_h, flip_v) in [(90.0, false, false), (0.0, true, false), (0.0, false, true), (45.0, true, false), (200.0, false, true)] {
            let (pos, _) = wordcraft_engine::cmd::objects::selected(&app.session).expect("the graphic is selected");
            let para = app.session.doc.para_mut(pos.story, &pos.path).unwrap();
            let float = para.objects.iter_mut().find_map(|o| o.float_mut()).expect("the graphic's frame");
            float.set_spin(wordcraft_geom::Spin::new(deg, flip_h, flip_v));
            para.touch();
            app.session.relayout();
            let layout = app.session.layout();
            let (hit, _) = crate::objects::active(&app, &layout).expect("the graphic is on the page");
            assert_eq!((hit.spin.flip_h, hit.spin.flip_v), (flip_h, flip_v), "the layout carries the spin");
            let (scale, page) = (1.5, Rect::from_min_size(pos2(40.0, 30.0), vec2(918.0, 1188.0)));
            let pages = vec![page; hit.page + 1];
            let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).unwrap();
            for item in [0, 3] {
                let [x, y, w, h] = wordcraft_engine::cmd::smart_art::item_box(&g, hit.rect.w, hit.rect.h, item).unwrap();
                let (mut u, mut v) = (x + w / 2.0, y + h / 2.0);
                if flip_h {
                    u = hit.rect.w - u;
                }
                if flip_v {
                    v = hit.rect.h - v;
                }
                let frame = crate::frame::to_screen(page, scale, hit.rect);
                let (sin, cos) = deg.to_radians().sin_cos();
                let (dx, dy) = (frame.min.x + u * scale - frame.center().x, frame.min.y + v * scale - frame.center().y);
                let at = pos2(frame.center().x + dx * cos - dy * sin, frame.center().y + dx * sin + dy * cos);
                assert_eq!(item_under(&app, &layout, &pages, scale, at), Some(item), "item {item} at {deg}° flip {flip_h}/{flip_v}");
            }
        }
    }

    /// #2467 review: Backspace in a picked item erases what it does in body text: a whole letter
    /// with its combining marks, a whole emoji sequence; one sign at a time in Thai.
    #[test]
    fn backspace_in_a_picked_item_erases_a_whole_grapheme() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.smartArt", json!({"layout": "hierarchy"})).unwrap();
        let text = |app: &WordApp| wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).unwrap().items[3].text.clone();
        let set = |app: &mut WordApp, t: &str| {
            let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).unwrap();
            app.run("smartArt.items", items_with(&g, 3, t)).unwrap();
        };
        app.smart_art_item = Some(3);
        // "Việt" with the ệ's two marks apart, then a family emoji (a ZWJ sequence).
        set(&mut app, "Vie\u{302}\u{323}t\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
        assert!(key(&mut app, egui::Key::Backspace, egui::Modifiers::NONE));
        assert_eq!(text(&app), "Vie\u{302}\u{323}t", "the whole emoji sequence goes");
        for _ in 0..2 {
            key(&mut app, egui::Key::Backspace, egui::Modifiers::NONE);
        }
        assert_eq!(text(&app), "Vi", "the letter goes with both its marks");
        set(&mut app, "\u{0E01}\u{0E35}");
        key(&mut app, egui::Key::Backspace, egui::Modifiers::NONE);
        assert_eq!(text(&app), "\u{0E01}", "Thai: one sign at a time, as in body text");
    }

    /// #2832: the separators between the picker's columns made the dialog as tall as the window.
    #[test]
    fn the_picker_row_stays_within_its_height() {
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))), ..Default::default() };
        let mut height = f32::MAX;
        ctx.run_ui(input, |ui| {
            let top = ui.cursor().top();
            picker_row(ui, |ui| {
                ui.label("a");
                ui.separator();
                ui.label("b");
            });
            height = ui.min_rect().bottom() - top;
        })
        .drop_without_applying_deltas();
        assert!(height <= PICKER_HEIGHT + 1.0, "the row is {height} pt tall");
    }
}
