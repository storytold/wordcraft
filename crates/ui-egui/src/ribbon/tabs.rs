use super::{Stroke, Tokens, Ui, WordApp, json, semibold};
use egui::{Rect, pos2, vec2};

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

/// How long a contextual tab takes to settle into place after it appears, seconds.
pub(super) const TAB_ENTRY_SECS: f64 = 0.35;
/// How far, in points, a new contextual tab starts to the left of its place.
pub(super) const TAB_ENTRY_SLIDE: f32 = 18.0;

/// Fast at first, then slowing to rest: 0 → 1.
pub(super) fn ease_out_cubic(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// Notes the contextual tabs shown this frame (each with its place in the strip), starts the
/// entry of those that were not shown the frame before, and returns the tabs that just left,
/// each with its old place and the time it left, while they still fade away (none with
/// animations off). The first frame starts nothing: tabs already there do not slide.
pub(super) fn note_contextual_tabs(ctx: &egui::Context, shown: &[(&str, usize)], now: f64, animate: bool) -> Vec<(String, usize, f64)> {
    let (shown_id, leaving_id) = (egui::Id::unique("contextual_tabs_shown"), egui::Id::unique("contextual_tabs_leaving"));
    let prev: Option<Vec<(String, usize)>> = ctx.data(|d| d.get_temp(shown_id));
    let mut leaving: Vec<(String, usize, f64)> = ctx.data(|d| d.get_temp(leaving_id)).unwrap_or_default();
    leaving.retain(|(n, _, start)| animate && now - start < TAB_ENTRY_SECS && !shown.iter().any(|(s, _)| s == n));
    // Positions in the stored shown list must count retained ghosts too, or tabs leaving on
    // different frames are reinserted using indices from different layouts.
    let shown_with_ghosts: Vec<(&str, usize)> = shown
        .iter()
        .map(|(tab, at)| {
            let before = leaving.iter().filter(|(_, ghost_at, _)| ghost_at <= at).count();
            (*tab, at + before)
        })
        .collect();
    if let Some(prev) = prev {
        for (tab, _) in shown_with_ghosts.iter().filter(|(t, _)| !prev.iter().any(|(p, _)| p == t)) {
            ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("contextual_tab_entry", *tab)), now));
        }
        for (tab, at) in prev.iter().filter(|(p, _)| animate && !shown_with_ghosts.iter().any(|(s, _)| s == p)) {
            if !leaving.iter().any(|(n, _, _)| n == tab) {
                leaving.push((tab.clone(), *at, now));
            }
        }
    }
    ctx.data_mut(|d| {
        d.insert_temp(shown_id, shown_with_ghosts.iter().map(|(t, at)| (t.to_string(), *at)).collect::<Vec<_>>());
        d.insert_temp(leaving_id, leaving.clone());
    });
    leaving
}

/// How far a contextual tab's entry has come: 0 as it appears, 1 once settled (always 1 with
/// animations off). Asks for repaints while it moves.
pub(super) fn tab_entry(ctx: &egui::Context, tab: &str, now: f64, animate: bool) -> f32 {
    let start: Option<f64> = ctx.data(|d| d.get_temp(egui::Id::unique(("contextual_tab_entry", tab))));
    let Some(start) = start.filter(|_| animate) else { return 1.0 };
    let p = ((now - start) / TAB_ENTRY_SECS).clamp(0.0, 1.0) as f32;
    if p < 1.0 {
        ctx.request_repaint();
    }
    ease_out_cubic(p)
}

/// How long the underline takes to slide to a newly chosen tab, seconds.
pub(super) const UNDERLINE_SLIDE_SECS: f64 = 0.2;
/// How long the ribbon's content takes to fade and slide in after a tab change, seconds.
pub(super) const CONTENT_SWITCH_SECS: f64 = 0.16;
/// How far, in points, the new content starts to the side of its place.
pub(super) const CONTENT_SWITCH_SLIDE: f32 = 14.0;

/// The tab underline: its centre and width (`from`, `to`) and when it set off.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Underline {
    pub(super) tab: String,
    pub(super) from: (f32, f32),
    pub(super) to: (f32, f32),
    pub(super) start: f64,
}

/// The underline once the active tab is at `target` (centre, width): a new tab starts a slide from
/// where the underline is now, the same tab only follows its tab (a resized window).
pub(super) fn next_underline(prev: Option<&Underline>, tab: &str, target: (f32, f32), now: f64, animate: bool) -> Underline {
    let snap = Underline { tab: tab.to_string(), from: target, to: target, start: now };
    match prev {
        Some(p) if animate && p.tab != tab => Underline { from: underline_at(p, now), ..snap },
        Some(p) if animate && now - p.start < UNDERLINE_SLIDE_SECS => Underline { to: target, ..p.clone() },
        _ => snap,
    }
}

/// Where the underline is `now`: sliding from its old place to its new one, fast at first.
pub(super) fn underline_at(u: &Underline, now: f64) -> (f32, f32) {
    let e = ease_out_cubic(((now - u.start) / UNDERLINE_SLIDE_SECS) as f32);
    (egui::lerp(u.from.0..=u.to.0, e), egui::lerp(u.from.1..=u.to.1, e))
}

/// Draws the underline under the active tab, sliding it from the tab chosen before.
pub(super) fn slide_underline(ui: &Ui, tab: &str, target: Rect, color: egui::Color32, now: f64, animate: bool) {
    let id = egui::Id::unique("tab_underline");
    let prev: Option<Underline> = ui.ctx().data(|d| d.get_temp(id));
    let next = next_underline(prev.as_ref(), tab, (target.center().x, target.width()), now, animate);
    if next.from != next.to && now - next.start < UNDERLINE_SLIDE_SECS {
        ui.ctx().request_repaint();
    }
    let (x, w) = underline_at(&next, now);
    ui.ctx().data_mut(|d| d.insert_temp(id, next));
    ui.painter().rect_filled(Rect::from_center_size(pos2(x, target.center().y), vec2(w, target.height())), 2.0, color);
}

/// The last tab change: when it happened and which way it went (+1 towards the end of the strip,
/// -1 towards the start, 0 for none).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TabChange {
    pub(super) tab: String,
    pub(super) index: usize,
    pub(super) start: f64,
    pub(super) dir: f32,
}

pub(super) fn next_tab_change(prev: Option<&TabChange>, tab: &str, index: usize, now: f64, animate: bool) -> TabChange {
    let same = TabChange { tab: tab.to_string(), index, start: prev.map_or(f64::MIN, |p| p.start), dir: prev.map_or(0.0, |p| p.dir) };
    match prev {
        Some(p) if p.tab != tab => {
            let dir = if !animate || index == p.index {
                0.0
            } else if index > p.index {
                1.0
            } else {
                -1.0
            };
            TabChange { start: now, dir, ..same }
        }
        Some(_) => same,
        None => TabChange { start: f64::MIN, dir: 0.0, ..same },
    }
}

/// How far the ribbon's content has come after a tab change, eased: 0 as the tab changes, 1 once
/// settled, with the side it comes from (+1 from the right, -1 from the left).
pub(super) fn tab_content_entry(ctx: &egui::Context, tab: &str, index: usize, now: f64, animate: bool) -> (f32, f32) {
    let id = egui::Id::unique("ribbon_tab_change");
    let prev: Option<TabChange> = ctx.data(|d| d.get_temp(id));
    let next = next_tab_change(prev.as_ref(), tab, index, now, animate);
    let p = ((now - next.start) / CONTENT_SWITCH_SECS).clamp(0.0, 1.0) as f32;
    let dir = next.dir;
    ctx.data_mut(|d| d.insert_temp(id, next));
    if p < 1.0 && dir != 0.0 {
        ctx.request_repaint();
        (ease_out_cubic(p), dir)
    } else {
        (1.0, 0.0)
    }
}

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
