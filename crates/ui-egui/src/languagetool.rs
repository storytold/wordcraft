//! LanguageTool checking in the app (#575): File › Options › Proofing and `ui.languageTool`.
//!
//! Off by default (text leaves the machine). When on, [`poll`] runs every frame: once the
//! document has been still for [`DEBOUNCE_S`], it sends the next body paragraph without a result
//! (starting at the caret) to a [`Worker`] thread, one paragraph at a time. What is sent is the
//! paragraph as it reads ([`SentText`]): tracked deletions, hidden text and text marked "Do not
//! check spelling or grammar" are left out, and inline objects go as spaces. Results land in
//! [`wordcraft_proof::external`], keyed by the paragraph's exact text and the language, so a
//! paragraph edited meanwhile keeps the built-in squiggles until its new text is checked; layout
//! and the Review commands (context menu suggestions, Editor, F7) read them from there. Typing
//! never waits.
//!
//! The Server, Username and API key fields in Options are drafts until committed (Enter or
//! leaving the field, #1283): a half-typed address never receives text.
//!
//! Failures pause checking and show in Options (translated, [`describe`]): no connection or a
//! timeout for [`RETRY_S`], "too many requests" for a minute; a paragraph the server won't take
//! (other 4xx answers, answers that aren't LanguageTool's JSON) is skipped; a rejected API key, a
//! bad server address or credentials that would go out unencrypted stop checking until the
//! settings change. The browser build can't check and says why.

use std::collections::HashSet;
use std::ops::Range;

use serde_json::{Value, json};
use wordcraft_engine::doc::{Document, Paragraph, StoryRef, para::OBJ, props::CharProps};
use wordcraft_languagetool::{DEFAULT_SERVER, Job, LtError, Settings, Worker, http, request};
use wordcraft_proof::Issue;

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
    /// The paragraph being checked.
    in_flight: Option<InFlight>,
    /// Debounce: the document revision last seen and when it changed.
    seen_rev: u64,
    changed_at: f64,
    /// The revision at which nothing was left to check (no scan until something changes).
    idle_rev: Option<u64>,
    /// No requests before this time.
    paused_until: f64,
    /// Checking stopped until the settings change (rejected key, bad address).
    stopped: Option<LtError>,
    /// (language, paragraph text) not to send again.
    skipped: HashSet<(String, String)>,
    /// Paragraphs checked since the settings last changed.
    pub checked: u64,
    /// The last failure, shown in Options.
    pub last_error: Option<LtError>,
    /// The Options fields being edited, not applied yet.
    pub drafts: Drafts,
    /// Test hook: a fake worker transport instead of HTTP.
    #[cfg(test)]
    pub(crate) fake: Option<std::sync::Arc<dyn Fn(&wordcraft_languagetool::Request) -> Result<String, LtError> + Send + Sync>>,
}

/// A paragraph sent to the worker: what went out and how its answer maps back.
struct InFlight {
    /// The paragraph's proofing text: the key its result is stored under.
    key: String,
    language: String,
    sent: SentText,
}

/// The text sent for a paragraph, with the way back to the paragraph's byte offsets.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct SentText {
    pub text: String,
    /// For each byte of `text`, the paragraph bytes of the character it came from.
    from: Vec<(usize, usize)>,
    /// Paragraph bytes left out (tracked deletions, hidden, "Do not check").
    removed: Vec<Range<usize>>,
}

impl SentText {
    /// What goes to the server for `p`: the paragraph as it reads. Tracked deletions, hidden
    /// text and text marked "Do not check spelling or grammar" are left out; inline objects go
    /// as one space each (never U+0001, which glues the words around it together).
    pub(crate) fn of(doc: &Document, p: &Paragraph) -> SentText {
        let mut out = SentText { text: String::with_capacity(p.text.len()), ..SentText::default() };
        for (range, c) in p.run_ranges() {
            let Some(run) = p.text.get(range.clone()) else { continue };
            let rc = doc.styles.resolve_char(p.props.style.as_deref(), c);
            if c.del.is_some() || rc.hidden || rc.no_proof {
                match out.removed.last_mut() {
                    Some(last) if last.end == range.start => last.end = range.end,
                    _ => out.removed.push(range),
                }
                continue;
            }
            for (i, ch) in run.char_indices() {
                let at = range.start.saturating_add(i);
                let src = (at, at.saturating_add(ch.len_utf8()));
                let ch = if ch == OBJ { ' ' } else { ch };
                out.text.push(ch);
                out.from.extend(std::iter::repeat_n(src, ch.len_utf8()));
            }
        }
        out
    }

    /// `issues` found in [`SentText::text`], in the paragraph's byte offsets. An issue that
    /// spans text left out (say, a doubled space where a deletion was) is dropped.
    pub(crate) fn map_back(&self, issues: Vec<Issue>) -> Vec<Issue> {
        issues
            .into_iter()
            .filter_map(|i| {
                let start = self.from.get(i.start)?.0;
                let end = self.from.get(i.end.checked_sub(1)?)?.1;
                let gap = self.removed.iter().any(|r| r.start < end && start < r.end);
                (start < end && !gap).then_some(Issue { start, end, ..i })
            })
            .collect()
    }
}

/// The Options fields (Server, Username, API key) as typed, before they are committed.
#[derive(Default)]
pub struct Drafts {
    pub server: Option<String>,
    pub username: Option<String>,
    pub api_key: Option<String>,
    /// Why the last commit was refused (translated).
    pub error: Option<String>,
}

/// A text field of Options › Proofing › LanguageTool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Server,
    Username,
    ApiKey,
}

impl Field {
    /// The `ui.languageTool` parameter it sets.
    fn param(self) -> &'static str {
        match self {
            Field::Server => "server",
            Field::Username => "username",
            Field::ApiKey => "apiKey",
        }
    }

    /// The text edit's id (tests focus it).
    pub fn id(self) -> egui::Id {
        egui::Id::new(("languagetool_field", self.param()))
    }

    fn draft(self, d: &mut Drafts) -> &mut Option<String> {
        match self {
            Field::Server => &mut d.server,
            Field::Username => &mut d.username,
            Field::ApiKey => &mut d.api_key,
        }
    }

    fn value(self, s: &Settings) -> &str {
        match self {
            Field::Server => &s.server,
            Field::Username => &s.username,
            Field::ApiKey => &s.api_key,
        }
    }
}

/// One of the LanguageTool text fields. Typing edits a draft; the draft takes effect through
/// `ui.languageTool` only when committed (Enter or leaving the field), so text is never sent to
/// a half-typed server address or with a half-typed key (#1283).
pub fn text_field(app: &mut WordApp, ui: &mut egui::Ui, field: Field, width: f32) {
    let saved = field.value(&app.ui.language_tool).to_string();
    let mut buf = field.draft(&mut app.language_tool.drafts).clone().unwrap_or(saved);
    let mut edit = egui::TextEdit::singleline(&mut buf).id(field.id()).desired_width(width);
    match field {
        Field::Server => edit = edit.hint_text(DEFAULT_SERVER),
        Field::ApiKey => edit = edit.password(true),
        Field::Username => {}
    }
    let r = ui.add(edit);
    if r.changed() {
        *field.draft(&mut app.language_tool.drafts) = Some(buf);
    }
    if r.lost_focus() {
        commit(app, field);
    }
}

/// Apply `field`'s draft. A refused value stays in the field with the reason under it.
fn commit(app: &mut WordApp, field: Field) {
    let Some(value) = field.draft(&mut app.language_tool.drafts).take() else { return };
    app.language_tool.drafts.error = None;
    if value.trim() == field.value(&app.ui.language_tool).trim() {
        return;
    }
    // The same checks `ui.languageTool` makes, first, for a translated message.
    let mut next = app.ui.language_tool.clone();
    match field {
        Field::Server => next.server = if value.trim().is_empty() { DEFAULT_SERVER.into() } else { value.trim().to_string() },
        Field::Username => next.username = value.trim().to_string(),
        Field::ApiKey => next.api_key = value.trim().to_string(),
    }
    let refused = match next.endpoint().and_then(|_| next.account_check()) {
        Err(e) => Some(describe(&e)),
        Ok(()) => app.run("ui.languageTool", json!({ field.param(): value })).err(),
    };
    if let Some(why) = refused {
        app.language_tool.drafts.error = Some(why);
        *field.draft(&mut app.language_tool.drafts) = Some(value);
    }
}

/// `e` for the Options page, in the interface language.
pub fn describe(e: &LtError) -> String {
    use crate::i18n::fmt;
    match e {
        LtError::BadServer(a) => fmt(tl!("The LanguageTool server address isn't a web address: {address}"), &[("address", a)]),
        LtError::TooLong { bytes, max } => {
            fmt(tl!("A paragraph is too long to check ({bytes} bytes; at most {max})."), &[("bytes", &bytes.to_string()), ("max", &max.to_string())])
        }
        LtError::Network(why) => fmt(tl!("Couldn't reach the LanguageTool server: {reason}"), &[("reason", why)]),
        LtError::Timeout => tl!("The LanguageTool server didn't answer in time.").to_string(),
        LtError::RateLimited => tl!("The LanguageTool server is limiting requests; checking resumes in a minute.").to_string(),
        LtError::Http { status, message } => {
            fmt(tl!("The LanguageTool server answered {status}: {message}"), &[("status", &status.to_string()), ("message", message)])
        }
        LtError::Protocol(why) => fmt(tl!("Unexpected answer from the LanguageTool server: {reason}"), &[("reason", why)]),
        LtError::InsecureAccount => {
            tl!("The username and API key are only sent over https:// or to this computer. Use an https:// server address.").to_string()
        }
        LtError::Unsupported(why) => tl!(why).to_string(),
    }
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
        self.checked = 0;
        self.last_error = None;
    }
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
        s.server = if server.is_empty() { wordcraft_languagetool::DEFAULT_SERVER.into() } else { server };
        s.endpoint().map_err(|e| e.to_string())?;
    }
    if let Some(u) = text("username")? {
        s.username = u;
    }
    if let Some(k) = text("apiKey")? {
        s.api_key = k;
    }
    if let Some(l) = text("language")? {
        if !(l == "auto" || l == "document" || wordcraft_languagetool::is_language_code(&l)) {
            return Err(format!("unknown language `{l}`; use auto, document or a code such as en-US"));
        }
        s.language = l;
    }
    if s.enabled && !http::available() {
        return Err(http::UNSUPPORTED_REASON.into());
    }
    // Credentials never go over plain http:// to another computer (#1283).
    if ["server", "username", "apiKey"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
        s.account_check().map_err(|e| e.to_string())?;
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
        "error": lt.stopped.as_ref().or(lt.last_error.as_ref()).map(LtError::to_string),
    })
}

/// A line for File › Options about what checking is doing.
pub fn status_line(app: &WordApp) -> Option<String> {
    let lt = &app.language_tool;
    if !app.ui.language_tool.enabled {
        return None;
    }
    if let Some(e) = lt.stopped.as_ref().or(lt.last_error.as_ref()) {
        return Some(describe(e));
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
    }
    let still = now - lt.changed_at;
    if still < DEBOUNCE_S {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((DEBOUNCE_S - still).clamp(0.05, DEBOUNCE_S)));
        return;
    }
    if lt.idle_rev == Some(rev) {
        return;
    }
    let (next, stored) = next_job(app, &settings);
    if stored {
        app.session.proofing_changed();
        ctx.request_repaint();
    }
    let Some(next) = next else {
        app.language_tool.idle_rev = Some(rev);
        return;
    };
    let lt = &mut app.language_tool;
    if lt.worker.is_none() {
        match spawn(lt, settings, ctx) {
            Ok(w) => lt.worker = Some(w),
            Err(e) => {
                lt.stopped = Some(e);
                return;
            }
        }
    }
    let job = Job { text: next.sent.text.clone(), language: next.language.clone() };
    if lt.worker.as_ref().is_some_and(|w| w.send(job)) {
        lt.in_flight = Some(next);
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
        // One job at a time: the answer is the in-flight paragraph's.
        let Some(job) = lt.in_flight.take_if(|j| j.sent.text == done.text && j.language == done.language) else { continue };
        lt.idle_rev = None;
        match done.result {
            Ok(issues) => {
                wordcraft_proof::external::put(&job.key, &job.language, job.sent.map_back(issues));
                lt.checked = lt.checked.saturating_add(1);
                lt.last_error = None;
                changed = true;
            }
            Err(e) if e.stops_checking() => lt.stopped = Some(e),
            Err(e) if e.skips_paragraph() => {
                if lt.skipped.len() >= MAX_SKIPPED {
                    lt.skipped.clear();
                }
                lt.skipped.insert((job.language, job.key));
                lt.last_error = Some(e);
            }
            Err(e) => {
                lt.paused_until = now + if e == LtError::RateLimited { RATE_LIMIT_S } else { RETRY_S };
                lt.last_error = Some(e);
            }
        }
    }
    if changed {
        app.session.proofing_changed();
        ctx.request_repaint();
    }
}

/// The next body paragraph without a result, from the caret's paragraph on, and whether a
/// result was stored on the way (a paragraph with nothing left to send gets an empty one).
fn next_job(app: &WordApp, settings: &Settings) -> (Option<InFlight>, bool) {
    let doc = &app.session.doc;
    let paths = doc.para_paths(StoryRef::Body);
    let caret = &app.session.sel.focus;
    let start = if caret.story == StoryRef::Body { paths.iter().position(|p| *p == caret.path).unwrap_or(0) } else { 0 };
    let lt = &app.language_tool;
    let n = paths.len();
    let mut stored = false;
    for k in 0..n {
        let Some(path) = paths.get((start + k) % n) else { continue };
        let Some(p) = doc.para(StoryRef::Body, path) else { continue };
        if p.text.trim().is_empty() || p.text.len() > request::MAX_TEXT_BYTES {
            continue;
        }
        let key = wordcraft_engine::layout::para::proof_text(p);
        let plain = CharProps::default();
        let first = p.run_ranges().next().map(|(_, c)| c).unwrap_or(&plain);
        let lang = doc.styles.resolve_char(p.props.style.as_deref(), first).lang;
        let language = settings.language_for(lang.as_deref());
        // A result in another language (the paragraph's language changed since) is asked for
        // again (#575 review).
        if wordcraft_proof::external::has(&key, &language) || lt.skipped.contains(&(language.clone(), key.clone())) {
            continue;
        }
        let sent = SentText::of(doc, p);
        if sent.text.trim().is_empty() {
            // Nothing to check (all of it left out): nothing to send.
            wordcraft_proof::external::put(&key, &language, Vec::new());
            stored = true;
            continue;
        }
        return (Some(InFlight { key, language, sent }), stored);
    }
    (None, stored)
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
        assert!(wordcraft_proof::external::get(text).is_none());
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
    }

    /// A fake server that counts requests and records (url, text, language).
    fn counting(app: &mut WordApp) -> Arc<Mutex<Vec<(String, String, String)>>> {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = sent.clone();
        app.language_tool.fake = Some(Arc::new(move |r: &wordcraft_languagetool::Request| {
            let f = wordcraft_languagetool::request::fields(&r.body);
            let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone()).unwrap_or_default();
            log.lock().unwrap().push((r.url.clone(), get("text"), get("language")));
            Ok(json!({"matches": []}).to_string())
        }));
        sent
    }

    /// One frame of Options › Proofing › LanguageTool with `events`, then a checking poll at `t`.
    fn options_frame(app: &mut WordApp, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
        let input = egui::RawInput { time: Some(t), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| crate::options::language_tool(app, ui));
        out.textures_delta.clear();
        super::poll_at(app, ctx, t);
    }

    /// #1283: typing in the Server field sends nothing (not to the half-typed address, not to
    /// the old one again) until the address is committed with Enter; then the next request goes
    /// to the new address.
    #[test]
    fn languagetool_server_field_applies_only_when_committed() {
        let _serial = serial();
        wordcraft_proof::external::clear();
        let mut app = app_with("Committed server paragraph 1283.");
        let sent = counting(&mut app);
        app.run("ui.languageTool", json!({"enabled": true, "server": "http://localhost:8081"})).unwrap();
        let ctx = egui::Context::default();
        let mut t = 0.0;
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.checked >= 1);
        assert_eq!(sent.lock().unwrap().len(), 1);
        // Focus the field and type, a frame per character, for longer than the debounce.
        ctx.memory_mut(|m| m.request_focus(super::Field::Server.id()));
        options_frame(&mut app, &ctx, t, Vec::new());
        let end = egui::Event::Key { key: egui::Key::End, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        options_frame(&mut app, &ctx, t, vec![end]);
        for ch in "1/x".chars() {
            for _ in 0..4 {
                t += 0.5;
                options_frame(&mut app, &ctx, t, Vec::new());
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            options_frame(&mut app, &ctx, t, vec![egui::Event::Text(ch.to_string())]);
        }
        assert_eq!(app.language_tool.drafts.server.as_deref(), Some("http://localhost:80811/x"));
        assert_eq!(app.ui.language_tool.server, "http://localhost:8081", "a draft isn't applied");
        assert_eq!(sent.lock().unwrap().len(), 1, "nothing sent while typing: {:?}", sent.lock().unwrap());
        // Enter commits it: checking starts over against the new address.
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        options_frame(&mut app, &ctx, t, vec![enter]);
        assert_eq!(app.ui.language_tool.server, "http://localhost:80811/x");
        assert!(app.language_tool.drafts.server.is_none());
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.checked >= 1);
        let log = sent.lock().unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[1].0, "http://localhost:80811/x/v2/check");
    }

    /// #1283: a refused value (credentials over plain http:// to another computer) stays in the
    /// field with a translated reason, and the settings keep the old value.
    #[test]
    fn languagetool_refused_field_keeps_the_draft_and_says_why() {
        let _serial = serial();
        let mut app = app_with("x");
        app.run("ui.languageTool", json!({"username": "me@example.org", "apiKey": "SECRET"})).unwrap();
        app.language_tool.drafts.server = Some("http://lt.example.org".into());
        super::commit(&mut app, super::Field::Server);
        assert_eq!(app.ui.language_tool.server, wordcraft_languagetool::DEFAULT_SERVER);
        assert_eq!(app.language_tool.drafts.server.as_deref(), Some("http://lt.example.org"));
        assert!(app.language_tool.drafts.error.as_deref().is_some_and(|e| e.contains("https://")));
        // The command refuses it too.
        assert!(app.run("ui.languageTool", json!({"server": "http://lt.example.org"})).is_err());
        assert!(app.run("ui.languageTool", json!({"server": "http://localhost:8081"})).is_ok());
    }

    /// #1283: while checking is off, nothing is sent, whatever is typed or changed.
    #[test]
    fn languagetool_sends_nothing_while_off() {
        let _serial = serial();
        let mut app = app_with("Private paragraph 1283.");
        let sent = counting(&mut app);
        let ctx = egui::Context::default();
        let mut t = 0.0;
        app.run("ui.languageTool", json!({"server": "http://localhost:8081", "language": "document"})).unwrap();
        ctx.memory_mut(|m| m.request_focus(super::Field::Server.id()));
        for k in 0..40 {
            t += 0.5;
            let events = if k % 4 == 0 { vec![egui::Event::Text("9".into())] } else { Vec::new() };
            options_frame(&mut app, &ctx, t, events);
            if k == 20 {
                app.run("text.insert", json!({"text": " More."})).unwrap();
            }
        }
        assert!(app.language_tool.worker.is_none());
        assert!(sent.lock().unwrap().is_empty());
    }

    /// #1283: two paragraphs with the same text in different languages are each checked once;
    /// their results don't evict each other into a request loop.
    #[test]
    fn languagetool_same_text_in_two_languages_does_not_loop() {
        let _serial = serial();
        wordcraft_proof::external::clear();
        let mut app = app_with("Same words 1283.");
        app.run("text.newParagraph", json!({})).unwrap();
        app.run("text.insert", json!({"text": "Same words 1283."})).unwrap();
        app.run("select.text", json!({"text": "Same words 1283.", "occurrence": 2})).unwrap();
        app.run("review.language", json!({"lang": "fr-FR"})).unwrap();
        let sent = counting(&mut app);
        app.run("ui.languageTool", json!({"enabled": true, "language": "document"})).unwrap();
        let ctx = egui::Context::default();
        let mut t = 0.0;
        run_until(&mut app, &ctx, &mut t, |a| a.language_tool.checked >= 2);
        for _ in 0..40 {
            t += 0.5;
            super::poll_at(&mut app, &ctx, t);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut langs: Vec<String> = sent.lock().unwrap().iter().map(|(_, _, l)| l.clone()).collect();
        langs.sort();
        assert_eq!(langs, ["en-US", "fr-FR"]);
    }

    /// #1283: what is sent is the paragraph as it reads: tracked deletions and text marked "Do
    /// not check" left out, an inline object as a space (not U+0001, which glues words); answers
    /// map back to the paragraph, and an issue across left-out text is dropped.
    #[test]
    fn languagetool_sends_the_paragraph_as_it_reads() {
        let mut s = Session::new(Document::new());
        s.run("text.insert", &json!({"text": "Keep brave new world."})).unwrap();
        s.run("select.text", &json!({"text": "brave "})).unwrap();
        s.run("review.language", &json!({"noProof": true})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.text", &json!({"text": "new "})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        s.run("select.text", &json!({"text": "world."})).unwrap();
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("references.footnote", &json!({})).unwrap();
        let path = s.doc.para_paths(wordcraft_engine::doc::StoryRef::Body).into_iter().next().unwrap();
        let p = s.doc.para(wordcraft_engine::doc::StoryRef::Body, &path).unwrap();
        assert!(p.text.starts_with("Keep brave new world."), "{:?}", p.text);
        let sent = super::SentText::of(&s.doc, p);
        assert_eq!(sent.text, "Keep world. ");
        let issue = |start, end| wordcraft_proof::Issue {
            start,
            end,
            kind: wordcraft_proof::IssueKind::Spelling,
            message: String::new(),
            suggestions: Vec::new(),
            lang: wordcraft_proof::ProofLang::En,
        };
        let back = sent.map_back(vec![issue(5, 10), issue(4, 6), issue(0, 4), issue(11, 12), issue(11, 40)]);
        let world = p.text.find("world").unwrap();
        let obj = p.text.find(wordcraft_engine::doc::para::OBJ).unwrap();
        assert_eq!(
            back.iter().map(|i| (i.start, i.end)).collect::<Vec<_>>(),
            [(world, world + 5), (0, 4), (obj, obj + wordcraft_engine::doc::para::OBJ.len_utf8())]
        );
    }
}
