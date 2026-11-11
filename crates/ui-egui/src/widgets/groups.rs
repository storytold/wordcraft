#[cfg(test)]
use super::TALL_GROUPS;
use super::buttons::{BIG_ICON_Y, CONTENT_H, DROPDOWN, LABEL_H, LABEL_LINE_H, bg, label_line};
use super::{Align2, Color32, Rect, Response, Sense, Stroke, Tokens, Ui, WordApp, icons, pos2, regular, vec2};
/// A ribbon group: content, a centred label below and a divider on the right.
pub fn group(ui: &mut Ui, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    // The English title keys the launcher's id; the drawn title is translated.
    group_titled(ui, title, tl!(title), launcher, app, add);
}

/// A ribbon group whose title is shown as is (a custom group's name, Customize Ribbon).
pub fn group_titled(ui: &mut Ui, id_title: &str, title: &str, launcher: Option<&str>, app: &mut WordApp, add: impl FnOnce(&mut Ui, &mut WordApp)) {
    let t = Tokens::get(ui.ctx());
    // Groups follow the ribbon's direction: in a right-to-left interface the content starts at
    // the right, and the launcher and the divider sit on the left.
    let rtl = crate::rtl::flows(ui);
    let start_cursor = ui.cursor();
    let start = start_cursor.min;
    // #662: the window may be too narrow for the tab; the plan says how this group fits.
    let position = crate::ribbon_fit::position(ui.ctx());
    let level = crate::ribbon_fit::level(ui.ctx(), id_title);
    if level == crate::ribbon_fit::COMPACT {
        compact_group(ui, id_title, position, title, launcher, app, add);
    } else {
        // Not folded: a drop-down left open while it was folded doesn't come back by itself.
        ui.data_mut(|d| d.remove::<bool>(compact_open_id(ui, id_title, position)));
        let label_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.0), t.group_label).size().x);
        // Reserve the paint slot before children so the glow stays beneath the controls.
        let glow = ui.painter().add(egui::Shape::Noop);
        let inner = crate::rtl::vertical(ui, |ui| {
            ui.set_min_width(label_w + if launcher.is_some() { 22.0 } else { 10.0 });
            let content = ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTENT_H), crate::rtl::row_layout(ui, egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                add(ui, app);
            });
            #[cfg(test)]
            if content.response.rect.height() > CONTENT_H + 0.5 {
                TALL_GROUPS.with(|g| g.borrow_mut().push(format!("{id_title}: {:.1}", content.response.rect.height())));
            }
            #[cfg(not(test))]
            let _ = content;
        });
        let r = inner.response.rect;
        let area = group_area(rtl, start, r);
        let over = ui.rect_contains_pointer(area);
        // Position is part of identity because duplicate custom group names can appear in tabs.
        let lit = ui.ctx().animate_bool_responsive(ui.scope_id().with(("group_glow", id_title, position)), over);
        if lit > 0.0 {
            ui.painter().set(glow, group_glow(area, t.accent, lit));
        }
        // The band runs the group's full width along the row. Take it from the row's cursor, not
        // from the child's rect: a COMPACT Font or Paragraph group can lay its content out past
        // that rect, and the label was then centred far off the card and never drawn.
        let band = if rtl {
            Rect::from_min_max(pos2(ui.cursor().max.x, start.y + CONTENT_H), pos2(start_cursor.max.x, start.y + CONTENT_H + LABEL_H))
        } else {
            Rect::from_min_max(pos2(start.x, start.y + CONTENT_H), pos2(ui.cursor().min.x, start.y + CONTENT_H + LABEL_H))
        };
        title_row(ui, band, id_title, title, launcher, app);
    }
    ui.add_space(5.0);
    let x = if rtl { ui.cursor().max.x } else { ui.cursor().min.x };
    ui.painter().line_segment([pos2(x, start.y + 4.0), pos2(x, start.y + CONTENT_H + LABEL_H - 4.0)], Stroke::new(1.0, t.border));
    ui.add_space(6.0);
    crate::ribbon_fit::record(ui.ctx(), id_title, level, group_width(rtl, start_cursor, ui.cursor()));
}

/// The hover area of a group. A right-to-left row has no finite left edge at the cursor, so the
/// area starts at the content's own left edge there.
fn group_area(rtl: bool, start: egui::Pos2, content: Rect) -> Rect {
    let left = if rtl { content.min.x } else { start.x };
    Rect::from_min_max(pos2(left, start.y), pos2(content.max.x, start.y + CONTENT_H + LABEL_H))
}

/// The width a group took: the cursor's way along the row, from `before` to `after`. A
/// right-to-left row fills from its right edge, where the cursor's left edge is negative infinity,
/// so there the width is how far the right edge moved.
fn group_width(rtl: bool, before: Rect, after: Rect) -> f32 {
    if rtl { before.max.x - after.max.x } else { after.min.x - before.min.x }
}

/// A soft accent glow along the bottom of a hovered ribbon group.
fn group_glow(area: Rect, accent: Color32, strength: f32) -> egui::Shape {
    let color = |alpha: f32| Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (alpha * strength * 255.0) as u8);
    let r = Rect::from_min_max(pos2(area.min.x, area.max.y - 7.0), pos2(area.max.x, area.max.y + 2.0));
    let soft = (r.width() * 0.3).min(22.0);
    let xs = [r.min.x, r.min.x + soft, r.max.x - soft, r.max.x];
    let mut mesh = egui::epaint::Mesh::default();
    for (y, alpha) in [(r.max.y, 0.3), (r.min.y, 0.0)] {
        for (i, x) in xs.iter().enumerate() {
            mesh.colored_vertex(pos2(*x, y), color(if i == 0 || i == 3 { 0.0 } else { alpha }));
        }
    }
    for i in 0..3u32 {
        mesh.add_triangle(i, i + 1, i + 5);
        mesh.add_triangle(i, i + 5, i + 4);
    }
    egui::Shape::mesh(mesh)
}

/// A group's title row (`r`): the centred title and, with a `launcher`, its dialog launcher.
fn title_row(ui: &mut Ui, r: Rect, id_title: &str, title: &str, launcher: Option<&str>, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let title_y = r.center().y;
    let rtl = crate::rtl::flows(ui);
    ui.painter().text(pos2(r.center().x, title_y), Align2::CENTER_CENTER, title, regular(11.0), t.group_label);
    #[cfg(test)]
    {
        let size = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.0), t.group_label).size());
        crate::widgets::GROUP_LABELS.with(|g| {
            g.borrow_mut().push(crate::widgets::DrawnGroupLabel {
                title: title.to_string(),
                band: r,
                text: Rect::from_center_size(pos2(r.center().x, title_y), size),
            });
        });
    }
    if let Some(cmd) = launcher {
        let lr = Rect::from_center_size(pos2(crate::rtl::trail_x(rtl, r, 6.0), title_y), vec2(11.0, 11.0));
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

/// Where a compact group remembers whether its drop-down is open.
fn compact_open_id(ui: &Ui, id_title: &str, position: usize) -> egui::Id {
    ui.scope_id().with(("compact_group", id_title, position))
}

/// A section drawn as one row (the window is too narrow for its natural rows, #662): a drop-down
/// button with the group's icon, name and chevron, and the group's name in the label band below,
/// as Word draws a section it has made one row. The drop-down holds the whole group as designed
/// with its title and launcher. The drop-down stays open while it is used (#1285): only a click
/// outside it, or on the button, closes it; clicks in the menus and lists its controls open (font
/// and size lists, Change Case, colours) count as inside.
fn compact_group(
    ui: &mut Ui,
    id_title: &str,
    position: usize,
    title: &str,
    launcher: Option<&str>,
    app: &mut WordApp,
    add: impl FnOnce(&mut Ui, &mut WordApp),
) {
    let t = Tokens::get(ui.ctx());
    let size = ui.ctx().fonts_mut(|f| f.layout_no_wrap(title.to_string(), regular(11.5), t.text).size());
    let w = (size.x + 18.0 + 6.0 + DROPDOWN + 14.0).max(64.0);
    // The row is as tall as every other section's content, so the label band stays on one line.
    let (r, _) = ui.allocate_exact_size(vec2(w, CONTENT_H), Sense::hover());
    let br = Rect::from_center_size(pos2(r.center().x, r.min.y + CONTENT_H / 2.0), vec2(w, 28.0));
    let resp = ui.interact(br, ui.scope_id().with(("compact_button", id_title, position)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let open_id = compact_open_id(ui, id_title, position);
    let was_open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    bg(ui, br, &resp, was_open, &t);
    // Icon, name and chevron, centred in the one row.
    let content_w = 18.0 + 6.0 + size.x + 4.0 + DROPDOWN;
    let x0 = br.center().x - content_w / 2.0;
    icons::paint(ui.painter(), Rect::from_min_size(pos2(x0, br.center().y - 9.0), vec2(18.0, 18.0)), group_icon(id_title), t.icon, t.accent);
    let text_x = x0 + 18.0 + 6.0;
    let galley = ui.painter().layout_no_wrap(title.to_string(), regular(11.5), t.text);
    ui.painter().galley(pos2(text_x, br.center().y - galley.size().y / 2.0), galley.clone(), t.text);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(text_x + size.x + 4.0 + DROPDOWN / 2.0, br.center().y), vec2(DROPDOWN, DROPDOWN)),
        "dropdown",
        t.text,
        t.accent,
    );
    let band = Rect::from_min_max(pos2(r.min.x, r.min.y + CONTENT_H), pos2(r.max.x, r.min.y + CONTENT_H + LABEL_H));
    title_row(ui, band, id_title, title, None, app);
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
        crate::rtl::menu(&resp).open_bool(&mut shown).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(|ui| {
            crate::rtl::vertical(ui, |ui| {
                crate::rtl::horizontal_top(ui, |ui| {
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
    crate::rtl::context_menu(resp, |ui| {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A right-to-left row's cursor reaches to negative infinity on the left, so the width of a
    /// group in it comes from the right edge: finite and positive, as the ribbon planner needs.
    #[test]
    fn a_group_in_a_right_to_left_row_has_a_finite_width() {
        let before = Rect::from_min_max(pos2(f32::NEG_INFINITY, 0.0), pos2(900.0, 90.0));
        let after = Rect::from_min_max(pos2(f32::NEG_INFINITY, 0.0), pos2(740.0, 90.0));
        assert_eq!(group_width(true, before, after), 160.0);
        assert!(!(before.min.x - after.min.x).is_finite(), "the left edge cannot measure it");
        let before = Rect::from_min_max(pos2(20.0, 0.0), pos2(f32::INFINITY, 90.0));
        let after = Rect::from_min_max(pos2(180.0, 0.0), pos2(f32::INFINITY, 90.0));
        assert_eq!(group_width(false, before, after), 160.0);
    }

    #[test]
    fn a_group_in_a_right_to_left_row_has_a_finite_hover_area() {
        let start = pos2(f32::NEG_INFINITY, 10.0);
        let content = Rect::from_min_max(pos2(600.0, 10.0), pos2(740.0, 80.0));
        let area = group_area(true, start, content);
        assert!(area.min.x.is_finite() && area.max.x.is_finite());
        assert_eq!((area.min.x, area.max.x), (600.0, 740.0));
        let next = group_area(true, start, Rect::from_min_max(pos2(450.0, 10.0), pos2(590.0, 80.0)));
        assert!(!area.intersects(next), "neighbouring groups do not overlap");
    }
}
