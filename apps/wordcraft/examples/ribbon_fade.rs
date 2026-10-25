//! RibbonFade demo: a small standalone ribbon (Home, Insert, Layout, View) whose tabs cross-fade
//! when you switch between them, built on `wordcraft_ui_egui::ribbon_fade` the same way WordCraft's
//! own ribbon is. Shows how little an egui app needs to adopt it:
//!
//! - wrap the tab's content in [`ribbon_fade::show`] with the selected tab as its key, inside a
//!   region of fixed height (here a top panel), and
//! - pass the selected tab's underline through [`ribbon_fade::indicator`].
//!
//! `cargo run -p wordcraft --example ribbon_fade`
//!
//! The panel below the ribbon changes the duration, the curves and the theme live. Keyboard: Tab
//! reaches the tab strip, Left/Right/Home/End move between tabs (and show them), Tab again goes
//! into the ribbon's buttons, Ctrl+1–4 jump to a tab. `WORDCRAFT_RIBBON_FADE_MS` overrides the
//! duration, for watching the motion slowed down. Everything is drawn in code: no images.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use egui::{Align2, Color32, CornerRadius, FontId, Key, Rect, Sense, Stroke, Ui, pos2, vec2};
use wordcraft_ui_egui::ribbon_fade::{self, EASINGS, Easing, FadeConfig};

/// The demo's tabs, each with its groups and each group's buttons.
const TABS: &[(&str, &[(&str, &[&str])])] = &[
    (
        "Home",
        &[
            ("Clipboard", &["Paste", "Cut", "Copy", "Format Painter"]),
            ("Font", &["Bold", "Italic", "Underline", "Text Colour", "Highlight", "Clear Formatting"]),
            ("Paragraph", &["Bullets", "Numbering", "Align Left", "Center", "Line Spacing"]),
            ("Styles", &["Normal", "Heading 1", "Heading 2", "Title"]),
            ("Editing", &["Find", "Replace", "Select"]),
        ],
    ),
    (
        "Insert",
        &[
            ("Pages", &["Cover Page", "Blank Page", "Page Break"]),
            ("Tables", &["Table"]),
            ("Illustrations", &["Pictures", "Shapes", "Icons", "Chart"]),
            ("Links", &["Link", "Bookmark", "Cross-reference"]),
            ("Text", &["Text Box", "Header", "Footer", "Page Number"]),
        ],
    ),
    (
        "Layout",
        &[
            ("Page Setup", &["Margins", "Orientation", "Size", "Columns", "Breaks"]),
            ("Paragraph", &["Indent Left", "Indent Right", "Spacing Before", "Spacing After"]),
            ("Arrange", &["Position", "Wrap Text", "Bring Forward", "Send Backward", "Align"]),
        ],
    ),
    (
        "View",
        &[
            ("Views", &["Read Mode", "Print Layout", "Web Layout", "Outline"]),
            ("Show", &["Ruler", "Gridlines", "Navigation Pane"]),
            ("Zoom", &["Zoom", "100%", "One Page", "Page Width"]),
            ("Window", &["New Window", "Split", "Side by Side"]),
        ],
    ),
];

/// The ribbon's height: buttons and group labels. Fixed, so a switch never moves anything.
const RIBBON_H: f32 = 96.0;
/// The height of a group's three rows of buttons.
const BUTTONS_H: f32 = 3.0 * 22.0 + 2.0 * 2.0;

/// The demo's own colours (not WordCraft's theme, so the example stands alone).
#[derive(Clone, Copy)]
struct Palette {
    strip: Color32,
    ribbon: Color32,
    page: Color32,
    text: Color32,
    dim: Color32,
    border: Color32,
    hover: Color32,
    accent: Color32,
}

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Palette {
                strip: Color32::from_rgb(0x29, 0x29, 0x29),
                ribbon: Color32::from_rgb(0x32, 0x32, 0x32),
                page: Color32::from_rgb(0x1f, 0x1f, 0x1f),
                text: Color32::from_rgb(0xf2, 0xf2, 0xf2),
                dim: Color32::from_rgb(0xb0, 0xb0, 0xb0),
                border: Color32::from_rgb(0x48, 0x48, 0x48),
                hover: Color32::from_rgb(0x44, 0x44, 0x44),
                accent: Color32::from_rgb(0x6c, 0xb4, 0xf0),
            }
        } else {
            Palette {
                strip: Color32::from_rgb(0xf3, 0xf3, 0xf3),
                ribbon: Color32::from_rgb(0xfb, 0xfb, 0xfb),
                page: Color32::from_rgb(0xe9, 0xe9, 0xe9),
                text: Color32::from_rgb(0x24, 0x24, 0x24),
                dim: Color32::from_rgb(0x61, 0x61, 0x61),
                border: Color32::from_rgb(0xd6, 0xd6, 0xd6),
                hover: Color32::from_rgb(0xe6, 0xe6, 0xe6),
                accent: Color32::from_rgb(0x1a, 0x5f, 0xb4),
            }
        }
    }
}

struct Demo {
    tab: usize,
    dark: bool,
    reduce_motion: bool,
    config: FadeConfig,
    last: String,
}

impl Demo {
    /// The configuration in force: off under "Reduce motion".
    fn animation(&self) -> FadeConfig {
        FadeConfig { enabled: self.config.enabled && !self.reduce_motion, ..self.config.clone() }
    }

    fn tab_strip(&mut self, ui: &mut Ui, p: Palette) {
        let animation = self.animation();
        let font = FontId::proportional(13.0);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
            for (i, (name, _)) in TABS.iter().enumerate() {
                let text_w = ui.fonts_mut(|f| f.layout_no_wrap((*name).to_string(), font.clone(), p.text).size().x);
                let (_, r) = ui.allocate_space(vec2(text_w + 24.0, 30.0));
                let resp = ui.interact(r, tab_id(i), Sense::click());
                let active = self.tab == i;
                // Screen readers hear a tab, and whether it is the selected one.
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, *name));
                if resp.hovered() && !active {
                    ui.painter().rect_filled(r.shrink2(vec2(0.0, 4.0)), 4.0, p.hover);
                }
                ui.painter().text(r.center(), Align2::CENTER_CENTER, *name, font.clone(), if active { p.accent } else { p.text });
                if active {
                    let u = Rect::from_center_size(pos2(r.center().x, r.max.y - 2.0), vec2(text_w, 3.0));
                    let u = ribbon_fade::indicator(ui, "demo_tabs", i, u, &animation);
                    ui.painter().rect_filled(u, 2.0, p.accent);
                }
                if resp.has_focus() {
                    ui.painter().rect_stroke(r.shrink2(vec2(1.0, 3.0)), 4.0, Stroke::new(1.5, p.text), egui::StrokeKind::Inside);
                    // Keep the arrows from egui's own focus moves: they go from tab to tab here.
                    ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
                    let last = TABS.len().saturating_sub(1);
                    let to = ui.input_mut(|inp| {
                        if inp.consume_key(egui::Modifiers::NONE, Key::ArrowRight) {
                            Some(if i >= last { 0 } else { i + 1 })
                        } else if inp.consume_key(egui::Modifiers::NONE, Key::ArrowLeft) {
                            Some(if i == 0 { last } else { i - 1 })
                        } else if inp.consume_key(egui::Modifiers::NONE, Key::Home) {
                            Some(0)
                        } else if inp.consume_key(egui::Modifiers::NONE, Key::End) {
                            Some(last)
                        } else {
                            None
                        }
                    });
                    if let Some(to) = to {
                        self.tab = to;
                        ui.memory_mut(|m| m.request_focus(tab_id(to)));
                    }
                }
                if resp.clicked() {
                    self.tab = i;
                }
            }
        });
    }

    fn ribbon(&mut self, ui: &mut Ui, p: Palette) {
        let animation = self.animation();
        let tab = self.tab;
        let last = &mut self.last;
        ribbon_fade::show(ui, "demo_ribbon", tab, &animation, |ui| {
            let Some((_, groups)) = TABS.get(tab) else { return };
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                for (title, buttons) in groups.iter() {
                    group(ui, p, title, buttons, last);
                    let (_, sep) = ui.allocate_space(vec2(1.0, RIBBON_H - 12.0));
                    ui.painter().vline(sep.center().x, sep.y_range(), Stroke::new(1.0, p.border));
                }
            });
        });
    }

    fn settings(&mut self, ui: &mut Ui, p: Palette) {
        ui.add_space(16.0);
        ui.heading("RibbonFade");
        ui.colored_label(p.dim, "Switch tabs with the mouse, or Tab to the strip and use the arrow keys.");
        ui.add_space(8.0);
        egui::Grid::new("settings").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
            ui.label("Animate tab switching");
            ui.checkbox(&mut self.config.enabled, "");
            ui.end_row();
            ui.label("Reduce motion (system)");
            ui.checkbox(&mut self.reduce_motion, "");
            ui.end_row();
            ui.label("Duration");
            ui.add(egui::Slider::new(&mut self.config.duration_ms, 0..=1000).suffix(" ms"));
            ui.end_row();
            ui.label("Incoming curve");
            easing_combo(ui, "easing", &mut self.config.easing);
            ui.end_row();
            ui.label("Outgoing curve");
            easing_combo(ui, "exit_easing", &mut self.config.exit_easing);
            ui.end_row();
            ui.label("Outgoing share");
            ui.add(egui::Slider::new(&mut self.config.exit_share, 0.1..=1.0));
            ui.end_row();
            ui.label("Slide the underline");
            ui.checkbox(&mut self.config.slide_indicator, "");
            ui.end_row();
            ui.label("Theme");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.dark, false, "Light");
                ui.selectable_value(&mut self.dark, true, "Dark");
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        if ui.button("Defaults").clicked() {
            self.config = FadeConfig::default();
        }
        ui.add_space(8.0);
        ui.colored_label(p.dim, format!("Last button: {}", if self.last.is_empty() { "—" } else { self.last.as_str() }));
    }
}

/// A tab's widget id, so the arrow keys can hand it the focus.
fn tab_id(i: usize) -> egui::Id {
    egui::Id::unique(("demo_tab", i))
}

/// One ribbon group: its buttons in columns of three, its title centred underneath.
fn group(ui: &mut Ui, p: Palette, title: &str, buttons: &[&str], last: &mut String) {
    ui.vertical(|ui| {
        let title_w = ui.fonts_mut(|f| f.layout_no_wrap(title.to_string(), FontId::proportional(11.0), p.dim).size().x);
        let row = ui
            .horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.set_min_width(title_w);
                for column in buttons.chunks(3) {
                    ui.vertical(|ui| {
                        for b in column {
                            let text = egui::RichText::new(*b).color(p.text).size(12.0);
                            if ui.add(egui::Button::new(text).frame(false).min_size(vec2(0.0, 22.0))).clicked() {
                                *last = (*b).to_string();
                            }
                        }
                    });
                }
            })
            .response
            .rect;
        // Every group's title on the same line, whatever its buttons' height.
        ui.add_space((BUTTONS_H - row.height()).max(0.0));
        ui.allocate_ui_with_layout(vec2(row.width(), 16.0), egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(title).color(p.dim).size(11.0));
        });
    });
}

fn easing_combo(ui: &mut Ui, id: &str, easing: &mut Easing) {
    let shown = easing.name().unwrap_or("custom");
    egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
        for (name, e) in EASINGS {
            ui.selectable_value(easing, *e, *name);
        }
    });
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let p = Palette::new(self.dark);
        ui.ctx().set_visuals(if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() });
        // Ctrl+1–4 jump to a tab.
        let jump = ui.input_mut(|i| [Key::Num1, Key::Num2, Key::Num3, Key::Num4].iter().position(|k| i.consume_key(egui::Modifiers::COMMAND, *k)));
        if let Some(t) = jump.filter(|t| *t < TABS.len()) {
            self.tab = t;
        }
        egui::Panel::top("demo_tabs")
            .exact_size(30.0)
            .frame(egui::Frame::NONE.fill(p.strip).inner_margin(egui::Margin { left: 8, right: 8, top: 0, bottom: 0 }))
            .show(ui, |ui| self.tab_strip(ui, p));
        egui::Panel::top("demo_ribbon")
            .exact_size(RIBBON_H)
            .frame(
                egui::Frame::NONE
                    .fill(p.ribbon)
                    .inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 })
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::ZERO),
            )
            .show(ui, |ui| self.ribbon(ui, p));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(p.page).inner_margin(16)).show(ui, |ui| self.settings(ui, p));
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("RibbonFade demo").with_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "RibbonFade demo",
        options,
        Box::new(|_cc| Ok(Box::new(Demo { tab: 0, dark: false, reduce_motion: false, config: FadeConfig::default(), last: String::new() }))),
    )
}
