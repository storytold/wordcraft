//! Read Aloud: the document as a queue of sentences that the system's speech program speaks one
//! at a time, so reading can be paused, skipped forwards and backwards, and sped up.
//!
//! Citation-manager fields can be left out: Zotero, Mendeley and EndNote citations and
//! bibliographies (range fields with an `ADDIN …` code) and Word's own `CITATION` and
//! `BIBLIOGRAPHY` fields. The citation's text simply isn't in the queue, so every citation
//! style is skipped the same way: "(Smith et al., 2020)", "[3]" or a superscript number.
//!
//! The speech itself runs on a worker thread (desktop only): it starts `say` (macOS), `espeak`
//! / `spd-say` (Linux, BSD) or the Windows speech synthesizer (PowerShell) for the current
//! sentence, and moves on when it finishes. Commands only change the shared state and wake the
//! worker, which stops the running sentence when the state changed under it.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use wordcraft_doc::para::{COLUMN_BREAK, LINE_BREAK, OBJ, PAGE_BREAK};
use wordcraft_doc::{Document, InlineObject, Pos};

/// Longest sentence handed to the speech program (longer ones are split at a space).
const MAX_SENTENCE: usize = 600;
/// Most sentences in one queue.
const MAX_SENTENCES: usize = 20_000;
/// Speed limits (1.0 = the normal speed).
pub const MIN_RATE: f32 = 0.5;
pub const MAX_RATE: f32 = 3.0;
/// Words per minute at speed 1.0 (`say` and `espeak` both default to about this).
#[cfg(not(target_arch = "wasm32"))]
const BASE_WPM: f32 = 180.0;

/// Words that end with a full stop without ending the sentence.
const ABBREVIATIONS: &[&str] = &[
    "al", "e.g", "i.e", "eg", "ie", "cf", "vs", "etc", "fig", "figs", "eq", "eqs", "no", "nos", "p", "pp", "vol", "vols", "ch", "chap", "sec", "tab",
    "ref", "refs", "dr", "mr", "mrs", "ms", "prof", "st", "approx", "ca", "resp", "ed", "eds", "suppl", "jr", "sr", "inc", "ltd", "co", "dept",
    "est", "min", "max", "viz", "ibid",
];

/// Is this field code a citation or a bibliography (Zotero, Mendeley, EndNote, Word)?
pub fn is_citation_code(instr: &str) -> bool {
    let code = instr.trim_start();
    let word = code.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    if word == "CITATION" || word == "BIBLIOGRAPHY" {
        return true;
    }
    if word != "ADDIN" {
        return false;
    }
    let rest = code.get(5..).unwrap_or("").trim_start().to_ascii_uppercase();
    ["ZOTERO_ITEM", "ZOTERO_BIBL", "CSL_CITATION", "CSL_BIBLIOGRAPHY", "MENDELEY", "EN.CITE", "EN.REFLIST"].iter().any(|k| rest.starts_with(k))
}

/// One sentence to speak: its text and where it is in the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Utterance {
    pub text: String,
    pub start: Pos,
    pub end: Pos,
}

impl Utterance {
    pub fn to_json(&self) -> Value {
        json!({"text": self.text, "start": crate::cmd::pos_json(&self.start), "end": crate::cmd::pos_json(&self.end)})
    }
}

/// The sentences from `from` (to `to`, if given) in `from`'s story, in reading order.
/// With `skip_citations`, citation and bibliography fields are left out.
pub fn sentences(doc: &Document, from: &Pos, to: Option<&Pos>, skip_citations: bool) -> Vec<Utterance> {
    let story = from.story;
    // Spans to leave out: whole range fields, both markers included.
    let skipped: Vec<(Pos, Pos)> = if skip_citations {
        doc.field_ranges(story).into_iter().filter(|r| is_citation_code(&r.instr)).map(|r| (r.start, r.end)).collect()
    } else {
        Vec::new()
    };
    let in_skipped = |p: &Pos| skipped.iter().any(|(a, b)| a <= p && p <= b);
    let mut out = Vec::new();
    for path in doc.para_paths(story) {
        if path < from.path {
            continue;
        }
        if let Some(to) = to
            && path > to.path
        {
            break;
        }
        let Some(para) = doc.para(story, &path) else { continue };
        // The spoken text, and for each char the paragraph offset it came from.
        let mut text = String::new();
        let mut map: Vec<(usize, usize)> = Vec::new(); // (start, end) offsets in the paragraph
        let mut k = 0;
        for (off, c) in para.text.char_indices() {
            let end = off + c.len_utf8();
            let obj = if c == OBJ {
                k += 1;
                para.objects.get(k - 1)
            } else {
                None
            };
            if (path == from.path && off < from.off) || to.is_some_and(|t| path == t.path && off >= t.off) {
                continue;
            }
            let pos = Pos::new(story, path.clone(), off);
            if in_skipped(&pos) {
                continue;
            }
            match obj {
                Some(InlineObject::Field { instr, .. }) if skip_citations && is_citation_code(instr) => {}
                Some(o) => {
                    for ch in o.plain_text().chars() {
                        text.push(ch);
                        map.push((off, end));
                    }
                }
                None if c == OBJ => {}
                None => {
                    let ch = if matches!(c, LINE_BREAK | PAGE_BREAK | COLUMN_BREAK | '\t') { ' ' } else { c };
                    text.push(ch);
                    map.push((off, end));
                }
            }
        }
        for (a, b) in split_sentences(&text) {
            if out.len() >= MAX_SENTENCES {
                return out;
            }
            let chars: Vec<char> = text.chars().skip(a).take(b - a).collect();
            let spoken = tidy(&chars.iter().collect::<String>());
            if !spoken.chars().any(char::is_alphanumeric) {
                continue;
            }
            let (Some(first), Some(last)) = (map.get(a), map.get(b.saturating_sub(1))) else { continue };
            out.push(Utterance { text: spoken, start: Pos::new(story, path.clone(), first.0), end: Pos::new(story, path.clone(), last.1) });
        }
    }
    out
}

/// Sentence spans in `text` as char index ranges, without surrounding whitespace.
fn split_sentences(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        let mut end = None;
        if matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            // Include closing quotes and brackets.
            let mut j = i + 1;
            while chars.get(j).is_some_and(|c| matches!(c, '"' | '\'' | '”' | '’' | ')' | ']' | '»' | '.' | '!' | '?')) {
                j += 1;
            }
            let at_end = j >= chars.len();
            let space_after = chars.get(j).is_some_and(|c| c.is_whitespace());
            if (at_end || space_after) && (c != '.' || !is_abbreviation(&chars, start, i)) {
                end = Some(j);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
        // Very long sentences are split at a space so skipping stays fine-grained.
        if end.is_none() && i - start >= MAX_SENTENCE && chars.get(i).is_some_and(|c| c.is_whitespace()) {
            end = Some(i);
        }
        if let Some(e) = end {
            push_trimmed(&chars, start, e, &mut out);
            start = e;
        }
    }
    push_trimmed(&chars, start, chars.len(), &mut out);
    out
}

fn push_trimmed(chars: &[char], mut a: usize, mut b: usize, out: &mut Vec<(usize, usize)>) {
    while a < b && chars.get(a).is_some_and(|c| c.is_whitespace()) {
        a += 1;
    }
    while b > a && chars.get(b - 1).is_some_and(|c| c.is_whitespace()) {
        b -= 1;
    }
    if a < b {
        out.push((a, b));
    }
}

/// Does the full stop at `dot` end an abbreviation ("et al.", "Fig.", "J. Smith") rather than a sentence?
fn is_abbreviation(chars: &[char], start: usize, dot: usize) -> bool {
    let mut w = dot;
    while w > start && chars.get(w - 1).is_some_and(|c| c.is_alphanumeric() || *c == '.') {
        w -= 1;
    }
    let word: String = chars.get(w..dot).unwrap_or(&[]).iter().collect::<String>().to_lowercase();
    if word.is_empty() {
        return false;
    }
    // An initial: "J. Smith", "A. B. Jones".
    let letters: Vec<char> = word.chars().filter(|c| *c != '.').collect();
    if letters.len() == 1 && letters.first().is_some_and(|c| c.is_alphabetic()) {
        return true;
    }
    ABBREVIATIONS.contains(&word.as_str())
}

/// Collapse whitespace and tidy what removed citations leave behind: "( )", "[]", " ." → ".".
fn tidy(s: &str) -> String {
    let mut t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let before = t.len();
        for (from, to) in [("()", ""), ("[]", ""), ("( )", ""), ("[ ]", ""), (" .", "."), (" ,", ","), (" ;", ";"), (" :", ":"), ("  ", " ")] {
            t = t.replace(from, to);
        }
        if t.len() == before {
            break;
        }
    }
    t.trim().to_string()
}

/// Reading state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Stopped => "stopped",
            State::Playing => "playing",
            State::Paused => "paused",
        }
    }
}

/// How sentences are spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The system's speech program.
    System,
    /// Silent; a sentence lasts until something interrupts it (tests, `WORDCRAFT_SPEECH=off`).
    Hold,
    /// Silent; every sentence finishes at once (tests).
    Instant,
}

struct Shared {
    queue: Vec<Utterance>,
    index: usize,
    state: State,
    rate: f32,
    /// Bumped on every change that should stop the sentence being spoken.
    generation: u64,
    error: Option<String>,
    /// A worker thread is running (cleared under the lock as it exits).
    running: bool,
}

/// The Read Aloud player.
pub struct ReadAloud {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    pub backend: Backend,
    /// Leave citations and bibliographies out of the next reading.
    pub skip_citations: bool,
    /// The player window is open (it stays open after the last sentence until closed).
    pub open: bool,
}

impl Default for ReadAloud {
    fn default() -> Self {
        let off = cfg!(test) || std::env::var("WORDCRAFT_SPEECH").is_ok_and(|v| v == "off");
        ReadAloud {
            shared: Arc::new((
                Mutex::new(Shared { queue: Vec::new(), index: 0, state: State::Stopped, rate: 1.0, generation: 0, error: None, running: false }),
                Condvar::new(),
            )),
            backend: if off { Backend::Hold } else { Backend::System },
            skip_citations: true,
            open: false,
        }
    }
}

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ReadAloud {
    /// Change the shared state, stop the current sentence and wake the worker.
    fn change(&mut self, f: impl FnOnce(&mut Shared)) {
        {
            let (m, cv) = &*self.shared;
            let mut s = lock(m);
            f(&mut s);
            s.generation = s.generation.wrapping_add(1);
            cv.notify_all();
        }
        self.ensure_worker();
    }

    /// Start reading `queue` from its first sentence.
    pub fn start(&mut self, queue: Vec<Utterance>) {
        self.open = true;
        self.change(|s| {
            s.state = if queue.is_empty() { State::Stopped } else { State::Playing };
            s.queue = queue;
            s.index = 0;
            s.error = None;
        });
    }

    /// Pause, or carry on (a finished reading starts again from the top).
    pub fn play_pause(&mut self) {
        self.change(|s| {
            s.state = match s.state {
                State::Playing => State::Paused,
                _ if s.queue.is_empty() => State::Stopped,
                _ => {
                    if s.index >= s.queue.len() {
                        s.index = 0;
                    }
                    State::Playing
                }
            };
        });
    }

    /// Move by `delta` sentences; a stopped player starts playing there.
    pub fn skip(&mut self, delta: i64) {
        self.change(|s| {
            let last = s.queue.len().saturating_sub(1) as i64;
            let at = (s.index.min(s.queue.len()) as i64).saturating_add(delta).clamp(0, last.max(0));
            s.index = at as usize;
            if s.state == State::Stopped && !s.queue.is_empty() {
                s.state = State::Playing;
            }
        });
    }

    /// Set the speed (1.0 = normal); the current sentence restarts at the new speed.
    pub fn set_rate(&mut self, rate: f32) {
        let r = if rate.is_finite() { rate.clamp(MIN_RATE, MAX_RATE) } else { 1.0 };
        let playing = self.state() == State::Playing;
        if playing {
            self.change(|s| s.rate = r);
        } else {
            lock(&self.shared.0).rate = r;
        }
    }

    /// Stop reading and close the player.
    pub fn stop(&mut self) {
        self.open = false;
        self.change(|s| s.state = State::Stopped);
    }

    pub fn state(&self) -> State {
        lock(&self.shared.0).state
    }
    pub fn rate(&self) -> f32 {
        lock(&self.shared.0).rate
    }
    /// The sentence being read (or paused on).
    pub fn current(&self) -> Option<Utterance> {
        let s = lock(&self.shared.0);
        if s.state == State::Stopped { None } else { s.queue.get(s.index).cloned() }
    }
    /// (index, number of sentences).
    pub fn progress(&self) -> (usize, usize) {
        let s = lock(&self.shared.0);
        (s.index, s.queue.len())
    }
    /// The last speech error (no speech program…), if any.
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.0).error.clone()
    }

    pub fn status(&self) -> Value {
        let s = lock(&self.shared.0);
        let current = if s.state == State::Stopped { None } else { s.queue.get(s.index) };
        json!({
            "state": s.state.name(),
            "open": self.open,
            "index": s.index,
            "count": s.queue.len(),
            "rate": s.rate,
            "skipCitations": self.skip_citations,
            "current": current.map(Utterance::to_json),
            "error": s.error,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn ensure_worker(&mut self) {
        let (m, _) = &*self.shared;
        let mut s = lock(m);
        if s.state == State::Playing && self.backend == Backend::System {
            s.state = State::Stopped;
            s.error = Some("Read Aloud isn't available in the browser".into());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ensure_worker(&mut self) {
        let mut s = lock(&self.shared.0);
        if s.state != State::Playing || s.running {
            return;
        }
        s.running = true;
        let shared = Arc::clone(&self.shared);
        let backend = self.backend;
        if let Err(e) = std::thread::Builder::new().name("read-aloud".into()).spawn(move || worker(&shared, backend)) {
            s.running = false;
            s.state = State::Stopped;
            s.error = Some(format!("couldn't start Read Aloud: {e}"));
        }
    }
}

impl Drop for ReadAloud {
    fn drop(&mut self) {
        let (m, cv) = &*self.shared;
        let mut s = lock(m);
        s.state = State::Stopped;
        s.generation = s.generation.wrapping_add(1);
        cv.notify_all();
    }
}

/// How a sentence ended.
#[cfg(not(target_arch = "wasm32"))]
enum Spoken {
    Done,
    Interrupted,
    Failed(String),
}

#[cfg(not(target_arch = "wasm32"))]
fn worker(shared: &(Mutex<Shared>, Condvar), backend: Backend) {
    let (m, cv) = shared;
    loop {
        let (text, generation, rate) = {
            let mut s = lock(m);
            while s.state == State::Paused {
                s = cv.wait(s).unwrap_or_else(PoisonError::into_inner);
            }
            if s.state == State::Stopped {
                s.running = false;
                return;
            }
            match s.queue.get(s.index) {
                Some(u) => (u.text.clone(), s.generation, s.rate),
                None => {
                    s.state = State::Stopped;
                    s.running = false;
                    return;
                }
            }
        };
        let changed = || lock(m).generation != generation;
        let outcome = match backend {
            Backend::System => speak(&text, rate, &changed),
            Backend::Instant => Spoken::Done,
            Backend::Hold => {
                let mut s = lock(m);
                while s.generation == generation {
                    s = cv.wait(s).unwrap_or_else(PoisonError::into_inner);
                }
                Spoken::Interrupted
            }
        };
        let mut s = lock(m);
        if s.generation != generation {
            continue;
        }
        match outcome {
            Spoken::Done => {
                s.index += 1;
                if s.index >= s.queue.len() {
                    s.state = State::Stopped;
                    s.running = false;
                    return;
                }
            }
            Spoken::Interrupted => {}
            Spoken::Failed(e) => {
                s.state = State::Stopped;
                s.error = Some(e);
                s.running = false;
                return;
            }
        }
    }
}

/// Speak one sentence with the system's speech program, stopping early when `changed()`.
#[cfg(not(target_arch = "wasm32"))]
fn speak(text: &str, rate: f32, changed: &dyn Fn() -> bool) -> Spoken {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let wpm = ((BASE_WPM * rate).round() as i64).to_string();
    // The text goes in on stdin so it can never be taken for an option.
    let mut tries: Vec<Command> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("say");
        c.args(["-r", &wpm]);
        tries.push(c);
    }
    #[cfg(target_os = "windows")]
    {
        // SAPI rate: -10 (a third of normal speed) … 10 (three times).
        let r = ((rate.ln() / 3f32.ln()) * 10.0).round().clamp(-10.0, 10.0) as i64;
        let script = format!(
            "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Rate = {r}; $s.Speak([Console]::In.ReadToEnd())"
        );
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        tries.push(c);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for prog in ["espeak-ng", "espeak"] {
            let mut c = Command::new(prog);
            c.args(["-s", &wpm, "--stdin"]);
            tries.push(c);
        }
    }
    let _ = &wpm;
    let mut child = None;
    let mut last_err = String::from("no speech program on this system");
    for mut c in tries {
        match c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(ch) => {
                child = Some(ch);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let Some(mut child) = child else {
        return Spoken::Failed(format!("Read Aloud couldn't start the speech program: {last_err}"));
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
        // Dropping stdin closes it, which tells the program the text is complete.
    }
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Spoken::Done,
            Ok(None) => {}
            Err(e) => return Spoken::Failed(e.to_string()),
        }
        if changed() {
            let _ = child.kill();
            let _ = child.wait();
            return Spoken::Interrupted;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &[Utterance]) -> Vec<&str> {
        v.iter().map(|u| u.text.as_str()).collect()
    }

    #[test]
    fn splits_sentences_but_not_abbreviations() {
        let t = "Smith et al. found 3.5 times more (see Fig. 2). J. Smith agreed! Did they? Yes… Pick one, e.g. this one.";
        let spans = split_sentences(t);
        let got: Vec<String> = spans.iter().map(|(a, b)| t.chars().skip(*a).take(b - a).collect()).collect();
        assert_eq!(got, ["Smith et al. found 3.5 times more (see Fig. 2).", "J. Smith agreed!", "Did they?", "Yes…", "Pick one, e.g. this one."]);
    }

    #[test]
    fn long_sentences_are_split() {
        let t = "word ".repeat(400);
        let spans = split_sentences(&t);
        assert!(spans.len() >= 3);
        assert!(spans.iter().all(|(a, b)| b - a <= MAX_SENTENCE + 5));
    }

    #[test]
    fn tidy_removes_citation_leftovers() {
        assert_eq!(tidy("as shown  ( ) ."), "as shown.");
        assert_eq!(tidy("as shown [] , and"), "as shown, and");
    }

    #[test]
    fn citation_codes() {
        assert!(is_citation_code(" ADDIN ZOTERO_ITEM CSL_CITATION {}"));
        assert!(is_citation_code("ADDIN ZOTERO_BIBL {\"uncited\":[]} CSL_BIBLIOGRAPHY"));
        assert!(is_citation_code("ADDIN EN.CITE <EndNote>"));
        assert!(is_citation_code("ADDIN CSL_CITATION {}"));
        assert!(is_citation_code("CITATION Smi20 \\l 1033"));
        assert!(is_citation_code("BIBLIOGRAPHY"));
        assert!(!is_citation_code("ADDIN ZOTERO_PREF_1"));
        assert!(!is_citation_code("PAGE"));
        assert!(!is_citation_code(""));
        assert!(!is_citation_code("ADD"));
    }

    #[test]
    fn plain_document_sentences() {
        let doc = Document::from_text("First one. Second one.\nThird paragraph");
        let s = sentences(&doc, &Pos::body(0, 0), None, true);
        assert_eq!(texts(&s), ["First one.", "Second one.", "Third paragraph"]);
        assert_eq!(s[1].start, Pos::body(0, 11));
        assert_eq!(s[1].end, Pos::body(0, 22));
        // From the caret, and only up to the end of a selection.
        let s = sentences(&doc, &Pos::body(0, 11), Some(&Pos::body(1, 5)), true);
        assert_eq!(texts(&s), ["Second one.", "Third"]);
    }

    #[test]
    fn player_moves_through_sentences() {
        let doc = Document::from_text("One. Two. Three.");
        let mut r = ReadAloud::default();
        r.backend = Backend::Hold;
        r.start(sentences(&doc, &Pos::body(0, 0), None, true));
        assert_eq!(r.state(), State::Playing);
        assert_eq!(r.current().map(|u| u.text), Some("One.".into()));
        r.skip(1);
        r.skip(5);
        assert_eq!(r.progress(), (2, 3));
        r.skip(-10);
        assert_eq!(r.progress(), (0, 3));
        r.play_pause();
        assert_eq!(r.state(), State::Paused);
        r.set_rate(99.0);
        assert_eq!(r.rate(), MAX_RATE);
        r.set_rate(f32::NAN);
        assert_eq!(r.rate(), 1.0);
        r.play_pause();
        assert_eq!(r.state(), State::Playing);
        r.stop();
        assert_eq!(r.state(), State::Stopped);
        assert!(!r.open);
        assert!(r.current().is_none());
    }

    #[test]
    fn player_finishes_on_its_own() {
        let doc = Document::from_text("One. Two. Three.");
        let mut r = ReadAloud::default();
        r.backend = Backend::Instant;
        r.start(sentences(&doc, &Pos::body(0, 0), None, true));
        for _ in 0..200 {
            if r.state() == State::Stopped {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(r.state(), State::Stopped);
        assert_eq!(r.progress(), (3, 3));
        assert!(r.open, "the player stays open after the last sentence");
        // Play again starts from the top.
        r.backend = Backend::Hold;
        r.play_pause();
        assert_eq!(r.progress().0, 0);
        assert_eq!(r.state(), State::Playing);
    }

    #[test]
    fn empty_queue_does_not_play() {
        let mut r = ReadAloud::default();
        r.start(Vec::new());
        assert_eq!(r.state(), State::Stopped);
        r.skip(1);
        r.skip(-1);
        r.play_pause();
        assert_eq!(r.state(), State::Stopped);
    }
}
