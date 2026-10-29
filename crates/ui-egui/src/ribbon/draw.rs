use super::common::{mi, stack};
use super::language::mi_check;
use super::{CONTENT_H, Rect, Sense, Stroke, Target, Tokens, Ui, WordApp, big, big_toggle, group, icons, json, menu_button, pos2, split, vec2};

pub(super) fn draw(app: &mut WordApp, ui: &mut Ui) {
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
        let hl_ink = crate::theme::c32(app.session.view.draw.settings(InkTool::Highlighter).color);
        let hl_on = mode == DrawMode::Pen(InkTool::Highlighter) && custom.is_none();
        crate::widgets::big_toggle_swatch(ui, app, "highlight", "Highlighter", "draw.highlighter", hl_on, hl_ink);
        // Pens added with Add Pen; right-click one to remove it.
        let pens = app.session.view.draw.pens.clone();
        for (i, pen) in pens.iter().enumerate() {
            gallery_pen(ui, app, i, *pen, custom == Some(i));
        }
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("addPen", Some("Add\nPen"), "Add Pen", true, ""), |ui, app| {
            for (label, kind) in [("Pen", InkTool::Pen), ("Pencil", InkTool::Pencil), ("Highlighter", InkTool::Highlighter)] {
                // A new pen starts as that kind's current colour and thickness.
                let cur = app.session.view.draw.settings(kind);
                mi(ui, app, label, "draw.addPen", json!({"kind": kind.name(), "color": cur.color.hex(), "width": cur.width}));
            }
        });
        stack(ui, |ui| {
            let sw = Some(crate::theme::c32(set.color));
            split(ui, app, "fontcolor", "Color", crate::widgets::ButtonAction::new(pen_command(tool), json!({}), false), sw, |ui, app| {
                let theme = app.session.doc.settings.theme_colors.clone();
                // With ink lasso-selected the colour goes to the selection, else to the pen.
                let cmd = if lasso_on { "draw.lassoColor" } else { pen_command(tool) };
                crate::colors::menu(ui, app, &theme, Target::cmd(cmd));
                if !lasso_on {
                    update_gallery_pen(app, custom, tool);
                }
            });
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("thickness", None, "Thickness", false, "draw.pen"), |ui, app| {
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
    icons::paint_state(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(28.0, 28.0)), icon, t.icon, color, false);
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
