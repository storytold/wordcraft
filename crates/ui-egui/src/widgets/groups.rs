use super::buttons::{BIG_ICON_Y, BIG_LABEL_Y, CONTENT_H, DROPDOWN, LABEL_H, bg, label_line};
use super::*;

/// A ribbon group: content, a centred label below and a divider on the right.
pub fn group(ui: &mut Ui, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    // The English title keys the launcher's id; the drawn title is translated.
    group_titled(ui, title, tl!(title), launcher, app, add);
}

/// A ribbon group whose title is shown as is (a custom group's name, Customize Ribbon).
pub fn group_titled(ui: &mut Ui, id_title: &str, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    let t = Tokens::get(ui.ctx());
    let start = ui.cursor().min;
    // #662: the window may be too narrow for the tab; the plan says how this group fits.
    let position = crate::ribbon_fit::position(ui.ctx());
    let level = crate::ribbon_fit::level(ui.ctx(), id_title);
    if level == crate::ribbon_fit::COLLAPSED {
        collapsed_group(ui, id_title, position, title, launcher, app, add);
    } else {
        // Not folded: a drop-down left open while it was folded doesn't come back by itself.
        ui.data_mut(|d| d.remove::<bool>(collapsed_open_id(ui, id_title, position)));
        let label_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.0), t.group_label).size().x);
        let inner = ui.vertical(|ui| {
            ui.set_min_width(label_w + if launcher.is_some() { 22.0 } else { 10.0 });
            crate::ribbon_fit::set_compact(ui.ctx(), level == crate::ribbon_fit::COMPACT);
            let content = ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTENT_H), egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                add(ui, app);
            });
            crate::ribbon_fit::end_compact(ui.ctx());
            #[cfg(test)]
            if content.response.rect.height() > CONTENT_H + 0.5 {
                TALL_GROUPS.with(|g| g.borrow_mut().push(format!("{id_title}: {:.1}", content.response.rect.height())));
            }
            #[cfg(not(test))]
            let _ = content;
        });
        let r = inner.response.rect;
        title_row(
            ui,
            Rect::from_min_max(pos2(start.x, start.y + CONTENT_H), pos2(r.max.x, start.y + CONTENT_H + LABEL_H)),
            id_title,
            title,
            launcher,
            app,
        );
    }
    ui.add_space(5.0);
    let x = ui.cursor().min.x;
    ui.painter().line_segment([pos2(x, start.y + 4.0), pos2(x, start.y + CONTENT_H + LABEL_H - 4.0)], Stroke::new(1.0, t.border));
    ui.add_space(6.0);
    crate::ribbon_fit::record(ui.ctx(), id_title, level, ui.cursor().min.x - start.x);
}

/// A group's title row (`r`): the centred title and, with a `launcher`, its dialog launcher.
fn title_row(ui: &mut Ui, r: Rect, id_title: &str, title: &str, launcher: Option<&str>, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let title_y = r.center().y;
    ui.painter().text(pos2(r.center().x, title_y), Align2::CENTER_CENTER, title, regular(11.0), t.group_label);
    if let Some(cmd) = launcher {
        let lr = Rect::from_center_size(pos2(r.max.x - 6.0, title_y), vec2(11.0, 11.0));
        let resp = ui.interact(lr, ui.scope_id().with(("launch", id_title)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(lr.expand(2.0), 2.0, t.hover);
        }
        icons::paint(ui.painter(), lr, "launcher", t.text_dim, t.accent);
        if resp.on_hover_text(crate::i18n::fmt(tl!("{group} settings"), &[("group", title)])).clicked() {
            let _ = app.run(cmd, serde_json::json!({}));
        }
    }
}

/// Where a collapsed group remembers whether its drop-down is open.
fn collapsed_open_id(ui: &Ui, id_title: &str, position: usize) -> egui::Id {
    ui.scope_id().with(("collapsed_group", id_title, position))
}

/// A group folded into one drop-down button (the window is too narrow, #662): the button, as tall
/// as the group with its title, shows the group's name, and its drop-down holds the whole group as
/// designed with its title and launcher. The drop-down stays open while it is used (#1285): only a
/// click outside it, or on the button, closes it; clicks in the menus and lists its controls open
/// (font and size lists, Change Case, colours) count as inside.
fn collapsed_group(
    ui: &mut Ui,
    id_title: &str,
    position: usize,
    title: &str,
    launcher: Option<&str>,
    app: &mut WordApp,
    add: impl FnOnce(&mut Ui, &mut WordApp),
) {
    let t = Tokens::get(ui.ctx());
    let text_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.5), t.text).size().x);
    let (r, resp) = ui.allocate_exact_size(vec2((text_w + DROPDOWN + 14.0).max(48.0), CONTENT_H + LABEL_H), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let open_id = collapsed_open_id(ui, id_title, position);
    let was_open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    bg(ui, r, &resp, was_open, &t);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(r.center().x, r.min.y + BIG_ICON_Y), vec2(32.0, 32.0)),
        group_icon(id_title),
        t.icon,
        t.accent,
    );
    label_line(ui, r.center().x, r.min.y + BIG_LABEL_Y, title, regular(11.5), t.text, true);
    let resp = resp.on_hover_text(title);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let mut open = was_open;
    if resp.clicked() {
        open = !open;
    } else if open && ui.input(|i| i.pointer.any_click()) {
        // Popups (the drop-down, its controls' menus and lists) sit above the ribbon; a click on
        // the window below them is outside.
        let pos = ui.input(|i| i.pointer.interact_pos());
        let layer = pos.and_then(|p| ui.ctx().layer_id_at(p));
        if !layer.is_some_and(|l| matches!(l.order, egui::Order::Foreground | egui::Order::Tooltip)) {
            open = false;
        }
    }
    // Esc closes an open list inside first, then the drop-down.
    let child_open = egui::Popup::is_any_open(ui.ctx());
    let mut shown = open;
    if open {
        // This popup isn't counted by `Popup::is_any_open`, so keep the same Esc from also leaving
        // full screen.
        app.escape_free = false;
        egui::Popup::menu(&resp).open_bool(&mut shown).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(|ui| {
            ui.vertical(|ui| {
                ui.horizontal_top(|ui| {
                    ui.set_min_height(CONTENT_H);
                    ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                    add(ui, app);
                });
                let (row, _) = ui.allocate_exact_size(vec2(ui.min_rect().width(), LABEL_H), Sense::hover());
                title_row(ui, row, id_title, title, launcher, app);
            });
        });
        if !shown && child_open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            shown = true;
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, shown));
}

/// The icon a collapsed group's button shows.
fn group_icon(title: &str) -> &'static str {
    match title {
        "Clipboard" => "paste",
        "Font" => "bold",
        "Paragraph" => "alignLeft",
        "Styles" => "styles",
        "Editing" => "find",
        "Voice" => "dictate",
        "Tables" => "table",
        "Links" => "link",
        _ => "more",
    }
}

/// The icon a command's button shows outside its own ribbon group (custom groups, the Quick
/// Access Toolbar): the one its ribbon button uses, which is usually named after the id's last
/// part. Names the icon set doesn't have draw a lettered tile ([`icons::paint`]).
pub fn command_icon(id: &str) -> &str {
    const ICONS: &[(&str, &str)] = &[
        ("arrange.wrap", "wrapText"),
        ("draw.highlighter", "highlight"),
        ("format.border", "charborder"),
        ("format.color", "fontcolor"),
        ("format.growFont", "grow"),
        ("format.shrinkFont", "shrink"),
        ("format.strikethrough", "strike"),
        ("format.highlight", "highlight"),
        ("insert.signatureLine", "signature"),
        ("mailings.envelopes", "envelope"),
        ("mailings.insertField", "mergeField"),
        ("para.ltr", "textLtr"),
        ("para.rtl", "textRtl"),
        ("para.bullets", "bullets"),
        ("para.numbering", "numbering"),
        ("table.insertRowAbove", "insertAbove"),
        ("view.commentsPane", "showComments"),
        ("view.marks", "pilcrow"),
        ("edit.repeat", "redo"),
        ("file.new", "blankPage"),
        ("tools.customizeRibbon", "more"),
    ];
    ICONS.iter().find(|(c, _)| *c == id).map_or_else(|| id.rsplit('.').next().unwrap_or(id), |(_, icon)| *icon)
}

/// Right-click on a command's button: put it on (or take it off) the Quick Access Toolbar, or
/// open Customize Ribbon (#379).
pub fn command_menu(resp: &Response, app: &mut WordApp, id: &str) {
    resp.context_menu(|ui| {
        let known = app.session.registry.get(id).is_some();
        let on_qat = app.session.ribbon.qat().contains(&id);
        if known && !on_qat && ui.button(tl!("Add to Quick Access Toolbar")).clicked() {
            let _ = app.run("tools.customizeRibbon", serde_json::json!({"qatAdd": id}));
            ui.close();
        }
        if on_qat && ui.button(tl!("Remove from Quick Access Toolbar")).clicked() {
            let _ = app.run("tools.customizeRibbon", serde_json::json!({"qatRemove": id}));
            ui.close();
        }
        if known {
            ui.separator();
        }
        customize_menu_items(ui, app);
    });
}

/// "Customize the Ribbon…" and "Customize Quick Access Toolbar…" (context menus), and "Show
/// Quick Access Toolbar" while the toolbar is hidden (#1945).
pub fn customize_menu_items(ui: &mut Ui, app: &mut WordApp) {
    if !app.ui.quick_access_toolbar && ui.button(tl!("Show Quick Access Toolbar")).clicked() {
        let _ = app.run("ui.quickAccessToolbar", serde_json::json!({"value": true}));
        ui.close();
    }
    if ui.button(tl!("Customize the Ribbon…")).clicked() {
        crate::dialogs_ribbon::open(app, crate::dialogs_ribbon::Page::Ribbon);
        ui.close();
    }
    if ui.button(tl!("Customize Quick Access Toolbar…")).clicked() {
        crate::dialogs_ribbon::open(app, crate::dialogs_ribbon::Page::QuickAccess);
        ui.close();
    }
}

/// A row of small buttons (used inside a vertical stack).
pub fn row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        add(ui);
    });
}
