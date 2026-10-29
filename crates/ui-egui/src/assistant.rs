//! The Assistant (#959): Nib, an original fountain-pen nib with eyes, floating over the window
//! with Tips, cheeky Sayings, and messages from agents.
//!
//! The engine ([`wordcraft_engine::assistant`]) decides what Nib would say; this module keeps the
//! record of what the user did that the engine reads (an [`Observation`]), decides when Nib may
//! speak (rate limits, never over dialogs or menus), draws Nib and its Balloon, and handles the
//! `ui.assistant*` commands.
//!
//! Off by default: Nib introduces itself once, then only appears when turned on. The
//! Introduction, Tips and Sayings run only when the host sets [`Assistant::automatic`] (the
//! desktop app does); tests, `ui_shot` and headless runs stay quiet unless a command shows Nib.
//! Agent messages work whenever there is a window, and can be muted. Nothing typed is ever lost:
//! Nib and its Balloon never take keyboard focus: a click or drag on either, or a choice in its
//! menu, hands focus back to the document.

mod nib;

use std::collections::VecDeque;

use egui::{Align, Align2, Color32, CornerRadius, Layout, Order, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::assistant::{self as engine, Kind, Observation, Offer, Source, Used};

pub use nib::Pose;

use crate::WordApp;
use crate::theme::Tokens;

/// How often Nib may volunteer a Saying.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sayings {
    Off,
    #[default]
    Sometimes,
    Annoying,
}

impl Sayings {
    pub fn code(self) -> &'static str {
        match self {
            Sayings::Off => "off",
            Sayings::Sometimes => "sometimes",
            Sayings::Annoying => "annoying",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Sayings::Off),
            "sometimes" => Some(Sayings::Sometimes),
            "annoying" => Some(Sayings::Annoying),
            _ => None,
        }
    }
    /// Seconds between Sayings at this level; `None` = never.
    fn gap(self) -> Option<f64> {
        match self {
            Sayings::Off => None,
            Sayings::Sometimes => Some(15.0 * 60.0),
            Sayings::Annoying => Some(3.0 * 60.0),
        }
    }
}

/// The Assistant's settings, kept in `ui.json` (`assistant`). Nothing from the document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// Nib is turned on.
    pub visible: bool,
    /// The Introduction has run (whatever the answer).
    pub introduced: bool,
    pub sayings: Sayings,
    /// Tips the user asked never to see again.
    pub dismissed_tips: Vec<String>,
    /// Where the user dragged Nib (window points, top-left); `None` = the default corner.
    pub position: Option<[f32; 2]>,
    pub agent_messages_muted: bool,
}

/// Most commands remembered for the Triggers.
const LOG: usize = 64;
/// Most dismissed Tips remembered (there are far fewer Tips; this only bounds a hostile file).
const MAX_DISMISSED: usize = 64;
/// Agent messages: waiting at most, per message limits, and the gap between new ones.
const QUEUE: usize = 3;
const TEXT_MAX: usize = 280;
const CHOICES_MAX: usize = 3;
const CHOICE_MAX: usize = 40;
const FROM_MAX: usize = 40;
const AGENT_GAP: f64 = 3.0;
/// Answers kept for `ui.assistantReply`.
const REPLIES: usize = 32;
/// How long a Balloon stays: a Saying, a plain agent message, and a Tip while the user types on.
const SAYING_S: f64 = 8.0;
const AGENT_S: f64 = 12.0;
const TIP_TYPING_S: f64 = 20.0;
/// Nib's size on screen, in points.
const NIB: Vec2 = vec2(64.0, 76.0);
const BALLOON_W: f32 = 260.0;

/// What the Balloon shows.
#[derive(Clone, Debug, PartialEq)]
enum Content {
    Intro,
    Offer(Offer),
    Agent { id: u64, from: String, text: String, choices: Vec<String> },
}

#[derive(Clone, Debug)]
struct Balloon {
    content: Content,
    opened: f64,
    /// When the user started typing again with a Tip open.
    typing_from: Option<f64>,
    /// "Don't show me this tip again" is ticked.
    dont_show: bool,
}

/// An agent message waiting for the Balloon.
#[derive(Clone, Debug)]
struct Queued {
    id: u64,
    from: String,
    text: String,
    choices: Vec<String>,
}

/// The Assistant while the app runs (not saved).
#[derive(Default)]
pub struct Assistant {
    /// The Introduction, Tips and Sayings run on their own. The desktop app turns this on;
    /// tests, `ui_shot` and headless runs leave it off.
    pub automatic: bool,
    log: VecDeque<Used>,
    /// Commands recorded so far (the Triggers look again only after a new one, or for idling).
    recorded: u64,
    checked: u64,
    balloon: Option<Balloon>,
    queue: VecDeque<Queued>,
    /// Answered messages: the choice (`None`: closed unanswered), and whether muting dropped it
    /// unseen.
    replies: VecDeque<(u64, Option<String>, bool)>,
    next_id: u64,
    now: f64,
    last_activity: f64,
    last_saying: Option<f64>,
    last_agent: Option<f64>,
    last_idle_check: f64,
    unsaved_since: Option<f64>,
    words_before: Option<usize>,
    deleted: usize,
    /// Tips shown this session: each at most once, so Nib doesn't nag.
    shown: Vec<&'static str>,
    /// Where the next command comes from (set by the ribbon and keyboard just before running).
    pub(crate) next_source: Option<Source>,
    pose: Pose,
    pose_since: f64,
    /// The welcome Tip ran since Nib was last turned on.
    welcomed: bool,
}

impl Assistant {
    /// Remember a command the user ran (`deleted` characters it removed), for the Triggers.
    pub(crate) fn record(&mut self, id: &str, deleted: usize) {
        let source = self.next_source.take().unwrap_or(Source::Other);
        if self.log.len() >= LOG {
            self.log.pop_front();
        }
        self.log.push_back(Used { id: id.to_string(), source, at: self.now });
        self.recorded = self.recorded.wrapping_add(1);
        self.last_activity = self.now;
        self.deleted = self.deleted.saturating_add(deleted);
        if id == "text.insert"
            && let Some(b) = &mut self.balloon
            && matches!(&b.content, Content::Offer(o) if o.kind == Kind::Tip)
        {
            b.typing_from.get_or_insert(self.now);
        }
    }

    fn open(&mut self, content: Content, pose: Pose) {
        self.balloon = Some(Balloon { content, opened: self.now, typing_from: None, dont_show: false });
        self.set_pose(pose);
    }

    fn set_pose(&mut self, pose: Pose) {
        self.pose = pose;
        self.pose_since = self.now;
    }

    /// Close the Balloon; an unanswered agent message is answered with nothing, and the
    /// Introduction closed any way but "Keep me around" (×, Hide) is declined, as "Not now".
    fn close(&mut self, prefs: &mut Prefs) {
        let Some(b) = self.balloon.take() else { return };
        match b.content {
            Content::Agent { id, .. } => self.answer(id, None),
            Content::Offer(o) if b.dont_show && o.kind == Kind::Tip => dismiss(prefs, o.id),
            Content::Intro => return self.answer_intro(prefs, false),
            _ => {}
        }
        self.set_pose(Pose::Idle);
    }

    /// Answer the Introduction: `keep` turns Nib on, otherwise it stays off. Either way it never
    /// asks again (`introduced` is saved with the other settings). Every way the Introduction
    /// closes comes here.
    fn answer_intro(&mut self, prefs: &mut Prefs, keep: bool) {
        if matches!(self.balloon, Some(Balloon { content: Content::Intro, .. })) {
            self.balloon = None;
        }
        prefs.introduced = true;
        prefs.visible = keep;
        self.welcomed = true;
        self.set_pose(if keep { Pose::Idle } else { Pose::Bow });
    }

    fn answer(&mut self, id: u64, choice: Option<String>) {
        self.reply(id, choice, false);
    }

    fn reply(&mut self, id: u64, choice: Option<String>, muted: bool) {
        if self.replies.len() >= REPLIES {
            self.replies.pop_front();
        }
        self.replies.push_back((id, choice, muted));
    }

    /// Muted: the agent messages still waiting are dropped, never shown (not even after
    /// unmuting), and each agent's `ui.assistantReply` says so. A message already showing stays.
    fn drop_waiting(&mut self) {
        while let Some(q) = self.queue.pop_front() {
            self.reply(q.id, None, true);
        }
    }

    fn agent_showing(&self) -> bool {
        matches!(self.balloon, Some(Balloon { content: Content::Agent { .. }, .. }))
    }
}

fn dismiss(prefs: &mut Prefs, id: &str) {
    if !prefs.dismissed_tips.iter().any(|d| d == id) && prefs.dismissed_tips.len() < MAX_DISMISSED {
        prefs.dismissed_tips.push(id.to_string());
    }
}

/// The text, translated, with its placeholders filled. `{command}` is a command's English
/// label, translated like the ribbon's.
fn offer_text(o: &Offer) -> String {
    let args: Vec<(&str, &str)> = o.args.iter().map(|(k, v)| (*k, if *k == "command" { tl!(v.as_str()) } else { v.as_str() })).collect();
    crate::i18n::fmt(tl!(o.text), &args)
}

/// The interface's own words around Nib (the Balloon, Nib's menu, File › Options), for the
/// catalog test; the Tips and Sayings come from [`engine::texts`].
#[cfg(test)]
pub(crate) const UI_TEXTS: [&str; 24] = [
    "Hi! I'm Nib, your writing assistant. I offer tips, shortcuts and the occasional unsolicited opinion. Shall I stick around?",
    "Keep me around",
    "Not now",
    "Tip",
    "From {name}",
    "an agent",
    "Close",
    "OK",
    "Don't show me this tip again",
    "Nib, your assistant. Drag to move; right-click for options.",
    "Hide",
    "Sayings",
    "Off",
    "Sometimes",
    "Annoying",
    "Mute agent messages",
    "Reset tips",
    "Options…",
    "Assistant",
    "Show the Assistant",
    "Sayings:",
    "Nib floats over the window with tips and the occasional remark.",
    "Show the tips you asked not to see again.",
    "Agents connected to WordCraft can show you messages through the Assistant. Muted, they're told you'd rather not.",
];

/// Clip to `max` characters (never mid-character).
fn clip(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect()
}

/// The sender name an agent message shows for `from`: trimmed, without control characters, at
/// most [`FROM_MAX`] characters; `None` when nothing is left. The control channel checks this
/// name, not the raw one, for WordCraft and Nib.
pub(crate) fn shown_from(from: &str) -> Option<String> {
    Some(clip(from.trim(), FROM_MAX).trim().to_string()).filter(|s| !s.is_empty())
}

/// The `ui.assistant*` commands; `None` for other ids.
pub(crate) fn command(app: &mut WordApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let a = &mut app.assistant;
    let prefs = &mut app.ui.assistant;
    Some(match id {
        "ui.assistant" => {
            // `show` true or false; no value toggles.
            let show = match p.get("show") {
                None => !prefs.visible,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Some(Err("`show` must be true or false".into())),
            };
            if show && !prefs.visible {
                a.welcomed = false;
                a.set_pose(Pose::Wave);
            }
            // Turning Nib on or off answers the Introduction too.
            if matches!(a.balloon, Some(Balloon { content: Content::Intro, .. })) {
                a.answer_intro(prefs, show);
            } else if !show && !a.agent_showing() {
                a.close(prefs);
            }
            prefs.visible = show;
            prefs.introduced = true;
            Ok(json!({"visible": prefs.visible}))
        }
        "ui.assistantSayings" => {
            if let Some(v) = p.get("level") {
                match v.as_str().and_then(Sayings::from_code) {
                    Some(l) => prefs.sayings = l,
                    None => return Some(Err("`level` must be off, sometimes or annoying".into())),
                }
            }
            Ok(json!({"level": prefs.sayings.code()}))
        }
        "ui.assistantResetTips" => {
            prefs.dismissed_tips.clear();
            a.shown.clear();
            Ok(json!({"dismissedTips": 0}))
        }
        "ui.assistantMute" => {
            // Agent messages: `value` true mutes them, false lets them through; no value reads it.
            match p.get("value") {
                None => {}
                Some(Value::Bool(b)) => prefs.agent_messages_muted = *b,
                Some(_) => return Some(Err("`value` must be true or false".into())),
            }
            if prefs.agent_messages_muted {
                a.drop_waiting();
            }
            Ok(json!({"muted": prefs.agent_messages_muted}))
        }
        "ui.assistantSay" => say(a, prefs, p),
        "ui.assistantReply" => {
            let Some(want) = p.get("id").and_then(Value::as_u64) else { return Some(Err("`id` (a number from ui.assistantSay) is required".into())) };
            if let Some((_, choice, muted)) = a.replies.iter().find(|(id, ..)| *id == want) {
                if *muted {
                    return Some(Ok(json!({"choice": null, "open": false, "muted": true})));
                }
                return Some(Ok(json!({"choice": choice, "open": false})));
            }
            let open =
                a.queue.iter().any(|q| q.id == want) || matches!(&a.balloon, Some(Balloon { content: Content::Agent { id, .. }, .. }) if *id == want);
            if !open {
                return Some(Err(format!("no message {want}")));
            }
            Ok(json!({"choice": null, "open": true}))
        }
        _ => return None,
    })
}

/// `ui.assistantSay`: queue an Agent message.
fn say(a: &mut Assistant, prefs: &Prefs, p: &Value) -> Result<Value, String> {
    if prefs.agent_messages_muted {
        return Err("the user muted agent messages".into());
    }
    let text = p.get("text").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if text.is_empty() {
        return Err("`text` is required".into());
    }
    if text.chars().count() > TEXT_MAX {
        return Err(format!("`text` is longer than {TEXT_MAX} characters"));
    }
    let choices: Vec<String> = match p.get("choices") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(c)) if c.len() <= CHOICES_MAX => {
            let mut out = Vec::new();
            for v in c {
                let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty() && s.chars().count() <= CHOICE_MAX) else {
                    return Err(format!("each choice must be text of 1 to {CHOICE_MAX} characters"));
                };
                out.push(clip(s, CHOICE_MAX));
            }
            out
        }
        Some(_) => return Err(format!("`choices` must be a list of at most {CHOICES_MAX} strings")),
    };
    let from = p.get("from").and_then(Value::as_str).and_then(shown_from);
    if a.queue.len() >= QUEUE {
        return Err("queue full: wait for the user to see the earlier messages".into());
    }
    if a.last_agent.is_some_and(|t| a.now - t < AGENT_GAP) {
        return Err(format!("too soon: one message every {AGENT_GAP} seconds"));
    }
    a.last_agent = Some(a.now);
    a.next_id = a.next_id.wrapping_add(1);
    let id = a.next_id;
    a.queue.push_back(Queued { id, from: from.unwrap_or_default(), text: clip(text, TEXT_MAX), choices });
    Ok(json!({"id": id}))
}

/// Nothing new appears over these.
fn busy(app: &WordApp, ctx: &egui::Context) -> bool {
    app.dialog.is_some()
        || app.ui.backstage
        || !app.has_document()
        || app.session.view.focus_mode
        || app.session.view.read_mode
        || egui::Popup::is_any_open(ctx)
}

/// Decide what Nib says this frame, then draw it.
pub(crate) fn show(app: &mut WordApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    app.assistant.now = now;
    // However it was muted, nothing waiting gets through.
    if app.ui.assistant.agent_messages_muted {
        app.assistant.drop_waiting();
    }
    expire(app);
    let busy = busy(app, ctx);
    if !busy {
        next_balloon(app);
    }
    let a = &app.assistant;
    let agent_visit = a.agent_showing();
    let intro = matches!(a.balloon, Some(Balloon { content: Content::Intro, .. }));
    if !(app.ui.assistant.visible || agent_visit || intro) || app.ui.backstage {
        return;
    }
    draw(app, ctx);
    ctx.request_repaint_after(std::time::Duration::from_secs_f32(repaint_after(&app.assistant, app.animations_on())));
}

/// Seconds until Nib needs another frame. A pose that moves (settling, dozing) needs frames often;
/// idle Nib stands still, so only the next blink; timers (fading, idling) need one now and then.
fn repaint_after(a: &Assistant, animate: bool) -> f32 {
    const TIMERS: f32 = 1.0;
    if !animate {
        return TIMERS;
    }
    if a.pose != Pose::Idle {
        return 1.0 / 30.0;
    }
    nib::idle_wait(a.now as f32).clamp(1.0 / 30.0, TIMERS)
}

/// Close a Balloon whose time is up.
fn expire(app: &mut WordApp) {
    let a = &mut app.assistant;
    let now = a.now;
    let done = match &a.balloon {
        Some(b) => match &b.content {
            Content::Offer(o) if o.kind == Kind::Saying => now - b.opened >= SAYING_S,
            Content::Offer(_) => b.typing_from.is_some_and(|t| now - t >= TIP_TYPING_S),
            Content::Agent { choices, .. } => choices.is_empty() && now - b.opened >= AGENT_S,
            Content::Intro => false,
        },
        None => false,
    };
    if done {
        a.close(&mut app.ui.assistant);
    }
}

/// Fill an empty Balloon: an agent message first, then the Introduction, a Tip, a Saying.
fn next_balloon(app: &mut WordApp) {
    // An agent message may replace a Saying, but waits for a Tip or the Introduction.
    let saying_open = matches!(&app.assistant.balloon, Some(Balloon { content: Content::Offer(o), .. }) if o.kind == Kind::Saying);
    if (app.assistant.balloon.is_none() || saying_open)
        && !app.ui.assistant.agent_messages_muted
        && let Some(q) = app.assistant.queue.pop_front()
    {
        app.assistant.balloon = None;
        app.assistant.open(Content::Agent { id: q.id, from: q.from, text: q.text, choices: q.choices }, Pose::Write);
        return;
    }
    // A Tip or Saying showing may give way to a newer Tip (below); the Introduction and agent
    // messages wait to be answered.
    let offer_open = matches!(app.assistant.balloon, Some(Balloon { content: Content::Offer(_), .. }));
    if (app.assistant.balloon.is_some() && !offer_open) || !app.assistant.automatic {
        return;
    }
    if !app.ui.assistant.introduced {
        app.assistant.open(Content::Intro, Pose::Wave);
        return;
    }
    if !app.ui.assistant.visible {
        return;
    }
    if !app.assistant.welcomed && !offer_open {
        app.assistant.welcomed = true;
        let daily = engine::tip_of_the_day(&app.session, wordcraft_engine::cmd::now_unix() / 86_400);
        if !app.ui.assistant.dismissed_tips.iter().any(|d| d == daily.id) {
            app.assistant.open(Content::Offer(daily), Pose::Wave);
        }
        return;
    }
    let a = &mut app.assistant;
    let now = a.now;
    // Look again after each new command, and every few seconds for idling.
    let fresh = a.recorded != a.checked;
    if !fresh && now - a.last_idle_check < 5.0 {
        return;
    }
    a.checked = a.recorded;
    a.last_idle_check = now;
    a.unsaved_since = if app.session.dirty { Some(a.unsaved_since.unwrap_or(now)) } else { None };
    // Word count at most once per command (it walks the document), handed to the Sayings so they
    // don't count again. Between commands the document hasn't changed: the last count stands.
    let (words_before, words) = match a.words_before {
        Some(w) if !fresh => (w, w),
        last => {
            let words = app.session.word_count();
            a.words_before = Some(words);
            (last.unwrap_or(words), words)
        }
    };
    let o = Observation {
        recent: a.log.iter().cloned().collect(),
        now,
        idle: now - a.last_activity,
        deleted: std::mem::take(&mut a.deleted),
        unsaved_for: a.unsaved_since.map_or(0.0, |t| now - t),
        words_before,
        words: Some(words),
        seed: a.recorded,
    };
    let dismissed = &app.ui.assistant.dismissed_tips;
    // The newest Tip is the one that fits what the user is doing: it replaces one still open.
    // Each Tip shows once a session, which is all the pacing Tips need.
    if let Some(tip) = engine::tips(&app.session, &o).into_iter().find(|t| !a.shown.contains(&t.id) && !dismissed.iter().any(|d| d == t.id)) {
        a.shown.push(tip.id);
        a.close(&mut app.ui.assistant);
        a.open(Content::Offer(tip), Pose::Write);
        return;
    }
    if offer_open {
        return;
    }
    let Some(gap) = app.ui.assistant.sayings.gap() else { return };
    if a.last_saying.is_some_and(|t| now - t < gap) {
        return;
    }
    if let Some(saying) = engine::sayings(&app.session, &o).into_iter().next() {
        let pose = match saying.id {
            "massDelete" => Pose::Startled,
            "idle" => Pose::Doze,
            _ => Pose::Idle,
        };
        a.last_saying = Some(now);
        a.open(Content::Offer(saying), pose);
    }
}

/// Where Nib stands: the saved place, else the bottom-right corner above the status bar; kept
/// inside the window.
fn nib_rect(app: &WordApp, screen: Rect) -> Rect {
    let fallback = Pos2::new(screen.max.x - NIB.x - 28.0, screen.max.y - NIB.y - 40.0);
    let pos = app.ui.assistant.position.map(|[x, y]| Pos2::new(x, y)).filter(|p| p.x.is_finite() && p.y.is_finite()).unwrap_or(fallback);
    let max = (screen.max - NIB).max(screen.min);
    Rect::from_min_size(pos.clamp(screen.min, max), NIB)
}

fn draw(app: &mut WordApp, ctx: &egui::Context) {
    let screen = ctx.content_rect();
    let rect = nib_rect(app, screen);
    let t = Tokens::get(ctx);
    let animate = app.animations_on();
    let since = (app.assistant.now - app.assistant.pose_since) as f32;
    let clock = app.assistant.now as f32;
    // Poses settle back to idle after a moment.
    if animate && !matches!(app.assistant.pose, Pose::Idle | Pose::Doze) && since > 2.5 {
        app.assistant.set_pose(Pose::Idle);
    }
    let pose = app.assistant.pose;
    let from_control = app.input_from_control(ctx);
    let area = egui::Area::new(nib_area()).order(Order::Foreground).fixed_pos(rect.min).interactable(true);
    let mut menu_action = None;
    let mut touched = false;
    area.show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(NIB, Sense::click_and_drag());
        nib::paint(ui.painter(), r, pose, if animate { clock } else { 0.0 }, if animate { since } else { 10.0 }, &t);
        let hint = tl!("Nib, your assistant. Drag to move; right-click for options.");
        // A screen reader reaches Nib as a button named by its tooltip (#959 review).
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, hint));
        let resp = resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(hint);
        if resp.dragged() {
            let p = rect.min + resp.drag_delta();
            app.ui.assistant.position = Some([p.x, p.y]);
        }
        touched |= resp.clicked() || resp.drag_stopped();
        if resp.clicked() && app.assistant.balloon.is_none() && app.ui.assistant.visible {
            // A click says hello: today's tip again.
            app.assistant.welcomed = false;
        }
        resp.context_menu(|ui| {
            if ui.button(tl!("Hide")).clicked() {
                menu_action = Some(("ui.assistant", json!({"show": false})));
            }
            ui.menu_button(tl!("Sayings"), |ui| {
                for (label, level) in [("Off", Sayings::Off), ("Sometimes", Sayings::Sometimes), ("Annoying", Sayings::Annoying)] {
                    if ui.radio(app.ui.assistant.sayings == level, tl!(label)).clicked() {
                        menu_action = Some(("ui.assistantSayings", json!({"level": level.code()})));
                    }
                }
            });
            let mut muted = app.ui.assistant.agent_messages_muted;
            // Only the user mutes or unmutes here; agents mute with `ui.assistantMute`.
            if ui.checkbox(&mut muted, tl!("Mute agent messages")).clicked() && !from_control {
                menu_action = Some(("ui.assistantMute", json!({"value": muted})));
            }
            if ui.button(tl!("Reset tips")).clicked() {
                menu_action = Some(("ui.assistantResetTips", json!({})));
            }
            if ui.button(tl!("Options…")).clicked() {
                menu_action = Some(("ui.backstage", json!({"value": true, "page": "options", "pane": "general"})));
            }
        });
    });
    touched |= menu_action.is_some();
    if let Some((id, params)) = menu_action {
        let _ = app.run(id, params);
    }
    if app.assistant.balloon.is_some() {
        touched |= balloon(app, ctx, rect, &t);
    }
    // Nib never keeps the keyboard: a click or drag anywhere on it or its Balloon, or a choice in
    // its menu, hands it back to the page, as a click on the ribbon does. egui drops the page's
    // focus on any click outside it, and Nib floats over the page, where the chrome check doesn't
    // look (#959 review). A right-click that opens the menu leaves the keyboard to the menu, and a
    // dialog or the File page a button opened keeps it.
    let on_nib = ctx
        .input(|i| if i.pointer.any_click() { i.pointer.interact_pos() } else { None })
        .and_then(|p| ctx.layer_id_at(p))
        .is_some_and(|l| l.id == nib_area() || l.id == balloon_area());
    let menu_open = egui::Popup::is_any_open(ctx);
    if (touched || (on_nib && !menu_open)) && app.dialog.is_none() && !app.ui.backstage {
        app.give_the_page_the_keyboard(ctx);
    }
}

/// Nib's and its Balloon's areas.
fn nib_area() -> egui::Id {
    egui::Id::unique("wc_assistant_nib")
}
fn balloon_area() -> egui::Id {
    egui::Id::unique("wc_assistant_balloon")
}

/// What a click in the Balloon asked for.
enum Clicked {
    Close,
    Run(&'static str, Value),
    Choose(String),
    Intro(bool),
}

/// Draws the Balloon and does what its buttons ask; whether one was pressed.
fn balloon(app: &mut WordApp, ctx: &egui::Context, nib: Rect, t: &Tokens) -> bool {
    let screen = ctx.content_rect();
    // Beside Nib, on the side with room, its bottom level with Nib's middle.
    let left = nib.min.x - BALLOON_W - 8.0 >= screen.min.x;
    let (pivot, at) = if left {
        (Align2::RIGHT_BOTTOM, Pos2::new(nib.min.x - 6.0, nib.center().y))
    } else {
        (Align2::LEFT_BOTTOM, Pos2::new(nib.max.x + 6.0, nib.center().y))
    };
    let Some(b) = app.assistant.balloon.clone() else { return false };
    let agent = matches!(b.content, Content::Agent { .. });
    let mut clicked = None;
    let mut dont_show = b.dont_show;
    let area = egui::Area::new(balloon_area()).order(Order::Foreground).pivot(pivot).fixed_pos(at).constrain(true).interactable(true);
    area.show(ctx, |ui| {
        let stroke = if agent { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.border_strong) };
        let frame = egui::Frame::new().fill(t.menu).stroke(stroke).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(10));
        frame.show(ui, |ui| {
            ui.set_width(BALLOON_W - 20.0);
            ui.horizontal(|ui| {
                match &b.content {
                    Content::Agent { from, .. } => {
                        let name = if from.is_empty() { tl!("an agent").to_string() } else { from.clone() };
                        ui.label(egui::RichText::new(crate::i18n::fmt(tl!("From {name}"), &[("name", &name)])).small().strong().color(t.accent_text));
                    }
                    Content::Offer(o) if o.kind == Kind::Tip => {
                        ui.label(egui::RichText::new(tl!("Tip")).small().strong());
                    }
                    _ => {
                        ui.label(egui::RichText::new("Nib").small().strong());
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text(tl!("Close")).clicked() {
                        clicked = Some(Clicked::Close);
                    }
                });
            });
            let text = match &b.content {
                Content::Intro => {
                    tl!("Hi! I'm Nib, your writing assistant. I offer tips, shortcuts and the occasional unsolicited opinion. Shall I stick around?")
                        .to_string()
                }
                Content::Offer(o) => offer_text(o),
                Content::Agent { text, .. } => text.clone(),
            };
            let said = ui.add(egui::Label::new(text).wrap());
            // A live region: screen readers announce the words when the Balloon opens or they
            // change, without moving the user's focus from the document (#959 review).
            ctx.accesskit_node_builder(said.id, |n| n.set_live(egui::accesskit::Live::Polite));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| match &b.content {
                Content::Intro => {
                    if ui.button(tl!("Keep me around")).clicked() {
                        clicked = Some(Clicked::Intro(true));
                    }
                    if ui.button(tl!("Not now")).clicked() {
                        clicked = Some(Clicked::Intro(false));
                    }
                }
                Content::Offer(o) => {
                    for btn in o.buttons.iter().filter(|btn| crate::widgets::enabled(app, btn.command)) {
                        if ui.button(tl!(btn.label)).clicked() {
                            clicked = Some(Clicked::Run(btn.command, btn.params.clone()));
                        }
                    }
                    if ui.button(tl!("OK")).clicked() {
                        clicked = Some(Clicked::Close);
                    }
                }
                Content::Agent { choices, .. } => {
                    for c in choices {
                        if ui.button(c.as_str()).clicked() {
                            clicked = Some(Clicked::Choose(c.clone()));
                        }
                    }
                    if choices.is_empty() && ui.button(tl!("OK")).clicked() {
                        clicked = Some(Clicked::Close);
                    }
                }
            });
            if matches!(&b.content, Content::Offer(o) if o.kind == Kind::Tip) {
                ui.checkbox(&mut dont_show, tl!("Don't show me this tip again"));
            }
        });
    });
    if let Some(open) = &mut app.assistant.balloon {
        open.dont_show = dont_show;
    }
    let Some(c) = clicked else { return false };
    // Only the user answers an agent's question: a click the control channel made (an agent
    // through MCP's `click`, say) leaves it open, as if it hadn't happened.
    if matches!(c, Clicked::Choose(_)) && app.input_from_control(ctx) {
        return true;
    }
    match c {
        Clicked::Close => app.assistant.close(&mut app.ui.assistant),
        Clicked::Run(id, params) => {
            app.assistant.close(&mut app.ui.assistant);
            let _ = app.run(id, params);
        }
        Clicked::Choose(choice) => {
            if let Some(Balloon { content: Content::Agent { id, .. }, .. }) = app.assistant.balloon.take() {
                app.assistant.answer(id, Some(choice));
            }
            app.assistant.set_pose(Pose::Idle);
        }
        Clicked::Intro(keep) => app.assistant.answer_intro(&mut app.ui.assistant, keep),
    }
    true
}

/// Nib's metal: the ArtCraft logo blue (`docs/brand/artcraft-mark.svg`), for light and dark alike.
pub(crate) const BLUE: Color32 = Color32::from_rgb(78, 123, 251);

#[cfg(test)]
mod tests;
