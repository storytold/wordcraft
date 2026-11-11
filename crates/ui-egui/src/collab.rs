//! Collaboration UI and app wiring (Review › Collaborate).
//!
//! The heavy lifting is in `wordcraft-collab-app`; this module only connects a
//! session to it, turns user commands into operations, polls answers, draws the
//! participants panel and remote carets, and keeps the single-user path intact
//! when the feature is off.

use egui::{Align2, Color32, FontId, Painter, Pos2, Stroke};

use wordcraft_doc::{Document, Pos};
use wordcraft_engine::CommandSpec;
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::canvas::Geometry;

/// Commands that are local to this window or the cabinet, never sent to a
/// room, even when the engine marks them as mutating.
const LOCAL_PREFIXES: &[&str] =
    &["file.", "ui.", "view.", "tools.", "collab.", "edit.undo", "edit.redo", "edit.repeat", "review.readAloud", "help.", "app."];

fn is_editing(app: &WordApp, id: &str) -> bool {
    if LOCAL_PREFIXES.iter().any(|prefix| id.starts_with(prefix)) {
        return false;
    }
    app.session.registry.get(id).is_some_and(|spec: &CommandSpec| spec.mutates)
}

/// The document before a user command, when a room is live and the command is
/// one we replicate. `None` keeps the call cheap in single-user mode.
pub(crate) fn before_command(app: &WordApp, id: &str) -> Option<Document> {
    if app.collab.is_none() || !is_editing(app, id) {
        return None;
    }
    Some(app.session.doc.clone())
}

/// Send the difference a local command made.
pub(crate) fn after_command(app: &mut WordApp, before: &Document, id: &str, params: &serde_json::Value) {
    let result = {
        let Some(collab) = app.collab.as_mut() else { return };
        collab.after_command(before, &app.session, id, params)
    };
    if let Err(message) = result {
        app.status(message);
    }
}

/// Why editing pauses right now, in words the writer can act on.
pub(crate) fn pause_message(status: &crate::collab_session::Status) -> &'static str {
    match status {
        // The edit is in this window and not in the room (finding 8): saying
        // "reconnecting" would hide a state a resync has to repair.
        crate::collab_session::Status::Diverged => "An edit did not reach the room; editing pauses until the session is synchronized",
        _ => "Collaboration is reconnecting; editing resumes when it is synchronized",
    }
}

/// Why the Commit button is disabled right now.
fn commit_blocked_message(status: &crate::collab_session::Status) -> &'static str {
    match status {
        crate::collab_session::Status::Diverged => {
            "An edit did not reach the room: the file would be built without it. Committing resumes after the resync"
        }
        _ => "Committing resumes when the session is synchronized",
    }
}

/// Refuse a document edit on a read-only session before the engine runs it.
pub(crate) fn viewer_refusal(app: &WordApp, id: &str) -> Option<String> {
    let collab = app.collab.as_ref()?;
    if !is_editing(app, id) {
        return None;
    }
    if *collab.status() != crate::collab_session::Status::InSync {
        return Some(pause_message(collab.status()).into());
    }
    if collab.role().can_write() {
        return None;
    }
    Some("This session is read-only".into())
}

/// Connect using the address the `collab.enable` command stored. The token is
/// read from the environment, never from preferences or the document.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn connect(app: &mut WordApp) {
    let server = app.session.prefs.collab.server.clone();
    let Ok(token) = std::env::var("WORDCRAFT_COLLAB_TOKEN") else {
        app.status("Collaboration needs WORDCRAFT_COLLAB_TOKEN");
        return;
    };
    let client_id = crate::collab_session::session_client_id("ui", app.session.document_id());
    match crate::collab_session::Collab::connect(&server, &token, &app.session.author, &client_id, &app.session.doc) {
        Ok(collab) => app.collab = Some(collab),
        Err(message) => app.status(message),
    }
}

/// On the web the host page (the cabinet) opens the session: there is no
/// server address or token to read locally, and no network call without
/// `wordcraft:session`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn connect(app: &mut WordApp) {
    app.status("Collaboration is opened by the page that hosts WordCraft");
}

pub(crate) fn disconnect(app: &mut WordApp) {
    app.collab = None;
}

/// Drop a collab command prefix from the wire (the session settings are local).
pub(crate) fn is_settings(id: &str) -> bool {
    id.starts_with("collab.")
}

/// Poll the transport, apply answers and publish this window's caret. Called
/// once per frame before layout, so a remote edit is laid out in this frame.
pub(crate) fn tick(app: &mut WordApp, ctx: &egui::Context) {
    let copy = {
        let Some(collab) = app.collab.as_mut() else { return };
        let changed = collab.poll(&mut app.session);
        if changed {
            app.session.relayout();
        }
        #[cfg(not(target_arch = "wasm32"))]
        collab.publish_presence(&app.session, std::time::Instant::now());
        #[cfg(target_arch = "wasm32")]
        collab.publish_presence_now(&app.session);
        if changed {
            ctx.request_repaint();
        } else {
            // Answers arrive on the transport thread; keep the window listening
            // without redrawing at full speed.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        collab.take_copy()
    };
    // The room built the file but the cabinet refused the commit (409): the
    // owner saves it as a copy, never writing the cabinet (TASK.md §6).
    if let Some(copy) = copy {
        app.save_room_copy(copy);
    }
}

/// The participants panel: status, role and who is here.
pub(crate) fn panel(app: &mut WordApp, ctx: &egui::Context) {
    let Some(collab) = app.collab.as_ref() else { return };
    let status = match collab.status() {
        crate::collab_session::Status::Connecting => "connecting…",
        crate::collab_session::Status::InSync => "synchronized",
        crate::collab_session::Status::Resyncing => "synchronizing…",
        crate::collab_session::Status::Diverged => "out of sync — an edit did not reach the room",
        crate::collab_session::Status::Offline => "offline, reconnecting",
        crate::collab_session::Status::Failed(_) => "error",
    };
    let role = collab.role();
    let can_commit = collab.can_commit();
    let commit_blocked = !can_commit && role.can_commit();
    let participants: Vec<_> = collab
        .participants()
        .iter()
        .map(|participant| (participant.client_id.clone(), participant.role.clone(), participant.color.clone()))
        .collect();
    // A refused commit (409) keeps its reason in `conflict`; the error line
    // shows it and the owner gets the copy action (TASK.md §6).
    let conflict = collab.conflict().map(str::to_string);
    let error = conflict.clone().or_else(|| collab.last_error().map(str::to_string));
    let can_save_copy = collab.role().can_commit();
    let last_commit = collab.last_commit().cloned();
    let mut commit_clicked = false;
    let mut copy_clicked = false;
    egui::Window::new("Collaboration").anchor(Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0)).resizable(false).collapsible(true).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(status);
            ui.separator();
            ui.label(role.as_str());
            if can_commit && ui.button("Commit").clicked() {
                commit_clicked = true;
            }
            // The owner keeps a Commit button while the session is paused, so
            // the reason a commit is refused is visible where the action is
            // (finding 8) instead of the button silently disappearing.
            if commit_blocked {
                ui.add_enabled(false, egui::Button::new("Commit")).on_hover_text(commit_blocked_message(collab.status()));
            }
        });
        ui.separator();
        for (index, (client, participant_role, color)) in participants.iter().enumerate() {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, egui::CornerRadius::same(2), participant_color(color.as_deref(), index));
                ui.label(format!("{client} · {participant_role}"));
            });
        }
        if let Some(error) = error {
            ui.separator();
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if conflict.is_some() && can_save_copy && ui.button(tl!("Save a copy…")).clicked() {
            copy_clicked = true;
        }
        if let Some(commit) = &last_commit {
            ui.separator();
            ui.label(format!("committed v{} · {}", commit.version, &commit.sha256[..commit.sha256.len().min(12)]));
            if let Some(cabinet) = &commit.cabinet {
                ui.label(format!("cabinet version {} · etag {}", cabinet.version, cabinet.etag));
            }
            for warning in &commit.warnings {
                ui.colored_label(ui.visuals().warn_fg_color, warning);
            }
        }
    });
    if commit_clicked {
        request_commit(app);
    }
    if copy_clicked && let Some(collab) = app.collab.as_mut() {
        collab.request_copy();
        app.status(tl!("Preparing a copy of the room's file…"));
    }
}

impl WordApp {
    /// The name "Save a copy" suggests: the document's save name with `(copy)`
    /// before its extension.
    pub(crate) fn room_copy_name(&self) -> String {
        let name = self.default_save_name();
        match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => format!("{stem} (copy).{ext}"),
            _ => format!("{name} (copy)"),
        }
    }

    /// Save the file the room built for a commit the cabinet refused with
    /// `409` (TASK.md §6). The cabinet is not written and the document is not
    /// marked saved: this is only a copy of the owner's work.
    pub(crate) fn save_room_copy(&mut self, copy: crate::collab_session::CommitCopy) {
        let name = self.room_copy_name();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let request = crate::file_dialogs::FileDialogRequest::Save { name: name.clone(), dir: None };
            let _ = self.ask_file(request, crate::file_dialogs::AfterPick::SaveCopy { name, bytes: copy.bytes });
        }
        #[cfg(target_arch = "wasm32")]
        {
            match self.services.download.as_ref() {
                Some(download) => match download(&name, &copy.bytes) {
                    Ok(()) => self.status(crate::i18n::fmt(tl!("Saved a copy as {name}"), &[("name", &name)])),
                    Err(error) => {
                        self.status(crate::i18n::fmt(tl!("Couldn't save a copy as {name}: {error}"), &[("name", &name), ("error", &error)]))
                    }
                },
                None => self.status(tl!("This page can't download a copy of the file")),
            }
        }
    }
}

/// Commit: the room builds the file, and the server publishes it to the cabinet
/// when it is configured with the commit callback (M87); the answer — the
/// cabinet's version and etag — shows in the panel. A host bridge that owns the
/// stored file instead gets `wordcraft:commit` (no `bytes`): that path is used
/// only when the app has no live session to commit through.
pub(crate) fn request_commit(app: &mut WordApp) {
    // A commit on a session outside `InSync` would build the room's version,
    // which may not hold the newest local edit (finding 8). Say so instead of
    // doing nothing.
    let blocked = app
        .collab
        .as_ref()
        .filter(|collab| !collab.can_commit())
        .map(|collab| if collab.role().can_commit() { commit_blocked_message(collab.status()) } else { "This session is read-only" });
    if let Some(message) = blocked {
        app.status(message.to_string());
        return;
    }
    if let Some(collab) = app.collab.as_mut() {
        collab.commit();
        // On wasm the host-owned commit path below is still reachable when the
        // session cannot commit through the room; on native it is compiled out,
        // which is why clippy cannot see a statement after this one.
        #[allow(clippy::needless_return)]
        return;
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(session) = app.host_session.clone()
        && let Some(commit) = &app.host_commit
    {
        // The cabinet assembles/fetches the file itself, so `bytes` is
        // omitted (the contract's `bytes?` is for a host that does not hold
        // it).
        let result = commit(session.session_id(), None);
        match result {
            Ok(()) => app.status("Committing…"),
            Err(message) => app.status(format!("Commit failed: {message}")),
        }
    }
}

/// Remote carets, drawn over the pages next to the local caret.
pub(crate) fn draw_carets(app: &WordApp, painter: &Painter, layout: &DocLayout, geo: &Geometry) {
    let Some(collab) = app.collab.as_ref() else { return };
    let own = collab.client_id();
    for (index, participant) in collab.participants().iter().enumerate() {
        if participant.client_id == own {
            continue;
        }
        let color = participant_color(participant.color.as_deref(), index);
        if let Some((anchor, focus)) = &participant.selection
            && let (Some(a), Some(b)) = (collab.cursor_position(anchor), collab.cursor_position(focus))
        {
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            for (page, rect) in layout.selection_rects(&app.session.doc, &a, &b, 0) {
                if let Some(screen) = geo.rects.get(page)
                    && let Some(page_layout) = layout.pages.get(page)
                {
                    let scale = crate::canvas::page_screen_scale(*screen, page_layout, geo.scale);
                    let on_screen = egui::Rect::from_min_size(
                        Pos2::new(screen.min.x + rect.x * scale, screen.min.y + rect.y * scale),
                        egui::vec2(rect.w * scale, rect.h * scale),
                    );
                    painter.rect_filled(on_screen, 0.0, color.gamma_multiply(0.22));
                }
            }
        }
        let Some(cursor) = &participant.cursor else { continue };
        let Some(pos) = collab.cursor_position(cursor) else { continue };
        if let Some(at) = caret_at(layout, geo, &pos) {
            painter.line_segment([at.0, at.1], Stroke::new(1.5, color));
            painter.text(Pos2::new(at.0.x + 2.0, at.0.y - 2.0), Align2::LEFT_BOTTOM, &participant.client_id, FontId::proportional(10.0), color);
        }
    }
}

fn caret_at(layout: &DocLayout, geo: &Geometry, pos: &Pos) -> Option<(Pos2, Pos2)> {
    let caret = layout.caret_on(pos, 0)?;
    let rect = geo.rects.get(caret.page)?;
    let page = layout.pages.get(caret.page)?;
    let scale = crate::canvas::page_screen_scale(*rect, page, geo.scale);
    let x = rect.min.x + caret.x * scale;
    let y0 = rect.min.y + caret.top * scale;
    Some((Pos2::new(x, y0), Pos2::new(x, y0 + caret.height * scale)))
}

fn participant_color(color: Option<&str>, index: usize) -> Color32 {
    if let Some(hex) = color
        && let Ok(value) = u32::from_str_radix(hex.trim_start_matches('#'), 6)
    {
        return Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8);
    }
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(0x1f, 0x77, 0xb4),
        Color32::from_rgb(0xd6, 0x27, 0x28),
        Color32::from_rgb(0x2c, 0xa0, 0x2c),
        Color32::from_rgb(0xff, 0x7f, 0x0e),
        Color32::from_rgb(0x94, 0x67, 0xbd),
        Color32::from_rgb(0x17, 0xbe, 0xcf),
    ];
    PALETTE[index % PALETTE.len()]
}
