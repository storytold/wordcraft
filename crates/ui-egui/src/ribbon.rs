//! The ribbon: tab strip and every tab's groups. Each control runs a command by id.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::colors::{Remember, Target};
use crate::options::MeasureExt;
use crate::theme::{Tokens, medium, regular, semibold};
use crate::widgets::{CONTENT_H, LABEL_H, big, big_toggle, group, menu_button, small, split};
use crate::{WordApp, icons};

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
fn reading_toolbar(app: &mut WordApp, ui: &mut Ui, t: Tokens) {
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
const MORE_TABS_W: f32 = 24.0;

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
const MIN_TAB_W: f32 = 44.0;

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

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.view.read_mode {
        reading_toolbar(app, ui, t);
        return;
    }
    // Editing an equation brings up the Equation tab; leaving it goes back.
    if app.session.math.is_some() {
        if app.equation_prev_tab.is_none() {
            app.equation_prev_tab = Some(app.ui.tab.clone());
            app.ui.tab = "Equation".into();
        }
    } else if let Some(prev) = app.equation_prev_tab.take()
        && app.ui.tab == "Equation"
    {
        app.ui.tab = prev;
    }
    // Outline view brings up the Outlining tab; closing it goes back.
    if app.session.view.mode == wordcraft_layout::ViewMode::Outline {
        if app.outline_prev_tab.is_none() {
            app.outline_prev_tab = Some(app.ui.tab.clone());
            app.ui.tab = "Outlining".into();
        }
    } else if let Some(prev) = app.outline_prev_tab.take()
        && app.ui.tab == "Outlining"
    {
        app.ui.tab = prev;
    }
    // The tabs shown: File, the user's tabs in their order (Customize Ribbon, #379), then the
    // contextual ones (the Outlining tab right after File). A contextual tab (Table, Picture
    // Format) may be stored while the selection moved away, and a tab may have been hidden.
    let user_tabs = app.session.ribbon.tabs().into_owned();
    let mut tabs: Vec<(String, bool)> = vec![("File".to_string(), false)];
    tabs.extend(user_tabs.iter().filter(|e| !e.hidden).map(|e| (e.name.clone(), e.custom)));
    for ct in contextual_tabs(&app.session) {
        if ct == "Outlining" {
            tabs.insert(1.min(tabs.len()), (ct.to_string(), false));
        } else if !tabs.iter().any(|(n, _)| n == ct) {
            tabs.push((ct.to_string(), false));
        }
    }
    {
        let names: Vec<&str> = tabs.iter().map(|(n, _)| n.as_str()).collect();
        let next = resolve_tab(&app.ui.tab, &names);
        let next = if names.contains(&next) { next } else { names.iter().copied().find(|n| *n != "File").unwrap_or(next) };
        if next != app.ui.tab {
            app.ui.tab = next.to_string();
        }
    }
    // Tab strip.
    egui::Panel::top("tabs")
        .exact_size(30.0)
        .frame(egui::Frame::NONE.fill(t.tab_strip).inner_margin(egui::Margin { left: 8, right: 10, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                // Share, Editing mode and Comments keep their room; the tabs get what is left of it
                // (#897, #2026), and the buttons give up their labels before a tab is drawn over them.
                let share = tl!("Share");
                let editing_label = if app.session.doc.settings.track_changes { tl!("Reviewing") } else { tl!("Editing") };
                let comments = tl!("Comments");
                let share_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(share.to_string(), medium(12.0), t.text).size().x);
                let spaced = |w: f32| w + 2.0;
                let button_widths = |level: usize| -> f32 {
                    let labels = level == 0;
                    let share_full = (share_w + 36.0).max(74.0);
                    let share_button = if level < 2 { share_full } else { 30.0 };
                    let editing = crate::widgets::icon_text_button_width(ui, editing_label, regular(12.0), true, labels);
                    let comment = crate::widgets::icon_text_button_width(ui, comments, regular(12.0), false, labels);
                    // The gaps (6 + 4 + 8) and the strip's margin.
                    spaced(share_button) + spaced(editing) + spaced(comment) + 18.0 + 12.0
                };
                let button_levels = [button_widths(0), button_widths(1), button_widths(2)];
                let labels: Vec<&str> = tabs.iter().map(|(tab, custom)| if *custom { tab.as_str() } else { tl!(tab) }).collect();
                let specs: Vec<TabSpec> = tabs
                    .iter()
                    .zip(&labels)
                    .map(|((tab, custom), shown)| TabSpec {
                        text_w: ui.ctx().fonts_mut(|f| f.layout_no_wrap(shown.to_string(), medium(12.5), t.text).size().x),
                        file: tab == "File",
                        active: *tab == app.ui.tab,
                        contextual: !custom && wordcraft_engine::ribbon::CONTEXTUAL_TABS.contains(&tab.as_str()),
                    })
                    .collect();
                let plan = plan_strip(&specs, button_levels, ui.available_width(), MORE_TABS_W + 2.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // No document open (#1129): nothing to share, edit or comment on — but only
                    // these buttons. Disabling the whole line took the tab strip with it: the
                    // tabs took clicks that did nothing, so the empty window had no way back to
                    // the Start screen (#3045).
                    ui.add_enabled_ui(app.has_document(), |ui| {
                        let (share_size, share_text) =
                            if plan.buttons < 2 { (((share_w + 36.0).max(74.0), 24.0), true) } else { ((30.0, 24.0), false) };
                        let (r, resp) = ui.allocate_exact_size(vec2(share_size.0, share_size.1), Sense::click());
                        ui.painter().rect_filled(r, 4.0, if resp.hovered() { t.accent_text } else { t.accent });
                        let icon_x = if share_text { r.min.x + 14.0 } else { r.center().x };
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(pos2(icon_x, r.center().y), vec2(14.0, 14.0)),
                            "share",
                            t.on_accent,
                            t.on_accent,
                        );
                        if share_text {
                            ui.painter().text(pos2(r.min.x + 26.0, r.center().y), Align2::LEFT_CENTER, share, medium(12.0), t.on_accent);
                        }
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, share));
                        if resp.on_hover_text(tl!("Export a copy to share (PDF, Word document)")).clicked() {
                            let _ = app.run("ui.backstage", json!({"value": true, "page": "export"}));
                        }
                        ui.add_space(6.0);
                        let track = app.session.doc.settings.track_changes;
                        let mode = if plan.buttons == 0 {
                            crate::widgets::icon_text_menu_button(ui, "pencil", editing_label, regular(12.0))
                        } else {
                            crate::widgets::icon_only_button(ui, "pencil", editing_label, regular(12.0), true)
                        };
                        egui::Popup::menu(&mode).show(|ui| {
                            if ui.selectable_label(!track, tl!(tl!("Editing — edit the document directly"))).clicked() {
                                let _ = app.run("review.trackChanges", json!({"value": false}));
                                ui.close();
                            }
                            if ui.selectable_label(track, tl!(tl!("Reviewing — edits become suggestions"))).clicked() {
                                let _ = app.run("review.trackChanges", json!({"value": true}));
                                ui.close();
                            }
                        });
                        ui.add_space(4.0);
                        let comments_button = if plan.buttons == 0 {
                            crate::widgets::icon_text_button(ui, "comment", comments, regular(12.0))
                        } else {
                            crate::widgets::icon_only_button(ui, "comment", comments, regular(12.0), false)
                        };
                        if comments_button.clicked() {
                            let _ = app.run("view.commentsPane", json!({}));
                        }
                    });
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        // The tabs that don't fit go into a » menu at the end of the strip (#897, #2026).
                        for (i, ((tab, custom), _)) in tabs.iter().zip(&plan.fits).enumerate().filter(|(_, (_, fits))| **fits) {
                            let (tab, custom) = (tab.as_str(), *custom);
                            let contextual = !custom && wordcraft_engine::ribbon::CONTEXTUAL_TABS.contains(&tab);
                            // A custom tab's name is the user's own text: shown as typed.
                            let shown = if custom { tab } else { tl!(tab) };
                            let w = plan.widths.get(i).copied().unwrap_or(0.0) - 2.0;
                            let cut = specs.get(i).is_some_and(|sp| sp.text_w + plan.pad > w + 0.5);
                            let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                            if app.ui.keytips == crate::keytips::Phase::Tabs {
                                crate::keytips::record_tab(r, tab, &app.ui.keytip_prefix, &mut app.keytip_rects);
                            } else if app.ui.keytips == crate::keytips::Phase::Legacy {
                                // Every tab: they only place the legacy keys' hint (#1697).
                                crate::keytips::record_tab(r, tab, "", &mut app.keytip_rects);
                            }
                            let active = app.ui.tab == tab && !app.ui.backstage;
                            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, shown));
                            if resp.hovered() && !active {
                                ui.painter().rect_filled(r.shrink2(vec2(0.0, 4.0)), 4.0, t.hover);
                            }
                            let color = if contextual || active { t.accent_text } else { t.text };
                            if cut {
                                let mut job = egui::text::LayoutJob::single_section(
                                    shown.to_string(),
                                    egui::text::TextFormat::simple(if active { semibold(12.5) } else { medium(12.5) }, color),
                                );
                                job.wrap = egui::text::TextWrapping {
                                    max_width: (w - plan.pad).max(1.0),
                                    max_rows: 1,
                                    break_anywhere: true,
                                    overflow_character: Some('…'),
                                };
                                let galley = ui.painter().layout_job(job);
                                ui.painter().galley(r.center() - galley.size() / 2.0, galley, color);
                            } else {
                                ui.painter().text(
                                    r.center(),
                                    Align2::CENTER_CENTER,
                                    shown,
                                    if active { semibold(12.5) } else { medium(12.5) },
                                    color,
                                );
                            }
                            let resp = if cut { resp.on_hover_text(shown) } else { resp };
                            if active {
                                let u = Rect::from_center_size(pos2(r.center().x, r.max.y - 2.0), vec2(w - 16.0, 3.0));
                                ui.painter().rect_filled(u, 2.0, t.accent);
                            }
                            if resp.clicked() {
                                let _ = app.run("ui.tab", json!({"tab": tab}));
                            }
                            if resp.double_clicked() && tab != "File" {
                                let _ = app.run("ui.collapseRibbon", json!({}));
                            }
                            resp.context_menu(|ui| crate::widgets::customize_menu_items(ui, app));
                        }
                        let left_out: Vec<(&str, &str)> = tabs
                            .iter()
                            .zip(&labels)
                            .zip(&plan.fits)
                            .filter(|(_, fits)| !**fits)
                            .map(|(((tab, _), shown), _)| (tab.as_str(), *shown))
                            .collect();
                        if !left_out.is_empty() {
                            let (r, resp) = ui.allocate_exact_size(vec2(MORE_TABS_W, 30.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(r.shrink2(vec2(0.0, 4.0)), 4.0, t.hover);
                            }
                            // The active tab is in the menu: the button says so.
                            let holds_active = left_out.iter().any(|(tab, _)| *tab == app.ui.tab);
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                "»",
                                medium(12.5),
                                if holds_active { t.accent_text } else { t.text },
                            );
                            // "»" means nothing read aloud: the button says what it holds.
                            let more = tl!("More tabs");
                            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, more));
                            let resp = resp.on_hover_text(more);
                            egui::Popup::menu(&resp).show(|ui| {
                                for (tab, shown) in left_out {
                                    if ui.selectable_label(app.ui.tab == tab, shown).clicked() {
                                        let _ = app.run("ui.tab", json!({"tab": tab}));
                                        ui.close();
                                    }
                                }
                            });
                        }
                    });
                });
            });
        });
    if app.ui.ribbon_collapsed {
        return;
    }
    egui::Panel::top("ribbon")
        .exact_size(CONTENT_H + LABEL_H + 10.0)
        .frame(
            egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 }).stroke(Stroke::new(1.0, t.border)),
        )
        .show(ui, |ui| {
            // Right-click on the ribbon (outside a button): Customize Ribbon. Registered first, so
            // the buttons drawn over it keep their own clicks.
            // No document open (#1129): every command is greyed out but Help's.
            if !app.has_document() && app.ui.tab != "Help" {
                ui.disable();
            }
            let bg = ui.interact(ui.max_rect(), ui.id().with("ribbon_background"), Sense::click());
            bg.context_menu(|ui| crate::widgets::customize_menu_items(ui, app));
            let entry = user_tabs.iter().find(|e| e.name == app.ui.tab);
            crate::ribbon_fit::begin(ui.ctx(), &app.ui.tab, ui.available_width() - 4.0);
            let scroll = egui::ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    match entry {
                        // Every tab hidden: nothing to show.
                        Some(e) if e.hidden => return,
                        Some(e) if e.custom => return custom_groups(app, ui, &e.groups),
                        _ => {}
                    }
                    match app.ui.tab.as_str() {
                        "Home" => home(app, ui),
                        "Insert" => insert(app, ui),
                        "Draw" => draw(app, ui),
                        "Design" => design(app, ui),
                        "Layout" => layout(app, ui),
                        "References" => references(app, ui),
                        "Mailings" => mailings(app, ui),
                        "Review" => review(app, ui),
                        "View" => view(app, ui),
                        "Developer" => developer(app, ui),
                        "Zotero" => zotero(app, ui),
                        "Help" => help(app, ui),
                        "Table Design" => table_design(app, ui),
                        "Table Layout" => table_layout(app, ui),
                        "Equation" if app.session.math.is_some() => crate::equation_tab::show(app, ui),
                        "Picture Format" => picture_format(app, ui),
                        "Shape Format" => shape_format(app, ui),
                        "SmartArt Design" => smart_art_design(app, ui),
                        "Outlining" => outlining(app, ui),
                        "Chart Design" => chart_design(app, ui),
                        _ => home(app, ui),
                    }
                    if let Some(e) = entry {
                        custom_groups(app, ui, &e.groups);
                    }
                });
            });
            crate::ribbon_fit::end(ui.ctx());
            // The bar is hidden, so a row wider than the window would be clipped with no sign. Edge chevrons show
            // where more is and scroll it on click. Every command also stays reachable through Search Commands.
            let r = scroll.inner_rect;
            let overflow = scroll.content_size.x - r.width();
            let offset = scroll.state.offset.x;
            let step = 160.0;
            let mut next = None;
            if offset > 1.0 {
                let left = Rect::from_min_max(r.left_top(), pos2(r.left() + 18.0, r.bottom()));
                if edge_cue(ui, left, "‹", tl!("Scroll ribbon left"), &t) {
                    next = Some(offset - step);
                }
            }
            if overflow - offset > 1.0 {
                let right = Rect::from_min_max(pos2(r.right() - 18.0, r.top()), r.right_bottom());
                if edge_cue(ui, right, "›", tl!("Scroll ribbon right"), &t) {
                    next = Some(offset + step);
                }
            }
            if let Some(x) = next {
                let mut state = scroll.state;
                state.offset.x = x.clamp(0.0, overflow.max(0.0));
                state.store(ui.ctx(), scroll.id);
            }
        });
}

/// Custom groups (Customize Ribbon, #379): up to three commands as large buttons, more as small
/// labelled buttons in columns of three. Each shows the command's own icon and label.
fn custom_groups(app: &mut WordApp, ui: &mut Ui, groups: &[wordcraft_engine::ribbon::CustomGroup]) {
    for g in groups {
        let ids: Vec<(&str, String)> =
            g.commands.iter().filter_map(|id| app.session.registry.get(id).map(|s| (s.id, tl!(s.label).to_string()))).collect();
        crate::widgets::group_titled(ui, &g.name, &g.name, None, app, |ui, app| {
            if ids.len() <= 3 {
                for (id, label) in &ids {
                    big(ui, app, crate::widgets::command_icon(id), &two_lines(label), id, json!({}), false);
                }
                return;
            }
            for column in ids.chunks(3) {
                stack(ui, |ui| {
                    for (id, label) in column {
                        small(ui, app, crate::widgets::command_icon(id), Some(label), label, id, json!({}), false);
                    }
                });
            }
        });
    }
}

/// A large button's label on two lines when it is long (broken at the space nearest the middle).
fn two_lines(label: &str) -> String {
    let n = label.chars().count();
    if n <= 10 {
        return label.to_string();
    }
    let mid = n / 2;
    let best = label.char_indices().filter(|(_, c)| *c == ' ').min_by_key(|(i, _)| label[..*i].chars().count().abs_diff(mid)).map(|(i, _)| i);
    match best {
        Some(i) => format!("{}\n{}", &label[..i], label[i..].trim_start()),
        None => label.to_string(),
    }
}

/// A chevron over a ribbon edge that is also a button; `true` on the frame it is clicked.
fn edge_cue(ui: &mut Ui, rect: Rect, glyph: &str, hint: &str, t: &Tokens) -> bool {
    let resp = ui.interact(rect, ui.id().with(glyph), Sense::click()).on_hover_text(hint);
    let ink = if resp.hovered() { t.text } else { t.text_dim };
    ui.painter().rect_filled(rect, 0.0, t.ribbon);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, egui::FontId::proportional(18.0), ink);
    resp.clicked()
}

/// Whether Esc just ended typing in a field (`r`, remembered under `id`). egui 0.36 keeps a
/// `DragValue`'s typed text after Esc and applies it a frame later, when the field still counts as
/// having just lost focus but Esc is no longer pressed: a change reported within two frames of the
/// Esc is that late value, to be dropped so that Esc cancels as in Word.
fn esc_cancelled(ui: &Ui, r: &egui::Response, id: egui::Id) -> bool {
    let now = ui.ctx().cumulative_pass_nr();
    let (focus_key, esc_key) = (id.with("focused"), id.with("esc"));
    // Read outside `data_mut`: both lock the context.
    let focused = r.has_focus();
    let had_focus = ui.data(|d| d.get_temp::<bool>(focus_key)).unwrap_or(false);
    ui.data_mut(|d| d.insert_temp(focus_key, focused));
    if had_focus && !focused && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.insert_temp(esc_key, now));
    }
    ui.data(|d| d.get_temp::<u64>(esc_key)).is_some_and(|at| now.saturating_sub(at) <= 2)
}

fn stack(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        add(ui);
    });
}

/// Layout › Size: the named paper sizes, then More Paper Sizes… for a custom size in Page
/// Setup (#1433).
pub(crate) fn size_menu(ui: &mut Ui, app: &mut WordApp) {
    // The current paper size is checked.
    let x = wordcraft_engine::cmd::page::sect(&app.session);
    let current = wordcraft_geom::paper_name(x.page_w, x.page_h);
    for (n, w, h) in wordcraft_geom::PAPER_SIZES {
        // Paper names are the industry's (Letter, A4, B5 (JIS)) and stay as they are; the
        // envelopes say what they are. Sizes in the measurement unit, written the region's
        // way (`21 cm × 29,7 cm`, #838).
        let name = if n.starts_with("Envelope ") { tl!(n) } else { n };
        let (width, height) = (crate::options::length(*w), crate::options::length(*h));
        let label = crate::i18n::fmt(tl!("{paper}   {width} × {height}"), &[("paper", name), ("width", &width), ("height", &height)]);
        mi_check(ui, app, &label, current == Some(*n), "layout.size", json!({"name": n}));
    }
    ui.separator();
    mi(ui, app, crate::dialogs::PAPER_LABELS.more, "ui.dialog", json!({"name": "pageSetup"}));
}

/// The underline styles Home › Underline offers as line samples: (`format.underline` style, name).
const UNDERLINE_STYLES: [(&str, &str); 8] = [
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
fn underline_menu(ui: &mut Ui, app: &mut WordApp) {
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
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

pub(crate) fn mi(ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, params: Value) {
    mi_text(ui, app, tl!(label), id, params);
}

/// [`mi`] with a drawn icon before the label.
fn mi_icon(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, params: Value) {
    let sc = crate::widgets::shortcut_text(app, id);
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled_ui(enabled, |ui| crate::widgets::icon_menu_item(ui, icon, tl!(label), &sc, 200.0)).inner;
    if resp.clicked() {
        crate::widgets::run_from_ribbon(app, id, params);
        ui.close();
    }
}

/// A menu item whose text is already in the interface language.
fn mi_text(ui: &mut Ui, app: &mut WordApp, text: &str, id: &str, params: Value) {
    let sc = crate::widgets::shortcut_text(app, id);
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(sc).min_size(vec2(200.0, 0.0)));
    if resp.clicked() {
        crate::widgets::run_from_ribbon(app, id, params);
        ui.close();
    }
}

/// The proofing languages the Language menu offers: tag and English name (translated when drawn).
pub(crate) const PROOFING_LANGUAGES: [(&str, &str); 12] = [
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("en-AU", "English (Australia)"),
    ("en-CA", "English (Canada)"),
    ("pl-PL", "Polish"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("it-IT", "Italian"),
    ("hr-HR", "Croatian"),
    ("tr-TR", "Turkish"),
    ("ru-RU", "Russian"),
    ("cs-CZ", "Czech"),
    ("id-ID", "Indonesian"),
];

/// The English name of a proofing language tag, as the status bar shows it (no tag: WordCraft's
/// default, US English): a listed language by its name; another region of one by the language
/// and the region ("French (Canada)"); a bare language subtag by its usual variant; else the tag.
pub(crate) fn language_name(tag: Option<&str>) -> String {
    let tag = tag.unwrap_or("en-US");
    let named =
        |t: &str| PROOFING_LANGUAGES.iter().chain(crate::dialogs_layout::LANGUAGES.iter()).find(|(l, _)| l.eq_ignore_ascii_case(t)).map(|(_, n)| *n);
    // Any language the Language dialog offers (French, German…) by its name, not its tag (#860).
    if let Some(name) = named(tag) {
        return name.to_string();
    }
    let mut parts = tag.split(['-', '_']);
    let primary = parts.next().unwrap_or("");
    // The region: the first two-letter or three-digit subtag after the language (and script).
    let region =
        parts.find(|p| (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic())) || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit())));
    match region {
        // A region the list doesn't have: the language's name without its own region or variant.
        Some(r) => {
            let language = crate::dialogs_layout::LANGUAGES
                .iter()
                .find(|(l, _)| l.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(primary)))
                .map(|(_, n)| n.split(" (").next().unwrap_or(n));
            match language {
                Some(language) => format!("{language} ({})", region_name(r)),
                None => tag.to_string(),
            }
        }
        // A bare language subtag ("pt", "pl"): the variant WordCraft proofs.
        None => match wordcraft_engine::proof::ProofLang::from_tag(Some(tag)) {
            wordcraft_engine::proof::ProofLang::Pl => "Polish".into(),
            wordcraft_engine::proof::ProofLang::Hr => "Croatian".into(),
            wordcraft_engine::proof::ProofLang::Pt => "Portuguese (Brazil)".into(),
            wordcraft_engine::proof::ProofLang::It => "Italian".into(),
            wordcraft_engine::proof::ProofLang::Tr => "Turkish".into(),
            wordcraft_engine::proof::ProofLang::Ru => "Russian".into(),
            wordcraft_engine::proof::ProofLang::Cs => "Czech".into(),
            wordcraft_engine::proof::ProofLang::Id => "Indonesian".into(),
            wordcraft_engine::proof::ProofLang::De => "German (Germany)".into(),
            wordcraft_engine::proof::ProofLang::En if tag.eq_ignore_ascii_case("en") => "English (United States)".into(),
            _ => tag.to_string(),
        },
    }
}

/// The English name of a region subtag the language list has no entry for (else the subtag).
fn region_name(code: &str) -> String {
    const REGIONS: [(&str, &str); 62] = [
        ("AE", "U.A.E."),
        ("AO", "Angola"),
        ("AR", "Argentina"),
        ("AT", "Austria"),
        ("AU", "Australia"),
        ("BA", "Bosnia and Herzegovina"),
        ("BE", "Belgium"),
        ("BO", "Bolivia"),
        ("BR", "Brazil"),
        ("BY", "Belarus"),
        ("CA", "Canada"),
        ("CH", "Switzerland"),
        ("CL", "Chile"),
        ("CN", "China"),
        ("CO", "Colombia"),
        ("CR", "Costa Rica"),
        ("CV", "Cabo Verde"),
        ("CY", "Cyprus"),
        ("DE", "Germany"),
        ("DO", "Dominican Republic"),
        ("EC", "Ecuador"),
        ("EG", "Egypt"),
        ("ES", "Spain"),
        ("FR", "France"),
        ("GB", "United Kingdom"),
        ("GT", "Guatemala"),
        ("HK", "Hong Kong SAR"),
        ("IE", "Ireland"),
        ("IN", "India"),
        ("IQ", "Iraq"),
        ("IT", "Italy"),
        ("JM", "Jamaica"),
        ("JO", "Jordan"),
        ("KW", "Kuwait"),
        ("KZ", "Kazakhstan"),
        ("LB", "Lebanon"),
        ("LI", "Liechtenstein"),
        ("LU", "Luxembourg"),
        ("MA", "Morocco"),
        ("MC", "Monaco"),
        ("ME", "Montenegro"),
        ("MO", "Macao SAR"),
        ("MX", "Mexico"),
        ("MY", "Malaysia"),
        ("MZ", "Mozambique"),
        ("NL", "Netherlands"),
        ("NZ", "New Zealand"),
        ("PE", "Peru"),
        ("PH", "Philippines"),
        ("PR", "Puerto Rico"),
        ("PT", "Portugal"),
        ("PY", "Paraguay"),
        ("QA", "Qatar"),
        ("RU", "Russia"),
        ("SG", "Singapore"),
        ("SY", "Syria"),
        ("TW", "Taiwan"),
        ("UA", "Ukraine"),
        ("US", "United States"),
        ("UY", "Uruguay"),
        ("VE", "Venezuela"),
        ("ZA", "South Africa"),
    ];
    REGIONS.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map_or_else(|| code.to_ascii_uppercase(), |(_, n)| (*n).to_string())
}

/// The language at the caret (resolved through the styles), translated, for the status bar.
pub(crate) fn caret_language(app: &WordApp) -> String {
    tl!(&language_name(caret_lang_tag(app).as_deref())).to_string()
}

/// The formatting at the caret, resolved through the styles and the document default.
fn caret_char(app: &WordApp) -> wordcraft_doc::resolve::ResolvedChar {
    let s = &app.session;
    let style = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.style.clone());
    s.doc.styles.resolve_char(style.as_deref(), &s.typing_props())
}

/// The language tag at the caret, resolved through the styles and the document default.
fn caret_lang_tag(app: &WordApp) -> Option<String> {
    caret_char(app).lang
}

/// Is the text at the caret itself marked Do not check spelling or grammar? (`format.state`
/// looks at the whole word around a caret, so a partly marked word would read as checked.)
pub(crate) fn caret_no_proof(app: &WordApp) -> bool {
    caret_char(app).no_proof
}

/// Does WordCraft check spelling and grammar in the caret's language at all (Bulgarian: no)?
pub(crate) fn caret_language_proofed(app: &WordApp) -> bool {
    wordcraft_engine::proof::has_dictionary(caret_lang_tag(app).as_deref())
}

/// The Hunspell dictionary the caret's language needs but doesn't have (`pl_PL`), if any (#1167).
pub(crate) fn caret_missing_dictionary(app: &WordApp) -> Option<&'static str> {
    wordcraft_engine::proof::missing_dictionary(caret_lang_tag(app).as_deref())
}

/// The dictionary file and reason when the caret's language has a dictionary installed that
/// couldn't be read, so "no dictionary" would send the user to reinstall it for nothing (#2830).
pub(crate) fn caret_unreadable_dictionary(app: &WordApp) -> Option<(std::path::PathBuf, String)> {
    wordcraft_engine::proof::unreadable_dictionary(caret_lang_tag(app).as_deref()).map(|(_, file, reason)| (file, reason))
}

/// Review › Language (and the status bar's language): set the selection's proofing language, or
/// leave it unchecked. The current choice has a check mark; a language outside the short list
/// (French, German…) shows checked above it. Set Proofing Language… opens the Language dialog
/// with every language and Set As Default, and Language Preferences… opens Options › Language
/// (#860: the short list alone left no way to pick French).
pub(crate) fn language_menu(ui: &mut Ui, app: &mut WordApp) {
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    // The selection's own language; else (nothing set on the text) the one it gets from its
    // style or the document default, as the status bar shows.
    let current = match st.get("lang") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) if app.session.sel.is_collapsed() => caret_lang_tag(app).unwrap_or_else(|| "en-US".into()),
        _ => String::new(),
    };
    let no_proof = st.get("noProof").and_then(Value::as_bool).unwrap_or(false);
    let listed = |tag: &str| {
        current.eq_ignore_ascii_case(tag)
            || (tag == "pl-PL" && current.eq_ignore_ascii_case("pl"))
            || (tag == "pt-BR" && current.eq_ignore_ascii_case("pt"))
            || (tag == "it-IT" && current.eq_ignore_ascii_case("it"))
            || (tag == "hr-HR" && current.eq_ignore_ascii_case("hr"))
            || (tag == "tr-TR" && current.eq_ignore_ascii_case("tr"))
            || (tag == "cs-CZ" && wordcraft_engine::proof::ProofLang::from_tag(Some(&current)) == wordcraft_engine::proof::ProofLang::Cs)
            || (tag == "id-ID" && wordcraft_engine::proof::ProofLang::from_tag(Some(&current)) == wordcraft_engine::proof::ProofLang::Id)
            || (tag == "en-US" && current.eq_ignore_ascii_case("en"))
    };
    let checks = language_checks(&current, listed);
    if let Some(name) = checks.extra {
        mi_check(ui, app, &name, true, "review.language", json!({"lang": current}));
    }
    for ((tag, name), on) in PROOFING_LANGUAGES.iter().zip(checks.listed) {
        mi_check(ui, app, name, on, "review.language", json!({"lang": tag}));
    }
    ui.separator();
    mi_check(ui, app, "Do Not Check Spelling or Grammar", no_proof, "review.language", json!({"noProof": !no_proof}));
    ui.separator();
    mi_check(ui, app, "Set Proofing Language…", false, "review.language", json!({}));
    mi_check(ui, app, "Language Preferences…", false, "file.options", json!({"pane": "language"}));
}

/// The Language menu's check marks for the `current` tag: which short-list entries are checked
/// (`listed` matches a tag), and the name of the current language when it is none of them,
/// shown checked above. A language named like a short-list entry checks that entry, so no name
/// shows twice.
struct LanguageChecks {
    extra: Option<String>,
    listed: [bool; PROOFING_LANGUAGES.len()],
}

fn language_checks(current: &str, listed: impl Fn(&str) -> bool) -> LanguageChecks {
    let name = (!current.is_empty()).then(|| language_name(Some(current)));
    let on = PROOFING_LANGUAGES.map(|(t, n)| listed(t) || name.as_deref() == Some(n));
    LanguageChecks { extra: name.filter(|_| !on.contains(&true)), listed: on }
}

/// A menu item with a check mark when `checked` (the current choice of several).
fn mi_check(ui: &mut Ui, app: &mut WordApp, label: &str, checked: bool, id: &str, params: Value) {
    // An invisible mark keeps the unchecked labels aligned with the checked one.
    let mark = egui::RichText::new("✓");
    let mark = if checked { mark } else { mark.color(egui::Color32::TRANSPARENT) };
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled(enabled, egui::Button::new((mark, tl!(label))).selected(checked).min_size(vec2(200.0, 0.0)));
    if resp.clicked() {
        crate::widgets::run_from_ribbon(app, id, params);
        ui.close();
    }
}

/// Word's text wrapping styles in menu order: label, model value, `arrange.wrap` parameter.
pub(crate) const WRAP_STYLES: [(&str, wordcraft_doc::para::Wrap, &str); 7] = {
    use wordcraft_doc::para::Wrap;
    [
        ("In Line with Text", Wrap::Inline, "inline"),
        ("Square", Wrap::Square, "square"),
        ("Tight", Wrap::Tight, "tight"),
        ("Through", Wrap::Through, "through"),
        ("Top and Bottom", Wrap::TopAndBottom, "topAndBottom"),
        ("Behind Text", Wrap::BehindText, "behindText"),
        ("In Front of Text", Wrap::InFrontOfText, "inFrontOfText"),
    ]
};

/// The Wrap Text menu for the selected picture, shape or text box: the seven styles (the
/// current one checked), then Move with Text / Fix Position on Page for a floating one. The
/// canvas context menu shows it; the ribbon's Wrap Text button can share it.
pub(crate) fn wrap_text_items(ui: &mut Ui, app: &mut WordApp) {
    use wordcraft_engine::cmd::objects::{moves_with_text, selected};
    let state = selected(&app.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, moves_with_text(f))));
    let wrap = state.map(|(w, _)| w);
    for (label, w, v) in WRAP_STYLES {
        mi_check(ui, app, label, wrap == Some(w), "arrange.wrap", json!({"wrap": v}));
    }
    ui.separator();
    // An inline object is neither: it's part of the text.
    let with_text = state.filter(|(w, _)| *w != wordcraft_doc::para::Wrap::Inline).map(|(_, m)| m);
    mi_check(ui, app, "Move with Text", with_text == Some(true), "arrange.moveWithText", json!({"value": true}));
    mi_check(ui, app, "Fix Position on Page", with_text == Some(false), "arrange.moveWithText", json!({"value": false}));
}

/// The Format Painter button. A click arms it for one use (blue border only); a double-click keeps
/// it on across several uses (full checked look), like Word. Esc or another click turns it off.
fn format_painter_button(ui: &mut Ui, app: &mut WordApp) {
    let sticky = matches!(app.session.painter, Some((_, _, true)));
    let armed = app.session.painter.is_some();
    let resp = small(ui, app, "painter", Some("Format"), "Format Painter", "edit.formatPainter", json!({}), sticky);
    if resp.double_clicked() {
        painter_double_click(app);
    } else if armed && !sticky {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_stroke(resp.rect, egui::CornerRadius::same(4), Stroke::new(1.0, t.accent.linear_multiply(0.5)), egui::StrokeKind::Inside);
    }
}

/// A double-click fires `clicked()` twice, so by now the painter has been armed and disarmed again.
/// Arm it once more, this time sticky.
fn painter_double_click(app: &mut WordApp) {
    if app.session.painter.is_some() {
        let _ = app.run("edit.formatPainter", json!({}));
    }
    let _ = app.run("edit.formatPainter", json!({"sticky": true}));
}

fn home(app: &mut WordApp, ui: &mut Ui) {
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    group(ui, "Clipboard", Some("edit.clipboardPane"), app, |ui, app| {
        menu_button(ui, app, "paste", Some("Paste"), "Paste (⌘V)", true, "edit.paste", |ui, app| {
            mi(ui, app, "Paste", "edit.paste", json!({}));
            mi(ui, app, "Keep Text Only", "edit.pasteText", json!({}));
            mi(ui, app, "Merge Formatting", "edit.pasteMerge", json!({}));
            mi(ui, app, "Paste Special…", "edit.pasteSpecial", json!({}));
        });
        stack(ui, |ui| {
            small(ui, app, "cut", Some("Cut"), "Cut", "edit.cut", json!({}), false);
            small(ui, app, "copy", Some("Copy"), "Copy", "edit.copy", json!({}), false);
            format_painter_button(ui, app);
        });
    });
    group(ui, "Font", Some("format.fontDialog"), app, |ui, app| {
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                let font = st.get("font").and_then(Value::as_str).unwrap_or("").to_string();
                let font = app.previews.font_shown(&font);
                let fams = app.previews.families();
                let recent: Vec<String> = app.ui.recent_fonts.iter().map(|f| app.previews.font_shown(f)).collect();
                let prev = app.previews.font_preview_fn();
                if let Some(f) = crate::widgets::font_combo_with_recent_hint(ui, "font", 150.0, &font, &fams, &recent, Some(&*prev), tl!("Font")) {
                    let _ = if f == crate::widgets::ADD_FONT {
                        app.run("font.add", json!({}))
                    } else if f == crate::widgets::LOAD_SYSTEM_FONTS {
                        app.run("font.loadSystem", json!({}))
                    } else {
                        let f = app.previews.font_stored(&f);
                        app.run("format.font", json!({"name": f}))
                    };
                }
                let size = st
                    .get("size")
                    .and_then(Value::as_f64)
                    .map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { format!("{s}") })
                    .unwrap_or_default();
                let sizes: Vec<String> =
                    wordcraft_engine::cmd::format::SIZES.iter().map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }).collect();
                if let Some(v) = crate::widgets::combo_hint(ui, "size", 52.0, &size, &sizes, None, tl!("Font Size"))
                    && let Ok(x) = v.trim().parse::<f64>()
                {
                    let _ = app.run("format.size", json!({"size": x}));
                }
                ui.add_space(3.0);
                small(ui, app, "grow", None, "Increase Font Size", "format.growFont", json!({}), false);
                small(ui, app, "shrink", None, "Decrease Font Size", "format.shrinkFont", json!({}), false);
                menu_button(ui, app, "case", None, "Change Case", false, "format.changeCase", |ui, app| {
                    mi(ui, app, "Sentence case.", "format.changeCase", json!({"mode": "sentence"}));
                    mi(ui, app, "lowercase", "format.changeCase", json!({"mode": "lower"}));
                    mi(ui, app, "UPPERCASE", "format.changeCase", json!({"mode": "upper"}));
                    mi(ui, app, "Capitalize Each Word", "format.changeCase", json!({"mode": "title"}));
                    mi(ui, app, "tOGGLE cASE", "format.changeCase", json!({"mode": "toggle"}));
                });
                small(ui, app, "clear", None, "Clear All Formatting", "format.clear", json!({}), false);
                small(ui, app, "phonetic", None, "Phonetic Guide", "format.phonetic", json!({}), false);
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                small(ui, app, "bold", None, "Bold", "format.bold", json!({}), flag("bold"));
                small(ui, app, "italic", None, "Italic", "format.italic", json!({}), flag("italic"));
                split(ui, app, "underline", "Underline", "format.underline", json!({}), flag("underline"), None, underline_menu);
                small(ui, app, "strike", None, "Strikethrough", "format.strikethrough", json!({}), flag("strike"));
                small(ui, app, "subscript", None, "Subscript", "format.subscript", json!({}), flag("subscript"));
                small(ui, app, "superscript", None, "Superscript", "format.superscript", json!({}), flag("superscript"));
                small(ui, app, "charborder", None, "Character Border", "format.border", json!({}), flag("border"));
                small(ui, app, "enclose", None, "Enclose Characters", "format.enclose", json!({}), false);
                ui.add_space(4.0);
                menu_button(ui, app, "effects", None, "Text Effects and Typography", false, "", |ui, app| {
                    let theme = app.session.doc.settings.theme_colors.clone();
                    crate::wordart_menus::style_gallery(ui, app, "format.textEffects");
                    ui.separator();
                    ui.menu_button(tl!("Text Fill"), |ui| crate::wordart_menus::fill_menu(ui, app, &theme));
                    let outline = crate::wordart_menus::current_outline(app);
                    ui.menu_button(tl!("Text Outline"), |ui| crate::wordart_menus::outline_menu(ui, app, &theme, outline));
                    crate::wordart_menus::effects_menu(ui, app, &theme, false);
                    ui.separator();
                    mi(ui, app, "Outline", "format.outline", json!({}));
                    mi(ui, app, "Shadow", "format.shadow", json!({}));
                    mi(ui, app, "Small Caps", "format.smallCaps", json!({}));
                    mi(ui, app, "All Caps", "format.allCaps", json!({}));
                    mi(ui, app, "Double Strikethrough", "format.doubleStrikethrough", json!({}));
                });
                let hl = app.canvas.last_highlight.clone();
                split(
                    ui,
                    app,
                    "highlight",
                    "Text Highlight Color",
                    "format.highlight",
                    json!({"color": hl}),
                    false,
                    None,
                    crate::mini_toolbar::highlight_menu,
                );
                let fc = app.canvas.last_font_color.clone();
                let sw = wordcraft_doc::Rgb::parse(&fc).map(crate::theme::c32);
                split(ui, app, "fontcolor", "Font Color", "format.color", json!({"color": fc}), false, sw, |ui, app| {
                    mi(ui, app, "Automatic", "format.color", json!({"color": "auto"}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    crate::colors::menu(ui, app, &theme, Target::cmd("format.color").remember(Remember::FontColor));
                });
            });
        });
    });
    group(ui, "Paragraph", Some("para.dialog"), app, |ui, app| {
        let rp = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props));
        // The buttons show alignment as seen on the page (Align Right is a right-to-left paragraph's start).
        let align = rp.as_ref().map(|r| r.align.visual(r.bidi)).unwrap_or_default();
        let rtl = rp.as_ref().is_some_and(|r| r.bidi);
        let lists = wordcraft_engine::cmd::para::list_buttons(&app.session.doc, rp.as_ref().and_then(|r| r.numbering));
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                split(ui, app, "bullets", "Bullets", "para.bullets", json!({}), lists.bullets, None, |ui, app| {
                    ui.label(egui::RichText::new(tl!("Bullet Library")).small().weak());
                    ui.horizontal(|ui| {
                        for c in ["•", "○", "▪", "◆", "➢", "✓", "–"] {
                            // The interface fonts have no ➢, so its tile draws the arrowhead.
                            let label = if c == "➢" { " " } else { c };
                            let resp = ui.button(egui::RichText::new(label).size(16.0));
                            if c == "➢" {
                                let m = resp.rect.center();
                                let pts = [(-4.0, -5.0), (5.0, 0.0), (-4.0, 5.0), (-1.5, 0.0)].map(|(x, y)| m + vec2(x, y));
                                ui.painter().add(egui::Shape::convex_polygon(vec![pts[0], pts[1], pts[3]], Tokens::get(ui.ctx()).text, Stroke::NONE));
                                ui.painter().add(egui::Shape::convex_polygon(vec![pts[3], pts[1], pts[2]], Tokens::get(ui.ctx()).text, Stroke::NONE));
                            }
                            if resp.clicked() {
                                let _ = app.run("para.bullets", json!({"kind": c}));
                                ui.close();
                            }
                        }
                    });
                    mi(ui, app, "None", "para.bullets", json!({"off": true}));
                });
                split(ui, app, "numbering", "Numbering", "para.numbering", json!({}), lists.numbering, None, |ui, app| {
                    for (l, k) in [
                        ("1. 2. 3.", "numbered"),
                        ("1) 2) 3)", "numberedParen"),
                        ("I. II. III.", "outline"),
                        ("A. B. C.", "upperLetter"),
                        ("a) b) c)", "lowerLetter"),
                        ("i. ii. iii.", "lowerRoman"),
                    ] {
                        mi(ui, app, l, "para.numbering", json!({"kind": k}));
                    }
                    ui.separator();
                    mi(ui, app, "Restart at 1", "para.restartNumbering", json!({}));
                    mi(ui, app, "None", "para.numbering", json!({"off": true}));
                });
                split(ui, app, "multilevel", "Multilevel List", "para.multilevel", json!({}), lists.multilevel, None, |ui, app| {
                    mi(ui, app, "1. 1.1. 1.1.1.", "para.multilevel", json!({"kind": "legal"}));
                    mi(ui, app, "I. A. 1. a.", "para.multilevel", json!({"kind": "outline"}));
                    ui.separator();
                    // Lists linked to the heading styles: the headings carry the numbers. Each item
                    // reads as its first three levels, number then (translated) style name.
                    for (numbers, k) in [
                        (["1", "1.1", "1.1.1"], "headings"),
                        (["Article I.", "Section 1.01", "(a)"], "article"),
                        (["I.", "A.", "1."], "outlineHeadings"),
                        (["Chapter 1", "", ""], "chapter"),
                    ] {
                        let text = numbers
                            .iter()
                            .zip(["Heading 1", "Heading 2", "Heading 3"])
                            .map(|(n, h)| if n.is_empty() { tl!(h).to_string() } else { format!("{} {}", tl!(n), tl!(h)) })
                            .collect::<Vec<_>>()
                            .join(" / ");
                        mi_text(ui, app, &text, "list.multilevelPreset", json!({"preset": k}));
                    }
                    ui.separator();
                    mi(ui, app, "Define New Multilevel List…", "list.define", json!({}));
                    ui.separator();
                    for lv in 0..5u64 {
                        mi(ui, app, &format!("Change to Level {}", lv + 1), "para.listLevel", json!({"level": lv}));
                    }
                });
                ui.add_space(4.0);
                small(ui, app, "outdent", None, "Decrease Indent", "para.outdent", json!({}), false);
                small(ui, app, "indent", None, "Increase Indent", "para.indent", json!({}), false);
                ui.add_space(4.0);
                small(ui, app, "sort", None, "Sort", "para.sort", json!({}), false);
                let marks = app.session.view.marks;
                small(ui, app, "pilcrow", None, "Show/Hide ¶", "view.marks", json!({}), marks);
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                use wordcraft_doc::Align as A;
                small(ui, app, "alignLeft", None, "Align Left", "para.alignLeft", json!({}), align == A::Left);
                small(ui, app, "alignCenter", None, "Center", "para.alignCenter", json!({}), align == A::Center);
                small(ui, app, "alignRight", None, "Align Right", "para.alignRight", json!({}), align == A::Right);
                small(ui, app, "justify", None, "Justify", "para.justify", json!({}), align == A::Justify);
                ui.add_space(4.0);
                small(ui, app, "textLtr", None, "Left-to-Right Text Direction", "para.ltr", json!({}), !rtl);
                small(ui, app, "textRtl", None, "Right-to-Left Text Direction", "para.rtl", json!({}), rtl);
                ui.add_space(4.0);
                menu_button(ui, app, "lineSpacing", None, "Line and Paragraph Spacing", false, "para.lineSpacing", |ui, app| {
                    for v in [1.0, 1.15, 1.5, 2.0, 2.5, 3.0] {
                        mi(ui, app, &format!("{v}"), "para.lineSpacing", json!({"value": v}));
                    }
                    ui.separator();
                    mi(ui, app, "Add Space Before Paragraph", "para.addSpaceBefore", json!({}));
                    mi(ui, app, "Remove Space After Paragraph", "para.removeSpaceAfter", json!({}));
                    mi(ui, app, "Line Spacing Options…", "para.dialog", json!({}));
                });
                split(ui, app, "shading", "Shading", "para.shading", json!({"color": app.canvas.last_shading.clone()}), false, None, |ui, app| {
                    mi(ui, app, "No Color", "para.shading", json!({"color": null}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    crate::colors::menu(ui, app, &theme, Target::cmd("para.shading").remember(Remember::Shading));
                });
                split(ui, app, "borders", "Borders", "para.borders", json!({"kind": "bottom"}), false, None, |ui, app| {
                    for (l, k) in [
                        ("Bottom Border", "bottom"),
                        ("Top Border", "top"),
                        ("Left Border", "left"),
                        ("Right Border", "right"),
                        ("No Border", "none"),
                        ("All Borders", "all"),
                        ("Outside Borders", "outside"),
                        ("Inside Borders", "inside"),
                    ] {
                        mi(ui, app, l, "para.borders", json!({"kind": k}));
                    }
                    ui.separator();
                    mi(ui, app, "Horizontal Line", "insert.horizontalLine", json!({}));
                    mi(ui, app, "Borders and Shading…", "para.borders", json!({}));
                });
            });
        });
    });
    group(ui, "Styles", Some("styles.pane"), app, |ui, app| {
        crate::previews::style_gallery(app, ui, &st);
    });
    group(ui, "Editing", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "find", Some("Find"), "Find", false, "edit.find", |ui, app| {
                // As in Word: Find searches in the Navigation pane; Advanced Find is the dialog (#712).
                mi(ui, app, "Find", "edit.find", json!({}));
                mi(ui, app, "Advanced Find…", "edit.advancedFind", json!({}));
                mi(ui, app, "Go To…", "ui.dialog", json!({"name": "goto"}));
            });
            small(ui, app, "replace", Some("Replace"), "Replace", "ui.dialog", json!({"name": "replace"}), false);
            menu_button(ui, app, "select", Some("Select"), "Select", false, "select.all", |ui, app| {
                mi(ui, app, "Select All", "select.all", json!({}));
                mi(ui, app, "Select Paragraph", "select.paragraph", json!({}));
                mi(ui, app, "Select Sentence", "select.sentence", json!({}));
            });
        });
    });
    group(ui, "Voice", None, app, |ui, app| {
        big(ui, app, "dictate", "Dictate", "tools.dictate", json!({}), false);
    });
    group(ui, "Editor", None, app, |ui, app| {
        big(ui, app, "editor", "Editor", "review.spelling", json!({}), false);
    });
}

fn insert(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Pages", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "coverPage", Some("Cover Page"), "Cover Page", false, "insert.coverPage", |ui, app| {
                mi(ui, app, "Studio cover", "insert.coverPage", json!({}));
            });
            small(ui, app, "blankPage", Some("Blank Page"), "Blank Page", "insert.blankPage", json!({}), false);
            small(ui, app, "pageBreak", Some("Page Break"), "Page Break", "insert.pageBreak", json!({}), false);
        });
    });
    group(ui, "Tables", None, app, |ui, app| {
        menu_button(ui, app, "table", Some("Table"), "Add a Table", true, "insert.table", |ui, app| {
            crate::dialogs::table_grid_picker(ui, app);
            ui.separator();
            mi(ui, app, "Insert Table…", "ui.dialog", json!({"name": "insertTable"}));
            let drawing = app.canvas.table_tool == Some(crate::table_pen::TableTool::Draw);
            mi_check(ui, app, "Draw Table", drawing, "table.draw", json!({}));
            mi(ui, app, "Convert Text to Table…", "table.fromText", json!({}));
            ui.menu_button(tl!("Quick Tables"), |ui| {
                mi(ui, app, "Tabular List", "table.quick", json!({"kind": "tabular"}));
                mi(ui, app, "Matrix", "table.quick", json!({"kind": "matrix"}));
                mi(ui, app, "Calendar", "table.quick", json!({"kind": "calendar"}));
            });
        });
    });
    group(ui, "Illustrations", None, app, |ui, app| {
        big(ui, app, "picture", "Pictures", "insert.picture", json!({}), false);
        menu_button(ui, app, "shapes", Some("Shapes"), "Shapes", true, "insert.shape", |ui, app| {
            for (l, k) in [
                ("Rectangle", "rectangle"),
                ("Rounded Rectangle", "roundedRectangle"),
                ("Oval", "ellipse"),
                ("Triangle", "triangle"),
                ("Diamond", "diamond"),
                ("Line", "line"),
                ("Arrow", "arrow"),
                ("Star", "star"),
                ("Heart", "heart"),
            ] {
                mi_icon(ui, app, &format!("shape.{k}"), l, "insert.shape", json!({"kind": k}));
            }
            ui.separator();
            mi_icon(ui, app, "canvas", "New Drawing Canvas", "insert.canvas", json!({}));
            // Connectors: glue their ends to shapes by dragging them (shape.connect).
            for (l, k, arrow) in [
                ("Straight Connector", "straightConnector", "none"),
                ("Straight Arrow Connector", "straightConnector", "end"),
                ("Elbow Connector", "elbowConnector", "none"),
                ("Elbow Arrow Connector", "elbowConnector", "end"),
                ("Curved Connector", "curvedConnector", "none"),
                ("Curved Arrow Connector", "curvedConnector", "end"),
            ] {
                let icon = if arrow == "end" { k.replace("Connector", "ArrowConnector") } else { k.to_string() };
                mi_icon(ui, app, &format!("shape.{icon}"), l, "insert.shape", json!({"kind": k, "arrow": arrow}));
            }
        });
        stack(ui, |ui| {
            small(ui, app, "icons", Some("Icons"), "Icons", "insert.icon", json!({}), false);
            small(ui, app, "smartArt", Some("SmartArt"), "SmartArt", "insert.smartArt", json!({}), false);
            small(ui, app, "chart", Some("Chart"), "Chart", "insert.chart", json!({}), false);
        });
    });
    group(ui, "Links", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "link", Some("Link"), "Link", "ui.dialog", json!({"name": "link"}), false);
            small(ui, app, "bookmark", Some("Bookmark"), "Bookmark", "ui.dialog", json!({"name": "bookmark"}), false);
            small(ui, app, "crossRef", Some("Cross-reference"), "Cross-reference", "insert.crossReference", json!({}), false);
        });
    });
    group(ui, "Comments", None, app, |ui, app| {
        big(ui, app, "newComment", "Comment", "review.newComment", json!({}), false);
    });
    group(ui, "Header & Footer", None, app, |ui, app| {
        menu_button(ui, app, "header", Some("Header"), "Header", true, "insert.header", |ui, app| {
            mi(ui, app, "Blank", "insert.header", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.header", json!({"preset": "blankThree"}));
            mi(ui, app, "Document Title", "insert.header", json!({"preset": "title"}));
            ui.separator();
            mi(ui, app, "Edit Header", "insert.editHeader", json!({}));
            mi(ui, app, "Remove Header", "insert.removeHeader", json!({}));
        });
        menu_button(ui, app, "footer", Some("Footer"), "Footer", true, "insert.footer", |ui, app| {
            mi(ui, app, "Blank", "insert.footer", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.footer", json!({"preset": "blankThree"}));
            mi(ui, app, "Page Number", "insert.footer", json!({"preset": "pageNumber"}));
            ui.separator();
            mi(ui, app, "Edit Footer", "insert.editFooter", json!({}));
            mi(ui, app, "Remove Footer", "insert.removeFooter", json!({}));
        });
        menu_button(ui, app, "pageNumber", Some("Page\nNumber"), "Page Number", true, "insert.pageNumber", |ui, app| {
            mi(ui, app, "Top of Page", "insert.pageNumber", json!({"position": "top", "align": "right"}));
            mi(ui, app, "Bottom of Page", "insert.pageNumber", json!({"position": "bottom", "align": "center"}));
            mi(ui, app, "Page X of Y", "insert.pageNumber", json!({"position": "bottom", "format": "x of y"}));
            mi(ui, app, "Current Position", "insert.pageNumber", json!({"position": "current"}));
            mi(ui, app, "Format Page Numbers…", "ui.dialog", json!({"name": "pageNumbers"}));
        });
    });
    group(ui, "Text", None, app, |ui, app| {
        big(ui, app, "textBox", "Text\nBox", "insert.textBox", json!({}), false);
        stack(ui, |ui| {
            menu_button(ui, app, "quickParts", Some("Quick Parts"), "Quick Parts", false, "insert.quickParts", |ui, app| {
                // This session's building blocks, then the document's own Quick Parts.
                let mut parts: Vec<String> = app.session.building_blocks.keys().cloned().collect();
                let mut seen: std::collections::HashSet<String> = parts.iter().cloned().collect();
                let glossary = app.session.doc.glossary.iter().filter(|e| e.gallery == wordcraft_doc::QUICK_PARTS_GALLERY);
                parts.extend(glossary.filter(|e| seen.insert(e.name.clone())).map(|e| e.name.clone()));
                for name in parts {
                    if ui.button(&name).clicked() {
                        let _ = app.run("insert.quickParts", json!({"insert": name}));
                        ui.close();
                    }
                }
                mi(ui, app, "Field…", "ui.dialog", json!({"name": "field"}));
            });
            menu_button(ui, app, "wordArt", Some("WordArt"), "WordArt", false, "", |ui, app| {
                crate::wordart_menus::style_gallery(ui, app, "insert.wordArt")
            });
            menu_button(ui, app, "dropCap", Some("Drop Cap"), "Drop Cap", false, "insert.dropCap", drop_cap_items);
        });
        stack(ui, |ui| {
            small(ui, app, "signature", None, "Signature Line", "insert.signatureLine", json!({}), false);
            small(ui, app, "dateTime", None, "Date & Time", "insert.dateTime", json!({}), false);
            small(ui, app, "object", None, "Object", "insert.object", json!({}), false);
        });
    });
    group(ui, "Symbols", None, app, |ui, app| {
        crate::equation_tab::insert_button(ui, app);
        menu_button(ui, app, "symbol", Some("Symbol"), "Symbol", true, "insert.symbol", |ui, app| {
            // Recently used symbols first (their font on hover), then common ones.
            let mut items: Vec<(String, String)> = app.ui.recent_symbols.iter().map(|r| (r.ch.clone(), r.font.clone())).collect();
            for c in [
                "©", "®", "™", "§", "¶", "€", "£", "¥", "°", "±", "≠", "≤", "≥", "÷", "×", "∞", "µ", "α", "β", "π", "Ω", "∑", "√", "→", "←", "✓",
                "★", "♥", "—", "…",
            ] {
                if !items.iter().any(|(ch, f)| ch == c && f.is_empty()) {
                    items.push((c.to_string(), String::new()));
                }
            }
            items.truncate(30);
            egui::Grid::new("syms").show(ui, |ui| {
                for (i, (c, font)) in items.iter().enumerate() {
                    let b = ui.button(egui::RichText::new(c).size(16.0));
                    let b = if font.is_empty() { b } else { b.on_hover_text(font) };
                    if b.clicked() {
                        let params = if font.is_empty() { json!({"char": c}) } else { json!({"char": c, "font": font}) };
                        if app.run("insert.symbol", params).is_ok() {
                            crate::dialogs_insert::remember_symbol(app, c, font);
                        }
                        ui.close();
                    }
                    if i % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
            ui.separator();
            mi(ui, app, "More Symbols…", "ui.dialog", json!({"name": "symbol"}));
        });
    });
}

/// Insert › Drop Cap's menu (#2601): None, Dropped and In margin (the current one checked),
/// then the Drop Cap dialog.
fn drop_cap_items(ui: &mut Ui, app: &mut WordApp) {
    let current = wordcraft_engine::cmd::insert::drop_cap_state(&app.session).position;
    for (label, v) in [("None", "none"), ("Dropped", "dropped"), ("In margin", "margin")] {
        mi_check(ui, app, label, current == v, "insert.dropCap", json!({"position": v}));
    }
    ui.separator();
    mi(ui, app, "Drop Cap Options…", "ui.dialog", json!({"name": "dropCap"}));
}

fn draw(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_doc::freeform::InkTool;
    use wordcraft_engine::cmd::draw::{DrawMode, lasso_selection};
    let mode = app.session.view.draw.mode;
    // The pen the colour and thickness menus change: the one in use, else the pen.
    let tool = mode.pen().unwrap_or(InkTool::Pen);
    let set = app.session.view.draw.settings(tool);
    // The gallery pen in use, if any: changing the colour or thickness changes it too.
    let custom =
        app.session.view.draw.pens.iter().position(|p| mode == DrawMode::Pen(p.kind) && p.color == set.color && (p.width - set.width).abs() < 0.01);
    let lasso_on = lasso_selection(&app.session).is_some();
    group(ui, "Drawing Tools", None, app, |ui, app| {
        big_toggle(ui, app, "select", "Select", "draw.select", mode == DrawMode::Select);
        big_toggle(ui, app, "lasso", "Lasso", "draw.lasso", mode == DrawMode::Lasso);
        big_toggle(ui, app, "eraser", "Eraser", "draw.eraser", mode == DrawMode::Eraser);
        big_toggle(ui, app, "pen", "Pen", "draw.pen", mode == DrawMode::Pen(InkTool::Pen) && custom.is_none());
        big_toggle(ui, app, "pencil", "Pencil", "draw.pencil", mode == DrawMode::Pen(InkTool::Pencil) && custom.is_none());
        big_toggle(ui, app, "highlight", "Highlighter", "draw.highlighter", mode == DrawMode::Pen(InkTool::Highlighter) && custom.is_none());
        // Pens added with Add Pen; right-click one to remove it.
        let pens = app.session.view.draw.pens.clone();
        for (i, pen) in pens.iter().enumerate() {
            gallery_pen(ui, app, i, *pen, custom == Some(i));
        }
        menu_button(ui, app, "addPen", Some("Add\nPen"), "Add Pen", true, "", |ui, app| {
            for (label, kind) in [("Pen", InkTool::Pen), ("Pencil", InkTool::Pencil), ("Highlighter", InkTool::Highlighter)] {
                // A new pen starts as that kind's current colour and thickness.
                let cur = app.session.view.draw.settings(kind);
                mi(ui, app, label, "draw.addPen", json!({"kind": kind.name(), "color": cur.color.hex(), "width": cur.width}));
            }
        });
        stack(ui, |ui| {
            let sw = Some(crate::theme::c32(set.color));
            split(ui, app, "fontcolor", "Color", pen_command(tool), json!({}), false, sw, |ui, app| {
                let theme = app.session.doc.settings.theme_colors.clone();
                // With ink lasso-selected the colour goes to the selection, else to the pen.
                let cmd = if lasso_on { "draw.lassoColor" } else { pen_command(tool) };
                crate::colors::menu(ui, app, &theme, Target::cmd(cmd));
                if !lasso_on {
                    update_gallery_pen(app, custom, tool);
                }
            });
            menu_button(ui, app, "thickness", None, "Thickness", false, "draw.pen", |ui, app| {
                let widths: &[f32] = if tool == InkTool::Highlighter { &[4.0, 8.0, 12.0, 18.0, 24.0] } else { &[0.5, 1.0, 1.5, 2.5, 3.5, 5.0] };
                for w in widths {
                    let label = crate::i18n::fmt(tl!("{n} pt"), &[("n", &w.to_string())]);
                    mi_check(ui, app, &label, (set.width - w).abs() < 0.01, pen_command(tool), json!({"width": w}));
                }
                update_gallery_pen(app, custom, tool);
            });
        });
    });
    group(ui, "Convert", None, app, |ui, app| {
        let on = app.session.view.draw.ink_to_shape;
        big_toggle(ui, app, "inkToShape", "Ink to\nShape", "draw.inkToShape", on);
        big(ui, app, "inkToMath", "Ink to\nMath", "draw.inkToMath", json!({}), false);
    });
    group(ui, "Insert", None, app, |ui, app| {
        big(ui, app, "canvas", "Drawing\nCanvas", "insert.canvas", json!({}), false);
    });
    group(ui, "Replay", None, app, |ui, app| {
        let on = app.session.view.draw.replay;
        big_toggle(ui, app, "replay", "Ink\nReplay", "draw.replay", on);
    });
}

/// The command that picks up `tool`.
fn pen_command(tool: wordcraft_doc::freeform::InkTool) -> &'static str {
    use wordcraft_doc::freeform::InkTool;
    match tool {
        InkTool::Pen => "draw.pen",
        InkTool::Pencil => "draw.pencil",
        InkTool::Highlighter => "draw.highlighter",
    }
}

/// After the colour or thickness menu changed `tool`'s settings, the gallery pen that was in use
/// (`custom`) takes them too.
fn update_gallery_pen(app: &mut WordApp, custom: Option<usize>, tool: wordcraft_doc::freeform::InkTool) {
    let set = app.session.view.draw.settings(tool);
    if let Some(p) = custom.and_then(|i| app.session.view.draw.pens.get_mut(i))
        && p.kind == tool
    {
        p.color = set.color;
        p.width = set.width;
    }
}

/// A pen of the gallery: its kind's icon over a bar of its colour and thickness; a click picks it
/// up, right-click offers Remove Pen.
fn gallery_pen(ui: &mut Ui, app: &mut WordApp, i: usize, pen: wordcraft_engine::cmd::draw::CustomPen, active: bool) {
    use wordcraft_doc::freeform::InkTool;
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(34.0, CONTENT_H), Sense::click());
    crate::widgets::bg(ui, r, &resp, active, &t);
    let icon = match pen.kind {
        InkTool::Pen => "pen",
        InkTool::Pencil => "pencil",
        InkTool::Highlighter => "highlight",
    };
    let color = crate::theme::c32(pen.color);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(28.0, 28.0)), icon, t.icon, color);
    let bar = (pen.width * 0.6).clamp(1.5, 8.0);
    let y = r.min.y + 46.0;
    ui.painter().line_segment([pos2(r.min.x + 6.0, y), pos2(r.max.x - 6.0, y)], Stroke::new(bar, color));
    let kind = tl!(match pen.kind {
        InkTool::Pen => "Pen",
        InkTool::Pencil => "Pencil",
        InkTool::Highlighter => "Highlighter",
    });
    let size = crate::i18n::fmt(tl!("{n} pt"), &[("n", &pen.width.to_string())]);
    let resp = resp.on_hover_text(format!("{kind}: {size}, #{}", pen.color.hex()));
    if resp.clicked() {
        let _ = app.run(pen_command(pen.kind), json!({"color": pen.color.hex(), "width": pen.width}));
    }
    resp.context_menu(|ui| {
        if ui.button(tl!("Remove Pen")).clicked() {
            let _ = app.run("draw.removePen", json!({"index": i}));
            ui.close();
        }
    });
}

/// The Design tab's style set gallery in a box of its own, sized from its thumbnails and clipped
/// to it, with a gap on each side: the Colors and Fonts column can't be drawn over, or cut by,
/// the thumbnails (#638). Returns the box and the space the thumbnails used.
fn style_set_box(app: &mut WordApp, ui: &mut Ui) -> (Rect, Rect) {
    ui.add_space(4.0);
    let (gallery, _) = ui.allocate_exact_size(vec2(crate::previews::STYLE_SET_GALLERY_W, CONTENT_H), Sense::hover());
    let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(gallery).layout(egui::Layout::left_to_right(egui::Align::Min)));
    gui.set_clip_rect(gallery.intersect(ui.clip_rect()));
    gui.spacing_mut().item_spacing = vec2(1.0, 1.0);
    crate::previews::style_set_gallery(app, &mut gui);
    ui.add_space(8.0);
    (gallery, gui.min_rect())
}

/// The Custom section at the top of the Colors, Fonts and Themes galleries (#1869): the person's
/// own sets by name, each applied with `apply`; right-click to Edit… (colours and fonts) or
/// Delete. `kind` is `design.deleteCustom`'s.
fn custom_section(ui: &mut Ui, app: &mut WordApp, kind: &str, names: &[String], apply: &str) {
    if names.is_empty() {
        return;
    }
    ui.label(egui::RichText::new(tl!("Custom")).font(crate::theme::semibold(11.5)));
    for name in names {
        let resp = ui.add(egui::Button::new(name.as_str()).min_size(vec2(200.0, 0.0)));
        if resp.clicked() {
            crate::widgets::run_from_ribbon(app, apply, json!({"name": name}));
            ui.close();
        }
        resp.context_menu(|ui| {
            if kind != "theme" && ui.button(tl!("Edit…")).clicked() {
                let d = match kind {
                    "colors" => crate::dialogs_themes::ColorsForm::edit(app, name).map(crate::dialogs_themes::ThemeDialog::Colors),
                    _ => crate::dialogs_themes::FontsForm::edit(app, name).map(crate::dialogs_themes::ThemeDialog::Fonts),
                };
                if let Some(d) = d {
                    app.dialog = Some(crate::dialogs::Dialog::Themes(Box::new(d)));
                }
                ui.close();
            }
            if ui.button(tl!("Delete")).clicked() {
                crate::widgets::run_from_ribbon(app, "design.deleteCustom", json!({"kind": kind, "name": name}));
                ui.close();
            }
        });
    }
    ui.separator();
}

fn design(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Themes", None, app, |ui, app| {
        menu_button(ui, app, "themes", Some("Themes"), "Themes", true, "design.themes", |ui, app| {
            // The person's saved themes first, as in Word (#1869).
            let custom: Vec<String> = app.session.custom_themes.themes.iter().map(|t| t.name.clone()).collect();
            custom_section(ui, app, "theme", &custom, "design.theme");
            for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                mi(ui, app, name, "design.theme", json!({"name": name}));
            }
            // The browser's file picker opens what it picks as a document: no theme files there.
            if app.services.open_async.is_none() {
                ui.separator();
                mi(ui, app, "Browse for Themes…", "design.openTheme", json!({}));
                mi(ui, app, "Save Current Theme…", "design.saveTheme", json!({}));
            }
        });
    });
    group(ui, "Document Formatting", None, app, |ui, app| {
        style_set_box(app, ui);
        stack(ui, |ui| {
            menu_button(ui, app, "colors", Some("Colors"), "Theme Colors", false, "design.themeColors", |ui, app| {
                let custom: Vec<String> = app.session.custom_themes.colors.iter().map(|c| c.name.clone()).collect();
                custom_section(ui, app, "colors", &custom, "design.themeColors");
                for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                    mi(ui, app, name, "design.themeColors", json!({"name": name}));
                }
                ui.separator();
                mi(ui, app, "Customize Colors…", "design.customColors", json!({}));
            });
            menu_button(ui, app, "fonts", Some("Fonts"), "Theme Fonts", false, "design.themeFonts", |ui, app| {
                let custom: Vec<String> = app.session.custom_themes.fonts.iter().map(|c| c.name.clone()).collect();
                custom_section(ui, app, "fonts", &custom, "design.themeFonts");
                for (name, h, b, _) in wordcraft_engine::cmd::design::THEMES {
                    mi_text(ui, app, &format!("{}: {h} / {b}", tl!(name)), "design.themeFonts", json!({"heading": h, "body": b}));
                }
                ui.separator();
                mi(ui, app, "Customize Fonts…", "design.customFonts", json!({}));
            });
        });
        stack(ui, |ui| {
            menu_button(ui, app, "paraSpacing", Some("Paragraph Spacing"), "Paragraph Spacing", false, "design.paragraphSpacing", |ui, app| {
                for (l, v) in [
                    ("Default", "default"),
                    ("No Paragraph Space", "none"),
                    ("Compact", "compact"),
                    ("Tight", "tight"),
                    ("Open", "open"),
                    ("Relaxed", "relaxed"),
                    ("Double", "double"),
                ] {
                    // "Open" here is roomy spacing, not the file verb: only a `spacing` row
                    // translates it, else it stays English.
                    if l == "Open" {
                        mi_text(ui, app, crate::i18n::get("spacing", l).unwrap_or(l), "design.paragraphSpacing", json!({"value": v}));
                        continue;
                    }
                    mi(ui, app, l, "design.paragraphSpacing", json!({"value": v}));
                }
            });
            small(ui, app, "effectsDesign", Some("Effects"), "Effects", "design.effects", json!({}), false);
            small(ui, app, "setDefault", Some("Set as Default"), "Set as Default", "design.setDefault", json!({}), false);
        });
    });
    group(ui, "Page Background", None, app, |ui, app| {
        menu_button(ui, app, "watermark", Some("Watermark"), "Watermark", true, "design.watermark", |ui, app| {
            for w in ["DRAFT", "CONFIDENTIAL", "DO NOT COPY", "SAMPLE", "ASAP", "URGENT"] {
                mi(ui, app, w, "design.watermark", json!({"text": w}));
            }
            ui.separator();
            mi(ui, app, "Custom Watermark…", "ui.dialog", json!({"name": "watermark"}));
            mi(ui, app, "Remove Watermark", "design.watermark", json!({"remove": true}));
        });
        menu_button(ui, app, "pageColor", Some("Page\nColor"), "Page Color", true, "design.pageColor", |ui, app| {
            mi(ui, app, "No Color", "design.pageColor", json!({"color": null}));
            let theme = app.session.doc.settings.theme_colors.clone();
            crate::colors::menu(ui, app, &theme, Target::cmd("design.pageColor"));
        });
        big(ui, app, "pageBorders", "Page\nBorders", "design.pageBorders", json!({}), false);
    });
}

// Layout › Margins, Orientation, Size and Columns: the caret section's setting has a check
// mark, as Text Direction's does (#1206).
fn margins_items(ui: &mut Ui, app: &mut WordApp) {
    let current = wordcraft_engine::cmd::page::margins_preset_at(&app.session);
    // The two uniform presets name their margin in the measurement unit (`2,54 cm`, #838).
    for (l, k, uniform) in [
        ("Normal  {size} all", "normal", Some(72.0)),
        ("Narrow  {size} all", "narrow", Some(36.0)),
        ("Moderate", "moderate", None),
        ("Wide", "wide", None),
        ("Mirrored", "mirrored", None),
        ("Office 2003 Default", "office2003", None),
    ] {
        let label = match uniform {
            Some(pt) => crate::i18n::fmt(tl!(l), &[("size", &crate::options::length(pt))]),
            None => l.to_string(),
        };
        // Mirrored margins come from the document as well as from the preset (#1181).
        let on = current == Some(k) || (k == "mirrored" && app.session.doc.settings.mirror_margins);
        mi_check(ui, app, &label, on, "layout.margins", json!({"preset": k}));
    }
    mi(ui, app, "Custom Margins…", "ui.dialog", json!({"name": "pageSetup"}));
}

fn orientation_items(ui: &mut Ui, app: &mut WordApp) {
    let land = wordcraft_engine::cmd::page::sect(&app.session).landscape;
    mi_check(ui, app, "Portrait", !land, "layout.orientation", json!({"value": "portrait"}));
    mi_check(ui, app, "Landscape", land, "layout.orientation", json!({"value": "landscape"}));
}

fn columns_items(ui: &mut Ui, app: &mut WordApp) {
    let current = wordcraft_engine::cmd::page::columns_preset_at(&app.session);
    for (l, k, params) in [
        ("One", "one", json!({"count": 1})),
        ("Two", "two", json!({"count": 2})),
        ("Three", "three", json!({"count": 3})),
        ("Left", "left", json!({"preset": "left"})),
        ("Right", "right", json!({"preset": "right"})),
    ] {
        mi_check(ui, app, l, current == Some(k), "layout.columns", params);
    }
    ui.separator();
    mi(ui, app, "More Columns…", "ui.dialog", json!({"name": "columns"}));
}

fn layout(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Page Setup", Some("layout.pageSetup"), app, |ui, app| {
        // Text Direction works on what the caret is in: table cells, a text box, else the section.
        let dir = wordcraft_engine::cmd::page::text_direction_at(&app.session);
        menu_button(ui, app, "textDirection", Some("Text\nDirection"), "Text Direction", true, "layout.textDirection", |ui, app| {
            use wordcraft_doc::props::TextDirection;
            for (label, d) in
                [("Horizontal", TextDirection::Horizontal), ("Rotate all text 90°", TextDirection::Down), ("Rotate all text 270°", TextDirection::Up)]
            {
                mi_check(ui, app, label, dir == d, "layout.textDirection", json!({"value": d.ooxml()}));
            }
        });
        menu_button(ui, app, "margins", Some("Margins"), "Margins", true, "layout.margins", margins_items);
        menu_button(ui, app, "orientation", Some("Orientation"), "Orientation", true, "layout.orientation", orientation_items);
        menu_button(ui, app, "size", Some("Size"), "Size", true, "layout.size", size_menu);
        menu_button(ui, app, "columns", Some("Columns"), "Columns", true, "layout.columns", columns_items);
        stack(ui, |ui| {
            menu_button(ui, app, "breaks", Some("Breaks"), "Breaks", false, "layout.break", |ui, app| {
                ui.label(egui::RichText::new(tl!("Page Breaks")).strong());
                mi(ui, app, "Page", "layout.break", json!({"kind": "page"}));
                mi(ui, app, "Column", "layout.break", json!({"kind": "column"}));
                mi(ui, app, "Text Wrapping", "layout.break", json!({"kind": "textWrapping"}));
                ui.label(egui::RichText::new(tl!("Section Breaks")).strong());
                mi(ui, app, "Next Page", "layout.break", json!({"kind": "nextPage"}));
                mi(ui, app, "Continuous", "layout.break", json!({"kind": "continuous"}));
                mi(ui, app, "Even Page", "layout.break", json!({"kind": "evenPage"}));
                mi(ui, app, "Odd Page", "layout.break", json!({"kind": "oddPage"}));
            });
            menu_button(ui, app, "lineNumbers", Some("Line Numbers"), "Line Numbers", false, "layout.lineNumbers", |ui, app| {
                for (l, v) in
                    [("None", "none"), ("Continuous", "continuous"), ("Restart Each Page", "restartPage"), ("Restart Each Section", "restartSection")]
                {
                    mi(ui, app, l, "layout.lineNumbers", json!({"value": v}));
                }
                ui.separator();
                mi(ui, app, "Line Numbering Options…", "ui.dialog", json!({"name": "lineNumbers"}));
            });
            let hy = app.session.doc.settings.auto_hyphenation;
            menu_button(ui, app, "hyphenation", Some("Hyphenation"), "Hyphenation", false, "layout.hyphenation", |ui, app| {
                mi(ui, app, if hy { "✓ Automatic" } else { "Automatic" }, "layout.hyphenation", json!({"value": true}));
                mi(ui, app, if hy { "None" } else { "✓ None" }, "layout.hyphenation", json!({"value": false}));
                mi(ui, app, "Manual…", "ui.dialog", json!({"name": "manualHyphenation"}));
                ui.separator();
                mi(ui, app, "Hyphenation Options…", "ui.dialog", json!({"name": "hyphenation"}));
            });
        });
    });
    group(ui, "Paragraph", Some("para.dialog"), app, |ui, app| {
        let rp = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props));
        let Some(rp) = rp else { return };
        // #654: header + two rows of 20 px fields fit the content height above the group title
        // (1.5 pt between rows; 3 ran 2.5 pt into it); labels and fields sit on shared columns,
        // every field the same width.
        const FIELD: egui::Vec2 = egui::Vec2::new(64.0, 20.0);
        ui.spacing_mut().interact_size.y = FIELD.y;
        // The gap before the Spacing column sits inside its cells: `add_space` in a grid moves the
        // next cell in release builds but trips a debug assertion.
        let spaced = |ui: &mut Ui, text: egui::RichText| {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(text);
            });
        };
        egui::Grid::new("layout_para").spacing(vec2(4.0, 1.5)).min_row_height(FIELD.y).show(ui, |ui| {
            ui.label(egui::RichText::new(tl!("Indent")).small().strong());
            ui.label("");
            spaced(ui, egui::RichText::new(tl!("Spacing")).small().strong());
            ui.end_row();
            // Indents in the File › Options unit.
            let u = crate::options::unit();
            let k = u.pt_per_unit();
            let mut l = rp.indent_left / k;
            let mut b = rp.space_before;
            ui.label(egui::RichText::new(tl!("Left:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut l).speed(3.6 / k).range(-792.0 / k..=1584.0 / k).measure(u).max_decimals(2)).changed() {
                let _ = app.run("para.indents", json!({"left": l * k}));
            }
            spaced(ui, egui::RichText::new(tl!("Before:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut b).speed(1.0).range(0.0..=1584.0).suffix(format!(" {}", tl!("pt")))).changed() {
                let _ = app.run("para.spacing", json!({"before": b}));
            }
            ui.end_row();
            let mut r = rp.indent_right / k;
            let mut a = rp.space_after;
            ui.label(egui::RichText::new(tl!("Right:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut r).speed(3.6 / k).range(-792.0 / k..=1584.0 / k).measure(u).max_decimals(2)).changed() {
                let _ = app.run("para.indents", json!({"right": r * k}));
            }
            spaced(ui, egui::RichText::new(tl!("After:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut a).speed(1.0).range(0.0..=1584.0).suffix(format!(" {}", tl!("pt")))).changed() {
                let _ = app.run("para.spacing", json!({"after": a}));
            }
            ui.end_row();
        });
    });
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        wrap_menu(ui, app);
        stack(ui, |ui| {
            small(ui, app, "bringForward", Some("Bring Forward"), "Bring Forward", "arrange.bringForward", json!({}), false);
            small(ui, app, "sendBackward", Some("Send Backward"), "Send Backward", "arrange.sendBackward", json!({}), false);
            small(ui, app, "selectionPane", Some("Selection Pane"), "Selection Pane", "arrange.selectionPane", json!({}), false);
        });
        stack(ui, |ui| {
            align_menu(ui, app, None);
            group_menu(ui, app, None);
            rotate_menu(ui, app, None);
        });
    });
}

/// Arrange › Rotate: turn 90° either way or flip (the angle and mirroring, not the pixels).
/// Arrange › Wrap Text: how the selected object sits in the text, and (Square, Tight, Through)
/// which of its sides text flows on.
fn wrap_menu(ui: &mut Ui, app: &mut WordApp) {
    let (wrap, side) = wordcraft_engine::cmd::objects::selected(&app.session)
        .and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, f.wrap_text)))
        .unwrap_or_default();
    menu_button(ui, app, "wrapText", Some("Wrap\nText"), "Wrap Text", true, "arrange.wrapText", |ui, app| {
        use wordcraft_doc::para::{Wrap, WrapText};
        for (l, w, v) in [
            ("In Line with Text", Wrap::Inline, "inline"),
            ("Square", Wrap::Square, "square"),
            ("Tight", Wrap::Tight, "tight"),
            ("Through", Wrap::Through, "through"),
            ("Top and Bottom", Wrap::TopAndBottom, "topAndBottom"),
            ("Behind Text", Wrap::BehindText, "behindText"),
            ("In Front of Text", Wrap::InFrontOfText, "inFrontOfText"),
        ] {
            mi_check(ui, app, l, wrap == w, "arrange.wrap", json!({"wrap": v}));
        }
        ui.separator();
        ui.menu_button(tl!("Wrap Text Sides"), |ui| {
            for (l, s, v) in [
                ("Both Sides", WrapText::BothSides, "bothSides"),
                ("Left Only", WrapText::Left, "left"),
                ("Right Only", WrapText::Right, "right"),
                ("Largest Only", WrapText::Largest, "largest"),
            ] {
                mi_check(ui, app, l, side == s, "arrange.wrapText", json!({"side": v}));
            }
        });
    });
}

fn rotate_menu(ui: &mut Ui, app: &mut WordApp, label: Option<&str>) {
    menu_button(ui, app, "rotate", label, "Rotate", false, "arrange.rotate", |ui, app| {
        mi(ui, app, "Rotate Right 90°", "arrange.rotate", json!({"direction": "right"}));
        mi(ui, app, "Rotate Left 90°", "arrange.rotate", json!({"direction": "left"}));
        mi(ui, app, "Flip Vertical", "arrange.rotate", json!({"direction": "flipVertical"}));
        mi(ui, app, "Flip Horizontal", "arrange.rotate", json!({"direction": "flipHorizontal"}));
    });
}

/// Size › Rotation: the selected object's exact angle (a drag of the field is one undo step).
fn rotation_field(ui: &mut Ui, app: &mut WordApp) {
    let deg0 = wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| f.spin().deg)).unwrap_or(0.0);
    ui.label(egui::RichText::new(tl!("Rotation:")).small());
    let mut deg = deg0;
    let r = ui.add(egui::DragValue::new(&mut deg).speed(1.0).range(-360.0..=360.0).max_decimals(1).suffix("°"));
    if r.changed() {
        if r.dragged() && !r.drag_started() {
            app.session.join_next_undo();
        }
        let _ = app.run("arrange.rotation", json!({"degrees": deg}));
    }
}

/// Arrange › Group: Group (Shift+click objects to select several) and Ungroup.
fn group_menu(ui: &mut Ui, app: &mut WordApp, label: Option<&str>) {
    menu_button(ui, app, "group", label, "Group", false, "arrange.group", |ui, app| {
        mi(ui, app, "Group", "arrange.group", json!({}));
        mi(ui, app, "Ungroup", "arrange.ungroup", json!({}));
    });
}

/// Arrange › Align: horizontal position relative to the margin (#3242 — a plain button ran
/// `arrange.align` with no `value`, which always centers; a menu reaches Left and Right too).
fn align_menu(ui: &mut Ui, app: &mut WordApp, label: Option<&str>) {
    menu_button(ui, app, "align", label, "Align", false, "arrange.align", |ui, app| {
        mi(ui, app, "Align Left", "arrange.align", json!({"value": "left"}));
        mi(ui, app, "Align Center", "arrange.align", json!({"value": "center"}));
        mi(ui, app, "Align Right", "arrange.align", json!({"value": "right"}));
    });
}

fn references(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table of Contents", None, app, |ui, app| {
        menu_button(ui, app, "toc", Some("Table of\nContents"), "Table of Contents", true, "references.toc", |ui, app| {
            mi(ui, app, "Automatic Table 1 (3 levels)", "references.toc", json!({"levels": 3}));
            mi(ui, app, "Automatic Table 2 (2 levels)", "references.toc", json!({"levels": 2, "title": "Table of Contents"}));
            ui.separator();
            mi(ui, app, "Custom Table of Contents…", "ui.dialog", json!({"name": "toc"}));
            mi(ui, app, "Remove Table of Contents", "references.removeToc", json!({}));
        });
        stack(ui, |ui| {
            menu_button(ui, app, "addText", Some("Add Text"), "Add Text", false, "references.addText", |ui, app| {
                mi(ui, app, "Do Not Show in TOC", "references.addText", json!({"level": 0}));
                for l in 1..=3u64 {
                    mi(ui, app, &format!("Level {l}"), "references.addText", json!({"level": l}));
                }
            });
            small(ui, app, "updateTable", Some("Update Table"), "Update Table", "references.updateToc", json!({}), false);
        });
    });
    group(ui, "Footnotes", Some("references.noteOptions"), app, |ui, app| {
        big(ui, app, "footnote", "Insert\nFootnote", "references.footnote", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "endnote", Some("Insert Endnote"), "Insert Endnote", "references.endnote", json!({}), false);
            small(ui, app, "nextFootnote", Some("Next Footnote"), "Next Footnote", "references.nextFootnote", json!({}), false);
            small(ui, app, "showNotes", Some("Show Notes"), "Show Notes", "references.notes", json!({}), false);
        });
    });
    group(ui, "Research", None, app, |ui, app| {
        big(ui, app, "researcher", "Researcher", "references.researcher", json!({}), false);
    });
    group(ui, "Citations & Bibliography", None, app, |ui, app| {
        // The document's sources to cite, then a new one (Create Source, #402).
        menu_button(ui, app, "citation", Some("Insert\nCitation"), "Insert Citation", true, "", |ui, app| {
            let sources: Vec<(String, String)> =
                app.session.doc.sources.iter().map(|s| (s.tag.clone(), format!("{} — {}", s.author, s.title))).collect();
            for (tag, label) in sources {
                mi_text(ui, app, &label, "references.citation", json!({"tag": tag}));
            }
            if !app.session.doc.sources.is_empty() {
                ui.separator();
            }
            mi(ui, app, "Add New Source…", "ui.dialog", json!({"name": "createSource"}));
        });
        stack(ui, |ui| {
            small(ui, app, "sources", Some("Manage Sources"), "Manage Sources", "references.sources", json!({}), false);
            let current = app.session.bib_style.clone();
            let label = crate::i18n::fmt(tl!("Style: {style}"), &[("style", &current)]);
            menu_button(ui, app, "styles", Some(&label), "Citation Style", false, "references.citationStyle", |ui, app| {
                for st in &wordcraft_engine::cmd::citations::STYLES {
                    mi_check(ui, app, st.name, st.name == current, "references.citationStyle", json!({"style": st.name}));
                }
            });
            small(ui, app, "bibliography", Some("Bibliography"), "Bibliography", "references.bibliography", json!({}), false);
        });
    });
    group(ui, "Captions", None, app, |ui, app| {
        big(ui, app, "caption", "Insert\nCaption", "references.caption", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "tableOfFigures",
                Some("Insert Table of Figures"),
                "Insert Table of Figures",
                "references.tableOfFigures",
                json!({}),
                false,
            );
            small(ui, app, "update", Some("Update Table"), "Update Table of Figures", "references.updateFigures", json!({}), false);
            small(ui, app, "crossRef", Some("Cross-reference"), "Cross-reference", "insert.crossReference", json!({}), false);
        });
    });
    group(ui, "Index", None, app, |ui, app| {
        big(ui, app, "markEntry", "Mark\nEntry", "references.markEntry", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "index", Some("Insert Index"), "Insert Index", "references.index", json!({}), false);
            small(ui, app, "update", Some("Update Index"), "Update Index", "references.updateIndex", json!({}), false);
        });
    });
    group(ui, "Table of Authorities", None, app, |ui, app| {
        big(ui, app, "markCitation", "Mark\nCitation", "references.markCitation", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "tableOfAuthorities",
                Some("Insert Table of Authorities"),
                "Insert Table of Authorities",
                "references.tableOfAuthorities",
                json!({}),
                false,
            );
            small(ui, app, "update", Some("Update Table"), "Update Table", "references.updateAuthorities", json!({}), false);
        });
    });
}

fn mailings(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Create", None, app, |ui, app| {
        big(ui, app, "envelope", "Envelopes", "mailings.envelopes", json!({}), false);
        big(ui, app, "labels", "Labels", "mailings.labels", json!({}), false);
    });
    group(ui, "Start Mail Merge", None, app, |ui, app| {
        // The kinds of merge document the engine makes, the current one checked.
        menu_button(ui, app, "mailMerge", Some("Start Mail\nMerge"), "Start Mail Merge", true, "mailings.start", |ui, app| {
            let kind = app.session.merge.kind.clone();
            for (label, k) in [
                ("Letters", "letters"),
                ("E-mail Messages", "emails"),
                ("Envelopes", "envelopes"),
                ("Labels", "labels"),
                ("Directory", "directory"),
                ("Normal Word Document", "normal"),
            ] {
                let current = kind == k || (k == "normal" && kind.is_empty());
                mi_check(ui, app, label, current, "mailings.start", json!({"kind": k}));
            }
        });
        // The command needs data, so the button offers the two ways to give it (#240).
        menu_button(ui, app, "recipients", Some("Select\nRecipients"), "Select Recipients", true, "mailings.recipients", |ui, app| {
            mi(ui, app, "Type a New List…", "ui.dialog", json!({"name": "newRecipientList"}));
            mi(ui, app, "Use an Existing List…", "ui.openRecipientList", json!({}));
        });
        big(ui, app, "editRecipients", "Edit\nRecipient List", "mailings.editRecipients", json!({}), false);
    });
    group(ui, "Write & Insert Fields", None, app, |ui, app| {
        big(ui, app, "highlightFields", "Highlight\nMerge Fields", "mailings.highlightFields", json!({}), false);
        big(ui, app, "addressBlock", "Address\nBlock", "mailings.addressBlock", json!({}), false);
        big(ui, app, "greetingLine", "Greeting\nLine", "mailings.greetingLine", json!({}), false);
        big(ui, app, "mergeField", "Insert Merge\nField", "mailings.insertField", json!({}), false);
        stack(ui, |ui| {
            // The merge rules the engine knows; If and Skip Record If ask for their condition.
            menu_button(ui, app, "rules", Some("Rules"), "Rules", false, "mailings.rules", |ui, app| {
                mi(ui, app, "If…Then…Else…", "mailings.rules", json!({"rule": "IF"}));
                mi(ui, app, "Merge Record #", "mailings.rules", json!({"rule": "MERGEREC"}));
                mi(ui, app, "Next Record", "mailings.rules", json!({"rule": "NEXT"}));
                mi(ui, app, "Skip Record If…", "mailings.rules", json!({"rule": "SKIPIF"}));
            });
            small(ui, app, "matchFields", Some("Match Fields"), "Match Fields", "mailings.matchFields", json!({}), false);
        });
    });
    group(ui, "Preview Results", None, app, |ui, app| {
        big(ui, app, "preview", "Preview\nResults", "mailings.preview", json!({}), false);
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                small(ui, app, "first", None, "First Record", "mailings.first", json!({}), false);
                small(ui, app, "previous", None, "Previous Record", "mailings.previous", json!({}), false);
                record_box(ui, app);
                small(ui, app, "next", None, "Next Record", "mailings.next", json!({}), false);
                small(ui, app, "last", None, "Last Record", "mailings.last", json!({}), false);
            });
            small(ui, app, "find", Some("Find Recipient"), "Find Recipient", "mailings.findRecipient", json!({}), false);
            small(ui, app, "checkErrors", Some("Check for Errors"), "Check for Errors", "mailings.checkErrors", json!({}), false);
        });
    });
    group(ui, "Finish", None, app, |ui, app| {
        menu_button(ui, app, "finish", Some("Finish &\nMerge"), "Finish & Merge", true, "mailings.finish", |ui, app| {
            mi(ui, app, "Edit Individual Documents…", "mailings.finish", json!({}));
            mi(ui, app, "Print Documents…", "mailings.printDocuments", json!({}));
        });
    });
}

/// Preview Results' record number: shows the previewed record; a number typed and Enter go there.
fn record_box(ui: &mut Ui, app: &mut WordApp) {
    let on = !app.session.merge.rows.is_empty();
    let shown = if on { (app.session.merge.record + 1).to_string() } else { String::new() };
    // The draft lives in temp memory while typing (frame-local text would lose keystrokes).
    let key = egui::Id::new("merge_record_box");
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| shown.clone());
    let r = ui.add_enabled(on, egui::TextEdit::singleline(&mut text).desired_width(36.0).horizontal_align(egui::Align::Center));
    let r = r.on_hover_text(tl!("Go to Record"));
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && text.trim() != shown {
            let _ = app.run("mailings.goToRecord", json!({"record": text.trim()}));
        }
    }
}

/// Zotero (`docs/zotero.md`): the Zotero desktop app does the work in its own window.
fn zotero(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Citations", None, app, |ui, app| {
        big(ui, app, "citation", "Add/Edit\nCitation", "ui.zotero.addEditCitation", json!({}), false);
        big(ui, app, "addNote", "Add\nNote", "ui.zotero.addNote", json!({}), false);
        big(ui, app, "pastCitation", "Move Past\nCitation", "caret.pastCitation", json!({}), false);
    });
    group(ui, "Bibliography", None, app, |ui, app| {
        big(ui, app, "bibliography", "Add/Edit\nBibliography", "ui.zotero.addEditBibliography", json!({}), false);
    });
    group(ui, "Document", None, app, |ui, app| {
        big(ui, app, "update", "Refresh", "ui.zotero.refresh", json!({}), false);
        big(ui, app, "docPrefs", "Document\nPreferences", "ui.zotero.setDocPrefs", json!({}), false);
        big(ui, app, "unlinkCitations", "Unlink\nCitations", "ui.zotero.removeCodes", json!({}), false);
    });
}

fn review(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Proofing", None, app, |ui, app| {
        big(ui, app, "spelling", "Spelling &\nGrammar", "review.spelling", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "thesaurus", Some("Thesaurus"), "Thesaurus", "review.thesaurus", json!({}), false);
            small(ui, app, "wordCount", Some("Word Count"), "Word Count", "ui.dialog", json!({"name": "wordCount"}), false);
        });
    });
    group(ui, "Speech", None, app, |ui, app| {
        big(ui, app, "readAloud", "Read\nAloud", "review.readAloud", json!({}), false);
    });
    group(ui, "Accessibility", None, app, |ui, app| {
        big(ui, app, "accessibility", "Check\nAccessibility", "file.accessibility", json!({}), false);
    });
    group(ui, "Language", None, app, |ui, app| {
        big(ui, app, "translate", "Translate", "review.translate", json!({}), false);
        menu_button(ui, app, "language", Some("Language"), "Language", true, "", language_menu);
    });
    group(ui, "Comments", None, app, |ui, app| {
        big(ui, app, "newComment", "New\nComment", "review.newComment", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "deleteComment", Some("Delete"), "Delete Comment", "review.deleteComment", json!({}), false);
            small(ui, app, "prevComment", Some("Previous"), "Previous Comment", "review.previousComment", json!({}), false);
            small(ui, app, "nextComment", Some("Next"), "Next Comment", "review.nextComment", json!({}), false);
        });
        let shown = app.session.view.comments_pane;
        big(ui, app, "showComments", "Show\nComments", "view.commentsPane", json!({"value": !shown}), false);
    });
    group(ui, "Tracking", Some("review.trackingOptions"), app, |ui, app| {
        let on = app.session.doc.settings.track_changes;
        let r = big(ui, app, "trackChanges", if on { "Track\nChanges ✓" } else { "Track\nChanges" }, "review.trackChanges", json!({}), false);
        let _ = r;
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                "markup",
                Some(if app.session.view.show_markup { "All Markup" } else { "No Markup" }),
                "Display for Review",
                false,
                "review.markup",
                |ui, app| {
                    mi(ui, app, "All Markup", "review.markup", json!({"value": "all"}));
                    mi(ui, app, "No Markup", "review.markup", json!({"value": "noMarkup"}));
                },
            );
            small(ui, app, "reviewingPane", Some("Reviewing Pane"), "Reviewing Pane", "review.changes", json!({}), false);
        });
    });
    group(ui, "Changes", None, app, |ui, app| {
        menu_button(ui, app, "accept", Some("Accept"), "Accept", true, "review.accept", |ui, app| {
            mi(ui, app, "Accept This Change", "review.accept", json!({}));
            mi(ui, app, "Accept All Changes", "review.acceptAll", json!({}));
        });
        menu_button(ui, app, "reject", Some("Reject"), "Reject", true, "review.reject", |ui, app| {
            mi(ui, app, "Reject Change", "review.reject", json!({}));
            mi(ui, app, "Reject All Changes", "review.rejectAll", json!({}));
        });
        stack(ui, |ui| {
            small(ui, app, "prevChange", Some("Previous"), "Previous Change", "review.previousChange", json!({}), false);
            small(ui, app, "nextChange", Some("Next"), "Next Change", "review.nextChange", json!({}), false);
        });
    });
    group(ui, "Compare", None, app, |ui, app| {
        big(ui, app, "compare", "Compare", "review.compare", json!({}), false);
    });
    group(ui, "Protect", None, app, |ui, app| {
        big(ui, app, "blockAuthors", "Block\nAuthors", "review.blockAuthors", json!({}), false);
        big(ui, app, "restrict", "Restrict\nEditing", "review.restrict", json!({}), false);
    });
    group(ui, "Ink", None, app, |ui, app| {
        let hidden = app.session.view.hide_ink;
        big_toggle(ui, app, "hideInk", "Hide\nInk", "review.hideInk", hidden);
    });
}

/// Developer › Controls: content controls.
fn developer(app: &mut WordApp, ui: &mut Ui) {
    let design = app.session.view.design_mode;
    group(ui, "Controls", None, app, |ui, app| {
        let rows: [[(&str, &str, &str); 3]; 3] = [
            [
                ("ccRichText", "Rich Text Content Control", "developer.richText"),
                ("ccPlainText", "Plain Text Content Control", "developer.plainText"),
                ("ccPicture", "Picture Content Control", "developer.picture"),
            ],
            [
                ("ccGallery", "Building Block Gallery Content Control", "developer.buildingBlock"),
                ("ccCheckBox", "Check Box Content Control", "developer.checkBox"),
                ("ccComboBox", "Combo Box Content Control", "developer.comboBox"),
            ],
            [
                ("ccDropDown", "Drop-Down List Content Control", "developer.dropDown"),
                ("ccDate", "Date Picker Content Control", "developer.datePicker"),
                ("ccRepeat", "Repeating Section Content Control", "developer.repeatingSection"),
            ],
        ];
        // Three rows of small buttons fill the group's height, like every other stack.
        stack(ui, |ui| {
            for row in rows {
                crate::widgets::row(ui, |ui| {
                    for (icon, tip, id) in row {
                        small(ui, app, icon, None, tip, id, json!({}), false);
                    }
                });
            }
        });
        stack(ui, |ui| {
            small(ui, app, "designMode", Some("Design Mode"), "Design Mode", "developer.designMode", json!({}), design);
            small(ui, app, "properties", Some("Properties"), "Properties", "developer.properties", json!({}), false);
        });
    });
}

fn view(app: &mut WordApp, ui: &mut Ui) {
    let v = app.session.view.clone();
    group(ui, "Views", None, app, |ui, app| {
        big(ui, app, "readMode", "Read\nMode", "view.readMode", json!({}), false);
        big(ui, app, "printLayout", "Print\nLayout", "view.printLayout", json!({}), false);
        big(ui, app, "webLayout", "Web\nLayout", "view.webLayout", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "outline", Some("Outline"), "Outline", "view.outline", json!({}), false);
            small(ui, app, "draft", Some("Draft"), "Draft", "view.draft", json!({}), false);
        });
    });
    group(ui, "Immersive", None, app, |ui, app| {
        big(ui, app, "focus", "Focus", "view.focus", json!({}), false);
        big(ui, app, "immersive", "Immersive\nReader", "view.immersive", json!({}), false);
    });
    group(ui, "Page Movement", None, app, |ui, app| {
        big(ui, app, "vertical", "Vertical", "view.vertical", json!({}), false);
        big(ui, app, "sideToSide", "Side\nto Side", "view.sideToSide", json!({}), false);
    });
    group(ui, "Show", None, app, |ui, app| {
        // Three rows fill the group's height; a fourth check box would run into its label.
        stack(ui, |ui| {
            let mut r = v.ruler;
            if ui.checkbox(&mut r, tl!("Ruler")).changed() {
                let _ = app.run("view.ruler", json!({"value": r}));
            }
            let mut g = v.gridlines;
            if ui.checkbox(&mut g, tl!("Gridlines")).changed() {
                let _ = app.run("view.gridlines", json!({"value": g}));
            }
            let mut n = v.nav_pane;
            if ui.checkbox(&mut n, tl!("Navigation Pane")).changed() {
                let _ = app.run("view.navigationPane", json!({"value": n}));
            }
        });
        stack(ui, |ui| {
            let mut w = v.hide_white_space;
            if ui.checkbox(&mut w, tl!("Hide White Space")).changed() {
                let _ = app.run("view.hideWhiteSpace", json!({"value": w}));
            }
        });
    });
    group(ui, "Zoom", None, app, |ui, app| {
        big(ui, app, "zoom", "Zoom", "ui.dialog", json!({"name": "zoom"}), false);
        big(ui, app, "zoom100", "100%", "view.zoom100", json!({}), false);
        // Step the zoom up and down by 10% (issue #67), from whatever the page shows now.
        stack(ui, |ui| {
            small(ui, app, "zoomIn", Some("Zoom In"), "Zoom In", "view.zoomIn", json!({}), false);
            small(ui, app, "zoomOut", Some("Zoom Out"), "Zoom Out", "view.zoomOut", json!({}), false);
        });
        stack(ui, |ui| {
            small(ui, app, "onePage", Some("One Page"), "One Page", "view.onePage", json!({}), v.fit == "onePage");
            small(ui, app, "multiplePages", Some("Multiple Pages"), "Multiple Pages", "view.multiplePages", json!({}), v.multi_page);
            small(ui, app, "pageWidth", Some("Page Width"), "Page Width", "view.pageWidth", json!({}), v.fit == "pageWidth");
        });
    });
    group(ui, "Dark Mode", None, app, |ui, app| {
        big(ui, app, "darkMode", "Switch\nModes", "view.darkMode", json!({}), false);
        menu_button(ui, app, "interfaceTheme", Some("Interface\nTheme"), "Interface Theme", true, "", |ui, app| {
            for a in crate::theme::Appearance::ALL {
                if ui.add(egui::Button::selectable(app.ui.theme == a, tl!(a.label())).min_size(vec2(200.0, 0.0))).clicked() {
                    let _ = app.run("view.interfaceTheme", json!({"value": a.code()}));
                    ui.close();
                }
            }
        });
    });
    group(ui, "Window", None, app, |ui, app| {
        big(ui, app, "newWindow", "New\nWindow", "view.newWindow", json!({}), false);
        big(ui, app, "arrangeAll", "Arrange\nAll", "view.arrangeAll", json!({}), false);
        big(ui, app, "split", "Split", "view.split", json!({}), false);
        // The Full Screen button carries its name for assistive technology (and for the
        // regression test that clicks it): big buttons are otherwise drawn unlabeled. Its
        // enabled state is the command's own, as every other ribbon control reports: the
        // browser has no full screen to ask for, and says so (#662).
        let on = crate::widgets::enabled(app, "view.fullScreen");
        let resp = big_toggle(ui, app, "fullScreen", "Full\nScreen", "view.fullScreen", v.full_screen);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, on, v.full_screen, "Full Screen"));
    });
    group(ui, "Macros", None, app, |ui, app| {
        big(ui, app, "macros", "Macros", "tools.macros", json!({}), false);
    });
    group(ui, "Templates", None, app, |ui, app| {
        big(ui, app, "documentTemplate", "Templates", "tools.templates", json!({}), false);
    });
}

/// The Outlining tab (Outline view): outline levels, moving, expanding and collapsing, what is
/// shown, and closing the view.
fn outlining(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_engine::layout::outline::{BODY, para_level};
    let o = app.session.view.outline.clone();
    let level = app.session.doc.para_at(&app.session.sel.focus).map(|p| para_level(&app.session.doc, p)).unwrap_or(BODY);
    let name = |l: u8| if l >= BODY { tl!("Body Text").to_string() } else { crate::i18n::fmt(tl!("Level {n}"), &[("n", &l.to_string())]) };
    group(ui, "Outline Tools", None, app, |ui, app| {
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                small(ui, app, "outlineTop", None, "Promote to Heading 1", "outline.promoteToHeading1", json!({}), false);
                small(ui, app, "outlinePromote", None, "Promote", "outline.promote", json!({}), false);
                let levels: Vec<String> = (1..=BODY).map(name).collect();
                if let Some(v) = crate::widgets::combo_hint(ui, "outlineLevel", 100.0, &name(level), &levels, None, tl!("Outline Level"))
                    && let Some(i) = levels.iter().position(|x| *x == v)
                {
                    let l = if i + 1 >= usize::from(BODY) { json!("body") } else { json!(i + 1) };
                    let _ = app.run("outline.level", json!({ "level": l }));
                }
                small(ui, app, "outlineDemote", None, "Demote", "outline.demote", json!({}), false);
                small(ui, app, "outlineBody", None, "Demote to Body Text", "outline.demoteToBody", json!({}), false);
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                small(ui, app, "outlineUp", None, "Move Up", "outline.moveUp", json!({}), false);
                small(ui, app, "outlineDown", None, "Move Down", "outline.moveDown", json!({}), false);
                small(ui, app, "outlineExpand", None, "Expand", "outline.expand", json!({}), false);
                small(ui, app, "outlineCollapse", None, "Collapse", "outline.collapse", json!({}), false);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("Show Level")).size(11.5));
                let mut shown: Vec<String> = vec![tl!("All Levels").to_string()];
                shown.extend((1..BODY).map(name));
                let current = shown.get(usize::from(o.show_level)).cloned().unwrap_or_default();
                if let Some(v) = crate::widgets::combo_hint(ui, "outlineShowLevel", 90.0, &current, &shown, None, tl!("Show Level"))
                    && let Some(i) = shown.iter().position(|x| *x == v)
                {
                    let l = if i == 0 { json!("all") } else { json!(i) };
                    let _ = app.run("outline.showLevel", json!({ "level": l }));
                }
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                let mut f = !o.hide_formatting;
                if ui.checkbox(&mut f, tl!("Show Text Formatting")).changed() {
                    let _ = app.run("outline.showFormatting", json!({ "value": f }));
                }
                let mut first = o.first_line_only;
                if ui.checkbox(&mut first, tl!("Show First Line Only")).changed() {
                    let _ = app.run("outline.firstLineOnly", json!({ "value": first }));
                }
            });
        });
    });
    group(ui, "Close", None, app, |ui, app| {
        big(ui, app, "closeOutline", "Close Outline\nView", "outline.close", json!({}), false);
    });
}

fn help(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Help", None, app, |ui, app| {
        big(ui, app, "help", "Help", "ui.dialog", json!({"name": "about"}), false);
        big(ui, app, "discord", "Community\nDiscord", "ui.discord", json!({}), false);
    });
    group(ui, "Agents", None, app, |ui, app| {
        big(ui, app, "macros", "Commands", "ui.dialog", json!({"name": "commands"}), false);
    });
}

/// SmartArt Design: the selected graphic's items (Add Shape, Promote, Demote, Text Pane), its
/// layout (a gallery of previews), its colours and Reset.
fn smart_art_design(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_doc::smart_art::{SmartArtColors, SmartArtLayout};
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return };
    let theme = app.session.doc.settings.theme_colors.clone();
    // The Text Pane's current item, else the last.
    let current = app.smart_art_item.filter(|i| *i < g.items.len());
    group(ui, "Create Graphic", None, app, |ui, app| {
        big(ui, app, "addShape", "Add\nShape", "smartArt.addShape", current.map_or(json!({}), |i| json!({"after": i})), false);
        stack(ui, |ui| {
            let idx = current.map_or(json!({}), |i| json!({"index": i}));
            small(ui, app, "promote", Some("Promote"), "Promote", "smartArt.promote", idx.clone(), false);
            small(ui, app, "demote", Some("Demote"), "Demote", "smartArt.demote", idx, false);
            small(ui, app, "textPane", Some("Text Pane"), "Text Pane", "smartArt.textPane", json!({}), app.session.view.smart_art_pane);
        });
    });
    group(ui, "Layouts", None, app, |ui, app| {
        let t = Tokens::get(ui.ctx());
        egui::Grid::new("smart_art_layout_gallery").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for (k, layout) in SmartArtLayout::ALL.into_iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(46.0, 31.0), Sense::click());
                let on = g.layout == layout;
                let fill = if on {
                    t.checked
                } else if resp.hovered() {
                    t.hover
                } else {
                    t.ribbon
                };
                ui.painter().rect(r, 3.0, fill, Stroke::new(1.0, if on { t.accent } else { t.border }), egui::StrokeKind::Inside);
                crate::dialogs_smart_art::preview(ui.painter(), r.shrink(2.0), layout, g.colors, &theme, t.text_dim);
                if resp.on_hover_text(tl!(layout.label())).clicked() && !on {
                    let _ = app.run("smartArt.layout", json!({"layout": layout.id()}));
                }
                if k % 4 == 3 {
                    ui.end_row();
                }
            }
        });
    });
    group(ui, "SmartArt Styles", None, app, |ui, app| {
        menu_button(ui, app, "smartArtColors", Some("Change\nColors"), "Change Colors", true, "", |ui, app| {
            for colors in SmartArtColors::ALL {
                let on = g.colors == colors;
                let resp = ui.add(egui::Button::new(crate::dialogs_smart_art::colors_label(colors)).selected(on).min_size(vec2(200.0, 24.0)));
                let sw = Rect::from_min_size(pos2(resp.rect.right() - 74.0, resp.rect.center().y - 6.0), vec2(66.0, 12.0));
                crate::dialogs_smart_art::swatches(ui.painter(), sw, colors, &theme);
                if resp.clicked() {
                    let _ = app.run("smartArt.colors", json!({"variant": colors.id()}));
                    ui.close();
                }
            }
        });
    });
    group(ui, "Reset", None, app, |ui, app| {
        big(ui, app, "resetGraphic", "Reset\nGraphic", "smartArt.reset", json!({}), false);
    });
}

/// Chart Design: the selected chart's elements (title, legend, data labels), its data and type,
/// and its size.
fn chart_design(app: &mut WordApp, ui: &mut Ui) {
    let Some(chart) = wordcraft_engine::cmd::chart::selected_chart(&app.session) else { return };
    group(ui, "Chart Layouts", None, app, |ui, app| {
        menu_button(ui, app, "chartTitle", Some("Chart Title"), "Chart Title", true, "", |ui, app| {
            mi_check(ui, app, "None", chart.title.is_none(), "chart.title", json!({"text": null}));
            let text = chart.title.clone().unwrap_or_else(|| tl!("Chart Title").to_string());
            mi_check(ui, app, "Above Chart", chart.title.is_some(), "chart.title", json!({"text": text}));
        });
        menu_button(ui, app, "legend", Some("Legend"), "Legend", true, "", |ui, app| {
            mi_check(ui, app, "None", chart.legend.is_none(), "chart.legend", json!({"pos": "none"}));
            for (label, pos) in [
                ("Right", wordcraft_doc::chart::LegendPos::Right),
                ("Top", wordcraft_doc::chart::LegendPos::Top),
                ("Left", wordcraft_doc::chart::LegendPos::Left),
                ("Bottom", wordcraft_doc::chart::LegendPos::Bottom),
            ] {
                mi_check(ui, app, label, chart.legend == Some(pos), "chart.legend", json!({"pos": pos.id()}));
            }
        });
        menu_button(ui, app, "dataLabels", Some("Data Labels"), "Data Labels", true, "", |ui, app| {
            mi_check(ui, app, "None", !chart.data_labels, "chart.dataLabels", json!({"on": false}));
            mi_check(ui, app, "Show", chart.data_labels, "chart.dataLabels", json!({"on": true}));
        });
    });
    group(ui, "Data", None, app, |ui, app| {
        big(ui, app, "chartData", "Edit\nData", "chart.editData", json!({}), false);
    });
    group(ui, "Type", None, app, |ui, app| {
        big(ui, app, "changeChartType", "Change\nChart Type", "chart.type", json!({}), false);
    });
    group(ui, "Size", None, app, |ui, app| {
        let (w0, h0) =
            wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h))).unwrap_or((0.0, 0.0));
        stack(ui, |ui| {
            for (label, key, v0) in [("H:", "height", h0), ("W:", "width", w0)] {
                crate::widgets::row(ui, |ui| {
                    ui.label(egui::RichText::new(tl!(label)).small());
                    let mut v = v0;
                    // The range bounds what is typed or dragged; a size outside it is shown as it
                    // is (clamped, egui would report a change and resize the chart unasked).
                    let r = ui.add(egui::DragValue::new(&mut v).speed(1.0).range(36.0..=2000.0).clamp_existing_to_range(false).suffix(" pt"));
                    if r.changed() && v != v0 {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let _ = app.run("picture.size", json!({key: v, "lockAspect": false}));
                    }
                });
            }
        });
    });
}

/// Shape Fill › Gradient: No Gradient, Light and Dark Variations of the shape's colour, More
/// Gradients… (the Format Shape pane).
pub(crate) fn gradient_menu(ui: &mut Ui, app: &mut WordApp, fill: Option<&wordcraft_doc::fill::Fill>) {
    use wordcraft_engine::cmd::shape_fill;
    let pos = wordcraft_engine::cmd::objects::selected(&app.session).map(|(p, _)| p);
    let main = match (fill, &pos) {
        (Some(f), Some(p)) => crate::format_shape::variation_base(ui.ctx(), p, f),
        (Some(f), None) => f.main_color(),
        _ => app.session.doc.settings.theme_colors.get(4).copied().unwrap_or(wordcraft_doc::Rgb(68, 114, 196)),
    };
    // No Gradient: the shape's colour, solid.
    ui.label(egui::RichText::new(tl!("No Gradient")).small().weak());
    if crate::format_shape::swatch(ui, &wordcraft_doc::fill::Fill::solid(main), "No Gradient") {
        let _ = app.run("shape.fill", json!({"color": main.hex()}));
        ui.close();
    }
    for (title, tiles) in ["Light Variations", "Dark Variations"].into_iter().zip(shape_fill::variations(main)) {
        ui.label(egui::RichText::new(tl!(title)).small().weak());
        egui::Grid::new(title).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
            for (i, (f, name)) in tiles.iter().enumerate() {
                if crate::format_shape::swatch(ui, f, name) {
                    let _ = app.run("shape.fill", shape_fill::params(Some(f)));
                    if let (Some(p), Some(Some(now))) = (pos.clone(), crate::format_shape::current(app)) {
                        crate::format_shape::remember_variation(ui.ctx(), p, now, main);
                    }
                    ui.close();
                }
                if i % 5 == 4 {
                    ui.end_row();
                }
            }
        });
    }
    ui.separator();
    if ui.button(tl!("More Gradients…")).clicked() {
        let _ = app.run("view.formatShapePane", json!({"value": true}));
        ui.close();
    }
}

/// Shape Format: fill, outline and effects of the selected shape, and its arrangement.
fn shape_format(app: &mut WordApp, ui: &mut Ui) {
    let (stroke, glow_size, fill) = match wordcraft_engine::cmd::objects::selected(&app.session) {
        Some((_, wordcraft_doc::para::InlineObject::Shape { stroke, effects, fill, .. })) => (stroke, effects.glow.map(|g| g.size), fill),
        Some((_, o)) => (None, None, o.canvas_style().and_then(|c| c.fill.clone())),
        _ => (None, None, None),
    };
    let theme = app.session.doc.settings.theme_colors.clone();
    group(ui, "Shape Styles", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "shading", Some("Shape Fill"), "Shape Fill", false, "shape.fill", |ui, app| {
                mi(ui, app, "No Color", "shape.fill", json!({"color": null}));
                crate::colors::menu(ui, app, &theme, Target::cmd("shape.fill"));
                ui.separator();
                ui.menu_button(tl!("Gradient"), |ui| gradient_menu(ui, app, fill.as_ref()));
            });
            menu_button(ui, app, "pen", Some("Shape Outline"), "Shape Outline", false, "shape.outline", |ui, app| {
                mi(ui, app, "No Color", "shape.outline", json!({"color": null}));
                crate::colors::menu(ui, app, &theme, Target::cmd("shape.outline"));
                ui.separator();
                ui.menu_button(tl!("Weight"), |ui| {
                    let color = stroke.unwrap_or(wordcraft_doc::Rgb::BLACK).hex();
                    for w in [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0] {
                        mi(ui, app, &format!("{w} pt"), "shape.outline", json!({"color": color, "width": w}));
                    }
                });
            });
            menu_button(ui, app, "shapeEffects", Some("Shape Effects"), "Shadow, glow and soft edges", false, "shape.effects", |ui, app| {
                shape_effects_menu(ui, app, &theme, glow_size)
            });
        });
    });
    alt_text_group(ui, app);
    if wordcraft_engine::cmd::canvas::selected_canvas(&app.session).is_some() {
        group(ui, "Drawing Canvas", None, app, |ui, app| {
            menu_button(ui, app, "canvas", Some("Drawing\nCanvas"), "Drawing Canvas", true, "", |ui, app| {
                mi(ui, app, "Fit", "canvas.fit", json!({}));
                mi(ui, app, "Expand", "canvas.expand", json!({}));
                ui.menu_button(tl!("Scale Drawing"), |ui| {
                    for pct in [50, 75, 125, 150, 200] {
                        mi(ui, app, &format!("{pct}%"), "canvas.scale", json!({"factor": pct as f32 / 100.0}));
                    }
                });
            });
        });
    }
    group(ui, "WordArt Styles", None, app, |ui, app| {
        menu_button(ui, app, "wordArt", Some("Styles"), "WordArt Styles", true, "", |ui, app| {
            crate::wordart_menus::style_gallery(ui, app, "wordArt.style")
        });
        stack(ui, |ui| {
            menu_button(ui, app, "textFill", Some("Text Fill"), "Text Fill", false, "", |ui, app| crate::wordart_menus::fill_menu(ui, app, &theme));
            let outline = crate::wordart_menus::current_outline(app);
            menu_button(ui, app, "textOutline", Some("Text Outline"), "Text Outline", false, "", |ui, app| {
                crate::wordart_menus::outline_menu(ui, app, &theme, outline)
            });
            menu_button(ui, app, "transform", Some("Text Effects"), "Text Effects", false, "", |ui, app| {
                crate::wordart_menus::effects_menu(ui, app, &theme, true)
            });
        });
    });
    shape_text_group(app, ui);
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        wrap_menu(ui, app);
        stack(ui, |ui| {
            rotate_menu(ui, app, Some("Rotate"));
            align_menu(ui, app, Some("Align"));
        });
    });
    group(ui, "Size", None, app, |ui, app| {
        let (w0, h0) =
            wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h))).unwrap_or((0.0, 0.0));
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                for (label, key, v0) in [("W:", "width", w0), ("H:", "height", h0)] {
                    ui.label(egui::RichText::new(tl!(label)).small());
                    let mut v = v0;
                    // `picture.size`'s own range; a size outside it (a thin line) is shown as it
                    // is (clamped, egui would report a change and resize the shape unasked).
                    let r = ui.add(egui::DragValue::new(&mut v).speed(1.0).range(1.0..=4000.0).clamp_existing_to_range(false).suffix(" pt"));
                    if r.changed() && v != v0 {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let _ = app.run("picture.size", json!({key: v, "lockAspect": false}));
                    }
                }
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| rotation_field(ui, app));
        });
    });
}

/// Shape Format › Text: the text box's Text Direction and Align Text, and Create Link (Break Link
/// once it is linked to another box).
fn shape_text_group(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_doc::props::{TextVert, VAlign};
    let target = wordcraft_engine::cmd::objects::text_box_target(&app.session);
    let body = target.and_then(|(_, id)| app.session.doc.parts.get(&id).map(|p| p.body)).unwrap_or_default();
    group(ui, "Text", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "textDirection", Some("Text Direction"), "Text Direction", false, "", |ui, app| {
                for (label, vert) in [
                    ("Horizontal", TextVert::Horz),
                    ("Rotate all text 90°", TextVert::Vert),
                    ("Rotate all text 270°", TextVert::Vert270),
                    ("Stacked", TextVert::Stacked),
                ] {
                    mi_check(ui, app, label, body.vert == vert, "shape.textDirection", json!({"dir": vert.ooxml()}));
                }
            });
            menu_button(ui, app, "alignText", Some("Align Text"), "Align Text", false, "", |ui, app| {
                for (label, anchor) in [("Top", VAlign::Top), ("Middle", VAlign::Center), ("Bottom", VAlign::Bottom)] {
                    mi_check(ui, app, label, body.anchor == anchor, "shape.alignText", json!({"anchor": wordcraft_doc::props::anchor_ooxml(anchor)}));
                }
            });
            if body.next.is_some() {
                small(ui, app, "textBoxUnlink", Some("Break Link"), "Break Link", "shape.breakLink", json!({}), false);
            } else {
                let waiting = app.canvas.link_from.is_some();
                small(ui, app, "textBoxLink", Some("Create Link"), "Create Link", "ui.linkTextBox", json!({}), waiting);
            }
        });
    });
}

/// Picture/Shape Format › Accessibility: Alt Text opens the Alt Text pane.
fn alt_text_group(ui: &mut Ui, app: &mut WordApp) {
    group(ui, "Accessibility", None, app, |ui, app| {
        big(ui, app, "altText", "Alt\nText", "view.altTextPane", json!({"value": true}), false);
    });
}

/// Shape Format › Shape Effects: Shadow, Glow and Soft Edges, each with presets and a "No …" entry.
fn shape_effects_menu(ui: &mut Ui, app: &mut WordApp, theme: &[wordcraft_doc::Rgb], glow_size: Option<f32>) {
    ui.menu_button(tl!("Shadow"), |ui| {
        mi(ui, app, "No Shadow", "shape.effects", json!({"shadow": null}));
        ui.separator();
        for (id, label) in wordcraft_doc::effects::SHADOW_PRESETS {
            mi(ui, app, label, "shape.effects", json!({"shadow": id}));
        }
    });
    ui.menu_button(tl!("Glow"), |ui| {
        mi(ui, app, "No Glow", "shape.effects", json!({"glow": null}));
        ui.separator();
        for size in [5.0, 8.0, 11.0, 18.0] {
            mi(ui, app, &format!("{size} pt"), "shape.effects", json!({"glow": size}));
        }
        ui.separator();
        ui.menu_button(tl!("Glow Colors"), |ui| {
            let target = Target::at("shape.effects", json!({"glow": {"color": null, "size": glow_size.unwrap_or(8.0)}}), "/glow/color");
            crate::colors::menu(ui, app, theme, target);
        });
    });
    ui.menu_button(tl!("Soft Edges"), |ui| {
        mi(ui, app, "No Soft Edges", "shape.effects", json!({"softEdge": null}));
        ui.separator();
        for r in [1.0, 2.5, 5.0, 10.0, 25.0, 50.0] {
            mi(ui, app, &format!("{r} pt"), "shape.effects", json!({"softEdge": r}));
        }
    });
}

fn picture_format(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Adjust", None, app, |ui, app| {
        menu_button(ui, app, "picture", Some("Corrections"), "Brightness, contrast and sharpness", true, "picture.corrections", |ui, app| {
            for (l, b, c) in
                [("Brighter +20%", 20.0, 0.0), ("Darker −20%", -20.0, 0.0), ("More Contrast +20%", 0.0, 20.0), ("Less Contrast −20%", 0.0, -20.0)]
            {
                mi(ui, app, l, "picture.corrections", json!({"brightness": b, "contrast": c}));
            }
            ui.separator();
            mi(ui, app, "Sharpen", "picture.corrections", json!({"sharpen": 40.0}));
            mi(ui, app, "Soften", "picture.corrections", json!({"sharpen": -40.0}));
        });
        menu_button(ui, app, "picture", Some("Color"), "Saturation and recolor", true, "picture.color", |ui, app| {
            for (l, v) in [("Full Saturation 100%", 100.0), ("Muted 50%", 50.0), ("Gray 0%", 0.0), ("Vivid 200%", 200.0)] {
                mi(ui, app, l, "picture.color", json!({"mode": "saturation", "saturation": v}));
            }
            ui.separator();
            mi(ui, app, "Grayscale", "picture.color", json!({"mode": "grayscale"}));
            mi(ui, app, "Sepia", "picture.color", json!({"mode": "sepia"}));
            mi(ui, app, "Washout", "picture.color", json!({"mode": "washout"}));
        });
        menu_button(ui, app, "icons", Some("Icon Color"), "Recolor a one-color picture, such as an icon", true, "picture.iconColor", |ui, app| {
            let theme = app.session.doc.settings.theme_colors.clone();
            crate::colors::menu(ui, app, &theme, Target::cmd("picture.iconColor"));
        });
        menu_button(ui, app, "picture", Some("Transparency"), "Picture transparency", true, "picture.transparency", |ui, app| {
            for v in [0.0, 15.0, 30.0, 50.0, 65.0, 80.0, 95.0] {
                mi(ui, app, &format!("{v:.0}%"), "picture.transparency", json!({"percent": v}));
            }
            ui.separator();
            // An exact value, dragged with a live preview: absolute from the original
            // pixels, one undo step for the whole drag (same pattern as Size › W/H).
            // The current value isn't stored anywhere, so the slider reopens where it
            // was last left for this picture.
            let (media, off) = match wordcraft_engine::cmd::objects::selected(&app.session) {
                Some((pos, wordcraft_doc::para::InlineObject::Image { media, .. })) => (media, pos.off),
                _ => (String::new(), 0),
            };
            let key = egui::Id::new(("picture_transparency", media, off));
            let mut v = ui.data(|d| d.get_temp::<f64>(key)).unwrap_or(0.0);
            let r = ui.add(egui::DragValue::new(&mut v).speed(1.0).range(0.0..=100.0).suffix("%"));
            if r.changed() {
                ui.data_mut(|d| d.insert_temp(key, v));
                if r.dragged() && !r.drag_started() {
                    app.session.join_next_undo();
                }
                let _ = app.run("picture.transparency", json!({"percent": v}));
            }
        });
        stack(ui, |ui| {
            small(ui, app, "picture", Some("Change"), "Change Picture", "ui.changePicture", json!({}), false);
            small(ui, app, "picture", Some("Reset"), "Reset Picture", "picture.reset", json!({}), false);
        });
    });
    group(ui, "Picture Styles", None, app, |ui, app| {
        menu_button(ui, app, "picture", Some("Styles"), "Frames and effects", true, "picture.style", |ui, app| {
            for (l, s) in [
                ("Simple Frame", "simpleFrame"),
                ("Thick Frame", "thickFrame"),
                ("Rounded", "rounded"),
                ("Soft Edge", "softEdge"),
                ("Shadow", "shadow"),
            ] {
                mi(ui, app, l, "picture.style", json!({"style": s}));
            }
        });
        menu_button(ui, app, "picture", Some("Border"), "Picture border", true, "picture.border", |ui, app| {
            for (l, w) in [("Thin", 2), ("Medium", 4), ("Thick", 8)] {
                mi(ui, app, l, "picture.border", json!({"width": w}));
            }
        });
    });
    alt_text_group(ui, app);
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        wrap_menu(ui, app);
        stack(ui, |ui| {
            rotate_menu(ui, app, Some("Rotate"));
            align_menu(ui, app, Some("Align"));
            group_menu(ui, app, Some("Group"));
        });
    });
    // The launcher opens the Size dialog: scale, Lock aspect ratio, the original size (#2573).
    group(ui, "Size", Some("picture.size"), app, |ui, app| {
        let (media, off, w0, h0, crop0, alt0) = match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((pos, wordcraft_doc::para::InlineObject::Image { media, w, h, alt, crop, .. })) => (media, pos.off, w, h, crop, alt),
            _ => (String::new(), 0, 0.0, 0.0, [0.0; 4], String::new()),
        };
        // `picture.size` locks the aspect ratio unless told otherwise, so one dimension suffices.
        // The fields show the interface unit and take a size typed in any unit, pixels too
        // (`300 px`, #1901), applied once typed (Enter, Tab or a click elsewhere; Esc cancels),
        // never per keystroke; a drag applies as it goes, one undo step.
        let unit = crate::options::unit();
        let k = unit.pt_per_unit();
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                for (label, key, pt) in [("W:", "width", w0), ("H:", "height", h0)] {
                    let name = ui.label(egui::RichText::new(tl!(label)).small());
                    let was = pt / k;
                    let mut v = was;
                    // Not `clamp_existing_to_range`: egui would clamp a size outside the range (a 1 px
                    // picture is 0.75 pt) and report it as a change, resizing the picture unasked.
                    let field = egui::DragValue::new(&mut v)
                        .speed(1.0 / k)
                        .range(1.0 / k..=4000.0 / k)
                        .clamp_existing_to_range(false)
                        .measure(unit)
                        .max_decimals(2);
                    let r = ui.add(field.update_while_editing(false)).labelled_by(name.id).on_hover_text(tl!(
                        "Type a size in any unit, such as 5 cm or 300 px. The other side follows, so the picture keeps its proportions."
                    ));
                    // While typing, the field reports each keystroke as a change; only a new
                    // value (Enter, Tab, a click elsewhere, a drag) resizes, and Esc cancels.
                    let cancelled = esc_cancelled(ui, &r, egui::Id::new(("picture_size_field", key)));
                    if r.changed() && v != was && !cancelled {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let _ = app.run("picture.size", json!({key: v * k}));
                    }
                }
                rotation_field(ui, app);
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                let cropping = crate::crop::active(app).is_some();
                small(ui, app, "crop", Some("Crop"), "Crop", "ui.crop", json!({}), cropping);
                for (i, side) in ["L", "T", "R", "B"].iter().enumerate() {
                    ui.label(egui::RichText::new(tl!(side)).small());
                    let mut v = crop0[i] * 100.0;
                    let r = ui.add(egui::DragValue::new(&mut v).speed(0.5).range(0.0..=95.0).suffix("%"));
                    if r.changed() {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let mut c = crop0;
                        c[i] = (v / 100.0).clamp(0.0, 0.95);
                        // The two sides of an axis keep 5% of the source between them.
                        let other = c[if i < 2 { i + 2 } else { i - 2 }];
                        c[i] = c[i].min((0.95 - other).max(0.0));
                        let _ = app.run("picture.crop", json!({"left": c[0], "top": c[1], "right": c[2], "bottom": c[3]}));
                    }
                }
                if ui.button(egui::RichText::new(tl!("Reset")).small()).on_hover_text(tl!("Reset Crop")).clicked() {
                    let _ = app.run("picture.crop", json!({"left": 0.0, "top": 0.0, "right": 0.0, "bottom": 0.0}));
                }
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                ui.label(egui::RichText::new(tl!("Alt:")).small());
                // Frame-local buffers lose keystrokes; keep the draft in egui temp memory keyed
                // by picture (same pattern as the comment editor in panes.rs).
                let key = egui::Id::new(("picture_alt", media.clone(), off));
                let mut alt = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| alt0.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut alt).desired_width(120.0).hint_text(tl!("Alt text")));
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(key, alt.clone()));
                }
                if r.lost_focus() {
                    ui.data_mut(|d| d.remove::<String>(key));
                    // The click that took focus may have selected another picture; never commit
                    // one picture's draft onto another.
                    if alt != alt0 && app.selected_picture_media().as_deref() == Some(media.as_str()) {
                        let _ = app.run("picture.altText", json!({"text": alt}));
                    }
                }
            });
        });
    });
}

fn table_design(app: &mut WordApp, ui: &mut Ui) {
    let look = app.session.sel.focus.path.cell().and_then(|(tp, _, _)| app.session.doc.table(app.session.sel.focus.story, &tp).map(|t| t.props.look));
    group(ui, "Table Style Options", None, app, |ui, app| {
        let Some(l) = look else { return };
        egui::Grid::new("look").show(ui, |ui| {
            for (row, items) in [
                [("Header Row", "headerRow", l.header_row), ("First Column", "firstColumn", l.first_column)],
                [("Total Row", "totalRow", l.total_row), ("Last Column", "lastColumn", l.last_column)],
                [("Banded Rows", "bandedRows", l.banded_rows), ("Banded Columns", "bandedColumns", l.banded_columns)],
            ]
            .iter()
            .enumerate()
            {
                let _ = row;
                for (label, key, val) in items {
                    let mut v = *val;
                    if ui.checkbox(&mut v, tl!(label)).changed() {
                        let _ = app.run("table.look", json!({ *key: v }));
                    }
                }
                ui.end_row();
            }
        });
    });
    group(ui, "Table Styles", None, app, |ui, app| {
        // The document's own styles first, so a new one shows without scrolling.
        let mut styles: Vec<(bool, String, String)> = app
            .session
            .doc
            .styles
            .styles
            .iter()
            .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && !s.hidden)
            .map(|s| (s.builtin, s.id.clone(), crate::i18n::style_name(s)))
            .collect();
        styles.sort_by_key(|(builtin, _, _)| *builtin);
        egui::ScrollArea::horizontal().max_width(420.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (_, id, name) in styles {
                    if crate::previews::table_style_tile(ui, app, &id).on_hover_text(name).clicked() {
                        let _ = app.run("table.style", json!({"style": id}));
                    }
                }
            });
        });
        stack(ui, |ui| {
            let current = look.and_then(|_| {
                let (tp, _, _) = app.session.sel.focus.path.cell()?;
                let id = app.session.doc.table(app.session.sel.focus.story, &tp)?.props.style.clone()?;
                app.session
                    .doc
                    .styles
                    .get(&id)
                    .filter(|st| st.kind == wordcraft_doc::StyleKind::Table)
                    .map(wordcraft_engine::cmd::table_style::is_builtin_table_style)
            });
            let can_modify = current.is_some();
            // Built-in table styles can't be deleted.
            let can_delete = current == Some(false);
            menu_button(ui, app, "styles", Some("Styles"), "Table Styles", false, "table.style", |ui, app| {
                mi(ui, app, "New Table Style…", "ui.dialog", json!({"name": "newTableStyle"}));
                if ui.add_enabled(can_modify, egui::Button::new(tl!("Modify Table Style…")).min_size(vec2(200.0, 0.0))).clicked() {
                    let _ = app.run("ui.dialog", json!({"name": "modifyTableStyle"}));
                    ui.close();
                }
                if ui.add_enabled(can_delete, egui::Button::new(tl!("Delete Table Style")).min_size(vec2(200.0, 0.0))).clicked() {
                    if let Err(e) = app.run("table.deleteStyle", json!({})) {
                        app.status(e);
                    }
                    ui.close();
                }
            });
            split(ui, app, "shading", "Shading", "table.shading", json!({"color": app.canvas.last_shading.clone()}), false, None, |ui, app| {
                mi(ui, app, "No Color", "table.shading", json!({"color": null}));
                let theme = app.session.doc.settings.theme_colors.clone();
                crate::colors::menu(ui, app, &theme, Target::cmd("table.shading").remember(Remember::Shading));
            });
        });
    });
    group(ui, "Borders", None, app, |ui, app| {
        menu_button(ui, app, "borders", Some("Borders"), "Borders", true, "table.borders", |ui, app| {
            for (l, k) in [
                ("All Borders", "all"),
                ("Outside Borders", "outside"),
                ("Inside Borders", "inside"),
                ("No Border", "none"),
                ("Top Border", "top"),
                ("Bottom Border", "bottom"),
            ] {
                mi(ui, app, l, "table.borders", json!({"kind": k}));
            }
        });
        big(ui, app, "borderPainter", "Border\nPainter", "table.borderPainter", json!({}), false);
    });
}

/// Table Layout › Cell Size: the caret cell's row height and column width, in inches. They show
/// the size on the page (a row without a set height shows the height its text gives it) and
/// editing runs `table.rowHeight` / `table.columnWidth`; dragging one is a single Undo.
fn cell_size_boxes(ui: &mut Ui, app: &mut WordApp) {
    let story = app.session.sel.focus.story;
    let Some((tp, r, c)) = app.session.sel.focus.path.cell() else { return };
    let Some(t) = app.session.doc.table(story, &tp) else { return };
    let row = t.rows.get(r).map(|x| x.props.clone()).unwrap_or_default();
    let stored_w = t.grid.get(t.grid_col(r, c)).copied();
    let laid = app.session.layout().pages.iter().flat_map(|pg| pg.items.iter()).find_map(|it| match it {
        wordcraft_layout::Placed::Cell { rect, table, row, cell, story: st, .. } if *table == tp && *row == r && *cell == c && *st == story => {
            Some(*rect)
        }
        _ => None,
    });
    // The row's own height: a vertically merged cell's box spans several rows, so take the
    // shortest box in this row.
    let row_h = app
        .session
        .layout()
        .pages
        .iter()
        .flat_map(|pg| pg.items.iter())
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Cell { rect, table, row, story: st, .. } if *table == tp && *row == r && *st == story => Some(rect.h),
            _ => None,
        })
        .reduce(f32::min);
    let unit = crate::options::unit();
    let k = unit.pt_per_unit();
    let h0 = row.height.or(row_h).unwrap_or(0.0);
    let w0 = laid.map(|x| x.w).or(stored_w).unwrap_or(0.0);
    let exact = row.height_rule == wordcraft_doc::props::HeightRule::Exact;
    // The edited value in points, and whether it continues a drag (joins the previous Undo step).
    let boxed = |ui: &mut Ui, label: &str, pt: f32| -> Option<(f32, bool)> {
        let mut out = None;
        crate::widgets::row(ui, |ui| {
            ui.add_sized(vec2(48.0, 18.0), egui::Label::new(egui::RichText::new(tl!(label)).small()));
            let mut v = pt / k;
            let r = ui.add_sized(vec2(72.0, 18.0), egui::DragValue::new(&mut v).speed(0.01).range(0.02..=22.0).measure(unit).max_decimals(2));
            if r.changed() {
                out = Some((v * k, r.dragged() && !r.drag_started()));
            }
        });
        ui.add_space(2.0);
        out
    };
    stack(ui, |ui| {
        if let Some((h, join)) = boxed(ui, "Height:", h0) {
            if join {
                app.session.join_next_undo();
            }
            let _ = app.run("table.rowHeight", json!({"height": h, "rule": if exact { "exact" } else { "atLeast" }}));
        }
        if let Some((w, join)) = boxed(ui, "Width:", w0) {
            if join {
                app.session.join_next_undo();
            }
            let _ = app.run("table.columnWidth", json!({"width": w}));
        }
    });
}

fn table_layout(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "selectTable", Some("Select"), "Select", false, "table.selectTable", |ui, app| {
                mi(ui, app, "Select Cell", "table.selectCell", json!({}));
                mi(ui, app, "Select Column", "table.selectColumn", json!({}));
                mi(ui, app, "Select Row", "table.selectRow", json!({}));
                mi(ui, app, "Select Table", "table.selectTable", json!({}));
            });
            let g = app.session.view.table_gridlines;
            small(ui, app, "gridlines", Some("View Gridlines"), "View Gridlines", "table.viewGridlines", json!({}), g);
            small(ui, app, "properties", Some("Properties"), "Table Properties", "table.properties", json!({}), false);
        });
    });
    group(ui, "Draw", None, app, |ui, app| {
        use crate::table_pen::TableTool;
        let tool = app.canvas.table_tool;
        crate::widgets::big_toggle(ui, app, "drawTable", "Draw\nTable", "table.draw", tool == Some(TableTool::Draw));
        crate::widgets::big_toggle(ui, app, "eraser", "Eraser", "table.eraser", tool == Some(TableTool::Erase));
    });
    group(ui, "Rows & Columns", None, app, |ui, app| {
        menu_button(ui, app, "deleteTable", Some("Delete"), "Delete", true, "table.deleteTable", |ui, app| {
            mi(ui, app, "Delete Cells", "table.deleteCells", json!({}));
            mi(ui, app, "Delete Columns", "table.deleteColumn", json!({}));
            mi(ui, app, "Delete Rows", "table.deleteRow", json!({}));
            mi(ui, app, "Delete Table", "table.deleteTable", json!({}));
        });
        big(ui, app, "insertAbove", "Insert\nAbove", "table.insertRowAbove", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "insertBelow", Some("Insert Below"), "Insert Below", "table.insertRowBelow", json!({}), false);
            small(ui, app, "insertLeft", Some("Insert Left"), "Insert Left", "table.insertColumnLeft", json!({}), false);
            small(ui, app, "insertRight", Some("Insert Right"), "Insert Right", "table.insertColumnRight", json!({}), false);
        });
    });
    group(ui, "Merge", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "merge", Some("Merge Cells"), "Merge Cells", "table.merge", json!({}), false);
            small(ui, app, "splitCells", Some("Split Cells"), "Split Cells", "table.split", json!({}), false);
            small(ui, app, "splitTable", Some("Split Table"), "Split Table", "table.splitTable", json!({}), false);
        });
    });
    group(ui, "Cell Size", None, app, |ui, app| {
        menu_button(ui, app, "autofit", Some("AutoFit"), "AutoFit", true, "table.autofit", |ui, app| {
            mi(ui, app, "AutoFit Contents", "table.autofit", json!({"mode": "contents"}));
            mi(ui, app, "AutoFit Window", "table.autofit", json!({"mode": "window"}));
            mi(ui, app, "Fixed Column Width", "table.autofit", json!({"mode": "fixed"}));
        });
        cell_size_boxes(ui, app);
        stack(ui, |ui| {
            small(ui, app, "distributeRows", Some("Distribute Rows"), "Distribute Rows", "table.distributeRows", json!({}), false);
            small(ui, app, "distributeCols", Some("Distribute Columns"), "Distribute Columns", "table.distributeColumns", json!({}), false);
        });
    });
    group(ui, "Alignment", None, app, |ui, app| {
        egui::Grid::new("cellalign").spacing(vec2(1.0, 1.0)).show(ui, |ui| {
            for row in [
                [("topLeft", "Align Top Left"), ("topCenter", "Align Top Center"), ("topRight", "Align Top Right")],
                [("centerLeft", "Align Center Left"), ("center", "Align Center"), ("centerRight", "Align Center Right")],
                [("bottomLeft", "Align Bottom Left"), ("bottomCenter", "Align Bottom Center"), ("bottomRight", "Align Bottom Right")],
            ] {
                for (v, tip) in row {
                    small(ui, app, "cellAlign", None, tip, "table.cellAlign", json!({"value": v}), false);
                }
                ui.end_row();
            }
        });
        big(ui, app, "textDirection", "Text\nDirection", "table.textDirection", json!({}), false);
        big(ui, app, "cellMargins", "Cell\nMargins", "table.cellMargins", json!({}), false);
    });
    group(ui, "Data", None, app, |ui, app| {
        big(ui, app, "sort", "Sort", "table.sort", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "repeatHeader", Some("Repeat Header Rows"), "Repeat Header Rows", "table.repeatHeader", json!({}), false);
            small(ui, app, "toText", Some("Convert to Text"), "Convert to Text", "table.toText", json!({}), false);
            small(ui, app, "formula", Some("Formula"), "Formula", "table.formula", json!({}), false);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_engine::Session;

    /// Two clicks arm and disarm the painter; the double-click handling then leaves it sticky (#2785).
    #[test]
    fn double_click_on_format_painter_makes_it_sticky() {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
        app.run("edit.formatPainter", json!({})).unwrap();
        assert!(matches!(app.session.painter, Some((_, _, false))), "one click arms it for one use");
        app.run("edit.formatPainter", json!({})).unwrap();
        assert!(app.session.painter.is_none(), "the second click of a double-click turns it off");
        painter_double_click(&mut app);
        assert!(matches!(app.session.painter, Some((_, _, true))), "the double-click leaves it sticky");
    }

    fn png_data() -> String {
        let img = image::RgbaImage::from_fn(20, 10, |x, y| {
            if x > 2 && x < 17 && y > 2 && y < 7 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let mut b = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        wordcraft_engine::cmd::insert::base64_encode(&b)
    }

    /// The style set gallery fills exactly its own box and the next control starts after a gap,
    /// so Colors and Fonts can't overlap the thumbnails (#638).
    #[test]
    fn design_style_set_gallery_stays_in_its_box() {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let mut seen = None;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 200.0)).build_ui(|ui| {
            ui.horizontal_top(|ui| {
                let (gallery, used) = style_set_box(&mut app, ui);
                seen = Some((gallery, used, ui.cursor().min.x));
            });
        });
        h.run_steps(2);
        drop(h);
        let (gallery, used, next_x) = seen.unwrap();
        assert!((gallery.width() - crate::previews::STYLE_SET_GALLERY_W).abs() < 0.01);
        assert!(used.right() <= gallery.right() + 0.01, "{used:?} {gallery:?}");
        assert!(next_x >= gallery.right() + 8.0 - 0.01, "{next_x} {gallery:?}");
    }

    /// #2738: Home › Underline's drop-down: every line sample is a named entry (assistive
    /// technology, tooltips) that applies its own style; Words only, None and Underline Color ›
    /// Automatic do what they say.
    #[test]
    fn underline_menu_entries_apply_their_style() {
        use egui_kittest::kittest::Queryable;
        use wordcraft_doc::props::Underline;
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
        a.run("select.all", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 700.0)).build_ui_state(|ui, app: &mut WordApp| underline_menu(ui, app), a);
        h.run_steps(2);
        let run =
            |h: &egui_kittest::Harness<'_, WordApp>| h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap().runs[0].props.clone();
        for (_, name) in UNDERLINE_STYLES {
            assert!(h.query_by_label(name).is_some(), "{name} is a named entry");
        }
        // Entries made with `mi` carry their shortcut text in their label (Words only: Ctrl+Shift+W).
        for (name, want) in
            [("Double", Underline::Double), ("Dot-dot-dash", Underline::DotDotDash), ("Wave", Underline::Wave), ("Words only", Underline::Words)]
        {
            h.get_by_label_contains(name).click();
            h.run_steps(2);
            assert_eq!(run(&h).underline, Some(want), "{name}");
        }
        h.get_by_label("None").click();
        h.run_steps(2);
        assert_eq!(run(&h).underline, Some(Underline::None));
        h.get_by_label("Double").click();
        h.run_steps(2);
        // The submenu opens on a click once the pointer is on it.
        let open_colors = |h: &mut egui_kittest::Harness<'_, WordApp>| {
            h.get_by_label("Underline Color").hover();
            h.run();
            h.get_by_label("Underline Color").click();
            h.run();
        };
        open_colors(&mut h);
        // Standard Colors' first swatch (C00000) sits just under its heading; swatches have no label.
        let heading = h.get_by_label("Standard Colors").rect();
        let at = egui::pos2(heading.left() + 7.0, heading.bottom() + 10.0);
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        assert_eq!(run(&h).underline_color, wordcraft_doc::Rgb::parse("C00000"));
        assert_eq!(run(&h).underline, Some(Underline::Double), "a colour keeps the style");
        open_colors(&mut h);
        h.get_by_label_contains("Automatic").click();
        h.run_steps(2);
        assert_eq!(run(&h).underline_color, None);
        assert_eq!(run(&h).underline, Some(Underline::Double));
    }

    /// #658: menu buttons grey out (and don't open) while the command their menu is about is
    /// disabled: the Table Layout menus outside a table; menus with no command stay on.
    #[test]
    fn menu_buttons_follow_their_command() {
        use crate::widgets::menu_enabled;
        let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
        for id in ["table.selectTable", "table.deleteTable", "table.autofit"] {
            assert!(!menu_enabled(&a, id), "{id} is off outside a table");
        }
        assert!(menu_enabled(&a, ""), "a menu without a command is always on");
        assert!(menu_enabled(&a, "insert.header"));
        a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        for id in ["table.selectTable", "table.deleteTable", "table.autofit"] {
            assert!(menu_enabled(&a, id), "{id} is on inside a table");
        }
    }

    /// #1206: Margins, Orientation, Size and Columns mark the caret section's setting, as Word
    /// does. Every value differs from a new document's, so a default can't satisfy it.
    #[test]
    fn page_setup_menus_mark_the_current_setting() {
        use egui_kittest::kittest::{By, NodeT, Queryable};
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
        app.run("layout.margins", json!({"preset": "narrow"})).unwrap();
        app.run("layout.orientation", json!({"value": "landscape"})).unwrap();
        app.run("layout.size", json!({"name": "A4"})).unwrap();
        app.run("layout.columns", json!({"preset": "right"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 1200.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                margins_items(ui, app);
                orientation_items(ui, app);
                size_menu(ui, app);
                columns_items(ui, app);
            },
            app,
        );
        h.run_steps(2);
        let checked: Vec<String> = h
            .query_all(By::new().predicate(|n| n.toggled() == Some(egui::accesskit::Toggled::True)))
            .filter_map(|n| n.accesskit_node().label())
            .collect();
        let first_words: Vec<&str> = checked.iter().filter_map(|l| l.trim_start_matches('✓').split_whitespace().next()).collect();
        assert_eq!(first_words, ["Narrow", "Landscape", "A4", "Right"], "{checked:?}");
    }

    /// #860 review: a tag the language list doesn't have shows by language and region, not as
    /// the tag; Portuguese (Angola) isn't Brazil's, and no name shows twice in the menu.
    #[test]
    fn other_regions_show_by_name_once() {
        assert_eq!(language_name(Some("fr-CA")), "French (Canada)");
        assert_eq!(language_name(Some("en-AU")), "English (Australia)");
        assert_eq!(language_name(Some("de_AT")), "German (Austria)");
        assert_eq!(language_name(Some("pt-AO")), "Portuguese (Angola)");
        assert_eq!(language_name(Some("sr-Latn-BA")), "Serbian (Bosnia and Herzegovina)");
        assert_eq!(language_name(Some("en-ZZ")), "English (ZZ)");
        assert_eq!(language_name(Some("xx-YY")), "xx-YY");
        let exact = |cur: &'static str| move |t: &str| t.eq_ignore_ascii_case(cur);
        for tag in ["fr-CA", "pt-AO", "en-SG"] {
            let c = language_checks(tag, exact(tag));
            let name = c.extra.expect("shown above the list");
            assert!(!PROOFING_LANGUAGES.iter().any(|(_, n)| *n == name), "{name} is listed once");
            assert!(!c.listed.contains(&true), "{tag}: nothing in the list checked");
        }
        // A tag named like a list entry checks that entry instead of showing it again.
        let c = language_checks("PT-br-x-test", |_| false);
        assert_eq!(c.extra, None);
        assert_eq!(c.listed, PROOFING_LANGUAGES.map(|(t, _)| t == "pt-BR"));
    }

    /// The Englishes WordCraft proofs differently are in the menu, checked by their tags, and
    /// every English region has a name (#1281, #1045).
    #[test]
    fn english_dialects_are_in_the_language_menu() {
        for (tag, name) in [
            ("en-US", "English (United States)"),
            ("en-GB", "English (United Kingdom)"),
            ("en-AU", "English (Australia)"),
            ("en-CA", "English (Canada)"),
        ] {
            assert!(PROOFING_LANGUAGES.contains(&(tag, name)), "{tag}");
            assert_eq!(language_name(Some(tag)), name);
            let c = language_checks(&tag.to_ascii_lowercase(), |t| t.eq_ignore_ascii_case(tag));
            assert_eq!((c.extra, c.listed), (None, PROOFING_LANGUAGES.map(|(t, _)| t == tag)), "{tag}");
        }
        for (tag, name) in
            [("en-NZ", "English (New Zealand)"), ("en-IE", "English (Ireland)"), ("en-ZA", "English (South Africa)"), ("en-IN", "English (India)")]
        {
            assert!(crate::dialogs_layout::LANGUAGES.contains(&(tag, name)), "{tag}");
            assert_eq!(language_name(Some(tag)), name);
        }
        assert_eq!(language_name(Some("en-SG")), "English (Singapore)");
    }

    #[test]
    fn indonesian_has_a_name_in_the_language_menu() {
        assert!(PROOFING_LANGUAGES.contains(&("id-ID", "Indonesian")));
        for tag in ["id", "id-ID", "ID-id", "ind", "in"] {
            assert_eq!(language_name(Some(tag)), "Indonesian", "{tag}");
        }
    }

    #[test]
    fn czech_has_a_name_in_the_language_menu() {
        assert!(PROOFING_LANGUAGES.contains(&("cs-CZ", "Czech")));
        for tag in ["cs", "cs-CZ", "CS-cz", "ces", "CZE"] {
            assert_eq!(language_name(Some(tag)), "Czech", "{tag}");
        }
    }

    #[test]
    fn portuguese_has_a_name_in_the_language_menu() {
        assert!(PROOFING_LANGUAGES.iter().any(|(t, _)| *t == "pt-BR"));
        assert_eq!(language_name(Some("pt-BR")), "Portuguese (Brazil)");
        assert_eq!(language_name(Some("pt")), "Portuguese (Brazil)");
        assert_eq!(language_name(Some("PT-BR".to_string().as_str())), "Portuguese (Brazil)");
    }

    #[test]
    fn turkish_has_a_name_in_the_language_menu() {
        assert!(PROOFING_LANGUAGES.iter().any(|(t, _)| *t == "tr-TR"));
        assert_eq!(language_name(Some("tr-TR")), "Turkish");
        assert_eq!(language_name(Some("tr")), "Turkish");
        assert_eq!(language_name(Some("TR-TR")), "Turkish");
    }

    #[test]
    fn picture_format_tab_appears_when_picture_selected() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        assert!(!has_picture_selected(&s));
        assert!(!contextual_tabs(&s).contains(&"Picture Format"));
        s.run("insert.picture", &json!({"data": png_data()})).unwrap();
        assert!(has_picture_selected(&s));
        assert!(contextual_tabs(&s).contains(&"Picture Format"));
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(!has_picture_selected(&s));
    }

    #[test]
    fn shape_format_tab_appears_when_shape_selected() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
        assert!(contextual_tabs(&s).contains(&"Shape Format"));
        assert!(!contextual_tabs(&s).contains(&"Picture Format"));
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(!contextual_tabs(&s).contains(&"Shape Format"));
    }

    #[test]
    fn page_setup_launcher_opens_the_dialog() {
        // A group launcher runs its command with no parameters (#471).
        let mut app = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("layout.pageSetup", json!({})).unwrap();
        assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::PageSetup { .. })));
        // `ui.dialog` needs a dialog name, so it can't be a launcher.
        let launchers: Vec<&str> = include_str!("ribbon.rs")
            .split("group(ui, \"")
            .filter_map(|g| g.split_once('"')?.1.strip_prefix(", Some(\"")?.split('"').next())
            .collect();
        assert!(launchers.contains(&"layout.pageSetup"), "{launchers:?}");
        assert!(!launchers.contains(&"ui.dialog"), "{launchers:?}");
    }

    #[test]
    fn smart_art_picker_inserts_and_design_tab_follows_the_selection() {
        let mut app = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        // Insert › SmartArt without a layout opens the picker.
        let _ = app.run("insert.smartArt", json!({}));
        assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("insertSmartArt"));
        app.dialog = None;
        app.run("insert.smartArt", json!({"layout": "cycle"})).unwrap();
        assert!(contextual_tabs(&app.session).contains(&"SmartArt Design"));
        app.run("smartArt.textPane", json!({})).unwrap();
        assert!(app.session.view.smart_art_pane);
        app.session.run("select.collapse", &json!({"end": true})).unwrap();
        app.session.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(!contextual_tabs(&app.session).contains(&"SmartArt Design"));
    }

    #[test]
    fn chart_design_tab_appears_when_chart_selected_and_buttons_open_dialogs() {
        let mut app = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        // Insert › Chart without a type opens the gallery; choosing inserts.
        let _ = app.run("insert.chart", json!({}));
        assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("insertChart"));
        app.dialog = None;
        app.run("insert.chart", json!({"type": "doughnut"})).unwrap();
        assert!(contextual_tabs(&app.session).contains(&"Chart Design"));
        let _ = app.run("chart.editData", json!({}));
        assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("chartData"));
        let _ = app.run("chart.type", json!({}));
        assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("changeChartType"));
        app.session.run("select.collapse", &json!({"end": true})).unwrap();
        app.session.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(!contextual_tabs(&app.session).contains(&"Chart Design"));
    }

    /// No group's content runs into its label: View › Show stacked four check boxes in a column
    /// made for three, so "Hide White Space" was drawn under "Show". Every tab, every language.
    #[test]
    fn ribbon_groups_fit_above_their_labels() {
        let app = WordApp::new(Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
        // Wide enough that no tab hides groups behind its scroll arrows.
        let mut h = egui_kittest::Harness::builder().with_size(vec2(2560.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        let mut tall = Vec::new();
        for lang in crate::i18n::Lang::all() {
            h.state_mut().run("ui.language", json!({"value": lang.code()})).unwrap();
            for tab in TABS.iter().filter(|t| **t != "File") {
                h.state_mut().run("ui.tab", json!({"tab": tab})).unwrap();
                h.run_steps(2);
                crate::widgets::TALL_GROUPS.with(|g| g.borrow_mut().clear());
                h.run_steps(1);
                let found = crate::widgets::TALL_GROUPS.with(|g| g.take());
                tall.extend(found.into_iter().map(|g| format!("{} {tab} › {g}", lang.code())));
            }
        }
        tall.sort();
        tall.dedup();
        assert!(tall.is_empty(), "groups taller than the ribbon: {tall:#?}");
    }

    /// #897: the tabs that didn't fit were drawn under Comments / Editing / Share, so at 1280 pt
    /// in Polish the table tabs were out of reach. They stay on the strip and a click opens them.
    #[test]
    fn table_tabs_stay_clickable_when_the_tab_strip_overflows() {
        if !crate::i18n::catalogs_embedded() {
            return;
        }
        use egui_kittest::kittest::Queryable;
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
        app.run("insert.table", json!({"rows": 3, "cols": 3})).unwrap();
        app.run("ui.language", json!({"value": "pl"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(3);
        let buttons = [tl!("Comments"), tl!("Editing")].map(|b| h.get_by_label(b).rect());
        for tab in ["Table Design", "Table Layout"] {
            let r = h.get_by_label(tl!(tab)).rect();
            assert!(r.right() <= 1280.0 && buttons.iter().all(|b| !b.intersects(r)), "{tab} {r:?} under {buttons:?}");
            h.get_by_label(tl!(tab)).click();
            h.run_steps(2);
            assert_eq!(h.state().ui.tab, tab);
        }
    }

    /// #2026: at every width and in the longest languages no tab is drawn over Comments /
    /// Editing / Share, the tabs stay inside the window, and the buttons keep their room.
    #[test]
    fn tab_strip_never_overlaps_the_right_buttons() {
        if !crate::i18n::catalogs_embedded() {
            return;
        }
        use egui_kittest::kittest::Queryable;
        for lang in ["en", "pl", "de"] {
            for width in [300.0, 420.0, 520.0, 640.0, 765.0, 1280.0] {
                let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
                app.run("insert.table", json!({"rows": 3, "cols": 3})).unwrap();
                app.run("ui.language", json!({"value": lang})).unwrap();
                app.run("ui.tab", json!({"tab": "Table Layout"})).unwrap();
                let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 600.0)).build_ui_state(
                    |ui, app: &mut WordApp| {
                        let ctx = ui.ctx().clone();
                        app.logic(&ctx);
                        app.ui(ui);
                    },
                    app,
                );
                h.run_steps(3);
                let buttons: Vec<Rect> =
                    [tl!("Comments"), tl!("Editing"), tl!("Share")].iter().filter_map(|b| h.query_by_label(b).map(|n| n.rect())).collect();
                assert_eq!(buttons.len(), 3, "{lang} {width}: the three buttons are on the strip");
                assert!(buttons.iter().all(|b| b.right() <= width + 0.5 && b.left() >= 0.0), "{lang} {width}: buttons inside {buttons:?}");
                for tab in crate::ribbon::TABS.iter().copied().chain(["Table Design", "Table Layout"]) {
                    // A group folded into a button can share the tab's name (Draw): only the node on
                    // the strip is the tab.
                    let Some(r) = h.query_all_by_label(tl!(tab)).map(|n| n.rect()).find(|r| r.top() <= 40.0) else { continue };
                    assert!(r.right() <= width + 0.5, "{lang} {width}: {tab} {r:?} past the window");
                    assert!(buttons.iter().all(|b| !b.intersects(r)), "{lang} {width}: {tab} {r:?} under {buttons:?}");
                }
                assert!(h.query_by_label(tl!("File")).is_some(), "{lang} {width}: File stays on the strip");
            }
        }
    }

    #[test]
    fn plan_strip_gives_way_in_order() {
        let spec = |text_w, file, active, contextual| TabSpec { text_w, file, active, contextual };
        let tabs = [spec(20.0, true, false, false), spec(30.0, false, true, false), spec(30.0, false, false, false), spec(40.0, false, false, true)];
        let buttons = [200.0, 150.0, 60.0];
        // Plenty of room: nothing gives way.
        let p = plan_strip(&tabs, buttons, 600.0, 26.0);
        assert_eq!((p.pad, p.buttons, p.fits.iter().all(|f| *f)), (18.0, 0, true));
        // Tighter: the padding goes first, then the Comments / Editing labels, then Share's.
        let p = plan_strip(&tabs, buttons, 380.0, 26.0);
        assert_eq!((p.pad, p.buttons), (10.0, 0));
        let p = plan_strip(&tabs, buttons, 330.0, 26.0);
        assert_eq!((p.pad, p.buttons), (10.0, 1));
        let p = plan_strip(&tabs, buttons, 240.0, 26.0);
        assert_eq!((p.pad, p.buttons), (10.0, 2));
        // Then the plain tab goes into »; File, the active and the contextual tab stay.
        let p = plan_strip(&tabs, buttons, 215.0, 26.0);
        assert_eq!(p.fits, [true, true, false, true]);
        // Then the labels of the active and contextual tabs are cut down.
        let p = plan_strip(&tabs, buttons, 208.0, 26.0);
        assert_eq!(p.fits, [true, true, false, true]);
        assert!(p.widths[3] < 52.0 && p.widths[3] >= MIN_TAB_W - 0.01, "{:?}", p.widths);
        // No room for that either: the contextual one goes into », the active one last.
        let p = plan_strip(&tabs, buttons, 190.0, 26.0);
        assert_eq!(p.fits, [true, true, false, false]);
        let p = plan_strip(&tabs, buttons, 140.0, 26.0);
        assert_eq!(p.fits, [true, false, false, false]);
        // The buttons' room is never taken, and hostile numbers don't panic.
        for avail in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            let _ = plan_strip(&tabs, buttons, avail, 26.0);
        }
        let _ = plan_strip(&[], buttons, 100.0, 26.0);
        let _ = plan_strip(&[spec(f32::NAN, false, true, false)], [f32::NAN; 3], 100.0, f32::NAN);
    }

    /// #1110: the strip's drawn buttons have names for screen readers: Share, and the "»" that
    /// holds the tabs that don't fit ("More tabs", not a glyph).
    #[test]
    fn the_tab_strip_buttons_have_names() {
        use egui_kittest::kittest::Queryable;
        let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(vec2(760.0, 700.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::Button, tl!("Share"));
        let more = h.get_by_role_and_label(egui::accesskit::Role::Button, tl!("More tabs"));
        more.click();
        h.run_steps(2);
        // The menu lists a tab that didn't fit (View is among the last on a narrow strip).
        assert!(h.query_by_label(tl!("View")).is_some(), "the » menu lists the tabs left out");
    }

    #[test]
    fn fit_tabs_keeps_the_kept_tabs_and_drops_from_the_end() {
        let tabs = [(40.0, true), (50.0, false), (50.0, false), (50.0, false), (60.0, true)];
        assert_eq!(fit_tabs(&tabs, 250.0, 20.0), [true; 5]);
        // 250 doesn't fit 240 with room for »: the last plain tabs give way, the kept ones stay.
        assert_eq!(fit_tabs(&tabs, 240.0, 20.0), [true, true, true, false, true]);
        // 200 would fit 210, but not with the » button too.
        assert_eq!(fit_tabs(&tabs, 210.0, 20.0), [true, true, false, false, true]);
        assert_eq!(fit_tabs(&tabs, 150.0, 20.0), [true, false, false, false, true]);
        // Nothing but the kept tabs fits: they stay anyway.
        assert_eq!(fit_tabs(&tabs, 10.0, 20.0), [true, false, false, false, true]);
        // Hostile numbers don't panic.
        let _ = fit_tabs(&[(f32::NAN, false), (f32::INFINITY, false)], f32::NAN, 20.0);
    }

    /// #860: Review › Language (and the status bar) reach every proofing language, not just the
    /// short list: Set Proofing Language… opens the Language dialog (French, Set As Default),
    /// Language Preferences… opens Options › Language, and French text shows by name, checked.
    #[test]
    fn language_menu_reaches_every_language() {
        use egui_kittest::kittest::{By, NodeT, Queryable};
        let item = |label: &'static str| By::new().label_contains(label);
        let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Bonjour")), crate::Services::default());
        let mut h =
            egui_kittest::Harness::builder().with_size(vec2(400.0, 400.0)).build_ui_state(|ui, app: &mut WordApp| language_menu(ui, app), app);
        h.run();
        assert!(h.query(item("French (France)")).is_none(), "the short list only, on an English document");
        assert!(!h.get(item("Set Proofing Language…")).accesskit_node().is_disabled(), "never greyed out on a fresh document");
        assert!(!h.get(item("English (United States)")).accesskit_node().is_disabled());
        h.get(item("Set Proofing Language…")).click();
        h.run();
        assert_eq!(h.state().dialog.as_ref().map(crate::dialogs::Dialog::name), Some("language"));
        h.state_mut().dialog = None;
        // French for the whole document: the menu and the status bar name it.
        h.state_mut().run("review.language", json!({"lang": "fr-FR", "default": true})).unwrap();
        h.run();
        assert_eq!(caret_language(h.state()), "French (France)");
        assert!(h.query(item("French (France)")).is_some(), "the current language is listed (only while it is the current one)");
        h.get(item("Language Preferences…")).click();
        h.run();
        assert!(h.state().ui.backstage, "File › Options opened");
        assert_eq!(h.state().ui.options_pane, "language");
        assert_eq!(language_name(Some("de-DE")), "German (Germany)");
    }

    /// #1281: every short-list entry (the four Englishes, Indonesian…) is picked in the menu:
    /// clicking it sets the selection's proofing language, and the entry is then the checked one.
    #[test]
    fn every_proofing_language_is_picked_from_the_language_menu() {
        use egui_kittest::kittest::{By, NodeT, Queryable};
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Colour")), crate::Services::default());
        app.run("select.all", json!({})).unwrap();
        let mut h =
            egui_kittest::Harness::builder().with_size(vec2(400.0, 500.0)).build_ui_state(|ui, app: &mut WordApp| language_menu(ui, app), app);
        h.run();
        for (tag, name) in PROOFING_LANGUAGES {
            h.get(By::new().label_contains(name)).click();
            h.run();
            let st = h.state_mut().session.run("format.state", &json!({})).unwrap();
            assert_eq!(st.get("lang").and_then(Value::as_str), Some(tag), "{name}");
            for (_, other) in PROOFING_LANGUAGES {
                let ticked = h.get(By::new().label_contains(other)).accesskit_node().toggled();
                assert_eq!(ticked == Some(egui::accesskit::Toggled::True), other == name, "{name}: {other} ticked {ticked:?}");
            }
        }
    }

    #[test]
    fn resolve_tab_falls_back_when_contextual_tab_expires() {
        let all = ["Home", "Table Design", "Picture Format"];
        assert_eq!(resolve_tab("Picture Format", &all), "Picture Format");
        assert_eq!(resolve_tab("Picture Format", &["Home"]), "Home");
        assert_eq!(resolve_tab("Table Design", &["Home"]), "Home");
        assert_eq!(resolve_tab("Home", &["Home"]), "Home");
    }

    /// #2573, #1901: Picture Format › Size's width shows the interface unit and takes a size typed
    /// in pixels. It applies once typed (Enter), not while typing, keeping the picture's proportions, as one
    /// undo step.
    #[test]
    fn picture_width_typed_in_pixels_keeps_the_proportions() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("file.options", json!({"units": "centimeters"})).unwrap();
        a.run("insert.picture", json!({"data": png_data()})).unwrap();
        a.run("ui.tab", json!({"tab": "Picture Format"})).unwrap();
        let size = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h)));
        assert_eq!(size(&a), Some((15.0, 7.5)), "20×10 px at 96 ppi");
        // Wide enough for the whole tab: the Size group is its last.
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 900.0)).build_ui_state(
            |ui, app: &mut crate::WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let shown = h.get_by_role_and_label(Role::SpinButton, "W:").value().unwrap_or_default();
        assert!(shown.ends_with(" cm"), "the interface unit: {shown:?}");
        h.get_by_role_and_label(Role::SpinButton, "W:").click();
        h.run_steps(2);
        // Editing, the field is still a spin button, now showing the bare number to type over.
        let editing = h.get_by_role_and_label(Role::SpinButton, "W:");
        assert!(editing.is_focused() && editing.value().is_some_and(|v| !v.ends_with(" cm")), "a click starts typing");
        editing.type_text("300 px");
        h.run_steps(2);
        assert_eq!(size(h.state()), Some((15.0, 7.5)), "nothing applies while typing");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let (w, ht) = size(h.state()).unwrap();
        assert!((w - 225.0).abs() < 0.01 && (ht - 112.5).abs() < 0.01, "300 px = 225 pt, the height follows: {w}×{ht}");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(size(h.state()), Some((15.0, 7.5)), "one undo step");
        // Esc throws a typed size away.
        h.get_by_role_and_label(Role::SpinButton, "W:").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::SpinButton, "W:").type_text("500 px");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(size(h.state()), Some((15.0, 7.5)), "Esc cancels the typing");
    }

    /// #2573 review: showing Picture Format never resizes a picture whose size is outside the
    /// fields' range (a 1×1 px spacer is 0.75 pt): egui clamps such a value by default and reports
    /// it as a change.
    #[test]
    fn picture_format_leaves_a_tiny_picture_alone() {
        let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png)})).unwrap();
        a.run("ui.tab", json!({"tab": "Picture Format"})).unwrap();
        let size = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h)));
        assert_eq!(size(&a), Some((0.75, 0.75)));
        let undo = a.session.undo_depth();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 900.0)).build_ui_state(
            |ui, app: &mut crate::WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(6);
        assert_eq!(size(h.state()), Some((0.75, 0.75)), "shown, not resized");
        assert_eq!(h.state().session.undo_depth(), undo, "no undo step");
    }

    /// #3242: Align used to run `arrange.align` directly with no `value`, which always centers
    /// (`crates/engine/src/cmd/objects.rs`) — there was no way to reach Left or Right from the
    /// ribbon short of dragging. Align now opens a menu, like Word's own Align dropdown.
    #[test]
    fn align_menu_reaches_left_and_right_not_just_center() {
        use egui_kittest::kittest::{By, Queryable};
        let item = |label: &'static str| By::new().label_contains(label);
        let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.picture", json!({"data": png_data()})).unwrap();
        let frame =
            |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, f.x)));
        assert!(matches!(frame(&a), Some((wordcraft_doc::para::Wrap::Inline, _))), "a freshly inserted picture is inline");
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 300.0))
            .build_ui_state(|ui, app: &mut crate::WordApp| align_menu(ui, app, None), a);
        h.run();
        h.get(item("Align")).click();
        h.run();
        assert!(h.query(item("Align Left")).is_some(), "Align opens a menu, instead of running arrange.align with no value");
        h.get(item("Align Right")).click();
        h.run();
        let (wrap, right_x) = frame(h.state()).unwrap();
        assert_eq!(wrap, wordcraft_doc::para::Wrap::Square, "aligning floats an inline picture, as arrange.align already does");
        // A positive x alone doesn't pin Right down: Center's x is positive too whenever the
        // picture is narrower than the text column, so Right could silently dispatch Center.
        h.get(item("Align")).click();
        h.run();
        h.get(item("Align Center")).click();
        h.run();
        let center_x = frame(h.state()).unwrap().1;
        h.get(item("Align")).click();
        h.run();
        h.get(item("Align Left")).click();
        h.run();
        let left_x = frame(h.state()).unwrap().1;
        assert!(left_x < center_x && center_x < right_x, "Left, Center and Right each land at a distinct x: {left_x} < {center_x} < {right_x}");
    }

    /// #2573 review: Shape Format and Chart Design leave sizes outside their fields' ranges alone
    /// too (a 0.5 pt high line; a chart narrower than 36 pt), as Picture Format does.
    #[test]
    fn shape_and_chart_formats_leave_out_of_range_sizes_alone() {
        let size = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h)));
        let shown = |a: crate::WordApp| {
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 900.0)).build_ui_state(
                |ui, app: &mut crate::WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            );
            h.run_steps(6);
            let a = h.state();
            (size(a), a.session.undo_depth())
        };
        let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("picture.size", json!({"width": 3000.0, "height": 1.0, "lockAspect": false})).unwrap();
        a.run("ui.tab", json!({"tab": "Shape Format"})).unwrap();
        let before = (size(&a), a.session.undo_depth());
        assert_eq!(before.0, Some((3000.0, 1.0)));
        assert_eq!(shown(a), before, "Shape Format shown, the shape untouched");

        let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.chart", json!({"type": "doughnut"})).unwrap();
        a.run("picture.size", json!({"width": 20.0, "height": 2500.0, "lockAspect": false})).unwrap();
        a.run("ui.tab", json!({"tab": "Chart Design"})).unwrap();
        let before = (size(&a), a.session.undo_depth());
        assert_eq!(before.0, Some((20.0, 2500.0)));
        assert_eq!(shown(a), before, "Chart Design shown, the chart untouched");
    }
}
