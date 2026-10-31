//! Harper's English grammar checks (#799), added to the built-in rules.
//!
//! [Harper](https://writewithharper.com) (`harper-core`, Apache-2.0) checks on this computer and
//! sends nothing anywhere. It takes about 25 µs a character in a release build, too slow to run
//! while pages are laid out, so one background thread checks paragraphs and keeps each result by
//! the exact text it was made for. A paragraph shows the built-in marks until its result arrives;
//! then its [`stamp`] changes and layout lays out that paragraph again. An edit changes the text,
//! so a result never lands on the wrong words.
//!
//! Harper's spell check and its copies of the built-in rules are off ([`OFF`]): WordCraft's own
//! dictionaries decide spelling, with the user dictionary, and nothing is marked twice.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use harper_core::linting::{Lint, LintGroup, LintKind, Linter, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};

use crate::{Issue, IssueKind, ProofLang};

/// Whether this build checks grammar with Harper (the web build doesn't).
pub const BUILT: bool = true;

/// Harper rules left off: its spell check and the rules the built-in English checks already have.
pub const OFF: [&str; 6] = ["SpellCheck", "SentenceCapitalization", "RepeatedWords", "AnA", "Spaces", "NoFrenchSpaces"];

/// Longest paragraph Harper checks, in bytes: about half a second of the background thread's time.
pub const MAX_TEXT_BYTES: usize = 20_000;
/// Most paragraph texts with results kept; past it the half used longest ago is forgotten.
pub const MAX_RESULTS: usize = 20_000;
/// Most text waiting to be checked, in bytes; past it the oldest requests are dropped.
pub const MAX_WAITING_BYTES: usize = 32 << 20;
/// Most issues kept for one paragraph.
const MAX_ISSUES: usize = 500;
/// Longest [`wait`].
const MAX_WAIT: Duration = Duration::from_secs(3600);

/// The results for one paragraph text.
struct Entry {
    /// Rises whenever a result for this text is stored ([`stamp`]).
    stamp: u64,
    /// When layout last asked about this text: results unused longest go first.
    used: u64,
    found: Vec<(ProofLang, Vec<Issue>)>,
}

/// A paragraph waiting to be checked.
struct Job {
    text: String,
    lang: ProofLang,
    /// The layout pass that asked ([`begin_pass`]).
    pass: u64,
}

#[derive(Default)]
struct State {
    results: HashMap<String, Entry>,
    /// Oldest first.
    waiting: VecDeque<Job>,
    /// The languages each text in `waiting` is waiting in.
    queued: HashMap<String, Vec<ProofLang>>,
    waiting_bytes: usize,
    /// The background thread is checking a paragraph.
    checking: bool,
    /// The background thread was started, or couldn't be (`failed`).
    started: bool,
    failed: bool,
    pass: u64,
    tick: u64,
}

impl State {
    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    /// The stored issues for `text` checked as `lang`.
    fn found(&mut self, text: &str, lang: ProofLang) -> Option<Vec<Issue>> {
        let tick = self.next_tick();
        let e = self.results.get_mut(text)?;
        e.used = tick;
        e.found.iter().find(|(l, _)| *l == lang).map(|(_, v)| v.clone())
    }

    fn stamp(&mut self, text: &str) -> u64 {
        let tick = self.next_tick();
        self.results.get_mut(text).map_or(0, |e| {
            e.used = tick;
            e.stamp
        })
    }

    /// Queue `text` to be checked as `lang`, unless it's waiting already.
    fn request(&mut self, text: &str, lang: ProofLang) {
        if self.queued.get(text).is_some_and(|ls| ls.contains(&lang)) {
            return;
        }
        self.queued.entry(text.to_string()).or_default().push(lang);
        self.waiting_bytes = self.waiting_bytes.saturating_add(text.len());
        self.waiting.push_back(Job { text: text.to_string(), lang, pass: self.pass });
        while self.waiting_bytes > MAX_WAITING_BYTES {
            let Some(old) = self.waiting.pop_front() else { break };
            self.unqueue(&old);
        }
    }

    /// The next paragraph to check: the first one the newest layout pass asked for, so an edited
    /// paragraph goes before the rest of a long document, which goes in order.
    fn take(&mut self) -> Option<Job> {
        let newest = self.waiting.back()?.pass;
        let first = self.waiting.iter().rposition(|j| j.pass != newest).map_or(0, |i| i + 1);
        let job = self.waiting.remove(first)?;
        self.unqueue(&job);
        Some(job)
    }

    fn unqueue(&mut self, job: &Job) {
        self.waiting_bytes = self.waiting_bytes.saturating_sub(job.text.len());
        if let Some(ls) = self.queued.get_mut(&job.text) {
            ls.retain(|l| *l != job.lang);
            if ls.is_empty() {
                self.queued.remove(&job.text);
            }
        }
    }

    /// Keep `found` as the issues in `text` checked as `lang`.
    fn put(&mut self, text: String, lang: ProofLang, found: Vec<Issue>) {
        if self.results.len() >= MAX_RESULTS && !self.results.contains_key(&text) {
            self.forget_older_half();
        }
        let tick = self.next_tick();
        let e = self.results.entry(text).or_insert(Entry { stamp: 0, used: tick, found: Vec::new() });
        e.stamp = tick;
        e.used = tick;
        e.found.retain(|(l, _)| *l != lang);
        e.found.push((lang, found));
    }

    fn forget_older_half(&mut self) {
        let mut used: Vec<u64> = self.results.values().map(|e| e.used).collect();
        used.sort_unstable();
        if let Some(&cut) = used.get(used.len() / 2) {
            self.results.retain(|_, e| e.used > cut);
        }
    }
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the background thread when a paragraph is waiting.
    work: Condvar,
    /// Wakes [`wait`] when nothing is left to check.
    idle: Condvar,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| Shared { state: Mutex::new(State::default()), work: Condvar::new(), idle: Condvar::new() })
}

fn lock() -> MutexGuard<'static, State> {
    shared().state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Rises whenever a result is stored ([`generation`]).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Harper's issues in `text` checked as `lang`, when they're ready. Otherwise the paragraph is
/// queued for the background thread and this returns `None`; [`stamp`] changes when it's done.
/// A language Harper doesn't check, or a paragraph over [`MAX_TEXT_BYTES`], has none.
pub(crate) fn issues(text: &str, lang: ProofLang) -> Option<Vec<Issue>> {
    if dialect(lang).is_none() || text.len() > MAX_TEXT_BYTES || text.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut st = lock();
    if let Some(found) = st.found(text, lang) {
        return Some(found);
    }
    if !st.started {
        st.started = true;
        st.failed = std::thread::Builder::new().name("harper".into()).spawn(work).is_err();
        if st.failed {
            log::warn!("Harper's grammar thread couldn't start; the built-in grammar checks stay");
        }
    }
    if st.failed {
        return Some(Vec::new());
    }
    st.request(text, lang);
    drop(st);
    shared().work.notify_one();
    None
}

/// Which result `text` shows (0: none yet). It changes when a result for the text arrives, so
/// layout keys the paragraph's cache on it.
pub fn stamp(text: &str) -> u64 {
    lock().stamp(text)
}

/// Rises whenever a result arrives, so a front end knows to lay out and draw again.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// How many paragraphs are waiting or being checked.
pub fn waiting() -> usize {
    let st = lock();
    st.waiting.len() + usize::from(st.checking)
}

/// Start a layout pass: the paragraphs it asks for are checked before those asked for earlier.
pub fn begin_pass() {
    let mut st = lock();
    st.pass = st.pass.saturating_add(1);
}

/// Wait until nothing is waiting or being checked, at most `timeout` (an hour at most). Returns
/// whether everything was checked.
pub fn wait(timeout: Duration) -> bool {
    let start = Instant::now();
    let timeout = timeout.min(MAX_WAIT);
    let mut st = lock();
    while st.checking || !st.waiting.is_empty() {
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            return false;
        }
        st = match shared().idle.wait_timeout(st, left) {
            Ok((g, _)) => g,
            Err(e) => e.into_inner().0,
        };
    }
    true
}

/// The background thread: checks waiting paragraphs for as long as the app runs.
fn work() {
    let mut linters: HashMap<ProofLang, LintGroup> = HashMap::new();
    loop {
        let job = {
            let mut st = lock();
            loop {
                if let Some(job) = st.take() {
                    st.checking = true;
                    break job;
                }
                st.checking = false;
                shared().idle.notify_all();
                st = shared().work.wait(st).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let found = check(&mut linters, &job.text, job.lang);
        lock().put(job.text, job.lang, found);
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Harper's issues in `text`. If Harper fails on it the paragraph gets none, and the linter for
/// `lang` is made again for the next one.
fn check(linters: &mut HashMap<ProofLang, LintGroup>, text: &str, lang: ProofLang) -> Vec<Issue> {
    let Some(dialect) = dialect(lang) else { return Vec::new() };
    let run = catch_unwind(AssertUnwindSafe(|| {
        let linter = linters.entry(lang).or_insert_with(|| linter(dialect));
        to_issues(text, linter.lint(&Document::new_curated(text, &PlainEnglish)), lang)
    }));
    run.unwrap_or_else(|_| {
        log::warn!("Harper failed on a paragraph; it keeps the built-in grammar checks");
        linters.remove(&lang);
        Vec::new()
    })
}

/// Harper's curated rules for `dialect`, less [`OFF`].
fn linter(dialect: Dialect) -> LintGroup {
    let mut group = LintGroup::new_curated(FstDictionary::curated(), dialect);
    for rule in OFF {
        group.config.set_rule_enabled(rule, false);
    }
    group
}

fn dialect(lang: ProofLang) -> Option<Dialect> {
    match lang {
        ProofLang::En => Some(Dialect::American),
        ProofLang::EnGb => Some(Dialect::British),
        ProofLang::EnCa => Some(Dialect::Canadian),
        _ => None,
    }
}

/// Harper's lints as issues in `text`'s bytes: errors only (spelling and style suggestions such
/// as the Oxford comma left out), none touching an inline object or tracked deletion (U+0001),
/// and of overlapping lints the most important.
fn to_issues(text: &str, mut lints: Vec<Lint>, lang: ProofLang) -> Vec<Issue> {
    let at: Vec<usize> = text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).collect();
    lints.retain(|l| !matches!(l.lint_kind, LintKind::Spelling | LintKind::Style | LintKind::Enhancement | LintKind::Readability));
    lints.sort_by_key(|l| (l.priority, l.span.start));
    let mut out: Vec<Issue> = Vec::new();
    for l in lints {
        let (Some(&start), Some(&end)) = (at.get(l.span.start), at.get(l.span.end)) else { continue };
        let Some(found) = text.get(start..end).filter(|f| !f.is_empty() && !f.contains('\u{1}')) else { continue };
        if out.iter().any(|o| start < o.end && o.start < end) {
            continue;
        }
        let mut suggestions: Vec<String> = Vec::new();
        for s in l.suggestions.iter().filter_map(|s| suggestion(found, s)) {
            if !suggestions.contains(&s) {
                suggestions.push(s);
            }
        }
        out.push(Issue { start, end, kind: IssueKind::Grammar, message: plain(&l.message), suggestions, lang });
        if out.len() >= MAX_ISSUES {
            break;
        }
    }
    out.sort_by_key(|i| (i.start, i.end));
    out
}

/// A suggestion as the text that replaces `found`.
fn suggestion(found: &str, s: &Suggestion) -> Option<String> {
    match s {
        Suggestion::ReplaceWith(c) => Some(c.iter().collect()),
        Suggestion::InsertAfter(c) => Some(found.chars().chain(c.iter().copied()).collect()),
        // The menu shows a suggestion as the text it puts in; a removal has none.
        Suggestion::Remove => None,
    }
}

/// Harper's Markdown message as plain text: code spans quoted, as the built-in messages quote words.
fn plain(message: &str) -> String {
    message.replace('`', "\"")
}

#[cfg(test)]
mod tests;
