//! The ribbon: tab strip and every tab's groups. Each control runs a command by id.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::colors::{Remember, Target};
use crate::options::MeasureExt;
use crate::theme::{Tokens, medium, regular, semibold};
use crate::widgets::{CONTENT_H, LABEL_H, big, big_toggle, group, menu_button, small, split};
use crate::{WordApp, icons};

mod arrange;
mod charts;
mod common;
mod design;
mod draw;
mod home;
mod insert;
mod language;
mod layout;
mod mailings;
mod picture;
mod references;
mod review;
mod shape;
mod table;
mod tabs;
mod underline;
mod view;

#[cfg(test)]
use arrange::align_menu;
pub(crate) use arrange::wrap_text_items;
use charts::{chart_design, smart_art_design};
pub(crate) use common::mi;
#[cfg(test)]
pub(crate) use common::size_menu;
use common::{custom_groups, edge_cue};
use design::design;
#[cfg(test)]
use design::style_set_box;
use draw::draw;
use home::home;
#[cfg(test)]
use home::painter_double_click;
use insert::insert;
#[cfg(test)]
pub(crate) use language::PROOFING_LANGUAGES;
#[cfg(test)]
pub(crate) use language::all_languages;
#[cfg(test)]
use language::language_checks;
#[cfg(test)]
pub(crate) use language::short_languages;
pub(crate) use language::sort_key;
pub(crate) use language::{
    caret_lang_tag, caret_language, caret_language_proofed, caret_missing_dictionary, caret_no_proof, caret_unreadable_dictionary, language_menu,
};
pub(crate) use language::{display_name, language_list, language_name, same_language, tag_for_pack};
use layout::layout;
#[cfg(test)]
use layout::{columns_items, margins_items, orientation_items};
use mailings::mailings;
use picture::picture_format;
use references::{references, zotero};
use review::{developer, help, review};
#[cfg(test)]
pub(crate) use shape::gradient_menu;
use shape::shape_format;
use table::{table_design, table_layout};
#[cfg(test)]
use tabs::MIN_TAB_W;
use tabs::{MORE_TABS_W, TAB_ENTRY_SECS, TAB_ENTRY_SLIDE, ease_out_cubic, note_contextual_tabs, reading_toolbar, tab_entry};
pub use tabs::{
    StripPlan, TABS, TabSpec, contextual_tabs, fit_tabs, has_chart_selected, has_picture_selected, has_shape_selected, has_smart_art_selected,
    in_table_public, plan_strip, resolve_tab,
};
#[cfg(test)]
use underline::{UNDERLINE_STYLES, underline_menu};
use view::{outlining, view};

/// The tab strip's height, and the room a peek of the commands starts below.
const TABS_H: f32 = 30.0;

/// The pin and the Ribbon Display Options arrow at the far end of the tab strip.
const PIN_W: f32 = 24.0;
const OPTIONS_W: f32 = 24.0;

/// How close to the top of the window the pointer has to be for an auto-hidden ribbon to come
/// down. Word's is a few pixels: enough to reach on purpose, too narrow to open by accident on the
/// way to the page.
const REVEAL_H: f32 = 6.0;

/// Word's Ribbon Display Options (the arrow at the tab row's far end, and `ui.ribbonDisplay`):
/// how much of the ribbon is on screen when the pointer is not asking for more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RibbonDisplay {
    /// The tab row and the commands under it.
    #[default]
    Commands,
    /// The tab row alone; the tab the pointer is on peeks its commands down.
    Tabs,
    /// Nothing; the top of the window brings the whole ribbon down over the page.
    AutoHide,
}

impl RibbonDisplay {
    /// In menu order, as Word's Ribbon Display Options lists them.
    pub const ALL: [RibbonDisplay; 3] = [RibbonDisplay::AutoHide, RibbonDisplay::Tabs, RibbonDisplay::Commands];

    /// The value used in `ui.json` and by the `ui.ribbonDisplay` command.
    pub fn code(self) -> &'static str {
        match self {
            RibbonDisplay::Commands => "commands",
            RibbonDisplay::Tabs => "tabs",
            RibbonDisplay::AutoHide => "autoHide",
        }
    }

    pub fn from_code(code: &str) -> Option<RibbonDisplay> {
        RibbonDisplay::ALL.into_iter().find(|d| d.code().eq_ignore_ascii_case(code.trim()))
    }

    /// The English label (translated with `tl!` where shown).
    pub fn label(self) -> &'static str {
        match self {
            RibbonDisplay::AutoHide => "Auto Hide Ribbon",
            RibbonDisplay::Tabs => "Show Tabs",
            RibbonDisplay::Commands => "Show Tabs and Commands",
        }
    }

    /// Whether the commands are put away: what `ui.collapseRibbon` toggles.
    pub fn collapsed(self) -> bool {
        self != RibbonDisplay::Commands
    }
}

/// Where the ribbon is drawn this frame (pure, tested). A peek covers the page instead of pushing
/// it down, as Word's does and as Focus's ribbon does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// The tab row and the commands, in the window's layout.
    Full,
    /// The tab row in the layout, the commands away.
    Strip,
    /// The tab row in the layout, the peeked commands over the page.
    StripPeek,
    /// Nothing at all.
    Hidden,
    /// The tab row and the commands both peeking over the page.
    Reveal,
}

fn place(display: RibbonDisplay, peek: bool) -> Place {
    match (display, peek) {
        (RibbonDisplay::Commands, _) => Place::Full,
        (RibbonDisplay::Tabs, false) => Place::Strip,
        (RibbonDisplay::Tabs, true) => Place::StripPeek,
        (RibbonDisplay::AutoHide, false) => Place::Hidden,
        (RibbonDisplay::AutoHide, true) => Place::Reveal,
    }
}

/// Whether the pointer is in the band that brings an auto-hidden ribbon down. `top` is the top of
/// the space the ribbon would take; a pointer pushed against the window's edge reports above it.
fn at_top(pointer: Option<egui::Pos2>, top: f32) -> bool {
    pointer.is_some_and(|p| p.y - top < REVEAL_H)
}

/// Whether a collapsed ribbon shows its commands this frame, and whether the peek is still
/// allowed. `inside` is the pointer being on a tab, over the commands a peek just showed, or over
/// a drop-down one of them opened; `fresh` is the frame a collapse lands in, where the
/// double-click or pin click that caused it still sits on the strip — so the peek waits until the
/// pointer has been away once (pure, tested).
fn peek_state(fresh: bool, inside: bool, allowed: bool) -> (bool, bool) {
    let allowed = if !inside {
        true
    } else if fresh {
        false
    } else {
        allowed
    };
    (inside && allowed, allowed)
}

/// Draws the Read toolbar or the full ribbon, switching to and from the contextual Equation and
/// Outlining tabs and showing the available tab strip.
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
    let (now, animate) = (ui.input(|i| i.time), app.animations_on());
    let leaving = {
        let user_names: Vec<&str> = user_tabs.iter().filter(|e| e.custom).map(|e| e.name.as_str()).collect();
        let contextual: Vec<(&str, usize)> = tabs
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.as_str(), i))
            .filter(|(n, _)| wordcraft_engine::ribbon::CONTEXTUAL_TABS.contains(n) && !user_names.contains(n))
            .collect();
        note_contextual_tabs(ui.ctx(), &contextual, now, animate)
    };
    // A tab that just left stays in its former slot while it slides and fades away.
    let ghosts: Vec<(String, usize, f64)> = leaving.iter().filter(|(n, ..)| !tabs.iter().any(|(t, _)| t == n)).cloned().collect();
    if !ghosts.is_empty() {
        ui.ctx().request_repaint();
    }
    for (name, at, _) in &ghosts {
        tabs.insert((*at).min(tabs.len()), (name.clone(), false));
    }
    // Tab strip.
    let peek_id = egui::Id::unique("wordcraft-ribbon-peek");
    // The ribbon's two halves, each drawn either in the window's layout or, while the ribbon is put
    // away, over the page — so a peek covers the page instead of pushing it down, as Word's does and
    // as Focus's ribbon already does.
    let draw_tabs = |app: &mut WordApp, ui: &mut Ui| {
        egui::Panel::top("tabs")
            .exact_size(TABS_H)
            .frame(egui::Frame::NONE.fill(t.tab_strip).inner_margin(egui::Margin { left: 8, right: 10, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                crate::rtl::root(ui, |ui| {
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
                            // The gaps (6 + 4 + 8 + 6), the two end buttons and the strip's margin.
                            spaced(share_button) + spaced(editing) + spaced(comment) + spaced(PIN_W) + spaced(OPTIONS_W) + 18.0 + 12.0
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
                        ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
                            // Word's Ribbon Display Options, at the tab row's far end: the three ways the
                            // ribbon can sit. The pin below toggles two of them, and only this menu
                            // reaches auto-hide, where the tabs go away with the commands.
                            let display = app.ui.ribbon_display;
                            let (or, options) = ui.allocate_exact_size(vec2(OPTIONS_W, TABS_H), Sense::click());
                            if options.hovered() {
                                ui.painter().rect_filled(or.shrink2(vec2(2.0, 6.0)), 4.0, t.hover);
                            }
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(pos2(or.center().x, or.center().y), vec2(14.0, 14.0)),
                                "ribbon-options",
                                t.text,
                                t.accent,
                            );
                            let options_label = tl!("Ribbon Display Options");
                            options.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, false, options_label));
                            crate::rtl::menu(&options.on_hover_text(options_label)).show(|ui| {
                                for d in RibbonDisplay::ALL {
                                    let name = tl!(d.label());
                                    if ui.selectable_label(display == d, name).clicked() {
                                        let _ = app.run("ui.ribbonDisplay", json!({"value": d.code()}));
                                        ui.close();
                                    }
                                }
                            });
                            ui.add_space(6.0);
                            // Word's pin, next to it: the mouse way to put the commands away and keep
                            // them. Double-clicking a tab and Ctrl+F1 do the same, but nothing on screen
                            // said so, and no keyboard is available on a touch screen.
                            let pinned = !display.collapsed();
                            let pin_label = if pinned { tl!("Hide the ribbon") } else { tl!("Show the ribbon") };
                            let (pr, pin) = ui.allocate_exact_size(vec2(PIN_W, TABS_H), Sense::click());
                            if pin.hovered() {
                                ui.painter().rect_filled(pr.shrink2(vec2(2.0, 6.0)), 4.0, t.hover);
                            }
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(pos2(pr.center().x, pr.center().y), vec2(14.0, 14.0)),
                                if pinned { "pin" } else { "unpin" },
                                t.text,
                                t.accent,
                            );
                            pin.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, pinned, pin_label));
                            if pin.on_hover_text(pin_label).clicked() {
                                let _ = app.run("ui.collapseRibbon", json!({}));
                            }
                            ui.add_space(6.0);
                            // No document open (#1129): nothing to share, edit or comment on — but only
                            // these buttons. Disabling the whole line took the tab strip with it: the
                            // tabs took clicks that did nothing, so the empty window had no way back to
                            // the Start screen (#3045).
                            ui.add_enabled_ui(app.has_document(), |ui| {
                                let (share_size, share_text) =
                                    if plan.buttons < 2 { (((share_w + 36.0).max(74.0), 24.0), true) } else { ((30.0, 24.0), false) };
                                let (r, resp) = ui.allocate_exact_size(vec2(share_size.0, share_size.1), Sense::click());
                                ui.painter().rect_filled(r, 4.0, if resp.hovered() { t.accent_text } else { t.accent });
                                let rtl = crate::rtl::on();
                                let icon_x = if share_text { crate::rtl::lead_x(rtl, r, 14.0) } else { r.center().x };
                                icons::paint(
                                    ui.painter(),
                                    Rect::from_center_size(pos2(icon_x, r.center().y), vec2(14.0, 14.0)),
                                    "share",
                                    t.on_accent,
                                    t.on_accent,
                                );
                                if share_text {
                                    ui.painter().text(
                                        pos2(crate::rtl::lead_x(rtl, r, 26.0), r.center().y),
                                        crate::rtl::lead_center(rtl),
                                        share,
                                        medium(12.0),
                                        t.on_accent,
                                    );
                                }
                                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, share));
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
                                crate::rtl::menu(&mode).show(|ui| {
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
                            // The tabs fill the room left of the cluster, from the side reading starts from.
                            // (By language: this row sits in the far-side cluster's layout, which fills from the other end.)
                            let tabs_layout = if crate::rtl::on() {
                                egui::Layout::right_to_left(egui::Align::Center)
                            } else {
                                egui::Layout::left_to_right(egui::Align::Center)
                            };
                            ui.with_layout(tabs_layout, |ui| {
                                // The tabs that don't fit go into a » menu at the end of the strip (#897, #2026).
                                for (i, ((tab, custom), _)) in tabs
                                    .iter()
                                    .zip(&plan.fits)
                                    .enumerate()
                                    .filter(|(_, ((tab, _), fits))| **fits || ghosts.iter().any(|(n, ..)| n == tab))
                                {
                                    let (tab, custom) = (tab.as_str(), *custom);
                                    let contextual = !custom && wordcraft_engine::ribbon::CONTEXTUAL_TABS.contains(&tab);
                                    // A custom tab's name is the user's own text: shown as typed.
                                    let shown = if custom { tab } else { tl!(tab) };
                                    let w = plan.widths.get(i).copied().unwrap_or(0.0) - 2.0;
                                    let cut = specs.get(i).is_some_and(|sp| sp.text_w + plan.pad > w + 0.5);
                                    let ghost = ghosts.iter().find(|(n, ..)| n == tab);
                                    let (r, resp) =
                                        ui.allocate_exact_size(vec2(w, 30.0), if ghost.is_some() { Sense::hover() } else { Sense::click() });
                                    if app.ui.keytips == crate::keytips::Phase::Tabs && ghost.is_none() {
                                        crate::keytips::record_tab(r, tab, &app.ui.keytip_prefix, &mut app.keytip_rects);
                                    } else if app.ui.keytips == crate::keytips::Phase::Legacy && ghost.is_none() {
                                        // Every tab: they only place the legacy keys' hint (#1697).
                                        crate::keytips::record_tab(r, tab, "", &mut app.keytip_rects);
                                    }
                                    let active = app.ui.tab == tab && !app.ui.backstage;
                                    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, shown));
                                    if resp.hovered() && ghost.is_none() {
                                        // A collapsed ribbon peeks its commands while the pointer is here,
                                        // and peeks the tab the pointer is on, as Word does.
                                        ui.data_mut(|d| d.insert_temp(peek_id.with("tab"), true));
                                        ui.data_mut(|d| d.insert_temp(peek_id.with("hover"), tab.to_string()));
                                    }
                                    if resp.hovered() && !active && ghost.is_none() {
                                        ui.painter().rect_filled(r.shrink2(vec2(0.0, 4.0)), 4.0, t.hover);
                                    }
                                    // Entering and departing contextual tabs move and fade without changing layout.
                                    let entry = match ghost {
                                        Some((_, _, left)) => {
                                            ui.ctx().request_repaint();
                                            1.0 - ease_out_cubic(((now - left) / TAB_ENTRY_SECS) as f32)
                                        }
                                        None if contextual => tab_entry(ui.ctx(), tab, now, animate),
                                        None => 1.0,
                                    };
                                    let shift = vec2(-(1.0 - entry) * TAB_ENTRY_SLIDE, 0.0);
                                    let color = (if contextual || active { t.accent_text } else { t.text }).gamma_multiply(entry);
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
                                        ui.painter().galley(r.center() + shift - galley.size() / 2.0, galley, color);
                                    } else {
                                        ui.painter().text(
                                            r.center() + shift,
                                            Align2::CENTER_CENTER,
                                            shown,
                                            if active { semibold(12.5) } else { medium(12.5) },
                                            color,
                                        );
                                    }
                                    let resp = if cut && ghost.is_none() { resp.on_hover_text(shown) } else { resp };
                                    if active {
                                        let u = Rect::from_center_size(pos2(r.center().x + shift.x, r.max.y - 2.0), vec2(w - 16.0, 3.0));
                                        ui.painter().rect_filled(u, 2.0, t.accent.gamma_multiply(entry));
                                    }
                                    if resp.clicked() {
                                        let _ = app.run("ui.tab", json!({"tab": tab}));
                                    }
                                    if resp.double_clicked() && tab != "File" {
                                        let _ = app.run("ui.collapseRibbon", json!({}));
                                    }
                                    if ghost.is_none() {
                                        resp.context_menu(|ui| crate::widgets::customize_menu_items(ui, app));
                                    }
                                }
                                let left_out: Vec<(&str, &str)> = tabs
                                    .iter()
                                    .zip(&labels)
                                    .zip(&plan.fits)
                                    .filter(|(((tab, _), _), fits)| !**fits && !ghosts.iter().any(|(n, ..)| n == tab))
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
                                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, more));
                                    let resp = resp.on_hover_text(more);
                                    crate::rtl::menu(&resp).show(|ui| {
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
                })
            });
    };
    // The commands under the tab row. `peek_tab` is the tab they come from: during a peek it is not
    // the tab the window is on, so the tab is borrowed for the commands alone and the rest of the
    // window keeps reading the real one.
    let draw_commands = |app: &mut WordApp, ui: &mut Ui, peek_tab: Option<String>| {
        let real_tab = app.ui.tab.clone();
        if let Some(tab) = &peek_tab {
            app.ui.tab = tab.clone();
        }
        let ribbon = egui::Panel::top("ribbon")
            .exact_size(CONTENT_H + LABEL_H + 10.0)
            .frame(
                egui::Frame::NONE
                    .fill(t.ribbon)
                    .inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 })
                    .stroke(Stroke::new(1.0, t.border)),
            )
            .show(ui, |ui| {
                // Right-click on the ribbon (outside a button): Customize Ribbon. Registered first, so
                // the buttons drawn over it keep their own clicks.
                // No document open (#1129): every command is greyed out but Help's.
                if !app.has_document() && app.ui.tab != "Help" {
                    ui.disable();
                }
                let bg = ui.interact(ui.max_rect(), ui.scope_id().with("ribbon_background"), Sense::click());
                crate::rtl::context_menu(&bg, |ui| crate::rtl::root(ui, |ui| crate::widgets::customize_menu_items(ui, app)));
                let entry = user_tabs.iter().find(|e| e.name == app.ui.tab);
                crate::ribbon_fit::begin(ui.ctx(), &app.ui.tab, ui.available_width() - 4.0);
                // 0.37 keeps the scroll offset private: an edge-chevron click leaves a one-frame
                // request here and the next scroll area is built at that offset.
                let pending_id = ui.scope_id().with("ribbon_scroll_pending");
                let pending = ui.data(|d| d.get_temp::<f32>(pending_id));
                // Right to left, the groups start at the right edge. A scroll area's content is
                // unbounded, so the row gets a width: the view's, or last frame's content if wider.
                // The scroll position is kept as a distance from the right, where reading starts.
                let rtl = crate::rtl::on();
                let view_w = ui.available_width();
                let row_id = ui.scope_id().with(("ribbon_row", app.ui.tab.as_str()));
                let (content_w, from_right, last_over) = ui.data(|d| d.get_temp::<(f32, f32, f32)>(row_id)).unwrap_or((0.0, 0.0, 0.0));
                let row_w = content_w.max(view_w);
                let set_offset = (last_over - from_right).max(0.0);
                let mut area = egui::ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
                if rtl {
                    area = area.horizontal_scroll_offset(set_offset);
                } else if let Some(x) = pending {
                    area = area.horizontal_scroll_offset(x);
                }
                let mut measured = 0.0;
                let scroll = area.show(ui, |ui| {
                    let size = vec2(if rtl { row_w } else { ui.available_size_before_wrap().x }, ui.available_size_before_wrap().y);
                    let body = |ui: &mut Ui| {
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
                    };
                    if rtl {
                        measured = ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Min), body).response.rect.width();
                    } else {
                        crate::rtl::horizontal_top(ui, body);
                    }
                });
                if rtl {
                    let over = (scroll.content_size.x - scroll.inner_rect.width()).max(0.0);
                    // Only the user's own scrolling moves the remembered distance: while the width
                    // is still settling, the offset set above lags a frame behind.
                    let scrolled = (scroll.state.clamped_offset().x - set_offset).abs() > 0.5 && set_offset <= over;
                    let from_right = if scrolled { (over - scroll.state.clamped_offset().x).max(0.0) } else { from_right.min(over) };
                    ui.data_mut(|d| d.insert_temp(row_id, (measured, from_right, over)));
                    if (over - last_over).abs() > 0.5 {
                        ui.ctx().request_repaint();
                    }
                }
                if pending.is_some() {
                    let _ = ui.data_mut(|d| d.remove_temp::<f32>(pending_id));
                }
                crate::ribbon_fit::end(ui.ctx());
                // The bar is hidden, so a row wider than the window would be clipped with no sign. Edge chevrons show
                // where more is and scroll it on click. Every command also stays reachable through Search Commands.
                let r = scroll.inner_rect;
                let overflow = scroll.content_size.x - r.width();
                let offset = scroll.state.clamped_offset().x;
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
                    let x = x.clamp(0.0, overflow.max(0.0));
                    if rtl {
                        ui.data_mut(|d| d.insert_temp(row_id, (measured, (overflow - x).max(0.0), overflow.max(0.0))));
                    }
                    ui.data_mut(|d| d.insert_temp(pending_id, x));
                    ui.ctx().request_repaint();
                }
            });
        // Which tab the commands are from: the next frame's peek keeps it while the pointer is in a
        // menu one of its buttons opened, and clears it when the peek ends.
        ui.data_mut(|d| d.insert_temp(peek_id.with("shown tab"), app.ui.tab.clone()));
        app.ui.tab = real_tab;
        // Where the commands are: the next frame's peek keeps them while the pointer is still
        // inside them, and clears the place when they go away.
        ui.data_mut(|d| d.insert_temp(peek_id.with("shown"), ribbon.response.rect));
    };

    // Where the ribbon goes this frame. Put away, it comes back while the pointer is on a tab, over
    // the ribbon a peek just showed, over a drop-down one of its buttons opened (a menu outlives the
    // ribbon that dropped it, so following it down must not close both) or, auto-hidden, with the
    // pointer pushed to the top of the window.
    let display = app.ui.ribbon_display;
    let last_display = ui.data(|d| d.get_temp::<RibbonDisplay>(peek_id.with("display")));
    ui.data_mut(|d| d.insert_temp(peek_id.with("display"), display));
    let (on_tab, last, allowed, hover, shown_tab) = ui.data(|d| {
        (
            d.get_temp::<bool>(peek_id.with("tab")).unwrap_or(false),
            d.get_temp::<Rect>(peek_id.with("shown")).unwrap_or(Rect::NOTHING),
            d.get_temp::<bool>(peek_id.with("allowed")).unwrap_or(true),
            d.get_temp::<String>(peek_id.with("hover")).unwrap_or_default(),
            d.get_temp::<String>(peek_id.with("shown tab")).unwrap_or_default(),
        )
    });
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let over_shown = pointer.is_some_and(|p| last.contains(p));
    let over_menu =
        pointer.is_some_and(|p| ui.ctx().layer_id_at(p).is_some_and(|l| matches!(l.order, egui::Order::Middle | egui::Order::Foreground)));
    // Only auto-hide reads the top band: a ribbon showing its tabs keeps the strip on screen, where
    // a tab is what peeks, and the strip's top pixels are the tab row's own.
    let at_edge = display == RibbonDisplay::AutoHide && at_top(pointer, ui.max_rect().top());
    let (peek, allowed) = peek_state(last_display != Some(display), on_tab || over_shown || over_menu || at_edge, allowed);
    ui.data_mut(|d| {
        d.insert_temp(peek_id.with("allowed"), allowed);
        // What the tabs report this frame decides the next one, so last frame's report is spent.
        d.insert_temp(peek_id.with("tab"), false);
        d.insert_temp(peek_id.with("hover"), String::new());
        if !peek {
            d.insert_temp(peek_id.with("shown"), Rect::NOTHING);
            d.insert_temp(peek_id.with("shown tab"), String::new());
        }
    });
    // The tab the peek shows, which is not always the tab the window is on (pure, tested): Word
    // peeks the tab the pointer is on, not the one last clicked, and a menu followed down from a
    // peek keeps the tab it came from, or the ribbon would drop it under the menu.
    let mut peek_tab: Option<String> = None;
    let want: &str = if hover.is_empty() { &shown_tab } else { &hover };
    if peek && !want.is_empty() && want != "File" && !app.ui.backstage && want != app.ui.tab {
        peek_tab = Some(want.to_string());
    }
    let at = ui.max_rect();
    match place(display, peek) {
        // Word's auto-hide: nothing of the ribbon is on screen, and the page takes the room.
        Place::Hidden => {}
        Place::Full => {
            draw_tabs(app, ui);
            draw_commands(app, ui, None);
        }
        Place::Strip => draw_tabs(app, ui),
        // The tab row keeps its place; only the commands come down, and they cover the page instead
        // of pushing it back.
        Place::StripPeek => {
            draw_tabs(app, ui);
            let shown =
                egui::Area::new(peek_id.with("peek")).fixed_pos(pos2(at.left(), at.top() + TABS_H)).order(egui::Order::Middle).show(ui.ctx(), |ui| {
                    ui.set_width(at.width());
                    ui.set_max_width(at.width());
                    draw_commands(app, ui, peek_tab);
                });
            ui.data_mut(|d| d.insert_temp(peek_id.with("shown"), shown.response.rect));
        }
        // Auto-hidden, the whole ribbon comes down over the page. Its place is what the next frame
        // measures the pointer against, so moving down from the top band into it keeps it open.
        Place::Reveal => {
            let shown = egui::Area::new(peek_id.with("peek")).fixed_pos(at.min).order(egui::Order::Middle).show(ui.ctx(), |ui| {
                ui.set_width(at.width());
                ui.set_max_width(at.width());
                draw_tabs(app, ui);
                draw_commands(app, ui, peek_tab);
            });
            ui.data_mut(|d| d.insert_temp(peek_id.with("shown"), shown.response.rect));
        }
    }
    // Only drawing the tabs tells where the pointer went, and the drawing is decided before it: so
    // when this frame's report differs from the one just used, ask for the frame that shows it.
    let (now, now_tab) =
        ui.data(|d| (d.get_temp::<bool>(peek_id.with("tab")).unwrap_or(false), d.get_temp::<String>(peek_id.with("hover")).unwrap_or_default()));
    if now != on_tab || now_tab != hover {
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests;
