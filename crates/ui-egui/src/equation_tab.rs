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
use crate::widgets::{BIG_ICON_Y, BIG_LABEL_Y, CONTENT_H, LABEL_PITCH, group, small};

/// What each structure button shows.
pub(crate) fn button_picture(id: &str) -> &'static str {
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
struct MathPreview<'a> {
    rect: Rect,
    key: &'a str,
    linear: &'a str,
    nodes: Option<wordcraft_doc::math::Arg>,
    display: bool,
    points: f32,
}

fn paint_math(app: &mut WordApp, ui: &Ui, preview: MathPreview<'_>) {
    let MathPreview { rect: r, key, linear, nodes, display, points: pt } = preview;
    let nodes = nodes.unwrap_or_else(|| parse_linear(linear));
    match crate::previews::math_texture(app, ui.ctx(), key, &nodes, display, pt, r.size()) {
        Some((tex, uv)) => {
            ui.painter().image(tex, r, uv, Tokens::get(ui.ctx()).text);
        }
        None => {
            let t = Tokens::get(ui.ctx());
            // Math previews are asynchronous. Interface fonts need not cover the math font's
            // repertoire, so use readable linear commands until the actual equation is ready.
            let placeholder = math_placeholder(ui, linear);
            ui.painter().with_clip_rect(r).text(r.center(), Align2::CENTER_CENTER, placeholder, regular(11.0), t.text_dim);
        }
    }
}

/// Builds a readable fallback for a math preview from the interface fonts' glyph coverage:
/// covered characters and whitespace stay, while a missing glyph becomes its UnicodeMath name
/// or a `U+XXXX` codepoint.
fn math_placeholder(ui: &Ui, linear: &str) -> String {
    let font = regular(11.0);
    math_placeholder_with(linear, |c| ui.fonts_mut(|fonts| fonts.has_glyph(&font, c)))
}

/// [`math_placeholder`] given which characters the interface fonts cover. A character they lack
/// becomes its UnicodeMath name (`┬` → `\below`), so the same template has a different
/// placeholder in a build without craft-fonts; the i18n audit recognizes the exact strings
/// (`placeholder_linears`) as math input rather than interface wording.
pub(crate) fn math_placeholder_with(linear: &str, covered: impl Fn(char) -> bool) -> String {
    let mut result = String::new();
    for c in linear.chars() {
        if c.is_whitespace() || covered(c) {
            result.push(c);
        } else if let Some(name) = wordcraft_doc::math_symbols::name_of(c) {
            result.push('\\');
            result.push_str(name);
        } else {
            use std::fmt::Write as _;
            let _ = write!(result, "U+{:04X}", c as u32);
        }
    }
    result
}

/// The Equation tab's own big button picture.
const EQUATION_PICTURE: &str = "∑_(i=1)^n";

/// Every linear text the Equation tab can paint as a placeholder while a preview loads: the big
/// buttons' pictures, every template and built-in equation, and every symbol tile.
#[cfg(test)]
pub(crate) fn placeholder_linears() -> impl Iterator<Item = String> {
    let pictures = STRUCTURES.iter().map(|g| button_picture(g.id).to_string());
    let templates = wordcraft_engine::math_gallery::templates().map(|t| t.linear.to_string());
    let symbols = SYMBOL_SETS.iter().flat_map(|(_, chars)| chars.chars()).map(|c| c.to_string());
    std::iter::once(EQUATION_PICTURE.to_string())
        .chain(pictures)
        .chain(templates)
        .chain(BUILT_INS.iter().map(|(_, _, linear)| (*linear).to_string()))
        .chain(symbols)
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
    let pic = Rect::from_center_size(pos2(r.center().x, r.min.y + BIG_ICON_Y), vec2(40.0, 32.0));
    ui.painter().rect_filled(pic, 3.0, t.input);
    paint_math(app, ui, MathPreview { rect: pic, key: &format!("btn:{picture}"), linear: picture, nodes: None, display: false, points: 12.0 });
    let lines: Vec<&str> = label.split('\n').collect();
    let mut y = r.min.y + BIG_LABEL_Y;
    for (i, l) in lines.iter().enumerate() {
        crate::widgets::label_line(ui, r.center().x, y, l, regular(11.5), t.text, i + 1 == lines.len());
        y += LABEL_PITCH;
    }
    crate::widgets::describe_button(&resp, &label.replace('\n', " "), ui.is_enabled(), None);
    crate::widgets::opens_menu(&resp);
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
    ui.painter().rect_filled(r, 2.0, t.input);
    if resp.hovered() {
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    let pt = if size.x > 120.0 { 13.0 } else { 14.0 };
    paint_math(
        app,
        ui,
        MathPreview { rect: r.shrink(2.0), key: &format!("tpl:{}", tpl.id), linear: tpl.linear, nodes: Some(tpl.nodes()), display: true, points: pt },
    );
    crate::widgets::describe_button(&resp, tl!(tpl.label), true, None);
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
            ui.painter().rect_filled(r, 2.0, t.input);
            if resp.hovered() {
                ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            }
            paint_math(
                app,
                ui,
                MathPreview { rect: r.shrink(2.0), key: &format!("builtin:{id}"), linear: lin, nodes: None, display: true, points: 11.0 },
            );
            crate::widgets::describe_button(&resp, tl!(label), true, None);
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
    let top = Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + BIG_LABEL_Y - 2.0));
    let bottom = Rect::from_min_max(pos2(r.min.x, r.min.y + BIG_LABEL_Y - 2.0), r.max);
    let rt = ui.interact(top, ui.scope_id().with("eq_insert_top"), Sense::click());
    let rb = ui.interact(bottom, ui.scope_id().with("eq_insert_bottom"), Sense::click());
    for (rr, resp) in [(top, &rt), (bottom, &rb)] {
        if resp.hovered() {
            ui.painter().rect_filled(rr, 4.0, t.hover);
        }
    }
    crate::icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(r.center().x, r.min.y + BIG_ICON_Y), vec2(32.0, 32.0)),
        "equation",
        t.icon,
        t.accent,
    );
    crate::widgets::label_line(ui, r.center().x, r.min.y + BIG_LABEL_Y, label, regular(11.5), t.text, true);
    // A split button to screen readers too: the picture inserts, the label opens the list.
    crate::widgets::describe_button(&rt, tl!("Insert New Equation"), ui.is_enabled(), None);
    crate::widgets::describe_button(&rb, label, ui.is_enabled(), None);
    crate::widgets::opens_menu(&rb);
    if rt.on_hover_text(tl!("Insert a new equation (Alt+=)")).clicked() {
        let _ = app.run("insert.equation", json!({}));
        app.canvas.want_focus = true;
    }
    egui::Popup::menu(&rb).show(|ui| builtin_menu(ui, app));
}

/// The Symbols group: a strip of the current set's symbols, and a menu with every set.
fn symbols(ui: &mut Ui, app: &mut WordApp) {
    let id = egui::Id::unique("eq_symbol_set");
    let set = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0).min(SYMBOL_SETS.len().saturating_sub(1));
    let Some((_, chars)) = SYMBOL_SETS.get(set) else { return };
    let start_id = egui::Id::unique("eq_symbol_start");
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

/// Draws one equation-tab symbol button, paints its preview, shows the linear name and codepoint
/// in a tooltip, and returns whether it was clicked.
fn symbol_button(ui: &mut Ui, app: &mut WordApp, c: char, t: &Tokens) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    ui.painter().rect_filled(r, 2.0, t.input);
    if resp.hovered() {
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    let s = c.to_string();
    let nodes = vec![wordcraft_doc::math::MNode::Run(wordcraft_doc::math::MRun { text: s.clone(), lit: true, ..Default::default() })];
    paint_math(app, ui, MathPreview { rect: r, key: &format!("sym:{s}"), linear: &s, nodes: Some(nodes), display: false, points: 11.0 });
    let display = math_placeholder(ui, &s);
    let name =
        wordcraft_doc::math_symbols::name_of(c).map(|n| format!("{display}  \\{n}")).unwrap_or_else(|| format!("{display}  U+{:04X}", c as u32));
    crate::widgets::describe_button(&resp, &name, ui.is_enabled(), None);
    resp.on_hover_text(name).clicked()
}

/// The Equation tab.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Tools", None, app, |ui, app| {
        math_menu_button(ui, app, EQUATION_PICTURE, "Equation", builtin_menu);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
            let latex = app.session.math_latex;
            small(
                ui,
                app,
                "equation",
                Some("Unicode"),
                "Type equations in the linear format (UnicodeMath)",
                crate::widgets::ButtonAction::toggle("equation.inputFormat", json!({"format": "unicode"}), !latex),
            );
            small(
                ui,
                app,
                "formula",
                Some("LaTeX"),
                "Type equations in LaTeX",
                crate::widgets::ButtonAction::toggle("equation.inputFormat", json!({"format": "latex"}), latex),
            );
            crate::widgets::menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "replace",
                    Some("Convert"),
                    "Convert between built-up and linear form",
                    false,
                    "equation.convert",
                ),
                |ui, app| {
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
                },
            );
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
            let normal = app.session.math_normal_text;
            small(
                ui,
                app,
                "case",
                Some("Normal Text"),
                "Type normal (non-math) text in the equation",
                crate::widgets::ButtonAction::toggle("equation.normalText", json!({}), normal),
            );
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
                crate::widgets::ButtonAction::toggle("equation.display", json!({"value": true}), display),
            );
            small(
                ui,
                app,
                "alignLeft",
                Some("Inline"),
                "Show the equation in the line of text",
                crate::widgets::ButtonAction::toggle("equation.display", json!({"value": false}), !display),
            );
            small(
                ui,
                app,
                "numbering",
                Some("Number"),
                "Number the equation; the number sits at the right margin (or type # then the number)",
                crate::widgets::ButtonAction::toggle("equation.number", json!({}), numbered),
            );
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
            crate::widgets::menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "justify",
                    Some("Justification"),
                    "Justification of a display equation",
                    false,
                    "equation.justify",
                ),
                |ui, app| {
                    for (label, jc) in [("Left", "left"), ("Right", "right"), ("Centered", "center"), ("Centered as Group", "centerGroup")] {
                        if ui.button(tl!(label)).clicked() {
                            let _ = app.run("equation.justify", json!({"jc": jc}));
                            ui.close();
                        }
                    }
                },
            );
            if ui.button(tl!("Close Equation")).on_hover_text(tl!("Leave the equation (Esc)")).clicked() {
                let _ = app.run("equation.exit", json!({}));
                app.canvas.want_focus = true;
            }
        });
    });
}

#[cfg(test)]
mod placeholder_tests {
    /// `∁` falls back to `\complement` in the placeholder while covered `x+y` is unchanged.
    #[test]
    fn a_missing_math_glyph_has_a_readable_linear_placeholder() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(!ui.fonts_mut(|fonts| fonts.has_glyph(&super::regular(11.0), '∁')));
            assert_eq!(super::math_placeholder(ui, "x+\u{2201}"), "x+\\complement");
            assert_eq!(super::math_placeholder(ui, "x+y"), "x+y");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn a_missing_math_glyph_falls_back_to_its_linear_name() {
        // Without craft-fonts the interface fonts may have no `┬`: the Equation button's picture
        // then reads `lim\below(n→∞)`. That is still math input, not interface wording.
        let covered = |c: char| c.is_ascii() || matches!(c, '∞' | '→');
        assert_eq!(super::math_placeholder_with("lim┬(n→∞)", covered), "lim\\below(n→∞)");
        assert!(super::placeholder_linears().any(|linear| linear == "lim┬(n→∞)"));
    }
}
