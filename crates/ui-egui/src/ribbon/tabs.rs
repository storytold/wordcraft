use super::{Stroke, Tokens, Ui, WordApp, json, semibold};

pub const TABS: [&str; 13] = wordcraft_engine::ribbon::TABS;

/// True when the caret/selection touches a picture (#147).
pub fn has_picture_selected(s: &wordcraft_engine::Session) -> bool {
    matches!(wordcraft_engine::cmd::objects::selected(s), Some((_, wordcraft_doc::para::InlineObject::Image { .. })))
}

/// Whether the selection is a shape, text box or Drawing Canvas (so Shape Format is shown).
pub fn has_shape_selected(s: &wordcraft_engine::Session) -> bool {
    match wordcraft_engine::cmd::objects::selected(s) {
        Some((_, o)) => matches!(o, wordcraft_doc::para::InlineObject::Shape { .. }) || o.is_canvas(),
        None => false,
    }
}

/// Reading controls replace editing tabs entirely. All settings are transient view controls.
pub(super) fn reading_toolbar(app: &mut WordApp, ui: &mut Ui, t: Tokens) {
    egui::Panel::top("reading_toolbar")
        .exact_size(46.0)
        .frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin::symmetric(12, 6)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(tl!("Reading Mode")).font(semibold(14.0)));
                ui.separator();
                let w = app.session.view.read_column_width;
                let active = match w {
                    None => "Auto",
                    Some(v) if v <= 180.0 => "Narrow",
                    Some(v) if v <= 240.0 => "Medium",
                    Some(v) if v <= 350.0 => "Wide",
                    _ => "Page",
                };
                ui.menu_button(format!("{}: {}", tl!("Column width"), tl!(active)), |ui| {
                    for (name, value) in [("Auto", "auto"), ("Narrow", "narrow"), ("Medium", "medium"), ("Wide", "wide"), ("Page", "page")] {
                        if ui.selectable_label(active == name, tl!(name)).clicked() {
                            let _ = app.run("view.readColumnWidth", json!({"value": value}));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                let pct = (app.session.view.read_zoom * 100.0).round() as u32;
                ui.menu_button(format!("{}: {pct}%", tl!("Text size")), |ui| {
                    for percent in [75, 90, 100, 110, 125, 150] {
                        if ui.selectable_label(pct == percent, format!("{percent}%")).clicked() {
                            let _ = app.run("view.readTextSize", json!({"value": percent}));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                let spacing = app.session.view.read_line_spacing;
                let label = spacing.map_or_else(|| tl!("Document").to_owned(), |v| format!("{v}×"));
                ui.menu_button(format!("{}: {}", tl!("Line spacing"), label), |ui| {
                    if ui.selectable_label(spacing.is_none(), tl!("Document")).clicked() {
                        let _ = app.run("view.readLineSpacing", json!({"value": "document"}));
                        ui.close();
                    }
                    for value in [1.0, 1.25, 1.5, 2.0] {
                        if ui.selectable_label(spacing == Some(value), format!("{value}×")).clicked() {
                            let _ = app.run("view.readLineSpacing", json!({"value": value}));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                use wordcraft_engine::cmd::view::ReadPageColor;
                let colors =
                    [(ReadPageColor::None, "None", "none"), (ReadPageColor::Sepia, "Sepia", "sepia"), (ReadPageColor::Inverse, "Inverse", "inverse")];
                let current = app.session.view.read_page_color;
                let name = colors.iter().find(|c| c.0 == current).map_or("None", |c| c.1);
                ui.menu_button(format!("{}: {}", tl!("Page color"), tl!(name)), |ui| {
                    for (color, name, value) in colors {
                        if ui.selectable_label(current == color, tl!(name)).clicked() {
                            let _ = app.run("view.readPageColor", json!({"value": value}));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                // Headings to jump to, in the Navigation pane beside the columns (Outline view
                // is an editing layout, so it isn't offered here).
                if ui.button(tl!("Navigation Pane")).clicked() {
                    let _ = app.run("view.navigationPane", json!({"value": true}));
                    app.ui.nav_tab = "headings".into();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tl!("Edit Document")).clicked() {
                        let _ = app.run("view.printLayout", json!({}));
                    }
                });
            });
        });
}

/// Whether the selection is a SmartArt graphic WordCraft can edit (so SmartArt Design is shown).
pub fn has_smart_art_selected(s: &wordcraft_engine::Session) -> bool {
    wordcraft_engine::cmd::smart_art::selected_smart_art(s).is_some()
}

/// Whether the selection is a chart WordCraft can edit (so Chart Design is shown).
pub fn has_chart_selected(s: &wordcraft_engine::Session) -> bool {
    wordcraft_engine::cmd::chart::selected_chart(s).is_some()
}

/// Contextual tabs for the current selection (pure, tested).
pub fn contextual_tabs(s: &wordcraft_engine::Session) -> Vec<&'static str> {
    let mut tabs = Vec::new();
    if s.sel.focus.path.cell().is_some() {
        tabs.push("Table Design");
        tabs.push("Table Layout");
    }
    if has_picture_selected(s) {
        tabs.push("Picture Format");
    }
    if has_shape_selected(s) {
        tabs.push("Shape Format");
    }
    if has_smart_art_selected(s) {
        tabs.push("SmartArt Design");
    }
    if has_chart_selected(s) {
        tabs.push("Chart Design");
    }
    // Editing an equation.
    if s.math.is_some() {
        tabs.push("Equation");
    }
    // Outline view (drawn first, next to File, as Word does).
    if s.view.mode == wordcraft_layout::ViewMode::Outline {
        tabs.push("Outlining");
    }
    tabs
}

/// Tab to show when the stored tab is no longer applicable (pure, tested).
pub fn resolve_tab<'a>(current: &'a str, available: &[&str]) -> &'a str {
    if available.contains(&current) { current } else { crate::DEFAULT_TAB }
}

/// Width of the » button that holds the tabs the strip has no room for.
pub(super) const MORE_TABS_W: f32 = 24.0;

/// Which tabs fit on a strip `avail` points wide (#897), from each tab's width and whether it must
/// stay. The others give way from the end, leaving `more` points for the » menu that holds them
/// (pure, tested).
pub fn fit_tabs(tabs: &[(f32, bool)], avail: f32, more: f32) -> Vec<bool> {
    let mut fits = vec![true; tabs.len()];
    let mut total: f32 = tabs.iter().map(|(w, _)| w).sum();
    if total <= avail {
        return fits;
    }
    for (fit, (w, keep)) in fits.iter_mut().zip(tabs).rev() {
        if total + more <= avail {
            break;
        }
        if !keep {
            *fit = false;
            total -= w;
        }
    }
    fits
}

/// One tab as the strip planner sees it (#2026).
#[derive(Debug, Clone, Copy)]
pub struct TabSpec {
    /// Width of the label at full size.
    pub text_w: f32,
    /// File: always on the strip.
    pub file: bool,
    /// The tab being shown.
    pub active: bool,
    /// A contextual tab (Table Design, Picture Format…).
    pub contextual: bool,
}

/// How the tab strip is laid out (#2026).
#[derive(Debug, Clone, PartialEq)]
pub struct StripPlan {
    /// Padding each side of a tab label.
    pub pad: f32,
    /// How much of Comments / Editing / Share is spelled out: 0 all, 1 Share only, 2 none.
    pub buttons: usize,
    /// Whether each tab is on the strip (the others are in the » menu).
    pub fits: Vec<bool>,
    /// Width each tab takes on the strip (a label may be cut with …).
    pub widths: Vec<f32>,
}

/// Smallest a tab label is cut down to before the tab goes into the » menu.
pub(super) const MIN_TAB_W: f32 = 44.0;

/// Lays the tabs and the buttons on their right out on `avail` points, giving way in this order:
/// the padding between tabs, the Comments / Editing labels, the Share label, the plain tabs (from
/// the end, into »), the labels of the active and contextual tabs (cut with …), and last those tabs
/// themselves. The buttons' room is always kept and File never leaves the strip. `buttons` is the
/// width of the three buttons at each level (pure, tested).
pub fn plan_strip(tabs: &[TabSpec], buttons: [f32; 3], avail: f32, more: f32) -> StripPlan {
    const STEPS: [(f32, usize); 4] = [(18.0, 0), (10.0, 0), (10.0, 1), (10.0, 2)];
    let widths_at = |pad: f32| -> Vec<f32> { tabs.iter().map(|t| t.text_w.max(0.0) + pad + 2.0).collect() };
    for (pad, level) in STEPS {
        let widths = widths_at(pad);
        if widths.iter().sum::<f32>() <= avail - buttons[level] {
            return StripPlan { pad, buttons: level, fits: vec![true; tabs.len()], widths };
        }
    }
    let (pad, level) = (10.0, 2);
    let mut widths = widths_at(pad);
    let room = avail - buttons[level];
    let specs: Vec<(f32, bool)> = widths.iter().zip(tabs).map(|(w, t)| (*w, t.file || t.active || t.contextual)).collect();
    let mut fits = fit_tabs(&specs, room, more);
    // The tabs that must stay share what the plain ones left; each is cut down to its share.
    let kept = |fits: &[bool], i: usize| fits.get(i).copied().unwrap_or(false) && tabs.get(i).is_some_and(|t| !t.file);
    loop {
        let n = (0..tabs.len()).filter(|&i| kept(&fits, i)).count();
        let others: f32 =
            (0..tabs.len()).filter(|&i| fits.get(i) == Some(&true) && !kept(&fits, i)).map(|i| widths.get(i).copied().unwrap_or(0.0)).sum();
        let own: f32 = (0..tabs.len()).filter(|&i| kept(&fits, i)).map(|i| widths.get(i).copied().unwrap_or(0.0)).sum();
        let extra = if fits.iter().all(|f| *f) { 0.0 } else { more };
        let share_room = room - extra - others;
        if n == 0 || own <= share_room {
            break;
        }
        if share_room >= n as f32 * MIN_TAB_W {
            let share = share_room / n as f32;
            for (i, w) in widths.iter_mut().enumerate() {
                if kept(&fits, i) {
                    *w = w.min(share);
                }
            }
            break;
        }
        // Not even cut down: the last of them (the active tab last of all) goes into ».
        let last = (0..tabs.len())
            .rev()
            .find(|&i| kept(&fits, i) && tabs.get(i).is_some_and(|t| !t.active))
            .or_else(|| (0..tabs.len()).rev().find(|&i| kept(&fits, i)));
        match last.and_then(|i| fits.get_mut(i)) {
            Some(f) => *f = false,
            None => break,
        }
    }
    StripPlan { pad, buttons: level, fits, widths }
}

/// Whether the caret is inside a table (so the contextual tabs and their badges are shown).
pub fn in_table_public(app: &WordApp) -> bool {
    app.session.sel.focus.path.cell().is_some()
}
