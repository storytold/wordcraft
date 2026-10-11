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
                p.add(egui::Shape::convex_polygon(pts, col(k), Stroke::new(1.0, Color32::WHITE)));
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

/// The picker; returns true to close.
pub fn picker_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SmartArtForm) -> bool {
    let t = Tokens::get(ui.ctx());
    let theme = app.session.doc.settings.theme_colors.clone();
    let mut chosen = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
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
        ui.vertical(|ui| {
            ui.set_width(3.0 * 96.0 + 16.0);
            ui.set_min_height(2.0 * 96.0 + 8.0);
            egui::Grid::new("smart_art_layouts").spacing(vec2(8.0, 8.0)).show(ui, |ui| {
                for (k, layout) in f.shown().into_iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(96.0, 88.0), Sense::click());
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
                    ui.painter().text(
                        pos2(rect.center().x, rect.bottom() - 11.0),
                        egui::Align2::CENTER_CENTER,
                        tl!(layout.label()),
                        regular(10.5),
                        t.text,
                    );
                    let resp = resp.on_hover_text(tl!(layout.label()));
                    if resp.clicked() {
                        f.layout = layout;
                    }
                    if resp.double_clicked() {
                        chosen = Some(layout);
                    }
                    if k % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        });
        ui.separator();
        ui.vertical(|ui| {
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
    let items: Vec<Value> =
        g.items.iter().enumerate().map(|(j, it)| json!({"text": if j == i { text } else { it.text.as_str() }, "level": it.level})).collect();
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
        .map(|(j, it)| json!({"text": it.text, "level": if under.contains(&j) { it.level.saturating_sub(1) } else { it.level }}))
        .collect();
    json!({"items": items})
}

/// The Text Pane: a row per item, indented by level. Typing changes the graphic as you go (one
/// undo step per item typed in); Enter adds an item after the current one.
pub fn text_pane(app: &mut WordApp, ui: &mut Ui) {
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return };
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("One line per item. Enter adds an item.")).color(t.text_dim));
    ui.add_space(4.0);
    let focus_id = egui::Id::new("smart_art_focus");
    let want_focus: Option<usize> = ui.data_mut(|d| d.remove_temp(focus_id));
    let mut typed = false;
    egui::ScrollArea::vertical().id_salt("smart_art_items").max_height((ui.available_height() - 40.0).max(60.0)).show(ui, |ui| {
        for (i, it) in g.items.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(f32::from(it.level) * 16.0);
                ui.label(egui::RichText::new("•").color(t.text_dim));
                let mut text = it.text.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut text).id_salt(("smart_art_item", i)).desired_width(f32::INFINITY));
                if want_focus == Some(i) {
                    r.request_focus();
                }
                if r.has_focus() || r.gained_focus() {
                    app.smart_art_item = Some(i);
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
    fn removing_an_item_lifts_the_items_under_it() {
        let g = SmartArtSpec::sample(SmartArtLayout::Hierarchy);
        let v = items_without(&g, 1);
        let levels: Vec<u64> = v["items"].as_array().unwrap().iter().map(|i| i["level"].as_u64().unwrap()).collect();
        assert_eq!(levels, [0, 1, 1, 2, 2]);
        assert_eq!(items_with(&g, 0, "Boss")["items"][0]["text"], "Boss");
    }
}
