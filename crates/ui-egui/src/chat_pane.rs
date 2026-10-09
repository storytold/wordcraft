//! The Chat add-in pane (right side), the Invite form and the Share window.

use egui::{Stroke, Ui};
use wordcraft_chat::{Lang, Role, Text};

use crate::WordApp;
use crate::theme::{Tokens, semibold};

/// Flatpak instance id, so the invite line finds this window; `"local"` outside Flatpak.
pub fn instance_id() -> String {
    wordcraft_chat::instance_id().unwrap_or_else(|| "local".into())
}

/// The owner's name must not start with `@` (agents' handles always do), so a member
/// can never sign as the owner. Empty names are not applied either.
pub fn valid_owner_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.starts_with('@')
}

/// What Enter (without Shift) does to the input box. The multiline `TextEdit` has already
/// inserted a `\n`; strip it. Returns (text to send, box content if the send succeeds or
/// there is nothing to send). A rejected send keeps `sent` without the stray newline.
pub fn on_enter(text: &str) -> (Option<String>, String) {
    let t = text.strip_suffix('\n').unwrap_or(text).trim();
    if t.is_empty() { (None, String::new()) } else { (Some(t.to_string()), String::new()) }
}

/// An invite waiting for its agent: the handle and code `Hub::invite` returned.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingInvite {
    pub handle: String,
    pub code: String,
}

impl PendingInvite {
    /// The line the owner gives the agent: always the handle bound to the code.
    pub fn line(&self, instance: &str, lang: Lang) -> String {
        lang.text(Text::InviteLine { instance, code: &self.code, handle: &self.handle })
    }
}

/// The invite while it still waits; `None` once the agent joined with it or it expired.
pub fn still_open(hub: &wordcraft_chat::Hub, inv: Option<PendingInvite>, now_ms: u64) -> Option<PendingInvite> {
    inv.filter(|i| hub.invite_open(&i.code, now_ms))
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if !app.ui.chat_pane {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::right("chat_pane")
        .default_size(320.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| pane(app, ui));
}

fn pane(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Chat").font(semibold(15.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").clicked() {
                app.ui.chat_pane = false;
            }
        });
    });
    let Some(hub) = app.chat.clone() else {
        ui.label(Lang::from_env().text(Text::ChatOff));
        return;
    };
    let lang = hub.lang();
    // Your name (author of your own edits).
    ui.horizontal(|ui| {
        ui.label("You:");
        let changed = ui.add(egui::TextEdit::singleline(&mut app.ui.owner_name).desired_width(160.0)).changed();
        if changed && valid_owner_name(&app.ui.owner_name) {
            app.session.author = app.ui.owner_name.trim().to_string();
        }
    });
    if app.ui.owner_name.trim().starts_with('@') {
        ui.label(egui::RichText::new(lang.text(Text::NameNoAt)).small().color(t.red));
    }
    if let Some(e) = hub.log_error() {
        ui.label(egui::RichText::new(lang.text(Text::LogNotSaved(&e))).small().color(t.red));
    }
    if app.session.path.is_none() {
        ui.label(egui::RichText::new(lang.text(Text::InMemory)).small().color(t.text_dim));
    }
    // Members.
    for m in hub.members() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&m.handle).font(semibold(12.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove").clicked() {
                    hub.remove(&m.handle);
                }
            });
        });
    }
    // Invite form. The line shows the handle bound to the code (never the field's live text)
    // and goes away once the agent joined with it or it expired.
    let hid = egui::Id::new("chat_invite_handle");
    let lid = egui::Id::new("chat_invite_pending");
    let eid_inv = egui::Id::new("chat_invite_error");
    let mut handle = ui.data(|d| d.get_temp::<String>(hid)).unwrap_or_default();
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut handle).hint_text("@claude").desired_width(140.0));
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(hid, handle.clone()));
        }
        if ui.button("Invite").clicked() {
            match hub.invite(&handle, wordcraft_chat::hub::now_ms()) {
                Ok((h, code)) => ui.data_mut(|d| {
                    d.insert_temp(lid, PendingInvite { handle: h, code });
                    d.remove::<String>(eid_inv);
                    d.insert_temp(hid, String::new());
                }),
                Err(e) => ui.data_mut(|d| {
                    d.insert_temp(eid_inv, lang.text(Text::InviteFailed(&e.to_string())));
                }),
            }
        }
    });
    if let Some(e) = ui.data(|d| d.get_temp::<String>(eid_inv)) {
        ui.label(egui::RichText::new(e).small().color(t.red));
    }
    let pending = still_open(&hub, ui.data(|d| d.get_temp::<PendingInvite>(lid)), wordcraft_chat::hub::now_ms());
    match pending {
        Some(inv) => {
            let line = inv.line(&instance_id(), lang);
            egui::Frame::NONE.fill(t.input).corner_radius(6).inner_margin(6).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&line).monospace().small()).wrap());
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(line.clone());
                }
            });
        }
        None => ui.data_mut(|d| d.remove::<PendingInvite>(lid)),
    }
    ui.separator();
    // Messages.
    let input_h = 56.0;
    egui::ScrollArea::vertical().stick_to_bottom(true).max_height((ui.available_height() - input_h).max(80.0)).show(ui, |ui| {
        let own = if valid_owner_name(&app.ui.owner_name) { app.ui.owner_name.trim().to_string() } else { "You".to_string() };
        for m in &hub.recent(500) {
            let (who, color) = match m.role {
                Role::Owner => (own.clone(), t.accent),
                Role::Agent => (m.from.clone(), t.text),
                Role::System => (String::new(), t.text_dim),
            };
            ui.horizontal_wrapped(|ui| {
                if !who.is_empty() {
                    ui.label(egui::RichText::new(format!("{who}:")).font(semibold(12.0)).color(color));
                }
                let text = egui::RichText::new(&m.text);
                ui.label(if m.role == Role::System { text.italics().color(t.text_dim) } else { text });
            });
        }
        if hub.brake_on() {
            ui.label(egui::RichText::new(lang.text(Text::Brake)).italics().color(t.text_dim));
        }
    });
    // Input (the only place that makes owner messages).
    let iid = egui::Id::new("chat_input");
    let mut text = ui.data(|d| d.get_temp::<String>(iid)).unwrap_or_default();
    let r = ui.add(egui::TextEdit::multiline(&mut text).desired_rows(2).desired_width(f32::INFINITY).hint_text(lang.text(Text::InputHint)));
    let eid = egui::Id::new("chat_input_err");
    let send = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    if send {
        let (to_send, after) = on_enter(&text);
        match to_send {
            None => {
                text = after;
                ui.data_mut(|d| d.remove::<String>(eid));
            }
            Some(msg) => match hub.post_owner(&app.ui.owner_name, &msg) {
                Ok(_) => {
                    text = after;
                    ui.data_mut(|d| d.remove::<String>(eid));
                }
                Err(e) => {
                    text = msg;
                    ui.data_mut(|d| d.insert_temp(eid, lang.text(Text::NotSent(&e.to_string()))));
                }
            },
        }
    }
    if let Some(e) = ui.data(|d| d.get_temp::<String>(eid)) {
        ui.label(egui::RichText::new(e).small().color(t.red));
    }
    ui.data_mut(|d| d.insert_temp(iid, text));
}

/// Share: invite an agent, save a copy, invite a person (later).
pub fn share_window(app: &mut WordApp, ctx: &egui::Context) {
    if !app.share_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Share").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        if ui.button("Invite an agent…").clicked() {
            app.ui.chat_pane = true;
            app.share_open = false;
        }
        if ui.button("Save a copy…").clicked() {
            let _ = app.run("ui.backstage", serde_json::json!({"value": true, "page": "export"}));
            app.share_open = false;
        }
        ui.add_enabled(false, egui::Button::new("Invite a person (later)"));
    });
    if !open {
        app.share_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wordcraft_engine::Session;

    #[test]
    fn log_follows_save_as() {
        let dir = std::env::temp_dir().join(format!("wc-pane-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.chat = Some(wordcraft_chat::Hub::new("k".into()));
        let p1 = dir.join("a.docx");
        let _ = a.run("file.save", json!({"path": p1.to_string_lossy()}));
        a.sync_chat_log();
        let _ = a.chat.as_ref().map(|h| h.post_owner("L", "first"));
        let p2 = dir.join("b.docx");
        let _ = a.run("file.save", json!({"path": p2.to_string_lossy()}));
        a.sync_chat_log();
        let _ = a.chat.as_ref().map(|h| h.post_owner("L", "second"));
        let log2 = std::fs::read_to_string(dir.join("b.docx.chat.jsonl")).unwrap_or_default();
        assert!(log2.contains("first") && log2.contains("second"), "log2 = {log2:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_switches_the_chat_and_save_as_keeps_it() {
        let dir = std::env::temp_dir().join(format!("wc-pane-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.chat = Some(wordcraft_chat::Hub::new("k".into()));
        let doc_a = dir.join("a.docx");
        let doc_b = dir.join("b.docx");
        let _ = a.run("file.save", json!({"path": doc_b.to_string_lossy()}));
        let _ = a.chat.as_ref().map(|h| h.post_owner("L", "@claude ordem antiga do B"));
        let _ = a.run("file.save", json!({"path": doc_a.to_string_lossy()}));
        let _ = a.run("file.new", json!({}));
        let texts = |a: &WordApp| a.chat.as_ref().map(|h| h.messages()).unwrap_or_default().into_iter().map(|m| m.text).collect::<Vec<_>>();
        assert_eq!(texts(&a), vec!["document: new"]);
        let _ = a.chat.as_ref().map(|h| h.post_owner("L", "no novo"));
        let _ = a.run("file.open", json!({"path": doc_b.to_string_lossy()}));
        let t = texts(&a);
        assert!(t.contains(&"@claude ordem antiga do B".to_string()), "{t:?}");
        assert!(!t.contains(&"no novo".to_string()), "{t:?}");
        assert_eq!(t.last().map(String::as_str), Some("document: b.docx"));
        // Save As keeps the conversation (and merges it into the new log).
        let doc_c = dir.join("c.docx");
        let _ = a.run("file.save", json!({"path": doc_c.to_string_lossy()}));
        let log_c = std::fs::read_to_string(dir.join("c.docx.chat.jsonl")).unwrap_or_default();
        assert!(log_c.contains("ordem antiga do B") && log_c.contains("document: b.docx"), "{log_c}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_restore_keeps_the_conversation() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.chat = Some(wordcraft_chat::Hub::new("k".into()));
        let _ = a.run("text.insert", json!({"text": "Alfa"}));
        let _ = a.run("file.versions", json!({"save": "v1"}));
        let _ = a.chat.as_ref().map(|h| h.post_owner("L", "@claude revê a cláusula"));
        let _ = a.run("text.insert", json!({"text": " beta"}));
        let _ = a.run("file.versions", json!({"restore": 0}));
        let texts: Vec<String> = a.chat.as_ref().map(|h| h.messages()).unwrap_or_default().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["@claude revê a cláusula"], "a version restore is the same document: no switch");
    }

    #[test]
    fn portal_documents_keep_their_chat_in_the_config_folder() {
        let dir = std::env::temp_dir().join(format!("wc-pane-portal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A document-portal path (Save As to a folder the sandbox cannot see).
        let portal = dir.join("run").join("user").join("1000").join("doc").join("abc123");
        assert!(std::fs::create_dir_all(&portal).is_ok());
        let logs = dir.join("cfg").join("wordcraft").join("chat-logs");
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.chat = Some(wordcraft_chat::Hub::new("k".into()));
        a.chat_logs_dir = Some(logs.clone());
        let _ = a.chat.as_ref().map(|h| h.post_owner("L", "antes de gravar"));
        let doc = portal.join("t.docx");
        assert!(a.run("file.save", json!({"path": doc.to_string_lossy()})).is_ok());
        let log = logs.join("abc123-t.docx.chat.jsonl");
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(text.contains("antes de gravar") && text.contains(&format!("chat saved in {}", log.display())), "{text:?}");
        let names: Vec<String> = std::fs::read_dir(&portal).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
        assert_eq!(names, vec!["t.docx".to_string()], "nothing but the document in the portal folder");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&logs).map(|m| m.permissions().mode() & 0o777).unwrap_or(0), 0o700);
        }
        assert!(a.chat.as_ref().and_then(|h| h.log_error()).is_none());
        // Open the same portal document later: its conversation comes back from there.
        let _ = a.run("file.new", json!({}));
        let _ = a.run("file.open", json!({"path": doc.to_string_lossy()}));
        let texts: Vec<String> = a.chat.as_ref().map(|h| h.messages()).unwrap_or_default().into_iter().map(|m| m.text).collect();
        assert!(texts.contains(&"antes de gravar".to_string()), "{texts:?}");
        assert_eq!(texts.iter().filter(|t| t.starts_with("document: t.docx")).count(), 1, "{texts:?}");
        // Any folder that refuses the log: the same fallback place.
        let plain = dir.join("plain");
        assert!(std::fs::create_dir_all(plain.join("u.docx.chat.jsonl")).is_ok());
        let _ = a.chat.as_ref().map(|h| h.post_owner("L", "noutro"));
        assert!(a.run("file.save", json!({"path": plain.join("u.docx").to_string_lossy()})).is_ok());
        assert!(a.chat.as_ref().and_then(|h| h.log_error()).is_none(), "{:?}", a.chat.as_ref().and_then(|h| h.log_error()));
        let moved = std::fs::read_dir(&logs).map(|r| r.flatten().filter(|e| e.file_name().to_string_lossy().ends_with("-u.docx.chat.jsonl")).count()).unwrap_or(0);
        assert_eq!(moved, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn portal_paths_seen_from_the_host_and_from_inside_the_sandbox() {
        let logs = std::path::Path::new("/c/wordcraft/chat-logs");
        for doc in ["/run/flatpak/doc/abc123/t.docx", "/run/user/1000/doc/abc123/t.docx"] {
            assert_eq!(crate::chat_log_places(std::path::Path::new(doc), Some(logs)), (None, Some(logs.join("abc123-t.docx.chat.jsonl"))), "{doc}");
        }
        let (next, fb) = crate::chat_log_places(std::path::Path::new("/home/l/Documents/t.docx"), Some(logs));
        assert_eq!(next, Some(std::path::PathBuf::from("/home/l/Documents/t.docx.chat.jsonl")));
        assert!(fb.is_some_and(|f| f.starts_with(logs) && f.to_string_lossy().ends_with("-t.docx.chat.jsonl")));
        // Not a portal: a folder that only looks a bit like one.
        assert!(crate::chat_log_places(std::path::Path::new("/run/flatpak/other/abc/t.docx"), Some(logs)).0.is_some());
    }

    #[test]
    fn invite_box_shows_the_bound_handle_until_used_or_expired() {
        let hub = wordcraft_chat::Hub::new("k".into());
        let now = wordcraft_chat::hub::now_ms();
        let (h, code) = hub.invite("@teste", now).unwrap_or_default();
        let inv = PendingInvite { handle: h, code: code.clone() };
        // The owner types something else in the field afterwards: the line keeps @teste.
        let line = inv.line("1585430700", wordcraft_chat::Lang::En);
        assert_eq!(line, format!("Join the WordCraft chat: wordcraft-chat join 1585430700:{code} --as @teste"));
        assert_eq!(inv.line("1585430700", wordcraft_chat::Lang::Pt), format!("Entra no chat do WordCraft: wordcraft-chat join 1585430700:{code} --as @teste"));
        assert!(still_open(&hub, Some(inv.clone()), now).is_some());
        assert!(still_open(&hub, Some(inv.clone()), now + wordcraft_chat::hub::INVITE_TTL_MS + 1).is_none(), "expired");
        assert!(hub.join(&code, now).is_ok());
        assert!(still_open(&hub, Some(inv), now).is_none(), "used: the box clears");
    }

    #[test]
    fn log_failures_are_visible() {
        let dir = std::env::temp_dir().join(format!("wc-pane-logerr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join("a.docx.chat.jsonl"));
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.chat = Some(wordcraft_chat::Hub::new("k".into()));
        let _ = a.run("file.save", json!({"path": dir.join("a.docx").to_string_lossy()}));
        assert!(a.chat.as_ref().and_then(|h| h.log_error()).is_some(), "Save As: attach failure must show");
        let _ = std::fs::create_dir_all(dir.join("b.docx.chat.jsonl"));
        let _ = std::fs::write(dir.join("b.docx"), b"");
        let _ = a.run("file.new", json!({}));
        assert!(a.chat.as_ref().and_then(|h| h.log_error()).is_none());
        let _ = a.run("file.save", json!({"path": dir.join("b.docx").to_string_lossy()}));
        let _ = a.run("file.open", json!({"path": dir.join("b.docx").to_string_lossy()}));
        assert!(a.chat.as_ref().and_then(|h| h.log_error()).is_some(), "Open: switch failure must show");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pane_hints_follow_the_chat_language() {
        use wordcraft_chat::{Lang, Text};
        assert_eq!(Lang::En.text(Text::InMemory), "The chat is kept in memory until you save the document.");
        assert_eq!(Lang::Pt.text(Text::InMemory), "A conversa fica em memória até gravares o documento.");
        for t in [Text::ChatOff, Text::NameNoAt, Text::Brake, Text::InputHint, Text::LogNotSaved("x"), Text::NotSent("x"), Text::InviteFailed("x")] {
            assert_ne!(Lang::En.text(t.clone()), Lang::Pt.text(t), "every pane line has both languages");
        }
    }

    #[test]
    fn enter_handling() {
        assert_eq!(on_enter(""), (None, String::new()));
        assert_eq!(on_enter("\n"), (None, String::new()));
        assert_eq!(on_enter("olá\n"), (Some("olá".into()), String::new()));
        assert_eq!(on_enter("a\nb\n"), (Some("a\nb".into()), String::new()));
    }

    #[test]
    fn instance_id_outside_flatpak() {
        if !std::path::Path::new("/.flatpak-info").exists() {
            assert_eq!(instance_id(), "local");
        }
    }

    #[test]
    fn owner_name_never_starts_with_at() {
        assert!(valid_owner_name("Owner"));
        assert!(valid_owner_name("  Owner "));
        assert!(!valid_owner_name("@claude"));
        assert!(!valid_owner_name("  @claude"));
        assert!(!valid_owner_name(""));
        assert!(!valid_owner_name("   "));
    }
}
