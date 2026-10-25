//! Paragraph dialogs (#320): Tabs, and Borders and Shading. Each ends by running commands
//! (`para.tabs`; `para.borders`, `para.shading`, `format.set`, `format.shading`,
//! `design.pageBorders`) as one undo step, so agents get the same result without the dialog.

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::props::{Border, BorderStyle, Borders, Rgb, TabAlign, TabLeader, TabStop};

use crate::WordApp;
use crate::dialogs::{buttons, color_menu};
use crate::options::MeasureExt;
use crate::theme::{Tokens, semibold};

/// Stops closer than this (points) are the same stop.
/// A leader choice: the value, its menu label (translated), and the fill the closed box shows.
const LEADERS: [(TabLeader, &str, &str); 5] = [
    (TabLeader::None, "None", ""),
    (TabLeader::Dot, "1 ……", "……"),
    (TabLeader::Hyphen, "2 ------", "------"),
    (TabLeader::Underscore, "3 ______", "______"),
    (TabLeader::MiddleDot, "4 ······", "······"),
];

/// [`wordcraft_doc::props::TAB_POS_EPS`]: the engine replaces stops within this tolerance.
const SAME_POS: f32 = wordcraft_doc::props::TAB_POS_EPS;

fn unit() -> wordcraft_geom::Unit {
    crate::options::unit()
}

/// The Tabs dialog's fields. Positions in `stops` and `cleared` are points; `default_tab` is in
/// the interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabsForm {
    pub stops: Vec<TabStop>,
    #[serde(skip)]
    pub basis: Vec<TabStop>,
    pub default_tab: f32,
    #[serde(skip)]
    pub basis_default: f32,
    /// Stops to be cleared when OK is pressed.
    pub cleared: Vec<f32>,
    pub clear_all: bool,
    pub selected: Option<usize>,
    /// The row whose position was last edited in place: it wins a merge in [`TabsForm::normalize`].
    #[serde(skip)]
    last_edit: Option<usize>,
}

impl TabsForm {
    /// The caret paragraph's stops and the document's default interval.
    pub fn read(app: &WordApp) -> Option<TabsForm> {
        let s = &app.session;
        let rp = s.doc.para_at(&s.sel.focus).map(|p| s.doc.styles.resolve_para(&p.props))?;
        let k = unit().pt_per_unit();
        let stops: Vec<TabStop> = rp.tabs.iter().copied().filter(|t| t.align != TabAlign::Clear && t.pos.is_finite()).take(64).collect();
        let first = stops.first().copied();
        let default_tab = s.doc.settings.default_tab / k;
        Some(TabsForm {
            basis: stops.clone(),
            stops,
            default_tab,
            basis_default: default_tab,
            cleared: Vec::new(),
            clear_all: false,
            selected: first.map(|_| 0),
            last_edit: None,
        })
    }

    /// Add a stop at the next free multiple of the default interval after the last stop —
    /// left aligned, no leader — selected and ready to edit in place.
    pub fn new_stop(&mut self) {
        let step = if self.default_tab.is_finite() && self.default_tab > 0.0 { self.default_tab * unit().pt_per_unit() } else { 36.0 };
        let taken = |p: f32| self.stops.iter().any(|t| (t.pos - p).abs() < SAME_POS) || self.cleared.iter().any(|c| (c - p).abs() < SAME_POS);
        let last = self.stops.iter().map(|t| t.pos).fold(0.0f32, f32::max);
        let mut p = (last / step).floor() * step + step;
        while taken(p) && p <= 1584.0 {
            p += step;
        }
        if !p.is_finite() || p > 1584.0 || self.stops.len() >= 64 {
            return;
        }
        self.stops.push(TabStop { pos: p, align: TabAlign::default(), leader: TabLeader::default() });
        self.stops.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        self.selected = self.stops.iter().position(|t| (t.pos - p).abs() < SAME_POS);
        self.last_edit = None;
    }

    /// The list edits a stop's position in place: clamp to the engine's ±1584 bound. The list
    /// stays in row order while the field is being edited; [`TabsForm::normalize`] restores
    /// position order when the edit ends.
    pub fn set_pos(&mut self, i: usize, pos: f32) {
        if !pos.is_finite() {
            return;
        }
        if let Some(t) = self.stops.get_mut(i) {
            t.pos = pos.clamp(-1584.0, 1584.0);
            let p = t.pos;
            self.cleared.retain(|c| (c - p).abs() >= SAME_POS);
            self.last_edit = Some(i);
        }
    }

    /// The list edits a stop's alignment in place; the fields follow when it is the selected row.
    pub fn set_align(&mut self, i: usize, align: TabAlign) {
        if let Some(t) = self.stops.get_mut(i) {
            t.align = align;
        }
    }

    /// The list edits a stop's leader in place; the fields follow when it is the selected row.
    pub fn set_leader(&mut self, i: usize, leader: TabLeader) {
        if let Some(t) = self.stops.get_mut(i) {
            t.leader = leader;
        }
    }

    /// Restore position order after in-place edits, merging stops moved onto one another
    /// (the edited row wins), and keep the selection on its stop.
    pub fn normalize(&mut self) {
        // Only an in-place position edit (set_pos) can disorder the list; without one this is
        // a no-op, so the dialog may call it every frame.
        if self.last_edit.is_none() {
            return;
        }
        let followed = self.selected.and_then(|i| self.stops.get(i)).copied();
        let edited = self.last_edit.take();
        let mut merged: Vec<(usize, TabStop)> = Vec::with_capacity(self.stops.len());
        for (j, t) in self.stops.iter().enumerate() {
            match merged.iter().position(|(_, x)| (x.pos - t.pos).abs() < SAME_POS) {
                // The edited stop holds its ground; the row that drifted onto it goes.
                Some(k) if edited == Some(merged[k].0) => {}
                Some(k) => {
                    merged.remove(k);
                    merged.push((j, *t));
                }
                None => merged.push((j, *t)),
            }
        }
        merged.sort_by(|a, b| a.1.pos.total_cmp(&b.1.pos));
        self.stops = merged.into_iter().map(|(_, t)| t).collect();
        self.selected = followed.and_then(|t| self.stops.iter().position(|x| (x.pos - t.pos).abs() < SAME_POS));
    }

    /// Clear stop `i` (the list's × button and the Delete key).
    pub fn remove_stop(&mut self, i: usize) {
        if i >= self.stops.len() {
            return;
        }
        let t = self.stops.remove(i);
        self.cleared.push(t.pos);
        self.last_edit = None;
        self.selected = match self.selected {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            s => s,
        };
    }

    pub fn clear_all(&mut self) {
        self.cleared.extend(self.stops.drain(..).map(|t| t.pos));
        self.clear_all = true;
        self.selected = None;
    }

    /// `para.tabs` parameters for what changed (`None` when nothing did).
    pub fn params(&self) -> Option<Value> {
        let mut v = serde_json::Map::new();
        if self.clear_all {
            v.insert("clearAll".into(), json!(true));
            if !self.stops.is_empty() {
                v.insert("set".into(), json!(self.stops));
            }
        } else {
            let set: Vec<&TabStop> = self.stops.iter().filter(|t| !self.basis.contains(t)).collect();
            let clear: Vec<f32> = self.basis.iter().filter(|b| !self.stops.iter().any(|t| (t.pos - b.pos).abs() < SAME_POS)).map(|b| b.pos).collect();
            if !set.is_empty() {
                v.insert("set".into(), json!(set));
            }
            if !clear.is_empty() {
                v.insert("clear".into(), json!(clear));
            }
        }
        if (self.default_tab - self.basis_default).abs() > 1e-4 {
            v.insert("default".into(), json!(self.default_tab * unit().pt_per_unit()));
        }
        (!v.is_empty()).then_some(Value::Object(v))
    }
}

/// The Tabs dialog body. Returns true to close.
pub fn tabs(app: &mut WordApp, ui: &mut Ui, f: &mut TabsForm) -> bool {
    let u = unit();
    let k = u.pt_per_unit();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui.button(tl!("New")).clicked() {
                    f.new_stop();
                }
                if ui.add_enabled(!f.stops.is_empty(), egui::Button::new(tl!("Clear All"))).clicked() {
                    f.clear_all();
                }
            });
            let mut pick = None;
            egui::Frame::new().stroke(Stroke::new(1.0, Tokens::get(ui.ctx()).input_border)).inner_margin(4.0).show(ui, |ui| {
                // A fixed width: the scroll area fills it (rows and scrollbar at its edge)
                // without growing the dialog.
                ui.set_width(278.0);
                ui.set_min_height(96.0);
                let mut remove = None;
                let mut row_active = false;
                if f.stops.is_empty() {
                    // #3031: a fresh paragraph has no stops; say the list is empty instead of
                    // showing a blank box.
                    ui.add_space(38.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(tl!("(none)")).small().color(Tokens::get(ui.ctx()).text_dim));
                    });
                } else {
                    // The stops as an inline-editable table: position, alignment, clear. Fill the
                    // frame, so rows span its width and the scrollbar sits at its edge.
                    egui::ScrollArea::vertical().max_height(96.0).auto_shrink([false, true]).show(ui, |ui| {
                        for i in 0..f.stops.len() {
                            let Some(t) = f.stops.get(i).copied() else { continue };
                            ui.push_id(i, |ui| {
                                let row = ui.horizontal(|ui| {
                                    // Position, edited in place: the interface unit by default, any
                                    // unit spelled out, either decimal separator (`.measure`).
                                    let mut pv = t.pos / k;
                                    let r = ui
                                        .add_sized(
                                            vec2(58.0, 18.0),
                                            egui::DragValue::new(&mut pv).speed(0.05).range(-1584.0 / k..=1584.0 / k).measure(u).max_decimals(2),
                                        )
                                        .accessible_name(format!("{} {}", tl!("Tab stop position:"), i + 1));
                                    if r.changed() {
                                        f.set_pos(i, pv * k);
                                    }
                                    if r.has_focus() || r.dragged() {
                                        row_active = true;
                                        if f.selected != Some(i) {
                                            pick = Some(i);
                                        }
                                    }
                                    // Alignment, picked in place.
                                    let mut a = t.align;
                                    egui::ComboBox::from_id_salt("tab_align")
                                        .width(86.0)
                                        .selected_text(tl!(crate::ruler_tabs::kind_label(a)))
                                        .show_ui(ui, |ui| {
                                            for v in crate::ruler_tabs::KINDS {
                                                ui.selectable_value(&mut a, v, tl!(crate::ruler_tabs::kind_label(v)));
                                            }
                                        })
                                        .response
                                        .accessible_name(format!("{} {}", tl!("Alignment:"), i + 1));
                                    if a != t.align {
                                        f.set_align(i, a);
                                        pick = Some(i);
                                    }
                                    // Leader, picked in place; the closed box shows just the fill.
                                    let mut l = t.leader;
                                    let shown = LEADERS
                                        .iter()
                                        .find(|(v, _, _)| *v == l)
                                        .map(|(_, _, fill)| if fill.is_empty() { tl!("None").to_string() } else { (*fill).to_string() })
                                        .unwrap_or_default();
                                    egui::ComboBox::from_id_salt("tab_leader")
                                        .width(64.0)
                                        .selected_text(shown)
                                        .show_ui(ui, |ui| {
                                            for (v, label, _) in LEADERS {
                                                ui.selectable_value(&mut l, v, tl!(label));
                                            }
                                        })
                                        .response
                                        .accessible_name(format!("{} {}", tl!("Leader"), i + 1));
                                    if l != t.leader {
                                        f.set_leader(i, l);
                                        pick = Some(i);
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_space(10.0); // clear of the scrollbar
                                        if ui.add(egui::Button::new("×").small().frame(false)).on_hover_text(tl!("Clear")).clicked() {
                                            remove = Some(i);
                                        }
                                    });
                                });
                                if f.selected == Some(i) {
                                    let tok = Tokens::get(ui.ctx());
                                    ui.painter().rect_stroke(
                                        row.response.rect.expand(1.0),
                                        2.0,
                                        Stroke::new(1.0, tok.accent),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                            });
                        }
                    });
                }
                if let Some(i) = remove {
                    f.remove_stop(i);
                } else if f.selected.is_some() && ui.memory(|m| m.focused().is_none()) && ui.input(|inp| inp.key_pressed(egui::Key::Delete)) {
                    // Delete clears the selected stop when nothing is being typed.
                    if let Some(i) = f.selected {
                        f.remove_stop(i);
                    }
                } else if !row_active {
                    // Rows go back into position order once no position field is being edited.
                    f.normalize();
                }
            });
            if let Some(i) = pick.filter(|i| *i < f.stops.len()) {
                f.selected = Some(i);
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_min_width(130.0);
            let default_label = ui.label(tl!("Default tab stops:"));
            ui.add(egui::DragValue::new(&mut f.default_tab).speed(0.05).range(1.0 / k..=1584.0 / k).measure(u).max_decimals(2))
                .labelled_by(default_label.id);
            ui.add_space(8.0);
            ui.label(tl!("Tab stops to be cleared:"));
            let list = if f.clear_all {
                tl!("All").to_string()
            } else {
                {
                    // `1,5 cm; 3 cm` where the comma is the decimal separator.
                    let sep = if crate::options::decimal() == ',' { "; " } else { ", " };
                    f.cleared.iter().map(|p| crate::options::length_in(u, *p)).collect::<Vec<_>>().join(sep)
                }
            };
            ui.label(egui::RichText::new(list).weak());
        });
    });
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Some(params) = f.params() {
        let _ = app.run("para.tabs", params);
    }
    ok || cancel
}

/// A border setting: the presets on the left of the Borders and Page Border tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Setting {
    None,
    Box,
    /// A box with heavier bottom and right lines.
    Shadow,
    /// A box with heavier top and left lines.
    ThreeD,
    Custom,
}

/// Border lines as the dialog edits them: one style, colour and width for the chosen sides.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineForm {
    pub setting: Setting,
    pub style: BorderStyle,
    /// `RRGGBB`, empty for automatic.
    pub color: String,
    pub width: f32,
    /// Top, bottom, left, right, between.
    pub sides: [bool; 5],
}

const TOP: usize = 0;
const BOTTOM: usize = 1;
const LEFT: usize = 2;
const RIGHT: usize = 3;
const BETWEEN: usize = 4;

const STYLES: [(BorderStyle, &str); 8] = [
    (BorderStyle::Single, "Single"),
    (BorderStyle::Dotted, "Dotted"),
    (BorderStyle::Dashed, "Dashed"),
    (BorderStyle::DotDash, "Dot dash"),
    (BorderStyle::Double, "Double"),
    (BorderStyle::Triple, "Triple"),
    (BorderStyle::Thick, "Thick"),
    (BorderStyle::Wave, "Wave"),
];

const WIDTHS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];

impl LineForm {
    fn read(b: Option<&Borders>) -> LineForm {
        let side = |x: Option<Border>| x.filter(Border::is_visible);
        let s = b.map(|b| [side(b.top), side(b.bottom), side(b.left), side(b.right), side(b.between)]).unwrap_or_default();
        let on = s.map(|x| x.is_some());
        let first = s.iter().flatten().next().copied();
        let w = |i: usize| s.get(i).copied().flatten().map(|x| x.width).unwrap_or(0.0);
        let four = on[TOP] && on[BOTTOM] && on[LEFT] && on[RIGHT] && !on[BETWEEN];
        let setting = if first.is_none() {
            Setting::None
        } else if four && w(TOP) == w(LEFT) && w(BOTTOM) == w(RIGHT) && w(BOTTOM) > w(TOP) {
            Setting::Shadow
        } else if four && w(TOP) == w(LEFT) && w(BOTTOM) == w(RIGHT) && w(TOP) > w(BOTTOM) {
            Setting::ThreeD
        } else if four && w(TOP) == w(BOTTOM) && w(LEFT) == w(RIGHT) && w(TOP) == w(LEFT) {
            Setting::Box
        } else {
            Setting::Custom
        };
        // The base width of a heavier-sided preset is the lighter side's.
        let width = match setting {
            Setting::Shadow => w(TOP),
            Setting::ThreeD => w(BOTTOM),
            _ => first.map(|f| f.width).unwrap_or(0.5),
        };
        LineForm {
            setting,
            style: first.map(|f| f.style).unwrap_or(BorderStyle::Single),
            color: first.and_then(|f| f.color).map(Rgb::hex).unwrap_or_default(),
            width: if width > 0.0 { width } else { 0.5 },
            sides: on,
        }
    }

    fn apply_setting(&mut self, s: Setting) {
        self.setting = s;
        match s {
            Setting::None => self.sides = [false; 5],
            Setting::Box | Setting::Shadow | Setting::ThreeD => self.sides = [true, true, true, true, false],
            Setting::Custom if !self.sides.iter().any(|x| *x) => self.sides = [true, true, true, true, false],
            Setting::Custom => {}
        }
    }

    fn toggle(&mut self, side: usize) {
        if let Some(x) = self.sides.get_mut(side) {
            *x = !*x;
        }
        self.setting = if self.sides.iter().any(|x| *x) { Setting::Custom } else { Setting::None };
    }

    /// The width of `side` under the setting (Shadow and 3-D double their heavier sides).
    fn side_width(&self, side: usize) -> f32 {
        let heavy = match self.setting {
            Setting::Shadow => side == BOTTOM || side == RIGHT,
            Setting::ThreeD => side == TOP || side == LEFT,
            _ => false,
        };
        let w = self.width.clamp(0.25, 6.0);
        if heavy { (w * 2.0).min(6.0) } else { w }
    }

    fn border(&self, side: usize) -> Border {
        Border { style: self.style, width: self.side_width(side), color: Rgb::parse(&self.color), space: 0.0 }
    }

    /// The `sides` parameter of `para.borders` / `design.pageBorders`.
    fn sides_param(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (i, k) in ["top", "bottom", "left", "right", "between"].into_iter().enumerate() {
            if self.sides.get(i).copied().unwrap_or(false) {
                let mut side = json!({"style": self.style.ooxml(), "width": self.side_width(i)});
                if Rgb::parse(&self.color).is_some() {
                    side["color"] = json!(self.color);
                }
                m.insert(k.into(), side);
            }
        }
        Value::Object(m)
    }
}

/// The Borders and Shading dialog's fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BordersForm {
    /// 0 Borders, 1 Page Border, 2 Shading.
    pub tab: u8,
    pub para: LineForm,
    /// Borders apply to the selected text (a character border) instead of the paragraphs.
    pub to_text: bool,
    pub page: LineForm,
    /// Page borders apply to the whole document instead of this section.
    pub whole_doc: bool,
    /// Fill `RRGGBB`, empty for none.
    pub fill: String,
    pub fill_to_text: bool,
    #[serde(skip)]
    basis: Option<Box<BordersForm>>,
}

impl BordersForm {
    pub fn read(app: &WordApp, tab: u8) -> Option<BordersForm> {
        let s = &app.session;
        let rp = s.doc.para_at(&s.sel.focus).map(|p| s.doc.styles.resolve_para(&p.props))?;
        let sect = wordcraft_engine::cmd::page::sect(s);
        let mut f = BordersForm {
            tab: tab.min(2),
            para: LineForm::read(rp.borders.as_ref()),
            to_text: false,
            page: LineForm::read(sect.page_borders.as_ref()),
            whole_doc: true,
            fill: rp.shading.map(Rgb::hex).unwrap_or_default(),
            fill_to_text: false,
            basis: None,
        };
        f.page.sides[BETWEEN] = false;
        f.basis = Some(Box::new(f.clone()));
        Some(f)
    }

    /// The commands OK runs, in order: only the parts that changed.
    pub fn commands(&self) -> Vec<(&'static str, Value)> {
        let Some(b) = self.basis.as_deref() else { return Vec::new() };
        let mut out = Vec::new();
        if self.para != b.para || self.to_text != b.to_text {
            if self.to_text {
                let first = [TOP, LEFT, BOTTOM, RIGHT].into_iter().find(|i| self.para.sides.get(*i).copied().unwrap_or(false));
                let border = match first {
                    Some(i) => self.para.border(i),
                    None => Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 },
                };
                out.push(("format.set", json!({"props": {"border": border}})));
            } else {
                out.push(("para.borders", json!({"sides": self.para.sides_param()})));
            }
        }
        if self.page != b.page || self.whole_doc != b.whole_doc {
            let apply = if self.whole_doc { "document" } else { "section" };
            out.push(("design.pageBorders", json!({"sides": self.page.sides_param(), "applyTo": apply})));
        }
        if self.fill != b.fill || self.fill_to_text != b.fill_to_text {
            let color = Rgb::parse(&self.fill).map(|c| json!(c.hex())).unwrap_or(Value::Null);
            out.push((if self.fill_to_text { "format.shading" } else { "para.shading" }, json!({"color": color})));
        }
        out
    }

    /// Run [`BordersForm::commands`] as one undo step.
    pub fn apply(&self, app: &mut WordApp) {
        for (i, (id, params)) in self.commands().into_iter().enumerate() {
            if i > 0 {
                app.session.join_next_undo();
            }
            let _ = app.run(id, params);
        }
    }
}

/// The Borders and Shading dialog body. Returns true to close.
pub fn borders(app: &mut WordApp, ui: &mut Ui, f: &mut BordersForm) -> bool {
    ui.horizontal(|ui| {
        for (i, l) in ["Borders", "Page Border", "Shading"].into_iter().enumerate() {
            if ui.selectable_label(f.tab == i as u8, tl!(l)).clicked() {
                f.tab = i as u8;
            }
        }
    });
    ui.separator();
    let theme = app.session.doc.settings.theme_colors.clone();
    match f.tab {
        0 => {
            let to_text = f.to_text;
            lines(ui, "para", &mut f.para, &theme, Rgb::parse(&f.fill), Target::from(to_text));
            ui.horizontal(|ui| {
                ui.label(tl!("Apply to:"));
                ui.radio_value(&mut f.to_text, false, tl!("Paragraph"));
                ui.radio_value(&mut f.to_text, true, tl!("Text"));
            });
        }
        1 => {
            lines(ui, "page", &mut f.page, &theme, None, Target::Page);
            ui.horizontal(|ui| {
                ui.label(tl!("Apply to:"));
                ui.radio_value(&mut f.whole_doc, true, tl!("Whole document"));
                ui.radio_value(&mut f.whole_doc, false, tl!("This section"));
            });
        }
        _ => {
            ui.label(egui::RichText::new(tl!("Fill")).font(semibold(12.5)));
            color_menu(ui, &theme, &mut f.fill);
            ui.add_space(6.0);
            let fill = Rgb::parse(&f.fill);
            preview(ui, &LineForm::read(None), fill, Target::from(f.fill_to_text));
            ui.horizontal(|ui| {
                ui.label(tl!("Apply to:"));
                ui.radio_value(&mut f.fill_to_text, false, tl!("Paragraph"));
                ui.radio_value(&mut f.fill_to_text, true, tl!("Text"));
            });
        }
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        f.apply(app);
    }
    ok || cancel
}

/// What the borders in a preview go around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Paragraph,
    Text,
    Page,
}

impl From<bool> for Target {
    /// `true`: the selected text; `false`: the paragraphs.
    fn from(text: bool) -> Target {
        if text { Target::Text } else { Target::Paragraph }
    }
}

/// Setting, style, colour and width on the left; the preview with its side toggles on the right.
fn lines(ui: &mut Ui, id: &str, l: &mut LineForm, theme: &[Rgb], fill: Option<Rgb>, target: Target) {
    let between = target == Target::Paragraph;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Setting:")).font(semibold(12.5)));
            for (s, label) in
                [(Setting::None, "None"), (Setting::Box, "Box"), (Setting::Shadow, "Shadow"), (Setting::ThreeD, "3-D"), (Setting::Custom, "Custom")]
            {
                if ui.radio(l.setting == s, tl!(label)).clicked() {
                    l.apply_setting(s);
                }
            }
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(tl!("Style:"));
            let current = STYLES.iter().find(|(s, _)| *s == l.style).map(|(_, n)| *n).unwrap_or("Single");
            egui::ComboBox::from_id_salt((id, "style")).selected_text(tl!(current)).width(120.0).show_ui(ui, |ui| {
                for (s, n) in STYLES {
                    ui.selectable_value(&mut l.style, s, tl!(n));
                }
            });
            ui.label(tl!("Color:"));
            color_menu(ui, theme, &mut l.color);
            ui.label(tl!("Width:"));
            egui::ComboBox::from_id_salt((id, "width"))
                .selected_text(crate::i18n::fmt(tl!("{n} pt"), &[("n", &l.width.to_string())]))
                .width(120.0)
                .show_ui(ui, |ui| {
                    for w in WIDTHS {
                        ui.selectable_value(&mut l.width, w, crate::i18n::fmt(tl!("{n} pt"), &[("n", &w.to_string())]));
                    }
                });
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Preview")).font(semibold(12.5)));
            if let Some(side) = preview(ui, l, fill, target) {
                l.toggle(side);
            }
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(180.0);
                let mut sides = vec![(TOP, "Top"), (BOTTOM, "Bottom"), (LEFT, "Left"), (RIGHT, "Right")];
                if between {
                    sides.push((BETWEEN, "Between"));
                }
                for (i, n) in sides {
                    if ui.selectable_label(l.sides.get(i).copied().unwrap_or(false), tl!(n)).clicked() {
                        l.toggle(i);
                    }
                }
            });
        });
    });
}

/// A sample page with lines of text, the chosen borders and fill. A click near an edge returns
/// that side.
fn preview(ui: &mut Ui, l: &LineForm, fill: Option<Rgb>, target: Target) -> Option<usize> {
    let between = target == Target::Paragraph;
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(180.0, 120.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter_at(r);
    p.rect(r, 2.0, t.input, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    // Two paragraphs of grey lines; the borders go around the first (or its text).
    let bar = t.text_dim.gamma_multiply(0.45);
    let para = Rect::from_min_max(pos2(r.min.x + 22.0, r.min.y + 22.0), pos2(r.max.x - 22.0, r.min.y + 66.0));
    let line_rows = |y0: f32, n: usize, last: f32| {
        for i in 0..n {
            let y = y0 + i as f32 * 9.0;
            let w = if i + 1 == n { last } else { 1.0 };
            p.rect_filled(Rect::from_min_size(pos2(para.min.x + 4.0, y), vec2((para.width() - 8.0) * w, 4.0)), 1.0, bar);
        }
    };
    let area = match target {
        Target::Text => Rect::from_min_max(para.min, pos2(para.min.x + para.width() * 0.62, para.min.y + 14.0)),
        Target::Paragraph => para,
        Target::Page => r.shrink(10.0),
    };
    if let Some(c) = fill {
        p.rect_filled(area, 0.0, crate::theme::c32(c));
    }
    line_rows(para.min.y + 6.0, 4, 0.6);
    line_rows(para.max.y + 10.0, 3, 0.4);
    let color = Rgb::parse(&l.color).map(crate::theme::c32).unwrap_or(t.text);
    let edges = [
        (TOP, [area.left_top(), area.right_top()]),
        (BOTTOM, [area.left_bottom(), area.right_bottom()]),
        (LEFT, [area.left_top(), area.left_bottom()]),
        (RIGHT, [area.right_top(), area.right_bottom()]),
        (BETWEEN, [pos2(area.min.x, area.center().y), pos2(area.max.x, area.center().y)]),
    ];
    for (side, seg) in edges {
        if l.sides.get(side).copied().unwrap_or(false) && (side != BETWEEN || between) {
            draw_line(&p, seg, l.style, l.side_width(side), color);
        }
    }
    if !resp.clicked() {
        return None;
    }
    let at = resp.interact_pointer_pos()?;
    let d = |side: usize| -> f32 { edges.iter().find(|(s, _)| *s == side).map(|(_, [a, b])| dist_to_segment(at, *a, *b)).unwrap_or(f32::MAX) };
    let mut sides = vec![TOP, BOTTOM, LEFT, RIGHT];
    if between {
        sides.push(BETWEEN);
    }
    sides.into_iter().map(|s| (s, d(s))).filter(|(_, dist)| *dist < 14.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(s, _)| s)
}

fn dist_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let k = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * k)).length()
}

/// One border line in the preview, roughly as it prints.
fn draw_line(p: &egui::Painter, [a, b]: [egui::Pos2; 2], style: BorderStyle, width: f32, color: egui::Color32) {
    let px = (width * 1.2).clamp(1.0, 6.0);
    let normal = (b - a).normalized().rot90();
    let stroke = Stroke::new(px, color);
    match style {
        BorderStyle::None => {}
        BorderStyle::Double | BorderStyle::Triple => {
            let n = if style == BorderStyle::Double { 2 } else { 3 };
            let thin = Stroke::new(1.0, color);
            for i in 0..n {
                let off = normal * ((i as f32 - (n as f32 - 1.0) / 2.0) * 3.0);
                p.line_segment([a + off, b + off], thin);
            }
        }
        BorderStyle::Dotted => {
            p.extend(egui::Shape::dashed_line(&[a, b], stroke, px, px * 1.5));
        }
        BorderStyle::Dashed => {
            p.extend(egui::Shape::dashed_line(&[a, b], stroke, px * 4.0, px * 2.0));
        }
        BorderStyle::DotDash => {
            p.extend(egui::Shape::dashed_line_with_offset(&[a, b], stroke, &[px * 4.0, px], &[px * 2.0, px * 2.0], 0.0));
        }
        BorderStyle::Wave => {
            let len = (b - a).length();
            let dir = (b - a).normalized();
            let n = (len / 4.0).ceil().max(1.0) as usize;
            let pts: Vec<egui::Pos2> =
                (0..=n.min(400)).map(|i| a + dir * (len * i as f32 / n as f32) + normal * if i % 2 == 0 { -1.5 } else { 1.5 }).collect();
            p.add(egui::Shape::line(pts, Stroke::new(1.0, color)));
        }
        BorderStyle::Single | BorderStyle::Thick => {
            p.line_segment([a, b], if style == BorderStyle::Thick { Stroke::new((px * 1.5).min(6.0), color) } else { stroke });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::Dialog;
    use wordcraft_engine::Session;

    fn app_with(text: &str) -> WordApp {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let _ = app.run("document.setText", json!({"text": text}));
        app
    }

    /// #320: OK in Borders and Shading applies a box border with the chosen style, colour and
    /// width, and the page border and fill with it — one undo step for all three.
    #[test]
    fn borders_dialog_ok_applies_box_in_one_undo_step() {
        let mut app = app_with("Boxed paragraph");
        let _ = app.run("para.borders", json!({}));
        let Some(Dialog::Borders { form }) = app.dialog.take() else { panic!("Borders and Shading opens from the menu item") };
        let mut f = *form;
        assert_eq!(f.para.setting, Setting::None);
        f.para.apply_setting(Setting::Box);
        f.para.style = BorderStyle::Double;
        f.para.width = 1.5;
        f.para.color = "1F4E79".into();
        f.page.apply_setting(Setting::Shadow);
        f.fill = "DDEEFF".into();
        let undo_before = app.session.undo_labels().len();
        f.apply(&mut app);
        let props = app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        let b = props.borders.unwrap_or_default();
        for side in [b.top, b.bottom, b.left, b.right] {
            let side = side.unwrap_or_default();
            assert_eq!((side.style, side.width, side.color), (BorderStyle::Double, 1.5, Rgb::parse("1F4E79")));
        }
        assert!(b.between.is_none());
        assert_eq!(props.shading, Rgb::parse("DDEEFF"));
        let pb = app.session.doc.last_section.page_borders.unwrap_or_default();
        assert_eq!(pb.right.map(|x| x.width), Some(1.0), "Shadow doubles the right and bottom lines");
        assert_eq!(pb.top.map(|x| x.width), Some(0.5));
        assert_eq!(app.session.undo_labels().len(), undo_before + 1, "one undo step");
        let _ = app.run("edit.undo", json!({}));
        let props = app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        assert!(props.borders.is_none() && props.shading.is_none());
        assert!(app.session.doc.last_section.page_borders.is_none());
        // Reading the result back gives the same setting.
        assert_eq!(LineForm::read(Some(&pb)).setting, Setting::Shadow);
        // Unchanged, OK runs nothing.
        let f = BordersForm::read(&app, 0).unwrap_or_else(|| panic!("form"));
        assert!(f.commands().is_empty());
    }

    /// #320: the Tabs dialog opens from the Paragraph dialog's command path, and the table's
    /// edits (New, in-place cells, ×, Clear All) become one `para.tabs` call on OK.
    #[test]
    fn tabs_table_builds_and_clears() {
        let mut app = app_with("a\tb");
        let _ = app.run("para.tabs", json!({}));
        let Some(Dialog::Tabs { form }) = app.dialog.take() else { panic!("Tabs opens without params") };
        let mut f = *form;
        assert!(f.stops.is_empty());
        // New, New: rows at one and two default intervals, each selected as it lands.
        f.new_stop();
        f.new_stop();
        assert_eq!((f.stops[0].pos, f.stops[1].pos, f.selected), (36.0, 72.0, Some(1)));
        f.set_align(1, TabAlign::Right);
        f.set_leader(1, TabLeader::Dot);
        f.remove_stop(0);
        let params = f.params().unwrap_or_default();
        let _ = app.run("para.tabs", params);
        let props = app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        assert_eq!(props.tabs, Some(vec![TabStop { pos: 72.0, align: TabAlign::Right, leader: TabLeader::Dot }]));
        // Unchanged, OK runs nothing; Clear All clears.
        let mut f = TabsForm::read(&app).unwrap_or_else(|| panic!("form"));
        assert!(f.params().is_none());
        f.clear_all();
        assert_eq!(f.params(), Some(json!({"clearAll": true})));
    }

    /// The list is an inline-edit table: changing a row's position or alignment stages the
    /// change (clear + set), rows resort once editing ends, and collisions merge.
    #[test]
    fn tabs_list_edits_stage_moves_and_alignment() {
        let mut app = app_with("a\tb");
        let _ = app.run("para.tabs", json!({"set": [{"pos": 72.0, "align": "right", "leader": "dot"}, {"pos": 144.0}]}));
        let mut f = TabsForm::read(&app).unwrap_or_else(|| panic!("form"));
        f.set_pos(0, 108.0);
        f.normalize();
        assert_eq!(f.params(), Some(json!({"set": [{"pos": 108.0, "align": "right", "leader": "dot"}], "clear": [72.0]})));
        assert_eq!(f.selected, Some(0), "the selection follows its stop");

        // Moving a row onto another merges them (the edited row wins), and junk input
        // changes nothing but clamping.
        f.set_pos(0, 144.0);
        f.normalize();
        assert_eq!(f.stops.len(), 1);
        assert_eq!(f.params(), Some(json!({"set": [{"pos": 144.0, "align": "right", "leader": "dot"}], "clear": [72.0]})));
        f.set_pos(0, f32::NAN);
        f.set_pos(7, 72.0);
        f.set_pos(0, 99999.0);
        f.normalize();
        assert_eq!(f.stops[0].pos, 1584.0, "positions clamp to the engine bound");

        // Alignment and leader picked on a row stage a set for just that stop.
        let mut f = TabsForm::read(&app).unwrap_or_else(|| panic!("form"));
        f.set_align(1, TabAlign::Decimal);
        f.set_leader(1, TabLeader::Underscore);
        assert_eq!(f.params(), Some(json!({"set": [{"pos": 144.0, "align": "decimal", "leader": "underscore"}]})));
        f.set_align(9, TabAlign::Bar);
        f.set_leader(9, TabLeader::Dot);

        // What the position field parses: the interface unit by default, any unit spelled out.
        assert_eq!(wordcraft_geom::parse_measure("1.5", wordcraft_geom::Unit::Inches), Some(108.0));
        assert_eq!(wordcraft_geom::parse_measure("2,54 cm", wordcraft_geom::Unit::Inches), Some(72.0));
    }

    /// New adds a row at the next free multiple of the default interval — skipping taken and
    /// cleared positions — and × / Delete clear rows, keeping the selection sensible.
    #[test]
    fn tabs_new_adds_and_remove_clears() {
        let mut app = app_with("a\tb");
        let _ = app.run("para.tabs", json!({"set": [{"pos": 72.0, "align": "right"}, {"pos": 144.0}]}));
        let mut f = TabsForm::read(&app).unwrap_or_else(|| panic!("form"));
        assert_eq!(f.default_tab * unit().pt_per_unit(), 36.0, "the document default interval");
        f.new_stop();
        assert_eq!((f.stops.len(), f.selected), (3, Some(2)));
        assert_eq!((f.stops[2].pos, f.stops[2].align, f.stops[2].leader), (180.0, TabAlign::Left, TabLeader::None));
        assert_eq!(f.params(), Some(json!({"set": [{"pos": 180.0, "align": "left", "leader": "none"}]})));

        // Cleared and taken positions are skipped.
        let mut f = TabsForm::read(&app).unwrap_or_else(|| panic!("form"));
        f.remove_stop(1);
        f.cleared.push(108.0);
        f.new_stop();
        assert_eq!(f.stops[1].pos, 180.0, "72 holds a stop; 108 and 144 are cleared; 180 is free");

        // × / Delete: clearing a row keeps the selection sensible.
        let mut f = TabsForm::read(&app).unwrap_or_else(|| panic!("form"));
        f.selected = Some(1);
        f.remove_stop(0);
        assert_eq!((f.stops.len(), f.selected, f.cleared.clone()), (1, Some(0), vec![72.0]));
        assert_eq!(f.params(), Some(json!({"clear": [72.0]})));
        f.remove_stop(0);
        assert_eq!((f.stops.len(), f.selected), (0, None));
        f.remove_stop(7);
        assert_eq!(f.cleared, vec![72.0, 144.0], "an index past the end changes nothing");

        // An empty paragraph starts at one interval.
        let app = app_with("plain");
        let mut f = TabsForm::read(&app).unwrap_or_else(|| panic!("form"));
        f.new_stop();
        assert_eq!(f.stops[0].pos, 36.0);
    }

    /// The inline tab-stop rows are named by their column, and the default interval by its label.
    #[test]
    fn tab_stop_fields_have_semantic_names() {
        use egui_kittest::kittest::Queryable;
        let mut app = app_with("a\tb");
        app.run("para.tabs", json!({"set": [{"pos": 72.0}]})).unwrap();
        app.dialog = crate::dialogs::Dialog::open("tabs", &mut app);
        assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::Tabs { .. })));
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Tab stop position: 1");
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Alignment: 1");
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Leader 1");
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Default tab stops:");
    }

    /// #3031: a paragraph with no stops — the default — says so in the list box instead of
    /// showing a blank frame.
    #[test]
    fn the_tabs_dialog_says_when_there_are_no_stops() {
        let mut app = app_with("plain");
        let _ = app.run("para.tabs", json!({}));
        assert!(matches!(&app.dialog, Some(Dialog::Tabs { form }) if form.stops.is_empty()));
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        let texts: Vec<String> = h
            .output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("(none)")), "empty-state hint shown");
    }
}
