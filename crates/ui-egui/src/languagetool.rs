//! LanguageTool checking in the app (#575): File › Options › Proofing and `ui.languageTool`.
//!
//! Off by default (text leaves the machine). When on, [`poll`] runs every frame: once the
//! document has been still for [`DEBOUNCE_S`], it sends the next body paragraph without a result
//! (starting at the caret) to a [`Worker`] thread, one paragraph at a time. Results land in
//! [`wordcraft_proof::external`], keyed by the paragraph's exact text, so a paragraph edited
//! meanwhile keeps the built-in squiggles until its new text is checked; layout and the Review
//! commands (context menu suggestions, Editor, F7) read them from there. Typing never waits.
//!
//! Failures pause checking and show in Options: no connection or a timeout for
//! [`RETRY_S`], "too many requests" for a minute, a rejected API key or a bad server address
//! until the settings change. The browser build can't check and says why.

use std::collections::HashSet;

use serde_json::{Value, json};
use wordcraft_engine::doc::{Document, PartKind, StoryRef, props::CharProps};
use wordcraft_languagetool::{Job, LtError, Settings, Worker, http, request};

use crate::WordApp;

/// Seconds the document must be still before a changed paragraph is sent.
pub const DEBOUNCE_S: f64 = 0.8;
/// Seconds checking pauses after a network failure.
pub const RETRY_S: f64 = 30.0;
/// Seconds checking pauses when the server limits requests.
pub const RATE_LIMIT_S: f64 = 60.0;
/// Paragraph texts remembered as not checkable (too long, unreadable answer).
const MAX_SKIPPED: usize = 5_000;

/// The app's LanguageTool state.
#[derive(Default)]
pub struct LtLink {
    worker: Option<Worker>,
    /// The settings `worker` and the stored results belong to.
    settings: Option<Settings>,
    /// The paragraph text being checked.
    in_flight: Option<String>,
    /// Debounce: the document revision last seen and when it changed.
    seen_rev: u64,
    changed_at: f64,
    /// The revision at which nothing was left to check (no scan until something changes).
    idle_rev: Option<u64>,
    /// No requests before this time.
    paused_until: f64,
    /// Checking stopped until the settings change (rejected key, bad address).
    stopped: Option<String>,
    /// (language, text) already sent at the current revision: one pass per revision, however
    /// many results the store keeps.
    sent: HashSet<(String, String)>,
    /// Texts not to send again.
    skipped: HashSet<(String, String)>,
    /// Paragraphs checked since the settings last changed.
    pub checked: u64,
    /// The last failure, shown in Options.
    pub last_error: Option<String>,
    /// Test hook: a fake worker transport instead of HTTP.
    #[cfg(test)]
    pub(crate) fake: Option<std::sync::Arc<dyn Fn(&wordcraft_languagetool::Request) -> Result<String, LtError> + Send + Sync>>,
}

impl LtLink {
    /// Forget results, the worker and failures (settings changed, or checking turned off).
    fn reset(&mut self) {
        self.worker = None;
        self.settings = None;
        self.in_flight = None;
        self.idle_rev = None;
        self.paused_until = 0.0;
        self.stopped = None;
        self.skipped.clear();
        self.sent.clear();
        self.checked = 0;
        self.last_error = None;
    }
}

/// Metadata for `ui.languageTool`, listed by `engine.commands` beside the registry's commands.
pub fn commands() -> Vec<Value> {
    vec![json!({
        "id": "ui.languageTool",
        "label": "LanguageTool checking",
        "location": "File › Options › Proofing",
        "shortcut": null,
        "params": "enabled?: bool; server?: http(s) address (empty: the public server; plain http with an account only for this computer); username?: string; apiKey?: string; language?: auto|document|code such as en-US. No params reads the settings (the key is never returned). Off by default: when on, paragraph text goes to the server.",
        "enabled": true,
    })]
}

/// Handle `ui.languageTool`. `None` when `id` isn't it.
///
/// Params (all optional; none reads the settings): `enabled`, `server`, `username`, `apiKey`,
/// `language` (`auto`, `document` or a code). The API key is never returned.
pub fn command(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "ui.languageTool" {
        return None;
    }
    Some(configure(app, params))
}

fn configure(app: &mut WordApp, p: &Value) -> Result<Value, String> {
    let mut s = app.ui.language_tool.clone();
    let old_key = s.api_key.clone();
    let mut server_changed = false;
    let text = |key: &str| -> Result<Option<String>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(t)) if t.len() <= 2048 => Ok(Some(t.trim().to_string())),
            Some(_) => Err(format!("`{key}` must be text")),
        }
    };
    if let Some(v) = p.get("enabled") {
        s.enabled = v.as_bool().ok_or("`enabled` must be true or false")?;
    }
    if let Some(server) = text("server")? {
        let server = if server.is_empty() { wordcraft_languagetool::DEFAULT_SERVER.into() } else { server };
        // A key belongs to the server it was entered for; a new address must not inherit it.
        if server != s.server {
            s.api_key.clear();
            server_changed = true;
        }
        s.server = server;
    }
    if let Some(u) = text("username")? {
        s.username = u;
    }
    if let Some(k) = text("apiKey")? {
        // The Options form resends the stored key with every edit; on a new server only a different key counts.
        s.api_key = if server_changed && k == old_key { String::new() } else { k };
    }
    if let Some(l) = text("language")? {
        if !(l == "auto" || l == "document" || wordcraft_languagetool::is_language_code(&l)) {
            return Err(format!("unknown language `{l}`; use auto, document or a code such as en-US"));
        }
        s.language = l;
    }
    s.endpoint().map_err(|e| e.to_string())?;
    if s.enabled && !http::available() {
        return Err(http::UNSUPPORTED_REASON.into());
    }
    app.ui.language_tool = s;
    Ok(status(app))
}

/// Settings and progress, without the API key.
pub fn status(app: &WordApp) -> Value {
    let s = &app.ui.language_tool;
    let lt = &app.language_tool;
    json!({
        "enabled": s.enabled,
        "server": s.server,
        "username": s.username,
        "apiKeySet": !s.api_key.is_empty(),
        "language": s.language,
        "available": http::available(),
        "reason": (!http::available()).then_some(http::UNSUPPORTED_REASON),
        "checked": lt.checked,
        "checking": lt.in_flight.is_some(),
        "error": lt.stopped.clone().or_else(|| lt.last_error.clone()),
    })
}

/// A line for File › Options about what checking is doing.
pub fn status_line(app: &WordApp) -> Option<String> {
    let lt = &app.language_tool;
    if !app.ui.language_tool.enabled {
        return None;
    }
    if let Some(e) = lt.stopped.as_ref().or(lt.last_error.as_ref()) {
        return Some(e.clone());
    }
    Some(crate::i18n::fmt(tl!("Paragraphs checked: {count}"), &[("count", &lt.checked.to_string())]))
}

/// Run checking: take results, then send the next paragraph when the document is still.
pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    poll_at(app, ctx, now);
}

/// [`poll`] at time `now` (seconds, egui's clock).
fn poll_at(app: &mut WordApp, ctx: &egui::Context, now: f64) {
    let settings = app.ui.language_tool.clone();
    if !settings.enabled || !http::available() {
        // Only an app that was checking clears the (process-wide) results.
        if app.language_tool.settings.is_some() {
            app.language_tool.reset();
            wordcraft_proof::external::clear();
            app.session.proofing_changed();
        }
        return;
    }
    if app.language_tool.settings.as_ref() != Some(&settings) {
        app.language_tool.reset();
        wordcraft_proof::external::clear();
        app.session.proofing_changed();
        app.language_tool.settings = Some(settings.clone());
    }
    wordcraft_proof::external::set_mode(&settings.language);
    take_results(app, ctx, now);
    let lt = &mut app.language_tool;
    if lt.stopped.is_some() || lt.in_flight.is_some() || !app.session.view.proofing {
        return;
    }
    if now < lt.paused_until {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((lt.paused_until - now).clamp(0.05, RATE_LIMIT_S)));
        return;
    }
    let rev = app.session.rev();
    if rev != lt.seen_rev {
        lt.seen_rev = rev;
        lt.changed_at = now;
        lt.idle_rev = None;
        lt.sent.clear();
    }
    let still = now - lt.changed_at;
    if still < DEBOUNCE_S {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((DEBOUNCE_S - still).clamp(0.05, DEBOUNCE_S)));
        return;
    }
    if lt.idle_rev == Some(rev) {
        return;
    }
    let Some(job) = next_job(app, &settings) else {
        app.language_tool.idle_rev = Some(rev);
        return;
    };
    let lt = &mut app.language_tool;
    if lt.worker.is_none() {
        match spawn(lt, settings, ctx) {
            Ok(w) => lt.worker = Some(w),
            Err(e) => {
                lt.stopped = Some(e.to_string());
                return;
            }
        }
    }
    let text = job.text.clone();
    let key = (job.language.clone(), job.text.clone());
    if lt.worker.as_ref().is_some_and(|w| w.send(job)) {
        lt.in_flight = Some(text);
        lt.sent.insert(key);
    } else {
        lt.worker = None;
    }
}

#[cfg(not(test))]
fn spawn(_lt: &LtLink, settings: Settings, ctx: &egui::Context) -> Result<Worker, LtError> {
    let wake = ctx.clone();
    Worker::spawn_http(settings, Box::new(move || wake.request_repaint()))
}

#[cfg(test)]
fn spawn(lt: &LtLink, settings: Settings, ctx: &egui::Context) -> Result<Worker, LtError> {
    let wake = ctx.clone();
    // Tests never touch the network: no fake, no checking.
    let fake = lt.fake.clone().ok_or(LtError::Unsupported("no network in tests"))?;
    let pacing = wordcraft_languagetool::worker::Pacing {
        gap: std::time::Duration::ZERO,
        backoff: std::time::Duration::ZERO,
        timeout: std::time::Duration::from_secs(1),
    };
    Worker::spawn(settings, pacing, Box::new(move |r: &wordcraft_languagetool::Request| fake(r)), Box::new(move || wake.request_repaint()))
}

fn take_results(app: &mut WordApp, ctx: &egui::Context, now: f64) {
    let mut changed = false;
    loop {
        let lt = &mut app.language_tool;
        let Some(w) = lt.worker.as_ref() else { break };
        let done = match w.try_recv() {
            Ok(Some(d)) => d,
            Ok(None) => break,
            Err(()) => {
                lt.worker = None;
                lt.in_flight = None;
                break;
            }
        };
        if lt.in_flight.as_deref() == Some(done.text.as_str()) {
            lt.in_flight = None;
        }
        lt.idle_rev = None;
        let key = (done.language.clone(), done.text.clone());
        match done.result {
            Ok(issues) => {
                wordcraft_proof::external::put(&done.text, &done.language, issues);
                lt.checked = lt.checked.saturating_add(1);
                lt.last_error = None;
                changed = true;
            }
            Err(e @ (LtError::TooLong { .. } | LtError::Protocol(_))) => {
                if lt.skipped.len() >= MAX_SKIPPED {
                    lt.skipped.clear();
                }
                lt.skipped.insert((done.language, done.text));
                lt.last_error = Some(e.to_string());
            }
            Err(e @ (LtError::BadServer(_) | LtError::Unsupported(_))) => lt.stopped = Some(e.to_string()),
            Err(e) if e.is_auth() => lt.stopped = Some(e.to_string()),
            Err(e) => {
                // Asked again after the pause.
                lt.sent.remove(&key);
                lt.paused_until = now + if e == LtError::RateLimited { RATE_LIMIT_S } else { RETRY_S };
                lt.last_error = Some(e.to_string());
            }
        }
    }
    if changed {
        app.session.proofing_changed();
        ctx.request_repaint();
    }
}

/// Every story LanguageTool checks: the body, then headers, footers, footnotes and endnotes
/// (#762). Comments and text boxes keep the built-in checker.
fn stories(doc: &Document) -> Vec<StoryRef> {
    let parts = doc.parts.iter().filter(|(_, p)| matches!(p.kind, PartKind::Header | PartKind::Footer | PartKind::Footnote | PartKind::Endnote));
    std::iter::once(StoryRef::Body).chain(parts.map(|(id, _)| StoryRef::Part(*id))).collect()
}

/// The next paragraph without a result: the body from the caret's paragraph on, then the
/// other stories.
fn next_job(app: &WordApp, settings: &Settings) -> Option<Job> {
    let doc = &app.session.doc;
    let caret = &app.session.sel.focus;
    let lt = &app.language_tool;
    for story in stories(doc) {
        let paths = doc.para_paths(story);
        let start = if caret.story == story { paths.iter().position(|p| *p == caret.path).unwrap_or(0) } else { 0 };
        let n = paths.len();
        for k in 0..n {
            let Some(path) = paths.get((start + k) % n) else { continue };
            let Some(p) = doc.para(story, path) else { continue };
            if p.text.trim().is_empty() || p.text.len() > request::MAX_TEXT_BYTES {
                continue;
            }
            let text = wordcraft_engine::layout::para::proof_text(p);
            let plain = CharProps::default();
            let first = p.run_ranges().next().map(|(_, c)| c).unwrap_or(&plain);
            let lang = doc.styles.resolve_char(p.props.style.as_deref(), first).lang;
            let language = settings.language_for(lang.as_deref());
            // A result in another language (the paragraph's language changed since) is asked for
            // again (#575 review).
            let key = (language, text);
            if wordcraft_proof::external::has(&key.1, &key.0) || lt.skipped.contains(&key) {
                continue;
            }
            let (language, text) = key;
            return Some(Job { text, language });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use wordcraft_engine::Session;
    use wordcraft_engine::doc::Document;

    use crate::{Services, WordApp};

    /// The results store is process-wide: these tests take turns.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: Mutex<()> = Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn app_with(text: &str) -> WordApp {
        let mut s = Session::new(Document::new());
        s.run("text.insert", &json!({"text": text})).unwrap();
        WordApp::new(s, Services::default())
    }

    /// Frames until `done` holds (results arrive from a thread), advancing the clock past the
    /// debounce.
    fn run_until(app: &mut WordApp, ctx: &egui::Context, t: &mut f64, done: impl Fn(&WordApp) -> bool) {
        for _ in 0..400 {
            *t += 0.25;
            super::poll_at(app, ctx, *t);
            if done(app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("timed out: {}", super::status(app));
    }

    /// #575: off by default; on, a paragraph is sent after the debounce, the answer's issues
    /// (UTF-16 offsets past an emoji) become the paragraph's squiggles and its context-menu
    /// suggestions, and an edited paragraph is checked again.
    #[test]
    fn languagetool_results_become_squiggles_and_suggestions() {
        let _serial = serial();
        let text = "\u{1F600} Thiss is fine 575.";
        let mut app = app_with(text);
        assert!(!app.ui.language_tool.enabled, "opt-in");
        let sent = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let log = sent.clone();
        app.language_tool.fake = Some(Arc::new(move |r: &wordcraft_languagetool::Request| {
            let f = wordcraft_languagetool::request::fields(&r.body);
            let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone()).unwrap_or_default();
            log.lock().unwrap().push((get("text"), get("language")));
            // "Thiss" starts at UTF-16 offset 3 (the emoji is two units, then a space).
            Ok(json!({"matches": [{"offset": 3, "length": 5, "message": "Possible typo", "replacements": [{"value": "This"}],
                "rule": {"issueType": "misspelling", "category": {"id": "TYPOS"}}}]})
            .to_string())
        }));
        let st = app.run("ui.languageTool", json!({"enabled": true, "language": "document", "apiKey": "SECRET", "username": "u"})).unwrap();
        assert_eq!(st["enabled"], true);
        assert_eq!(st["apiKeySet"], true);
        assert!(!st.to_string().contains("SECRET"), "{st}");
        let ctx = egui::Context::default();
        let mut t = 0.0;
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.checked >= 1);
        assert_eq!(sent.lock().unwrap().first().cloned(), Some((text.to_string(), "en-US".to_string())), "document language from the default style");
        // The context menu's issue under "Thiss" is LanguageTool's, with its suggestion.
        let at = text.find("Thiss").unwrap() + 2;
        let caret = app.session.sel.focus.clone();
        app.session.sel = wordcraft_engine::Selection::caret(wordcraft_engine::doc::Pos { off: at, ..caret });
        let issue = app.run("review.suggestions", json!({})).unwrap();
        assert_eq!(issue["text"], "Thiss");
        assert_eq!(issue["kind"], "spelling");
        assert_eq!(issue["suggestions"], json!(["This"]));
        assert_eq!(issue["message"], "Possible typo");
        let l = app.session.layout();
        let squiggles: Vec<(usize, usize, bool)> = l
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .filter_map(|it| match it {
                wordcraft_engine::layout::Placed::Lines { para, .. } => Some(para.issues.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(squiggles.contains(&(text.find("Thiss").unwrap(), text.find("Thiss").unwrap() + 5, false)), "{squiggles:?}");
        // An edit: the new text is sent after the debounce.
        app.run("text.insert", json!({"text": "!"})).unwrap();
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.checked >= 2);
        assert_eq!(sent.lock().unwrap().len(), 2);
        // Off: results are dropped and nothing more is sent.
        app.run("ui.languageTool", json!({"enabled": false})).unwrap();
        super::poll_at(&mut app, &ctx, t + 5.0);
        assert!(wordcraft_proof::external::is_empty());
    }

    /// #575: failures pause or stop checking with a message instead of retrying in a loop.
    #[test]
    fn languagetool_failures_pause_or_stop_checking() {
        let _serial = serial();
        let mut app = app_with("Rate limited paragraph 575.");
        let calls = Arc::new(Mutex::new(0));
        let n = calls.clone();
        app.language_tool.fake = Some(Arc::new(move |_r: &wordcraft_languagetool::Request| {
            *n.lock().unwrap() += 1;
            Err(wordcraft_languagetool::LtError::RateLimited)
        }));
        app.run("ui.languageTool", json!({"enabled": true})).unwrap();
        let ctx = egui::Context::default();
        let mut t = 0.0;
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.last_error.is_some());
        // Paused: more frames inside the minute send nothing.
        for _ in 0..20 {
            t += 1.0;
            super::poll_at(&mut app, &ctx, t);
        }
        assert_eq!(*calls.lock().unwrap(), 1);
        assert!(super::status(&app)["error"].as_str().unwrap().contains("limiting"));
        // A rejected key stops until the settings change.
        let mut app = app_with("Bad key paragraph 575.");
        app.language_tool.fake = Some(Arc::new(|_r: &wordcraft_languagetool::Request| {
            Err(wordcraft_languagetool::LtError::Http { status: 403, message: "bad key".into() })
        }));
        app.run("ui.languageTool", json!({"enabled": true})).unwrap();
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.stopped.is_some());
    }

    /// #762: with the body done, headers, footers, footnotes and endnotes are sent too; comments
    /// are not.
    #[test]
    fn languagetool_checks_headers_footers_and_notes_after_the_body() {
        use wordcraft_engine::doc::{Paragraph, PartKind, para_block, props::CharProps};
        let _serial = serial();
        let mut app = app_with("Body paragraph 762.");
        let texts = [
            (PartKind::Header, "Header words 762."),
            (PartKind::Footer, "Footer words 762."),
            (PartKind::Footnote, "Footnote words 762."),
            (PartKind::Endnote, "Endnote words 762."),
            (PartKind::Comment, "Comment words 762."),
        ];
        for (kind, text) in texts {
            let p = Paragraph::with_text(text, CharProps::default());
            app.session.doc.add_part(kind, vec![para_block(p)]);
        }
        let sent = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = sent.clone();
        app.language_tool.fake = Some(Arc::new(move |r: &wordcraft_languagetool::Request| {
            let f = wordcraft_languagetool::request::fields(&r.body);
            log.lock().unwrap().push(f.iter().find(|(a, _)| a == "text").map(|(_, b)| b.clone()).unwrap_or_default());
            Ok(json!({"matches": []}).to_string())
        }));
        app.run("ui.languageTool", json!({"enabled": true})).unwrap();
        let ctx = egui::Context::default();
        let mut t = 0.0;
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.checked >= 5);
        for _ in 0..20 {
            t += 1.0;
            super::poll_at(&mut app, &ctx, t);
        }
        let sent = sent.lock().unwrap().clone();
        assert_eq!(sent.first().map(String::as_str), Some("Body paragraph 762."), "body first: {sent:?}");
        assert_eq!(sent.len(), 5, "{sent:?}");
        for (_, text) in &texts[..4] {
            assert!(sent.iter().any(|s| s == text), "{text} not sent: {sent:?}");
        }
        assert!(!sent.iter().any(|s| s.starts_with("Comment")), "{sent:?}");
    }

    #[test]
    fn languagetool_command_validates_and_persists() {
        let _serial = serial();
        let mut app = app_with("x");
        assert!(app.run("ui.languageTool", json!({"server": "ftp://nope"})).is_err());
        assert!(app.run("ui.languageTool", json!({"language": "en US"})).is_err());
        assert!(app.run("ui.languageTool", json!({"enabled": "yes"})).is_err());
        assert_eq!(app.ui.language_tool, wordcraft_languagetool::Settings::default(), "a rejected call changes nothing");
        let st = app.run("ui.languageTool", json!({"server": "http://localhost:8081/", "language": "de-DE"})).unwrap();
        assert_eq!(st["server"], "http://localhost:8081/");
        let saved = serde_json::to_string(&app.prefs()).unwrap();
        let mut b = app_with("y");
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.ui.language_tool.language, "de-DE");
        // `ui.inspect`-style dumps never carry the key.
        app.run("ui.languageTool", json!({"apiKey": "SECRET"})).unwrap();
        assert!(!format!("{:?}", app.prefs()).contains("SECRET"));
        // A new server address drops the key typed for the old one; a key sent along is kept.
        app.run("ui.languageTool", json!({"server": "http://localhost:9090"})).unwrap();
        assert_eq!(app.run("ui.languageTool", json!({})).unwrap()["apiKeySet"], false);
        app.run("ui.languageTool", json!({"server": "http://localhost:9091", "apiKey": "NEW"})).unwrap();
        assert_eq!(app.run("ui.languageTool", json!({})).unwrap()["apiKeySet"], true);
        // The form resends the old key along with a new address: it must not travel to the new server.
        app.run("ui.languageTool", json!({"server": "http://localhost:9092", "apiKey": "NEW"})).unwrap();
        assert_eq!(app.run("ui.languageTool", json!({})).unwrap()["apiKeySet"], false);
    }
}
