//! Live sessions (co-editing, plugin protocol 2) in the interface: the engine's pump each frame,
//! the title bar's Share button and the people in the session (circles with their initials in
//! their colours), the session popover (room, people, Copy Invitation, Leave), the Share dialog
//! (start or join through a transport add-in), the session's state in the status bar, and
//! others' carets and selections drawn in their colours with name tags, their edits briefly
//! highlighted.
//!
//! The engine does the work (`wordcraft_engine::collab`); this only shows it. Everything is drawn
//! in code with theme tokens.

use egui::{Align2, Color32, Painter, Rect, Response, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_engine::collab::{self, Peer, RoomState};
use wordcraft_engine::doc::{Path, Pos, StoryRef};
use wordcraft_engine::layout::DocLayout;

use crate::WordApp;
use crate::theme::{Tokens, medium, regular, semibold};

/// How long others' edits stay highlighted (milliseconds).
const HIGHLIGHT_MS: f64 = 2_000.0;
/// How often the room's state is written for recovery.
const RECOVERY_MS: f64 = 30_000.0;
/// People shown as circles before "+N".
const MAX_CIRCLES: usize = 4;

/// `#RRGGBB` as a colour (grey when it isn't one).
pub fn color(hex: &str) -> Color32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x80_80_80);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Runs the engine's co-editing between frames.
pub fn pump(app: &mut WordApp, ctx: &egui::Context) {
    if app.session.collab.room.is_none() && !wordcraft_plugin::push::pending() {
        return;
    }
    let now = crate::now_ms();
    if app.session.collab_poll(now.max(0.0) as u64) {
        ctx.request_repaint();
    }
    for (story, path) in app.session.collab.take_remote_edits() {
        app.collab.highlights.push((story, path, now));
    }
    app.collab.highlights.retain(|h| now - h.2 < HIGHLIGHT_MS);
    if app.collab.highlights.len() > 500 {
        let extra = app.collab.highlights.len() - 500;
        app.collab.highlights.drain(..extra);
    }
    for n in app.session.collab_take_notices() {
        app.status(n);
    }
    if now - app.collab.recovery_at > RECOVERY_MS {
        app.collab.recovery_at = now;
        app.session.save_collab_recovery();
    }
    if app.session.collab.room.is_some() {
        // Remote messages arrive between frames: keep looking.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// The interface's co-editing state.
#[derive(Default)]
pub struct CollabUi {
    /// Paragraphs others changed, highlighted briefly: (story, path, when).
    pub highlights: Vec<(StoryRef, Path, f64)>,
    /// When the room's state was last kept for recovery.
    pub recovery_at: f64,
}

/// The session's state in words.
fn state_text(app: &WordApp) -> Option<String> {
    let room = app.session.active_room()?;
    if let Some(why) = collab::read_only(&app.session).filter(|_| room.joining) {
        return Some(tl!(why.as_str()).to_string());
    }
    Some(
        match room.state {
            RoomState::Online => tl!("Live"),
            RoomState::Connecting => tl!("Connecting…"),
            RoomState::Offline => tl!("Offline"),
            RoomState::Ended => tl!("Session ended"),
        }
        .to_string(),
    )
}

/// Whether any co-editing transport add-in is loaded and on.
fn has_transport() -> bool {
    !wordcraft_plugin::transports().is_empty()
}

/// The people in a live session, in the title bar (right-to-left layout): nothing outside one.
pub fn title_bar(app: &mut WordApp, ui: &mut Ui, control_h: f32) {
    if app.session.active_room().is_some() {
        people(app, ui, control_h);
    }
}

/// The tab strip's Share button (`r`, `resp`) in a live session's terms: a dot for the session's
/// state, the session popover in one, and starting one when a co-editing transport is loaded.
/// Gives `resp` back when sharing means what it does without live sessions (Export).
pub fn share_button(app: &mut WordApp, ui: &Ui, r: Rect, resp: Response) -> Option<Response> {
    let t = Tokens::get(ui.ctx());
    let in_room = app.session.active_room().is_some();
    if in_room {
        // The session's state: a dot in the button's corner.
        let dot = match app.session.active_room().map(|r| (r.state, r.joining)) {
            Some((_, true)) | Some((RoomState::Connecting, _)) => t.orange,
            Some((RoomState::Online, _)) => t.green,
            _ => t.red,
        };
        let c = pos2(if crate::rtl::on() { r.min.x + 7.0 } else { r.max.x - 7.0 }, r.min.y + 6.0);
        ui.painter().circle_filled(c, 4.0, dot);
        ui.painter().circle_stroke(c, 4.0, Stroke::new(1.0, t.on_accent));
        let tip = match state_text(app) {
            Some(s) => format!("{} — {s}", tl!("Live session")),
            None => tl!("Live session").to_string(),
        };
        let resp = resp.on_hover_text(tip);
        egui::Popup::menu(&resp).show(|ui| session_popover(ui, app));
        return None;
    }
    if !has_transport() {
        return Some(resp);
    }
    if resp.on_hover_text(tl!("Share this document in a live session")).clicked() {
        let _ = app.run("collab.share", json!({}));
    }
    None
}

/// Circles for the people in the session, overlapping slightly: at most four, then "+N". Hovering
/// one names them; clicking one goes to their caret or selection.
fn people(app: &mut WordApp, ui: &mut Ui, control_h: f32) {
    let t = Tokens::get(ui.ctx());
    let Some(room) = app.session.active_room() else { return };
    let me = room.user.clone();
    let peers = room.peers();
    // Ourselves last (rightmost of the people, next to the button), as a person among the others.
    let mut list: Vec<(Option<Peer>, wordcraft_engine::collab::User)> = peers.iter().map(|p| (Some(p.clone()), p.user.clone())).collect();
    list.push((None, me));
    let shown = list.len().min(MAX_CIRCLES);
    let more = list.len() - shown;
    let d = (control_h - 2.0).min(22.0);
    let step = d - 5.0;
    if more > 0 {
        let txt = format!("+{more}");
        let (r, resp) = ui.allocate_exact_size(vec2(26.0, d), Sense::hover());
        ui.painter().text(r.center(), Align2::CENTER_CENTER, txt, medium(11.0), t.text_dim);
        let names: Vec<String> = list.iter().skip(shown).map(|(_, u)| u.name.clone()).collect();
        resp.on_hover_text(names.join("\n"));
    }
    let width = d + step * (shown.saturating_sub(1)) as f32;
    let (area, _) = ui.allocate_exact_size(vec2(width, d), Sense::hover());
    let mut jump: Option<Peer> = None;
    // Drawn left to right, each over the one before.
    let visible: Vec<&(Option<Peer>, wordcraft_engine::collab::User)> = list.iter().take(shown).collect();
    for (i, (peer, user)) in visible.iter().enumerate() {
        let c = pos2(area.min.x + d / 2.0 + step * i as f32, area.center().y);
        let rect = Rect::from_center_size(c, vec2(d, d));
        let resp = ui.interact(rect, ui.scope_id().with(("collab_person", i)), Sense::click());
        ui.painter().circle_filled(c, d / 2.0, t.title_bar);
        ui.painter().circle_filled(c, d / 2.0 - 1.5, color(&user.color));
        let initials = user.initials();
        ui.painter().text(c, Align2::CENTER_CENTER, initials, semibold(if user.initials().chars().count() > 1 { 9.0 } else { 10.5 }), Color32::WHITE);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &user.name));
        let tip = if peer.is_some() { user.name.clone() } else { crate::i18n::fmt(tl!("{name} (you)"), &[("name", &user.name)]) };
        if resp.on_hover_text(tip).clicked()
            && let Some(p) = peer
        {
            jump = Some(p.clone());
        }
    }
    if let Some(p) = jump {
        go_to(app, &p);
    }
}

/// Moves the selection to a person's caret or selection.
fn go_to(app: &mut WordApp, p: &Peer) {
    let Some(focus) = p.focus.clone() else {
        app.status(crate::i18n::fmt(tl!("{name}'s caret isn't in a shared paragraph"), &[("name", &p.user.name)]));
        return;
    };
    let anchor = p.anchor.clone().filter(|a| a.story == focus.story).unwrap_or_else(|| focus.clone());
    let pos = |x: &Pos| serde_json::to_value(x).unwrap_or(Value::Null);
    if app.run("select.range", json!({"anchor": pos(&anchor), "focus": pos(&focus)})).is_ok() {
        app.canvas.scroll_to_caret = true;
        app.canvas.want_focus = true;
    }
}

/// The session popover: the room, its state, the people, Copy Invitation, Leave.
fn session_popover(ui: &mut Ui, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    ui.set_min_width(260.0);
    ui.set_max_width(340.0);
    let Some(room) = app.session.active_room() else { return };
    let state = state_text(app).unwrap_or_default();
    let (name, plugin, invitation, encrypted, message) =
        (room.room.clone(), room.plugin.clone(), room.invitation.clone(), room.encrypted(), room.refused.clone().or_else(|| room.message.clone()));
    let me = room.user.clone();
    let peers = room.peers();
    ui.label(RichText::new(tl!("Live session")).font(semibold(13.0)).color(t.text));
    ui.label(RichText::new(format!("{state} · {name}")).font(regular(11.5)).color(t.text_dim)).on_hover_text(plugin);
    if encrypted {
        ui.label(RichText::new(tl!("Encrypted end to end")).font(regular(11.5)).color(t.text_dim));
    }
    if let Some(m) = message {
        ui.label(RichText::new(m).font(regular(11.5)).color(t.red));
    }
    ui.separator();
    ui.label(RichText::new(tl!("People here")).font(medium(11.5)).color(t.text_dim));
    let mut jump = None;
    for (peer, user) in peers.iter().map(|p| (Some(p), &p.user)).chain(std::iter::once((None, &me))) {
        let row = ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 8.5, color(&user.color));
            ui.painter().text(r.center(), Align2::CENTER_CENTER, user.initials(), semibold(8.5), Color32::WHITE);
            let name = if peer.is_some() { user.name.clone() } else { crate::i18n::fmt(tl!("{name} (you)"), &[("name", &user.name)]) };
            ui.label(RichText::new(name).font(regular(12.0)).color(t.text));
        });
        if let Some(p) = peer
            && row.response.interact(Sense::click()).on_hover_text(tl!("Go to their caret")).clicked()
        {
            jump = Some(p.clone());
        }
    }
    ui.separator();
    if let Some(link) = invitation
        && ui.button(tl!("Copy Invitation")).clicked()
    {
        ui.ctx().copy_text(link);
        app.status(tl!("Invitation copied").to_string());
        ui.close();
    }
    if ui.button(tl!("Leave Live Session")).clicked() {
        if let Err(e) = app.run("collab.leave", json!({})) {
            app.status(e);
        }
        ui.close();
    }
    if let Some(p) = jump {
        go_to(app, &p);
        ui.close();
    }
}

/// The session's state in the status bar, and the room's message (a wrong key, another app's
/// room) when there is one.
pub fn status(app: &WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(room) = app.session.active_room() else { return };
    let state = state_text(app).unwrap_or_default();
    let n = room.peers().len();
    let text = if n > 0 {
        format!("{} · {}", tl!("Live session"), crate::i18n::fmt(tl!("{count} others"), &[("count", &n.to_string())]))
    } else {
        tl!("Live session").to_string()
    };
    ui.label(RichText::new(format!("{text} · {state}")).font(regular(12.0)).color(t.text_dim)).on_hover_text(room.room.clone());
    if let Some(m) = room.refused.clone().or_else(|| room.message.clone()) {
        ui.label(RichText::new(m).font(regular(12.0)).color(t.red));
    }
}

/// Others' selections (shaded in their colours), carets (a bar with their name) and recent
/// edits (a fading mark). `scale(page, page rect)` is the page's screen scale.
pub fn paint_remote(app: &WordApp, painter: &Painter, layout: &DocLayout, rects: &[Rect], scale: &dyn Fn(usize, Rect) -> f32) {
    let to_screen = |pi: usize, x: f32, y: f32, w: f32, h: f32| -> Option<Rect> {
        let pr = *rects.get(pi)?;
        let s = scale(pi, pr);
        Some(Rect::from_min_size(pos2(pr.min.x + x * s, pr.min.y + y * s), vec2(w * s, h * s)))
    };
    let now = crate::now_ms();
    let doc = &app.session.doc;
    let hint = app.session.page_hint;
    for (story, path, at) in &app.collab.highlights {
        let Some(p) = doc.para(*story, path) else { continue };
        let fade = (1.0 - (now - at) / HIGHLIGHT_MS).clamp(0.0, 1.0) as f32;
        let (a, b) = (Pos { story: *story, path: path.clone(), off: 0 }, Pos { story: *story, path: path.clone(), off: p.text.len() });
        for (pi, r) in layout.selection_rects(doc, &a, &b, hint).into_iter().take(200) {
            if let Some(sr) = to_screen(pi, r.x - 4.0, r.y, 2.5, r.h) {
                painter.rect_filled(sr, 1.0, Color32::from_rgba_unmultiplied(255, 176, 0, (200.0 * fade) as u8));
            }
        }
    }
    let Some(room) = app.session.active_room() else { return };
    for peer in room.peers() {
        let c = color(&peer.user.color);
        let Some(focus) = peer.focus.clone().map(|f| doc.clamp(&f)) else { continue };
        if let Some(anchor) = peer.anchor.clone().map(|a| doc.clamp(&a))
            && anchor != focus
            && anchor.story == focus.story
        {
            let (a, b) = if anchor <= focus { (&anchor, &focus) } else { (&focus, &anchor) };
            for (pi, r) in layout.selection_rects(doc, a, b, hint).into_iter().take(2_000) {
                if let Some(sr) = to_screen(pi, r.x, r.y, r.w, r.h) {
                    painter.rect_filled(sr, 0.0, c.gamma_multiply(0.18));
                }
            }
        }
        let Some(caret) = layout.caret_on(&focus, hint) else { continue };
        let Some(bar) = to_screen(caret.page, caret.x, caret.top, 0.0, caret.height) else { continue };
        painter.line_segment([bar.left_top(), bar.left_bottom()], Stroke::new(2.0, c));
        let galley = painter.layout_no_wrap(peer.user.name.clone(), medium(10.0), Color32::WHITE);
        let tag = Rect::from_min_size(pos2(bar.min.x - 1.0, bar.min.y - galley.size().y - 3.0), galley.size() + vec2(8.0, 3.0));
        painter.rect_filled(tag, egui::CornerRadius { nw: 3, ne: 3, se: 3, sw: 0 }, c);
        painter.galley(tag.min + vec2(4.0, 1.5), galley, Color32::WHITE);
    }
}

// ---------------------------------------------------------------- the Share dialog

/// The Share dialog: start a live session from this document, or join one, through a transport
/// add-in (its `share` and `join` commands answer with the room to join).
#[derive(Clone, Debug, serde::Serialize)]
pub struct ShareDialog {
    pub join: bool,
    /// Transport add-ins: (id, label).
    pub transports: Vec<(String, String)>,
    pub transport: usize,
    pub name: String,
    pub color: String,
    pub encrypt: bool,
    pub link: String,
    /// Joining replaces the document: the user agreed to lose its unsaved changes.
    pub discard: bool,
    pub error: String,
}

impl ShareDialog {
    pub fn new(app: &mut WordApp, join: bool) -> ShareDialog {
        let st = app.session.run("collab.status", &json!({})).unwrap_or_default();
        let transports = st["transports"]
            .as_array()
            .map(|a| a.iter().map(|t| (t["id"].as_str().unwrap_or("").to_string(), t["label"].as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        let user = &st["user"];
        ShareDialog {
            join,
            transports,
            transport: 0,
            name: user["name"].as_str().unwrap_or("").to_string(),
            color: user["color"].as_str().unwrap_or("#3B7DD8").to_string(),
            encrypt: true,
            link: String::new(),
            discard: false,
            error: String::new(),
        }
    }
}

enum Action {
    None,
    Close,
    Go,
}

pub fn show_dialog(app: &mut WordApp, ctx: &egui::Context) {
    let Some(crate::dialogs::Dialog::Share(mut d)) = app.dialog.take() else { return };
    let mut open = true;
    let mut action = Action::None;
    let dirty = app.session.dirty;
    let id = egui::Id::unique("collab-share-dialog");
    let window = egui::Window::new(tl!("Live Session")).id(id).collapsible(false).resizable(false);
    let shown = crate::dialogs::place_dialog(ctx, id, window).open(&mut open).show(ctx, |ui| {
        ui.set_min_width(380.0);
        ui.set_max_width(460.0);
        action = body(ui, &mut d, dirty);
    });
    if shown.is_some() {
        crate::dialogs::count_dialog_frame(ctx, id);
    }
    if !open {
        action = Action::Close;
    }
    match action {
        Action::None => app.dialog = Some(crate::dialogs::Dialog::Share(d)),
        Action::Close => app.canvas.want_focus = true,
        Action::Go => {
            if let Err(e) = go(app, &d) {
                d.error = e;
                app.dialog = Some(crate::dialogs::Dialog::Share(d));
            } else {
                app.canvas.want_focus = true;
            }
        }
    }
}

fn body(ui: &mut Ui, d: &mut ShareDialog, dirty: bool) -> Action {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.radio_value(&mut d.join, false, tl!("Start a live session"));
        ui.radio_value(&mut d.join, true, tl!("Join a live session"));
    });
    ui.add_space(6.0);
    if d.transports.is_empty() {
        ui.label(RichText::new(tl!("Live sessions need a co-editing add-in (a transport). Add one in File › Options › Add-ins.")).color(t.text_dim));
    }
    egui::Grid::new("share_grid").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
        ui.label(tl!("Connection:"));
        let current = d.transports.get(d.transport).map(|(_, l)| l.clone()).unwrap_or_default();
        egui::ComboBox::from_id_salt("share_transport").selected_text(current).width(240.0).show_ui(ui, |ui| {
            for (i, (_, label)) in d.transports.iter().enumerate() {
                ui.selectable_value(&mut d.transport, i, label);
            }
        });
        ui.end_row();
        ui.label(tl!("Your name:"));
        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(240.0));
        ui.end_row();
        ui.label(tl!("Your colour:"));
        let mut c = color(&d.color);
        if ui.color_edit_button_srgba(&mut c).changed() {
            d.color = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
        }
        ui.end_row();
        if d.join {
            ui.label(tl!("Invitation:"));
            ui.add(egui::TextEdit::singleline(&mut d.link).desired_width(240.0).hint_text(tl!("Paste the invitation link")));
            ui.end_row();
        }
    });
    if d.join {
        if dirty {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Joining replaces this document, which has unsaved changes.")).color(t.orange));
            ui.checkbox(&mut d.discard, tl!("Discard my unsaved changes"));
        }
    } else {
        ui.checkbox(&mut d.encrypt, tl!("Encrypt end to end"));
    }
    if !d.error.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new(&d.error).color(t.red));
    }
    ui.add_space(8.0);
    let ready = !d.transports.is_empty() && !d.name.trim().is_empty() && (!d.join || (!d.link.trim().is_empty() && (!dirty || d.discard)));
    let mut action = Action::None;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(tl!("Cancel")).clicked() {
            action = Action::Close;
        }
        let go = if d.join { tl!("Join") } else { tl!("Start") };
        if ui.add_enabled(ready, egui::Button::new(go)).clicked() {
            action = Action::Go;
        }
    });
    action
}

/// Runs the transport's `share` or `join` command; its answer's `collab.join` starts the session
/// (see `plugin_ui::after_run`).
fn go(app: &mut WordApp, d: &ShareDialog) -> Result<(), String> {
    let (plugin, _) = d.transports.get(d.transport).cloned().ok_or_else(|| tl!("Choose a connection").to_string())?;
    app.run("collab.setUser", json!({"name": d.name.trim(), "color": d.color}))?;
    let params = if d.join {
        if d.discard {
            // The user chose to lose the unsaved changes: joining may replace the document.
            app.session.dirty = false;
        }
        json!({"link": d.link.trim(), "name": d.name.trim(), "color": d.color})
    } else {
        json!({"name": d.name.trim(), "color": d.color, "encrypt": d.encrypt})
    };
    let command = if d.join { "join" } else { "share" };
    if wordcraft_plugin::get(&plugin).is_some_and(|p| p.manifest.command(command).is_none()) {
        return Err(crate::i18n::fmt(tl!("This add-in has no {command} command"), &[("command", command)]));
    }
    app.run("plugins.run", json!({"plugin": plugin, "command": command, "params": params}))?;
    if app.session.active_room().is_none() && app.dialog.is_none() {
        return Err(tl!("The add-in didn't start a live session").to_string());
    }
    Ok(())
}
