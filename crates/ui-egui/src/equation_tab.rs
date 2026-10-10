//! The Equation tab (shown while editing an equation) and the Insert › Equation gallery:
//! ready-made equations, input format and conversion, symbol sets, and the structure galleries
//! (fractions, scripts, radicals, integrals, large operators, brackets, functions, accents,
//! limits and logs, operators, matrices). Tiles and button pictures are rendered by the math
//! engine itself.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::math::parse_linear;
use wordcraft_engine::math_gallery::{BUILT_INS, STRUCTURES, SYMBOL_SETS, Template};

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{CONTENT_H, group, small};

/// What each structure button shows.
fn button_picture(id: &str) -> &'static str {
    match id {
        "fraction" => "x/y",
        "script" => "e^x",
        "radical" => "√(n&x)",
        "integral" => "∫_(−x)^x",
        "largeOperator" => "∑_(i=0)^n",
        "bracket" => "{()}",
        "function" => "sin⁡θ",
        "accent" => "ä",
        "limitLog" => "lim┬(n→∞)",
        "operator" => "≜",
        "matrix" => "[■(1&0@0&1)]",
        _ => "x",
    }
}

/// A rendered equation drawn into `r` (centred), or its linear text while it renders.
fn paint_math(app: &mut WordApp, ui: &Ui, r: Rect, key: &str, linear: &str, nodes: Option<wordcraft_doc::math::Arg>, display: bool, pt: f32) {
    let nodes = nodes.unwrap_or_else(|| parse_linear(linear));
    match crate::previews::math_texture(app, ui.ctx(), key, &nodes, display, pt, r.size()) {
        Some(tex) => {
            ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        None => {
            let t = Tokens::get(ui.ctx());
            ui.painter().text(r.center(), Align2::CENTER_CENTER, linear, regular(11.0), t.text_dim);
        }
    }
}

/// A big ribbon button whose picture is an equation; opens `menu`.
fn math_menu_button(ui: &mut Ui, app: &mut WordApp, picture: &str, label: &str, menu: impl FnOnce(&mut Ui, &mut WordApp)) {
    let label = tl!(label);
    let t = Tokens::get(ui.ctx());
    let galley_w =
        label.split('\n').map(|l| ui.ctx().fonts_mut(|f| f.layout_no_wrap(l.to_string(), regular(11.5), t.text).size().x)).fold(0.0, f32::max);
    let w = (galley_w + 22.0).max(46.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, CONTENT_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let pic = Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(40.0, 34.0));
    ui.painter().rect_filled(pic, 3.0, egui::Color32::WHITE);
    paint_math(app, ui, pic, &format!("btn:{picture}"), picture, None, false, 12.0);
    let lines: Vec<&str> = label.split('\n').collect();
    let mut y = r.min.y + 43.0;
    for (i, l) in lines.iter().enumerate() {
        let txt = if i + 1 == lines.len() { format!("{l} ▾") } else { (*l).to_string() };
        ui.painter().text(pos2(r.center().x, y), Align2::CENTER_TOP, txt, regular(11.5), t.text);
        y += 13.0;
    }
    let resp = resp.on_hover_text(label.replace('\n', " "));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(260.0);
        menu(ui, app);
    });
}

/// One gallery tile: the template rendered, its name as a tooltip.
fn tile(ui: &mut Ui, app: &mut WordApp, tpl: &Template, size: egui::Vec2) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(r, 2.0, egui::Color32::WHITE);
    if resp.hovered() {
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    let pt = if size.x > 120.0 { 13.0 } else { 14.0 };
    paint_math(app, ui, r.shrink(2.0), &format!("tpl:{}", tpl.id), tpl.linear, Some(tpl.nodes()), true, pt);
    resp.on_hover_text(tl!(tpl.label)).clicked()
}

fn section_heading(ui: &mut Ui, name: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 20.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.hover);
    ui.painter().text(pos2(r.min.x + 6.0, r.center().y), Align2::LEFT_CENTER, tl!(name), semibold(11.5), t.text);
}

/// A structure gallery's menu.
fn structure_menu(ui: &mut Ui, app: &mut WordApp, gallery_id: &str) {
    let Some(g) = STRUCTURES.iter().find(|g| g.id == gallery_id) else { return };
    egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
        ui.set_width(330.0);
        for (name, templates) in g.sections {
            section_heading(ui, name);
            let wide = name.starts_with("Common") || templates.iter().all(|t| !t.linear.contains('⬚')) && templates.len() <= 3;
            let size = if wide { vec2(158.0, 60.0) } else { vec2(60.0, 56.0) };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for tpl in templates.iter() {
                    if tile(ui, app, tpl, size) {
                        let _ = app.run("equation.insertStructure", json!({"id": tpl.id}));
                        app.canvas.want_focus = true;
                        ui.close();
                    }
                }
            });
            ui.add_space(4.0);
        }
    });
}

/// The ready-made equations (Insert › Equation and Equation › Equation).
pub fn builtin_menu(ui: &mut Ui, app: &mut WordApp) {
    egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
        ui.set_width(360.0);
        section_heading(ui, "Built-In");
        for (id, label, lin) in BUILT_INS {
            let t = Tokens::get(ui.ctx());
            ui.label(egui::RichText::new(tl!(label)).font(semibold(11.5)).color(t.text));
            let (r, resp) = ui.allocate_exact_size(vec2(350.0, 64.0), Sense::click());
            ui.painter().rect_filled(r, 2.0, egui::Color32::WHITE);
            if resp.hovered() {
                ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            }
            paint_math(app, ui, r.shrink(2.0), &format!("builtin:{id}"), lin, None, true, 11.0);
            if resp.clicked() {
                let _ = app.run("insert.equation", json!({"builtin": id}));
                app.canvas.want_focus = true;
                ui.close();
            }
            ui.add_space(4.0);
        }
    });
    ui.separator();
    if ui.button(tl!("Insert New Equation")).clicked() {
        let _ = app.run("insert.equation", json!({}));
        app.canvas.want_focus = true;
        ui.close();
    }
}

/// Insert › Symbols › Equation: a split button (insert a new equation, or pick a ready-made one).
pub fn insert_button(ui: &mut Ui, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let label = tl!("Equation");
    let w = (ui.ctx().fonts_mut(|f| f.layout_no_wrap(label.to_string(), regular(11.5), t.text).size().x) + 22.0).max(48.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, CONTENT_H), Sense::hover());
    let top = Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 40.0));
    let bottom = Rect::from_min_max(pos2(r.min.x, r.min.y + 40.0), r.max);
    let rt = ui.interact(top, ui.id().with("eq_insert_top"), Sense::click());
    let rb = ui.interact(bottom, ui.id().with("eq_insert_bottom"), Sense::click());
    for (rr, resp) in [(top, &rt), (bottom, &rb)] {
        if resp.hovered() {
            ui.painter().rect_filled(rr, 4.0, t.hover);
        }
    }
    crate::icons::paint(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(32.0, 32.0)), "equation", t.icon, t.accent);
    ui.painter().text(pos2(r.center().x, r.min.y + 43.0), Align2::CENTER_TOP, format!("{label} ▾"), regular(11.5), t.text);
    if rt.on_hover_text(tl!("Insert a new equation (Alt+=)")).clicked() {
        let _ = app.run("insert.equation", json!({}));
        app.canvas.want_focus = true;
    }
    egui::Popup::menu(&rb).show(|ui| builtin_menu(ui, app));
}

/// The Symbols group: a strip of the current set's symbols, and a menu with every set.
fn symbols(ui: &mut Ui, app: &mut WordApp) {
    let id = egui::Id::new("eq_symbol_set");
    let set = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0).min(SYMBOL_SETS.len().saturating_sub(1));
    let Some((_, chars)) = SYMBOL_SETS.get(set) else { return };
    let start_id = egui::Id::new("eq_symbol_start");
    let per_row = 10usize;
    let rows = 3usize;
    let all: Vec<char> = chars.chars().collect();
    let start = ui.data(|d| d.get_temp::<usize>(start_id)).unwrap_or(0).min(all.len().saturating_sub(1));
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
        for row in 0..rows {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                for col in 0..per_row {
                    let k = start + row * per_row + col;
                    let Some(c) = all.get(k) else {
                        ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        continue;
                    };
                    if symbol_button(ui, app, *c, &t) {
                        let _ = app.run("equation.insertSymbol", json!({"char": c.to_string()}));
                        app.canvas.want_focus = true;
                    }
                }
            });
        }
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
        let page = per_row * rows;
        if ui.add_enabled(start > 0, egui::Button::new("▲").min_size(vec2(16.0, 22.0))).clicked() {
            ui.data_mut(|d| d.insert_temp(start_id, start.saturating_sub(per_row)));
        }
        if ui.add_enabled(start + page < all.len(), egui::Button::new("▼").min_size(vec2(16.0, 22.0))).clicked() {
            ui.data_mut(|d| d.insert_temp(start_id, start + per_row));
        }
        let resp = ui.add(egui::Button::new("…").min_size(vec2(16.0, 22.0))).on_hover_text(tl!("All symbol sets"));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_width(420.0);
            ui.horizontal_wrapped(|ui| {
                for (i, (name, _)) in SYMBOL_SETS.iter().enumerate() {
                    if ui.selectable_label(i == set, tl!(name)).clicked() {
                        ui.data_mut(|d| {
                            d.insert_temp(id, i);
                            d.insert_temp(start_id, 0usize);
                        });
                    }
                }
            });
            ui.separator();
            let set = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
            if let Some((_, chars)) = SYMBOL_SETS.get(set) {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                        for c in chars.chars() {
                            if symbol_button(ui, app, c, &t) {
                                let _ = app.run("equation.insertSymbol", json!({"char": c.to_string()}));
                                app.canvas.want_focus = true;
                            }
                        }
                    });
                });
            }
        });
    });
}

fn symbol_button(ui: &mut Ui, app: &mut WordApp, c: char, t: &Tokens) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    ui.painter().rect_filled(r, 2.0, egui::Color32::WHITE);
    if resp.hovered() {
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    let s = c.to_string();
    let nodes = vec![wordcraft_doc::math::MNode::Run(wordcraft_doc::math::MRun { text: s.clone(), lit: true, ..Default::default() })];
    paint_math(app, ui, r, &format!("sym:{s}"), &s, Some(nodes), false, 11.0);
    let name = wordcraft_doc::math_symbols::name_of(c).map(|n| format!("{s}  \\{n}")).unwrap_or_else(|| format!("{s}  U+{:04X}", c as u32));
    resp.on_hover_text(name).clicked()
}

/// The Equation tab.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Tools", None, app, |ui, app| {
        math_menu_button(ui, app, "∑_(i=1)^n", "Equation", builtin_menu);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
            let latex = app.session.math_latex;
            small(
                ui,
                app,
                "equation",
                Some("Unicode"),
                "Type equations in the linear format (UnicodeMath)",
                "equation.inputFormat",
                json!({"format": "unicode"}),
                !latex,
            );
            small(ui, app, "formula", Some("LaTeX"), "Type equations in LaTeX", "equation.inputFormat", json!({"format": "latex"}), latex);
            crate::widgets::menu_button(ui, app, "replace", Some("Convert"), "Convert between built-up and linear form", false, |ui, app| {
                for (label, to, all) in [
                    ("Current - Professional", "professional", false),
                    ("Current - Linear", "linear", false),
                    ("All - Professional", "professional", true),
                    ("All - Linear", "linear", true),
                ] {
                    if ui.button(tl!(label)).clicked() {
                        let _ = app.run("equation.convert", json!({"to": to, "all": all}));
                        app.canvas.want_focus = true;
                        ui.close();
                    }
                }
            });
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
            let normal = app.session.math_normal_text;
            small(ui, app, "case", Some("Normal Text"), "Type normal (non-math) text in the equation", "equation.normalText", json!({}), normal);
        });
    });
    group(ui, "Symbols", None, app, symbols);
    group(ui, "Structures", None, app, |ui, app| {
        for g in STRUCTURES {
            let id = g.id;
            math_menu_button(ui, app, button_picture(id), g.label, |ui, app| structure_menu(ui, app, id));
        }
    });
    group(ui, "Equation Options", None, app, |ui, app| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
            let info = app.session.run("equation.get", &json!({})).ok();
            let display = info.as_ref().and_then(|v| v.get("display")).and_then(|v| v.as_bool()).unwrap_or(false);
            let numbered = info.as_ref().and_then(|v| v.get("linear")).and_then(|v| v.as_str()).is_some_and(|l| l.contains('#'));
            small(
                ui,
                app,
                "alignCenter",
                Some("Display"),
                "Show the equation on a line of its own",
                "equation.display",
                json!({"value": true}),
                display,
            );
            small(
                ui,
                app,
                "alignLeft",
                Some("Inline"),
                "Show the equation in the line of text",
                "equation.display",
                json!({"value": false}),
                !display,
            );
            small(
                ui,
                app,
                "numbering",
                Some("Number"),
                "Number the equation; the number sits at the right margin (or type # then the number)",
                "equation.number",
                json!({}),
                numbered,
            );
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
            crate::widgets::menu_button(ui, app, "justify", Some("Justification"), "Justification of a display equation", false, |ui, app| {
                for (label, jc) in [("Left", "left"), ("Right", "right"), ("Centered", "center"), ("Centered as Group", "centerGroup")] {
                    if ui.button(tl!(label)).clicked() {
                        let _ = app.run("equation.justify", json!({"jc": jc}));
                        ui.close();
                    }
                }
            });
            if ui.button(tl!("Close Equation")).on_hover_text(tl!("Leave the equation (Esc)")).clicked() {
                let _ = app.run("equation.exit", json!({}));
                app.canvas.want_focus = true;
            }
        });
    });
}
