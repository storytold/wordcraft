//! Document chat in the interface (craft-collab `docs/chat.md`): the title bar's chat button
//! (next to the people circles, with the number of unread messages, flashing when someone
//! mentions you), and the chat panel at the side (where task panes go): each message with its
//! author's circle, colour and name, its time (relative for today), the text (selectable, links
//! clickable, `@Name` highlighted), replies quoted, "edited" and "Message deleted" markers; who's
//! typing; a box that sends with Enter (Shift+Enter for a new line); and the menu with Export,
//! Clear chat… (asks first) and the note that Office drops the chat when it saves.
//!
//! The engine does the work (`chat.*` commands, `wordcraft_engine::cmd::chat`); this only shows
//! it. Everything is drawn in code with theme tokens.

use egui::{Align2, Color32, Key, Modifiers, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_engine::doc::chat::{Chat, Message};

use crate::WordApp;
use crate::collab_ui::color;
use crate::theme::{Tokens, medium, regular, semibold};

/// How long a mention makes the chat button flash (milliseconds).
const FLASH_MS: f64 = 4_000.0;
/// "Typing…" ends this long after the last keystroke.
const TYPING_MS: f64 = 5_000.0;
/// Messages drawn at most (the newest); older ones are in the file and `chat.list`.
const MAX_SHOWN: usize = 500;

/// The interface's chat state.
#[derive(Default)]
pub struct ChatUi {
    /// The message being written.
    pub draft: String,
    /// The message the draft answers.
    pub reply_to: Option<String>,
    /// The (own) message the draft edits.
    pub editing: Option<String>,
    /// Messages already seen: (document, number of messages, when cleared).
    seen: (u64, usize, Option<i64>),
    /// New messages from others since the panel was last open.
    pub unread: usize,
    /// A mention of you flashes the button until then.
    pub flash_until: f64,
    /// Clear chat… asked, waiting for the answer.
    pub confirm_clear: bool,
    /// When the draft last changed while "typing" was on.
    typed_at: f64,
    /// The text box takes the keyboard on the next frame.
    focus_input: bool,
}

/// My name, for mentions: the room's, else the author name.
fn my_name(app: &WordApp) -> String {
    app.session.active_room().map(|r| r.user.name.clone()).unwrap_or_else(|| app.session.author.clone())
}

/// Whether `text` mentions `name` (`@Name`, any case).
pub fn mentions(text: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    lower.match_indices('@').any(|(i, _)| lower.get(i + 1..).is_some_and(|rest| rest.starts_with(&name.to_lowercase())))
}

/// Counts unread messages and mentions (call every frame), and ends "typing…" after a pause.
pub fn track(app: &mut WordApp) {
    let now = crate::now_ms();
    let doc = app.session.document_id();
    let (len, cleared) = (app.session.doc.chat.messages.len(), app.session.doc.chat.cleared_at);
    let open = app.session.view.chat_pane;
    let st = &mut app.collab.chat;
    let (seen_doc, seen_len, seen_cleared) = st.seen;
    if seen_doc != doc || seen_cleared != cleared || len < seen_len {
        // Another document, or the chat was cleared: start counting from here.
        st.seen = (doc, len, cleared);
        st.unread = 0;
        if seen_doc != doc {
            st.reply_to = None;
            st.editing = None;
            st.confirm_clear = false;
        }
    } else if len > seen_len {
        if !open {
            let me = app.session.chat_user_id();
            let name = my_name(app);
            let st = &mut app.collab.chat;
            for m in app.session.doc.chat.messages.get(seen_len..).unwrap_or_default() {
                if Some(&m.author.id) != me.as_ref() && !m.deleted {
                    st.unread = st.unread.saturating_add(1);
                    if mentions(m.text(), &name) {
                        st.flash_until = now + FLASH_MS;
                    }
                }
            }
        }
        app.collab.chat.seen = (doc, len, cleared);
    }
    if app.session.view.chat_pane {
        app.collab.chat.unread = 0;
    }
    let st = &app.collab.chat;
    if st.typed_at > 0.0 && (now - st.typed_at > TYPING_MS || st.draft.is_empty()) {
        app.collab.chat.typed_at = 0.0;
        let _ = app.session.run("chat.typing", &json!({"typing": false}));
    }
}

/// Whether the title bar shows the chat button: in a live session, when the document has a
/// chat, or while the panel is open.
pub fn has_button(app: &WordApp) -> bool {
    app.session.active_room().is_some() || !app.session.doc.chat.messages.is_empty() || app.session.view.chat_pane
}

/// The title bar's chat button with its unread count. Called in the title bar's right-to-left
/// layout, between the Share button and the people.
pub fn button(app: &mut WordApp, ui: &mut Ui, control_h: f32) {
    let t = Tokens::get(ui.ctx());
    let now = crate::now_ms();
    let unread = app.collab.chat.unread;
    let flashing = now < app.collab.chat.flash_until;
    let h = control_h.min(24.0);
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, h), Sense::click());
    let on = app.session.view.chat_pane;
    let fill = if flashing {
        // A gentle pulse in the accent colour.
        let phase = ((now / 300.0).sin() * 0.5 + 0.5) as f32;
        t.accent.gamma_multiply(0.35 + 0.5 * phase)
    } else if resp.hovered() {
        t.hover
    } else if on {
        t.checked
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, 5.0, fill);
    crate::icons::paint(ui.painter(), egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0)), "chat", t.icon, t.accent);
    if unread > 0 {
        let label = if unread > 9 { "9+".to_string() } else { unread.to_string() };
        let c = pos2(r.max.x - 6.0, r.min.y + 6.0);
        let w = if label.len() > 1 { 7.5 } else { 6.0 };
        ui.painter().rect_filled(egui::Rect::from_center_size(c, vec2(w * 2.0, 12.0)), 6.0, t.red);
        ui.painter().text(c, Align2::CENTER_CENTER, label, semibold(8.5), Color32::WHITE);
    }
    if flashing {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    }
    let name = if unread > 0 { crate::i18n::fmt(tl!("Chat, {count} unread"), &[("count", &unread.to_string())]) } else { tl!("Chat").to_string() };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    if resp.on_hover_text(name.clone()).clicked() {
        let _ = app.run("view.chat", json!({}));
        if app.session.view.chat_pane {
            app.collab.chat.focus_input = true;
            app.collab.chat.flash_until = 0.0;
        }
    }
}

/// A message's time: "just now", "5 min ago" and "3 h ago" for today, else the date and time.
pub fn when(at: i64, now_ms: f64) -> String {
    let now = now_ms as i64;
    let off = local_offset_ms(now);
    let day = |ms: i64| (ms + off).div_euclid(86_400_000);
    let ago = now.saturating_sub(at) / 1000;
    if day(at) == day(now) && ago >= -60 {
        if ago < 60 {
            return tl!("just now").to_string();
        }
        if ago < 3_600 {
            return crate::i18n::fmt(tl!("{n} min ago"), &[("n", &(ago / 60).to_string())]);
        }
        return crate::i18n::fmt(tl!("{n} h ago"), &[("n", &(ago / 3_600).to_string())]);
    }
    wordcraft_engine::doc::chat::iso_ms(at.saturating_add(off))
}

/// The local time zone's offset from UTC (milliseconds): the browser's on the web. The desktop
/// build shows UTC for messages before today (it reads no time zone database).
fn local_offset_ms(_now: i64) -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        let minutes = js_sys::Date::new_0().get_timezone_offset();
        if minutes.is_finite() { -(minutes as i64) * 60_000 } else { 0 }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

/// A piece of a message's text.
#[derive(Debug, PartialEq)]
pub enum Piece<'a> {
    Text(&'a str),
    Link(&'a str),
    Mention(&'a str),
}

/// Splits a line into text, links (`http://`, `https://`) and mentions of `names`.
pub fn pieces<'a>(line: &'a str, names: &[String]) -> Vec<Piece<'a>> {
    let mut out = Vec::new();
    let mut plain_from = 0;
    let mut i = 0;
    let lower = line.to_lowercase();
    let same_len = lower.len() == line.len();
    while i < line.len() {
        let rest = line.get(i..).unwrap_or("");
        let at_word = i == 0 || line.get(..i).and_then(|b| b.chars().next_back()).is_some_and(char::is_whitespace);
        let mut take: Option<(usize, bool)> = None;
        if at_word && (rest.starts_with("http://") || rest.starts_with("https://")) {
            let mut end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            while end > 0 && rest.get(..end).and_then(|s| s.chars().next_back()).is_some_and(|c| ".,;:!?)]}'\"".contains(c)) {
                end -= rest.get(..end).and_then(|s| s.chars().next_back()).map_or(1, char::len_utf8);
            }
            if end > "https://".len() {
                take = Some((end, true));
            }
        } else if at_word && rest.starts_with('@') && same_len {
            let low = lower.get(i + 1..).unwrap_or("");
            let best = names.iter().filter(|n| !n.is_empty() && low.starts_with(&n.to_lowercase())).map(|n| n.len()).max();
            if let Some(n) = best {
                take = Some((1 + n, false));
            }
        }
        match take {
            Some((len, link)) => {
                if plain_from < i {
                    out.push(Piece::Text(line.get(plain_from..i).unwrap_or("")));
                }
                let s = line.get(i..i + len).unwrap_or("");
                out.push(if link { Piece::Link(s) } else { Piece::Mention(s) });
                i += len;
                plain_from = i;
            }
            None => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    if plain_from < line.len() {
        out.push(Piece::Text(line.get(plain_from..).unwrap_or("")));
    }
    out
}

/// The people a mention can name: the room's (others and you) and the chat's authors.
fn known_names(app: &WordApp) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(r) = app.session.active_room() {
        names.push(r.user.name.clone());
        names.extend(r.peers().into_iter().map(|p| p.user.name));
    } else {
        names.push(app.session.author.clone());
    }
    for m in &app.session.doc.chat.messages {
        if !names.contains(&m.author.name) {
            names.push(m.author.name.clone());
        }
        if names.len() > 200 {
            break;
        }
    }
    names.retain(|n| !n.trim().is_empty());
    names
}

fn circle(ui: &mut Ui, initials: &str, c: Color32, d: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(d, d), Sense::hover());
    ui.painter().circle_filled(r.center(), d / 2.0, c);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, initials, semibold(if initials.chars().count() > 1 { 8.5 } else { 10.0 }), Color32::WHITE);
}

/// The text of a message, links and mentions included.
fn body(ui: &mut Ui, t: &Tokens, text: &str, names: &[String], mine: &str) {
    for line in text.split('\n') {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if line.is_empty() {
                ui.label(RichText::new(" ").font(regular(12.5)));
            }
            for p in pieces(line, names) {
                match p {
                    Piece::Text(s) => {
                        ui.add(egui::Label::new(RichText::new(s).font(regular(12.5)).color(t.text)).selectable(true));
                    }
                    Piece::Link(url) => {
                        ui.hyperlink_to(RichText::new(url).font(regular(12.5)).color(t.accent_text), url);
                    }
                    Piece::Mention(s) => {
                        let me = s.get(1..).is_some_and(|n| n.eq_ignore_ascii_case(mine));
                        let rt = RichText::new(s).font(semibold(12.5)).color(t.accent_text);
                        ui.add(egui::Label::new(if me { rt.background_color(t.accent.gamma_multiply(0.18)) } else { rt }).selectable(true));
                    }
                }
            }
        });
    }
}

/// What to do after drawing the messages.
enum Act {
    Reply(String),
    Edit(String, String),
    Delete(String),
    Copy(String),
}

/// What every message is drawn with.
struct Ctx<'a> {
    t: &'a Tokens,
    chat: &'a Chat,
    /// My user id.
    me: Option<&'a str>,
    names: &'a [String],
    mine_name: &'a str,
    now: f64,
}

fn message(ui: &mut Ui, cx: &Ctx, m: &Message) -> Option<Act> {
    let Ctx { t, chat, me, names, mine_name, now } = *cx;
    let mut act = None;
    let own = me == Some(m.author.id.as_str());
    let resp = ui
        .horizontal_top(|ui| {
            circle(ui, &m.author.initials(), color(&m.author.color), 24.0);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&m.author.name).font(semibold(12.0)).color(color(&m.author.color)));
                    ui.label(RichText::new(when(m.at, now)).font(regular(10.5)).color(t.text_dim));
                    if m.edited.is_some() && !m.deleted {
                        ui.label(RichText::new(tl!("edited")).font(regular(10.5)).color(t.text_dim));
                    }
                });
                if let Some(r) = &m.reply_to {
                    let quoted = chat.get(r);
                    let line = match quoted {
                        Some(q) if !q.deleted => {
                            let first: String = q.text().lines().next().unwrap_or("").chars().take(80).collect();
                            format!("{}: {first}", q.author.name)
                        }
                        _ => tl!("Message deleted").to_string(),
                    };
                    let bar = quoted.map(|q| color(&q.author.color)).unwrap_or(t.border_strong);
                    egui::Frame::NONE.fill(t.input).inner_margin(egui::Margin { left: 8, right: 6, top: 2, bottom: 2 }).corner_radius(3.0).show(
                        ui,
                        |ui| {
                            let r = ui.label(RichText::new(line).font(regular(11.0)).color(t.text_dim));
                            let x = r.rect.min.x - 6.0;
                            ui.painter().line_segment([pos2(x, r.rect.min.y), pos2(x, r.rect.max.y)], Stroke::new(2.0, bar));
                        },
                    );
                }
                if m.deleted {
                    ui.label(RichText::new(tl!("Message deleted")).font(regular(12.0)).italics().color(t.text_dim));
                } else {
                    body(ui, t, m.text(), names, mine_name);
                }
            });
        })
        .response;
    resp.interact(Sense::click()).context_menu(|ui| {
        if ui.button(tl!("Reply")).clicked() {
            act = Some(Act::Reply(m.id.clone()));
            ui.close();
        }
        if !m.deleted && ui.button(tl!("Copy")).clicked() {
            act = Some(Act::Copy(m.text().to_string()));
            ui.close();
        }
        if own && !m.deleted {
            if ui.button(tl!("Edit")).clicked() {
                act = Some(Act::Edit(m.id.clone(), m.text().to_string()));
                ui.close();
            }
            if ui.button(tl!("Delete")).clicked() {
                act = Some(Act::Delete(m.id.clone()));
                ui.close();
            }
        }
    });
    act
}

/// The chat panel's contents.
pub fn pane(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let now = crate::now_ms();
    // The header: the title, the menu, close.
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Chat")).font(semibold(15.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::icon_button(ui, "close", tl!("Close")).clicked() {
                close = true;
            }
            let more = crate::widgets::icon_button(ui, "more", tl!("Chat options"));
            more.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Chat options")));
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(220.0);
                if ui.button(tl!("Export")).clicked() {
                    match app.session.run("chat.export", &json!({"format": "text"})) {
                        Ok(v) => {
                            ui.ctx().copy_text(v["text"].as_str().unwrap_or("").to_string());
                            app.status(tl!("Chat copied to the clipboard").to_string());
                        }
                        Err(e) => app.status(e.to_string()),
                    }
                    ui.close();
                }
                if ui.add_enabled(!app.session.doc.chat.messages.is_empty(), egui::Button::new(tl!("Clear chat…"))).clicked() {
                    app.collab.chat.confirm_clear = true;
                    ui.close();
                }
                ui.separator();
                ui.label(RichText::new(tl!("Kept in the file; Office drops it when it saves")).font(regular(11.0)).color(t.text_dim));
            });
        });
    });
    if close {
        let _ = app.run("view.chat", json!({"value": false}));
        return;
    }
    ui.add_space(4.0);
    if app.collab.chat.confirm_clear {
        egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).inner_margin(8).corner_radius(4.0).show(ui, |ui| {
            ui.label(RichText::new(tl!("Remove the whole chat from the document for everyone?")).font(regular(12.0)).color(t.text));
            ui.horizontal(|ui| {
                if ui.button(tl!("Clear Chat")).clicked() {
                    app.collab.chat.confirm_clear = false;
                    if let Err(e) = app.run("chat.clear", json!({})) {
                        app.status(e);
                    }
                }
                if ui.button(tl!("Cancel")).clicked() {
                    app.collab.chat.confirm_clear = false;
                }
            });
        });
        ui.add_space(4.0);
    }

    // The messages.
    let chat = app.session.doc.chat.clone();
    let me = app.session.chat_user_id();
    let names = known_names(app);
    let mine_name = my_name(app);
    let typing = app.session.chat_typing_names();
    let input_h = 118.0 + if typing.is_empty() { 0.0 } else { 18.0 };
    let list_h = (ui.available_height() - input_h).max(80.0);
    let mut act = None;
    egui::ScrollArea::vertical()
        .id_salt("chat_messages")
        .max_height(list_h)
        .min_scrolled_height(list_h)
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if chat.messages.is_empty() {
                ui.label(RichText::new(tl!("No messages yet. Messages are kept in the document.")).font(regular(12.0)).color(t.text_dim));
            }
            let from = chat.messages.len().saturating_sub(MAX_SHOWN);
            for m in chat.messages.get(from..).unwrap_or_default() {
                let cx = Ctx { t: &t, chat: &chat, me: me.as_deref(), names: &names, mine_name: &mine_name, now };
                if let Some(a) = message(ui, &cx, m) {
                    act = Some(a);
                }
                ui.add_space(8.0);
            }
        });
    match act {
        Some(Act::Reply(id)) => {
            app.collab.chat.reply_to = Some(id);
            app.collab.chat.editing = None;
            app.collab.chat.focus_input = true;
        }
        Some(Act::Edit(id, text)) => {
            app.collab.chat.editing = Some(id);
            app.collab.chat.reply_to = None;
            app.collab.chat.draft = text;
            app.collab.chat.focus_input = true;
        }
        Some(Act::Delete(id)) => {
            if let Err(e) = app.run("chat.delete", json!({"id": id})) {
                app.status(e);
            }
        }
        Some(Act::Copy(text)) => ui.ctx().copy_text(text),
        None => {}
    }

    // Who's typing.
    if !typing.is_empty() {
        let line = if typing.len() == 1 {
            crate::i18n::fmt(tl!("{name} is typing…"), &[("name", &typing[0])])
        } else {
            crate::i18n::fmt(tl!("{names} are typing…"), &[("names", &typing.join(", "))])
        };
        ui.label(RichText::new(line).font(regular(11.0)).italics().color(t.text_dim));
    }
    ui.separator();
    // Replying or editing.
    let banner = match (&app.collab.chat.editing, &app.collab.chat.reply_to) {
        (Some(_), _) => Some(tl!("Editing your message").to_string()),
        (None, Some(r)) => {
            let name = chat.get(r).map(|m| m.author.name.clone()).unwrap_or_default();
            Some(crate::i18n::fmt(tl!("Replying to {name}"), &[("name", &name)]))
        }
        _ => None,
    };
    if let Some(b) = banner {
        ui.horizontal(|ui| {
            ui.label(RichText::new(b).font(medium(11.0)).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::widgets::icon_button(ui, "close", tl!("Cancel")).clicked() {
                    if app.collab.chat.editing.take().is_some() {
                        app.collab.chat.draft.clear();
                    }
                    app.collab.chat.reply_to = None;
                }
            });
        });
    }
    // The box: Enter sends, Shift+Enter starts a new line.
    let id = egui::Id::unique("chat_input");
    let focused = ui.memory(|m| m.has_focus(id));
    // (`consume_key` matches Shift too: look for a plain Enter by hand.)
    let enter = focused
        && ui.input_mut(|i| {
            let plain = |m: &Modifiers| !m.shift && !m.alt && !m.ctrl && !m.command && !m.mac_cmd;
            let at =
                i.events.iter().position(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if plain(modifiers)));
            at.map(|k| i.events.remove(k)).is_some()
        });
    let before = app.collab.chat.draft.clone();
    let edit = egui::TextEdit::multiline(&mut app.collab.chat.draft)
        .id(id)
        .desired_rows(2)
        .desired_width(f32::INFINITY)
        .hint_text(tl!("Write a message"))
        .font(regular(12.5));
    let resp = ui.add(edit);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tl!("Write a message")));
    if std::mem::take(&mut app.collab.chat.focus_input) {
        resp.request_focus();
    }
    if app.collab.chat.draft != before && !app.collab.chat.draft.is_empty() {
        if app.collab.chat.typed_at == 0.0 {
            let _ = app.session.run("chat.typing", &json!({"typing": true}));
        }
        app.collab.chat.typed_at = now;
    }
    let mut send = enter;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ready = !app.collab.chat.draft.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new(tl!("Send"))).clicked() {
                send = true;
            }
        });
    });
    if send && !app.collab.chat.draft.trim().is_empty() {
        let text = app.collab.chat.draft.trim_end_matches(['\n', '\r']).to_string();
        let r = match app.collab.chat.editing.clone() {
            Some(id) => app.run("chat.edit", json!({"id": id, "text": text})),
            None => {
                let mut p = json!({"text": text});
                if let Some(r) = &app.collab.chat.reply_to {
                    p["replyTo"] = json!(r);
                }
                app.run("chat.send", p)
            }
        };
        match r {
            Ok(_) => {
                app.collab.chat.draft.clear();
                app.collab.chat.reply_to = None;
                app.collab.chat.editing = None;
                app.collab.chat.typed_at = 0.0;
                let _ = app.session.run("chat.typing", &json!({"typing": false}));
                app.collab.chat.focus_input = true;
            }
            Err(e) => app.status(e),
        }
    }
    // Messages read while the panel is open.
    app.collab.chat.unread = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_mentions() {
        let names = vec!["Ann Lee".to_string(), "Bo".to_string()];
        assert_eq!(
            pieces("See https://example.org/a?b=c, @ann lee and @Bo!", &names),
            vec![
                Piece::Text("See "),
                Piece::Link("https://example.org/a?b=c"),
                Piece::Text(", "),
                Piece::Mention("@ann lee"),
                Piece::Text(" and "),
                Piece::Mention("@Bo"),
                Piece::Text("!"),
            ]
        );
        assert_eq!(pieces("mail@Bo.org", &names), vec![Piece::Text("mail@Bo.org")]);
        assert_eq!(pieces("", &names), vec![]);
        assert_eq!(pieces("https://", &names), vec![Piece::Text("https://")]);
        assert!(mentions("hey @ANN LEE look", "Ann Lee"));
        assert!(!mentions("hey Ann Lee", "Ann Lee"));
        assert!(!mentions("@", ""));
    }

    #[test]
    fn times() {
        let now = 1_760_605_200_000.0;
        assert_eq!(when(1_760_605_200_000 - 5_000, now), "just now");
        assert_eq!(when(1_760_605_200_000 - 5 * 60_000, now), "5 min ago");
        assert_eq!(when(1_760_605_200_000 - 3 * 3_600_000, now), "3 h ago");
        assert_eq!(when(1_760_605_200_000 - 3 * 86_400_000, now), "2025-10-13 09:00");
    }
}
