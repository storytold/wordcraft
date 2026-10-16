//! The Assistant's Tips and Sayings, and the Triggers that pick them (#959).
//!
//! The Assistant (Nib, drawn by the UI) offers a **Tip** when something the user just did has a
//! better way, and now and then a deliberately cheeky **Saying**. This module decides *what* it
//! would say: pure functions over the [`Session`] plus an [`Observation`] the caller builds from
//! what the user did (the UI keeps that log; the session doesn't). *How often* it speaks, and
//! drawing it, belong to the UI.
//!
//! Text is English source, translated by the UI (`{name}` placeholders filled from `args`).
//! Sayings are jokes: translators replace them with a local one rather than translate them.

use serde_json::{Value, json};

use crate::Session;

/// Where a command came from, as far as the Triggers care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Ribbon,
    Menu,
    Keyboard,
    Search,
    Agent,
    Other,
}

/// A command the user ran, `at` seconds on the caller's clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Used {
    pub id: String,
    pub source: Source,
    pub at: f64,
}

/// What the user has been doing, as the caller saw it. Every field is a hint: odd values (NaN,
/// negative, huge) only make Triggers stay quiet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observation {
    /// Recent commands, oldest first. Only the last [`RECENT`] are looked at.
    pub recent: Vec<Used>,
    /// The caller's clock, in seconds (same clock as [`Used::at`]).
    pub now: f64,
    /// Seconds since the user last typed or clicked in the document.
    pub idle: f64,
    /// Local hour of day, 0–23, when known.
    pub hour: Option<u8>,
    /// Characters removed by the last command.
    pub deleted: usize,
    /// Seconds the document has had unsaved changes.
    pub unsaved_for: f64,
    /// The word count when the caller last asked (for milestones).
    pub words_before: usize,
    /// Picks between equally good lines, so Sayings vary.
    pub seed: u64,
}

/// How many recent commands the Triggers look at.
pub const RECENT: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tip,
    Saying,
}

/// A button on a Tip: runs `command` with `params`, as the ribbon button for it would.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub label: &'static str,
    pub command: &'static str,
    pub params: Value,
}

/// Something the Assistant could say now.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// Stable id: what "Don't show me this tip again" remembers.
    pub id: &'static str,
    pub kind: Kind,
    /// English source text with `{name}` placeholders.
    pub text: &'static str,
    pub args: Vec<(&'static str, String)>,
    pub buttons: Vec<Button>,
}

impl Offer {
    fn tip(id: &'static str, text: &'static str) -> Self {
        Offer { id, kind: Kind::Tip, text, args: Vec::new(), buttons: Vec::new() }
    }
    fn saying(id: &'static str, text: &'static str) -> Self {
        Offer { id, kind: Kind::Saying, text, args: Vec::new(), buttons: Vec::new() }
    }
    fn button(mut self, label: &'static str, command: &'static str) -> Self {
        self.buttons.push(Button { label, command, params: json!({}) });
        self
    }
    fn arg(mut self, name: &'static str, value: String) -> Self {
        self.args.push((name, value));
        self
    }
    /// The English text with its placeholders filled.
    pub fn english(&self) -> String {
        self.args.iter().fold(self.text.to_string(), |t, (k, v)| t.replace(&format!("{{{k}}}"), v))
    }
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": match self.kind { Kind::Tip => "tip", Kind::Saying => "saying" },
            "text": self.english(),
            "buttons": self.buttons.iter().map(|b| json!({"label": b.label, "command": b.command, "params": b.params})).collect::<Vec<_>>(),
        })
    }
}

/// The last [`RECENT`] commands, oldest first.
fn recent(o: &Observation) -> &[Used] {
    let n = o.recent.len();
    o.recent.get(n.saturating_sub(RECENT)..).unwrap_or(&[])
}

fn last_id(o: &Observation) -> Option<&str> {
    recent(o).last().map(|u| u.id.as_str())
}

fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// The caret paragraph's text, capped (the Triggers only look at its start).
fn caret_text(s: &Session) -> String {
    s.doc.para_at(&s.sel.focus).map(|p| p.plain_text().chars().take(200).collect()).unwrap_or_default()
}

/// Tips whose Triggers match now, most specific first.
pub fn tips(s: &Session, o: &Observation) -> Vec<Offer> {
    let mut out = Vec::new();
    let typing = matches!(last_id(o), Some("text.insert"));
    let text = caret_text(s);

    // Letter: a paragraph that starts "Dear …".
    if typing && text.starts_with("Dear ") && text.chars().count() <= 60 {
        out.push(
            Offer::tip("letter", "It looks like you're writing a letter. Want an envelope for it, or a letter template to start from?")
                .button("Envelopes", "mailings.envelopes")
                .button("Letter template", "file.newFromTemplate"),
        );
    }

    // List started: AutoFormat just turned the paragraph into a list item.
    let in_list = s.doc.para_at(&s.sel.focus).is_some_and(|p| p.props.numbering.is_some_and(|n| n.num != 0));
    if typing && in_list && text.trim().is_empty() {
        out.push(
            Offer::tip(
                "listStarted",
                "You started a list. Press Enter twice to end it, or Backspace at the start of an item to turn it back into text.",
            )
            .button("Restart at 1", "para.restartNumbering"),
        );
    }

    // Shortcut: the same command from the ribbon or a menu three times, when it has a shortcut.
    if let Some(u) = recent(o).last()
        && matches!(u.source, Source::Ribbon | Source::Menu)
        && recent(o).iter().filter(|r| r.id == u.id && matches!(r.source, Source::Ribbon | Source::Menu)).count() >= 3
        && let Some(spec) = s.registry.get(&u.id)
        && !spec.shortcut.is_empty()
    {
        let key = spec.shortcut.split(" / ").next().unwrap_or(spec.shortcut).replace("Mod+", if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" });
        out.push(Offer::tip("shortcut", "Faster: {command} is {key} on the keyboard.").arg("command", spec.label.to_string()).arg("key", key));
    }

    // Save reminder: AutoSave off and twenty minutes of unsaved work.
    if !s.autosave && s.dirty && finite(o.unsaved_for).is_some_and(|t| t >= 20.0 * 60.0) {
        out.push(Offer::tip("saveReminder", "You haven't saved in a while, and AutoSave is off. Save now?").button("Save", "file.save"));
    }
    out
}

/// General Tips, one at a time ("tip of the day").
const DAILY: [(&str, &str); 10] = [
    ("daily.search", "Can't find a command? Press Alt+Q and type what you want to do."),
    ("daily.pageBreak", "Ctrl+Enter starts a new page. Much tidier than pressing Enter until it happens."),
    ("daily.marks", "Show/Hide ¶ (Ctrl+Shift+8) shows the spaces, tabs and breaks hiding in your text."),
    ("daily.repeat", "F4 repeats your last action. Formatting ten headings is suddenly one click and nine taps."),
    ("daily.lineBreak", "Shift+Enter starts a new line without starting a new paragraph."),
    ("daily.wordCount", "Ctrl+Shift+G counts your words, characters and paragraphs."),
    ("daily.spelling", "F7 checks the spelling and grammar of the whole document."),
    ("daily.link", "Ctrl+K turns the selected text into a link."),
    ("daily.findReplace", "Ctrl+H finds and replaces text, formatting and special characters."),
    ("daily.focus", "View › Focus hides everything but your words. I'll wait outside."),
];

/// A tip of the day, chosen by `n` (the UI rotates it).
pub fn tip_of_the_day(n: u64) -> Offer {
    let len = DAILY.len() as u64;
    let (id, text) = DAILY.get((n % len) as usize).or(DAILY.first()).copied().unwrap_or(("daily.search", ""));
    Offer::tip(id, text)
}

/// Every Tip id the UI may remember as dismissed (for "Reset tips" and tests).
pub fn tip_ids() -> impl Iterator<Item = &'static str> {
    ["letter", "listStarted", "shortcut", "saveReminder"].into_iter().chain(DAILY.iter().map(|(id, _)| *id))
}

/// Saying lines per Trigger. Original, light, never about the user's writing being bad.
const IDLE: [&str; 5] = [
    "Writer's block? I find staring at the cursor helps. It's been blinking at me for years.",
    "Still there? I can hear you thinking.",
    "Take your time. The page isn't going anywhere. Neither am I. Ever.",
    "Fun fact: a blank page is just a draft that hasn't been told yet.",
    "I'd write something myself, but I'm all nib and no ink.",
];
const MASS_DELETE: [&str; 3] = [
    "Bold choice. Ctrl+Z is right there if you need it.",
    "Gone! Like it was never written. Mostly because it now wasn't.",
    "Ah, the editing phase. My favourite. Very decisive.",
];
const SAVE_SPAM: [&str; 3] = [
    "Saved. Saved again. Still saved. It's very saved.",
    "I promise it's saved. I checked. Twice.",
    "Your document is now the most saved document in history.",
];
const LOREM: [&str; 2] = ["Ah, Latin. The language of deadlines.", "Lorem ipsum: because the real words are still at the café."];
const MIDNIGHT: [&str; 3] = [
    "Writing after midnight? The best ideas and the worst typos happen now.",
    "It's late. I'll keep the lamp on.",
    "Burning the midnight ink, I see.",
];
const UNDO: [&str; 3] = [
    "Undo, undo, undo… we're time travelling now.",
    "Careful, any further back and we'll undo the invention of paper.",
    "Rewinding. Tell me when to stop.",
];
const MILESTONE: [&str; 3] = [
    "{words} words! I'd throw confetti but I'd have to clean it up.",
    "{words} words. That's {words} more than I've written today.",
    "{words} words and counting. Literally. I'm counting.",
];

/// Word-count milestones a Saying celebrates.
const MILESTONES: [usize; 7] = [500, 1000, 2500, 5000, 10_000, 25_000, 50_000];

fn pick(lines: &[&'static str], seed: u64) -> &'static str {
    let n = lines.len().max(1) as u64;
    lines.get((seed % n) as usize).copied().unwrap_or("")
}

/// Sayings whose Triggers match now.
pub fn sayings(s: &Session, o: &Observation) -> Vec<Offer> {
    let mut out = Vec::new();
    let r = recent(o);
    if o.deleted >= 200 {
        out.push(Offer::saying("massDelete", pick(&MASS_DELETE, o.seed)));
    }
    if r.len() >= 5 && r.iter().rev().take(5).all(|u| u.id == "edit.undo") {
        out.push(Offer::saying("undoStreak", pick(&UNDO, o.seed)));
    }
    let now = finite(o.now).unwrap_or(0.0);
    let saves: Vec<&Used> = r.iter().rev().take(3).collect();
    if !s.dirty && saves.len() == 3 && saves.iter().all(|u| u.id == "file.save" && finite(u.at).is_some_and(|t| now - t <= 30.0)) {
        out.push(Offer::saying("saveSpam", pick(&SAVE_SPAM, o.seed)));
    }
    if matches!(last_id(o), Some("text.insert")) && caret_text(s).to_lowercase().contains("lorem ipsum") {
        out.push(Offer::saying("lorem", pick(&LOREM, o.seed)));
    }
    let words = s.word_count();
    if let Some(m) = MILESTONES.iter().rev().find(|&&m| o.words_before < m && words >= m) {
        out.push(Offer::saying("milestone", pick(&MILESTONE, o.seed)).arg("words", m.to_string()));
    }
    if o.hour.is_some_and(|h| h < 4) && finite(o.idle).is_some_and(|i| i < 60.0) && matches!(last_id(o), Some("text.insert")) {
        out.push(Offer::saying("midnight", pick(&MIDNIGHT, o.seed)));
    }
    if finite(o.idle).is_some_and(|i| i >= 180.0) {
        out.push(Offer::saying("idle", pick(&IDLE, o.seed)));
    }
    out
}

/// `assistant.suggest`: the Tips and Sayings the document alone triggers now (no record of what
/// the user did: that lives in the UI). Read-only; for agents and tests.
pub(crate) fn suggest(s: &mut Session, _v: &Value) -> crate::CmdResult {
    let o = Observation {
        words_before: s.word_count(),
        recent: vec![Used { id: "text.insert".into(), source: Source::Keyboard, at: 0.0 }],
        ..Default::default()
    };
    let all: Vec<Value> = tips(s, &o).iter().chain(sayings(s, &o).iter()).map(Offer::to_json).collect();
    Ok(json!({"offers": all}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &mut Session, text: &str) -> Observation {
        s.run("text.insert", &json!({"text": text})).unwrap();
        Observation {
            recent: vec![Used { id: "text.insert".into(), source: Source::Keyboard, at: 0.0 }],
            words_before: s.word_count(),
            ..Default::default()
        }
    }

    fn ids(v: &[Offer]) -> Vec<&'static str> {
        v.iter().map(|o| o.id).collect()
    }

    #[test]
    fn a_letter_gets_letter_help() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let o = typed(&mut s, "Dear Sam,");
        let t = tips(&s, &o);
        assert_eq!(ids(&t), ["letter"]);
        assert_eq!(t[0].buttons.iter().map(|b| b.command).collect::<Vec<_>>(), ["mailings.envelopes", "file.newFromTemplate"]);
        // Not for "Dear" mid-sentence, nor when the last thing wasn't typing.
        let mut s = Session::new(wordcraft_doc::Document::new());
        let o = typed(&mut s, "Oh Dear me");
        assert!(tips(&s, &o).is_empty());
        let mut s = Session::new(wordcraft_doc::Document::new());
        let mut o = typed(&mut s, "Dear Sam,");
        o.recent.clear();
        assert!(tips(&s, &o).is_empty());
    }

    #[test]
    fn autoformatting_a_list_explains_lists() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "1."})).unwrap();
        let o = typed(&mut s, " ");
        assert_eq!(ids(&tips(&s, &o)), ["listStarted"]);
        let mut s = Session::new(wordcraft_doc::Document::new());
        let o = typed(&mut s, "1 thing");
        assert!(tips(&s, &o).is_empty());
    }

    #[test]
    fn the_ribbon_three_times_shows_the_shortcut() {
        let s = Session::new(wordcraft_doc::Document::new());
        let used = |source| Used { id: "format.bold".into(), source, at: 0.0 };
        let o = Observation { recent: vec![used(Source::Ribbon), used(Source::Menu), used(Source::Ribbon)], ..Default::default() };
        let t = tips(&s, &o);
        assert_eq!(ids(&t), ["shortcut"]);
        assert!(t[0].english().contains("Bold") && t[0].english().contains("B"), "{}", t[0].english());
        // Twice isn't a habit; the keyboard is already the fast way.
        let o = Observation { recent: vec![used(Source::Ribbon), used(Source::Ribbon)], ..Default::default() };
        assert!(tips(&s, &o).is_empty());
        let o = Observation { recent: vec![used(Source::Keyboard), used(Source::Keyboard), used(Source::Keyboard)], ..Default::default() };
        assert!(tips(&s, &o).is_empty());
    }

    #[test]
    fn long_unsaved_work_without_autosave_gets_a_reminder() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Draft"})).unwrap();
        s.autosave = false;
        let o = Observation { unsaved_for: 25.0 * 60.0, ..Default::default() };
        assert_eq!(ids(&tips(&s, &o)), ["saveReminder"]);
        s.autosave = true;
        assert!(tips(&s, &o).is_empty());
        s.autosave = false;
        assert!(tips(&s, &Observation { unsaved_for: 60.0, ..Default::default() }).is_empty());
    }

    #[test]
    fn sayings_have_their_triggers() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let o = typed(&mut s, "lorem ipsum dolor");
        assert_eq!(ids(&sayings(&s, &o)), ["lorem"]);

        let s = Session::new(wordcraft_doc::Document::new());
        assert_eq!(ids(&sayings(&s, &Observation { deleted: 500, ..Default::default() })), ["massDelete"]);
        assert_eq!(ids(&sayings(&s, &Observation { idle: 300.0, ..Default::default() })), ["idle"]);
        let undo = |at| Used { id: "edit.undo".into(), source: Source::Keyboard, at };
        assert_eq!(ids(&sayings(&s, &Observation { recent: (0..5).map(|i| undo(i as f64)).collect(), ..Default::default() })), ["undoStreak"]);
        assert!(sayings(&s, &Observation { recent: (0..4).map(|i| undo(i as f64)).collect(), ..Default::default() }).is_empty());
        let save = |at| Used { id: "file.save".into(), source: Source::Keyboard, at };
        assert_eq!(ids(&sayings(&s, &Observation { recent: vec![save(1.0), save(2.0), save(3.0)], now: 5.0, ..Default::default() })), ["saveSpam"]);
        assert!(sayings(&s, &Observation { recent: vec![save(1.0), save(2.0), save(3.0)], now: 500.0, ..Default::default() }).is_empty());
    }

    #[test]
    fn midnight_and_milestones() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let mut o = typed(&mut s, "Night");
        o.hour = Some(1);
        assert_eq!(ids(&sayings(&s, &o)), ["midnight"]);
        o.hour = Some(13);
        assert!(sayings(&s, &o).is_empty());
        let words = "word ".repeat(500);
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": words})).unwrap();
        let o = Observation { words_before: 499, ..Default::default() };
        let said = sayings(&s, &o);
        assert_eq!(ids(&said), ["milestone"]);
        assert!(said[0].english().contains("500"), "{}", said[0].english());
        assert!(sayings(&s, &Observation { words_before: 500, ..Default::default() }).is_empty());
    }

    #[test]
    fn tips_of_the_day_rotate_and_ids_are_unique() {
        assert_ne!(tip_of_the_day(0).id, tip_of_the_day(1).id);
        assert_eq!(tip_of_the_day(0).id, tip_of_the_day(DAILY.len() as u64).id);
        let ids: Vec<_> = tip_ids().collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn hostile_observations_stay_quiet() {
        let s = Session::new(wordcraft_doc::Document::new());
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300] {
            let o = Observation {
                now: x,
                idle: x,
                unsaved_for: x,
                hour: Some(255),
                deleted: usize::MAX,
                words_before: usize::MAX,
                seed: u64::MAX,
                recent: (0..10_000).map(|_| Used { id: "file.save".into(), source: Source::Agent, at: x }).collect(),
            };
            let _ = tips(&s, &o);
            let _ = sayings(&s, &o);
        }
    }

    #[test]
    fn suggest_reports_what_the_document_triggers() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Dear Ana,"})).unwrap();
        let v = s.run("assistant.suggest", &json!({})).unwrap();
        assert_eq!(v["offers"][0]["id"], "letter");
        assert_eq!(v["offers"][0]["kind"], "tip");
    }
}
