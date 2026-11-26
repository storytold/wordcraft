use craft::menu::CommandRef;
use craft::ui::contract::{Props as _, Show as _};
use craft::ui::display::Divider;
use craft::ui::list::ThumbAspect;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::FacePaint;

use egui::{Rect, Stroke, pos2};

use super::{Ui, WordApp, json};
use crate::colors::Target;

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
    for (idx, (style, name)) in UNDERLINE_STYLES.into_iter().enumerate() {
        // The sample stands for the label; the name is the tooltip and what assistive technology reads.
        MenuItem::command(CommandRef::with_params("format.underline", json!({"style": style})))
            .label(tl!(name).to_string())
            .tooltip(tl!(name).to_string())
            .sample(ThumbAspect::new(4, 1), FacePaint::new(idx as u64, move |cx, r| paint_underline(&cx.painter, r, style, cx.ink())))
            .sample_only()
            .hide_shortcut()
            .show(ui);
    }
    MenuItem::command(CommandRef::new("format.wordUnderline")).label(tl!("Words only").to_string()).show(ui);
    // Ctrl+U beside None would read as its shortcut, but Ctrl+U toggles a single underline.
    MenuItem::command(CommandRef::with_params("format.underline", json!({"value": false}))).label(tl!("None").to_string()).hide_shortcut().show(ui);
    Divider::horizontal().show(ui);
    MenuItem::command(CommandRef::new("format.fontDialog")).label(tl!("More Underlines…").to_string()).show(ui);
    Menu::submenu_row(ui, tl!("Underline Color").to_string(), |ui, _| {
        MenuItem::command(CommandRef::with_params("format.underlineColor", json!({"color": "auto"}))).label(tl!("Automatic").to_string()).show(ui);
        let _ = crate::colors::menu(ui, app, Target::cmd("format.underlineColor"));
    });
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
