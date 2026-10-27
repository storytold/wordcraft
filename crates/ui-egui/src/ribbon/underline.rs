use super::common::mi;
use super::*;

/// The underline styles Home › Underline offers as line samples: (`format.underline` style, name).
pub(super) const UNDERLINE_STYLES: [(&str, &str); 8] = [
    ("single", "Single"),
    ("double", "Double"),
    ("thick", "Thick"),
    ("dotted", "Dotted"),
    ("dash", "Dashed"),
    ("dotDash", "Dot-dash"),
    ("dotDotDash", "Dot-dot-dash"),
    ("wave", "Wave"),
];

/// Home › Font › Underline's drop-down (#2738): a sample of each line, Words only, None, More
/// Underlines… (the Font dialog) and Underline Color.
pub(super) fn underline_menu(ui: &mut Ui, app: &mut WordApp) {
    for (style, name) in UNDERLINE_STYLES {
        underline_item(ui, app, style, name);
    }
    mi(ui, app, "Words only", "format.wordUnderline", json!({}));
    // Not `mi`: Ctrl+U beside None would read as its shortcut, but Ctrl+U toggles a single underline.
    let enabled = crate::widgets::enabled(app, "format.underline");
    if ui.add_enabled(enabled, egui::Button::new(tl!("None")).min_size(vec2(200.0, 0.0))).clicked() {
        crate::widgets::run_from_ribbon(app, "format.underline", json!({"value": false}));
        ui.close();
    }
    ui.separator();
    mi(ui, app, "More Underlines…", "format.fontDialog", json!({}));
    ui.menu_button(tl!("Underline Color"), |ui| {
        mi(ui, app, "Automatic", "format.underlineColor", json!({"color": "auto"}));
        let theme = app.session.doc.settings.theme_colors.clone();
        crate::colors::menu(ui, app, &theme, Target::cmd("format.underlineColor"));
    });
}

/// A menu entry drawn as a sample of its underline; its name is the tooltip and what assistive
/// technology reads.
fn underline_item(ui: &mut Ui, app: &mut WordApp, style: &str, name: &str) {
    let enabled = crate::widgets::enabled(app, "format.underline");
    let resp = ui.add_enabled(enabled, egui::Button::new("").min_size(vec2(200.0, 22.0)));
    let label = tl!(name);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let t = Tokens::get(ui.ctx());
    let color = if enabled { t.text } else { t.text_disabled };
    paint_underline(ui.painter(), resp.rect.shrink2(vec2(10.0, 0.0)), style, color);
    if resp.on_hover_text(label).clicked() {
        crate::widgets::run_from_ribbon(app, "format.underline", json!({"style": style}));
        ui.close();
    }
}

/// A line in `style` across `r` at its middle: our own drawing of each underline kind.
fn paint_underline(p: &egui::Painter, r: Rect, style: &str, color: egui::Color32) {
    let y = r.center().y.round() + 0.5;
    let (x0, x1) = (r.left(), r.right());
    let line = |y: f32, w: f32| {
        p.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(w, color));
    };
    // Repeats a pattern of (length, gap) pairs along the line; a length of 0 is a dot.
    let pattern = |pat: &[(f32, f32)]| {
        let mut x = x0;
        'run: loop {
            for &(len, gap) in pat {
                if x >= x1 {
                    break 'run;
                }
                if len == 0.0 {
                    p.circle_filled(pos2(x + 0.75, y), 0.75, color);
                    x += 1.5 + gap;
                } else {
                    p.line_segment([pos2(x, y), pos2((x + len).min(x1), y)], Stroke::new(1.0, color));
                    x += len + gap;
                }
            }
        }
    };
    match style {
        "double" => {
            line(y - 1.5, 1.0);
            line(y + 1.5, 1.0);
        }
        "thick" => line(y, 2.5),
        "dotted" => pattern(&[(0.0, 2.0)]),
        "dash" => pattern(&[(6.0, 4.0)]),
        "dotDash" => pattern(&[(6.0, 3.0), (0.0, 3.0)]),
        "dotDotDash" => pattern(&[(6.0, 3.0), (0.0, 2.5), (0.0, 3.0)]),
        "wave" => {
            let pts: Vec<_> =
                (0..=((x1 - x0) as usize)).map(|i| pos2(x0 + i as f32, y + 1.6 * (i as f32 * std::f32::consts::TAU / 6.0).sin())).collect();
            p.add(egui::Shape::line(pts, Stroke::new(1.0, color)));
        }
        _ => line(y, 1.0),
    }
}
