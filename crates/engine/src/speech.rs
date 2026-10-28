//! Read Aloud: the document as a queue of sentences that the system's speech program speaks one
//! at a time, so reading can be paused, skipped forwards and backwards, and sped up.
//!
//! Citation-manager fields can be left out: Zotero, Mendeley and EndNote citations and
//! bibliographies (range fields with an `ADDIN …` code) and Word's own `CITATION` and
//! `BIBLIOGRAPHY` fields. The citation's text simply isn't in the queue, so every citation
//! style is skipped the same way: "(Smith et al., 2020)", "[3]" or a superscript number.
//!
//! Each sentence is spoken in its own language: the run's language tag, or else the script of its
//! letters (Cyrillic, Greek, Arabic, Hebrew, CJK, Thai…), so a Russian sentence gets a Russian voice
//! where one is installed instead of being spelled out letter by letter. The system's own voice
//! stays whenever it already speaks the language (English text on an English system keeps the
//! voice the user chose), and without a voice for the language the default voice speaks.
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
/// Longest utterance, even with no space to split at (a long URL, a run of letters): it is cut
/// there, mid-word.
const MAX_UTTERANCE: usize = 2 * MAX_SENTENCE;
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
    /// The language to speak it in (a BCP 47 tag), or `None` for the system's default voice.
    pub lang: Option<String>,
}

impl Utterance {
    pub fn to_json(&self) -> Value {
        json!({"text": self.text, "start": crate::cmd::pos_json(&self.start), "end": crate::cmd::pos_json(&self.end)})
    }
}

/// The sentences from `from` (to `to`, if given) in `from`'s story, in reading order.
/// With `skip_citations`, citation and bibliography fields are left out. At most
/// [`MAX_SENTENCES`]; when that cuts the text short a warning is logged, so the stop is not silent.
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
        // Sliced per sentence; skipping from the start each time was quadratic in the paragraph.
        let text_chars: Vec<char> = text.chars().collect();
        for (a, b) in split_sentences(&text) {
            if out.len() >= MAX_SENTENCES {
                log::warn!(
                    "Read Aloud: the queue holds at most {MAX_SENTENCES} sentences; the rest of the text is not read (start again from where it stops)"
                );
                return out;
            }
            let spoken = tidy(&text_chars.get(a..b).unwrap_or(&[]).iter().collect::<String>());
            if !spoken.chars().any(char::is_alphanumeric) {
                continue;
            }
            let (Some(first), Some(last)) = (map.get(a), map.get(b.saturating_sub(1))) else { continue };
            // The language of the sentence's first character, through styles and the defaults.
            let run = para.props_of_char(first.0);
            let explicit = doc.styles.resolve_char(para.props.style.as_deref(), run).lang;
            let lang = speech_language(&spoken, explicit.as_deref(), run.lang_bidi.as_deref());
            out.push(Utterance { text: spoken, start: Pos::new(story, path.clone(), first.0), end: Pos::new(story, path.clone(), last.1), lang });
        }
    }
    out
}

/// The language of `text` from its script, as a BCP 47 primary tag: Cyrillic is Russian (Ukrainian
/// with one of its letters `іїєґ`), Greek, Arabic, Hebrew, Thai and Hangul map to `el`, `ar`, `he`, `th`
/// and `ko`, kana to Japanese, Han letters alone to Chinese. `None` for Latin text (the script
/// doesn't say which language) and text without letters. Mixed text goes to its largest non-Latin
/// script once that script has a real share of the letters (a fifth), even when Latin letters
/// outnumber it ("Я открыл Microsoft Word and PowerPoint"): the default voice would spell those
/// letters out, while a Russian voice reads the English words well enough. Ties go to the script
/// listed first.
pub fn detect_language(text: &str) -> Option<&'static str> {
    // Letters per script: Cyrillic, Greek, Arabic, Hebrew, Thai, Hangul, kana, Han, Latin and others.
    let mut n = [0usize; 10];
    let mut ukrainian = false;
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        let i = match c {
            '\u{0400}'..='\u{052F}' | '\u{1C80}'..='\u{1C8F}' | '\u{2DE0}'..='\u{2DFF}' | '\u{A640}'..='\u{A69F}' => {
                ukrainian |= matches!(c, 'і' | 'ї' | 'є' | 'ґ' | 'І' | 'Ї' | 'Є' | 'Ґ');
                0
            }
            '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}' => 1,
            '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}' => 2,
            '\u{0590}'..='\u{05FF}' | '\u{FB1D}'..='\u{FB4F}' => 3,
            '\u{0E00}'..='\u{0E7F}' => 4,
            '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}' => 5,
            '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' => 6,
            '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{2FFFF}' => 7,
            _ => 8,
        };
        if let Some(k) = n.get_mut(i) {
            *k += 1;
        }
    }
    // Japanese text mixes kana and Han letters; any kana makes it Japanese.
    if n[6] > 0 {
        n[6] += n[7];
        n[7] = 0;
    }
    // The first script with the most letters (`max_by_key` would keep the last of a tie).
    let (best, count) = n.iter().take(8).copied().enumerate().fold((0, 0), |b, (i, k)| if k > b.1 { (i, k) } else { b });
    let total: usize = n.iter().sum();
    if count == 0 || count.saturating_mul(5) < total {
        return None;
    }
    Some(match best {
        0 if ukrainian => "uk",
        0 => "ru",
        1 => "el",
        2 => "ar",
        3 => "he",
        4 => "th",
        5 => "ko",
        6 => "ja",
        _ => "zh",
    })
}

/// A language tag that is safe to hand to a speech program or put in a script: letters, digits
/// and hyphens only, starting with a letter, at most 12 characters (a tag from a document is
/// hostile input).
fn plain_tag(tag: &str) -> Option<&str> {
    let t = tag.trim();
    (!t.is_empty() && t.len() <= 12 && t.starts_with(|c: char| c.is_ascii_alphabetic()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .then_some(t)
}

/// The script group a language is written in, for the languages [`detect_language`] can tell
/// apart by script (the primary subtag in, the detected tag out).
fn script_language(primary: &str) -> Option<&'static str> {
    Some(match primary {
        "ru" | "uk" | "be" | "bg" | "sr" | "mk" | "kk" | "ky" | "mn" | "tg" => "ru",
        "el" => "el",
        "ar" | "fa" | "ur" | "ps" | "ku" | "sd" | "ug" => "ar",
        "he" | "yi" => "he",
        "th" => "th",
        "ko" => "ko",
        "ja" => "ja",
        "zh" => "zh",
        _ => return None,
    })
}

/// The language to speak `text` in: the document's `explicit` tag when it fits the text, else the
/// language of its script, else `None` (the default voice). When the text is in a script
/// [`detect_language`] recognises, a tag written in another script (the `en-US` Word writes on
/// every run, on Cyrillic text) gives way to the script, or the voice would spell the letters out;
/// right-to-left text tries the run's complex-script tag (`w:bidi`, e.g. `fa-IR`) first, and Han
/// letters alone keep a Japanese or Korean tag. Latin text keeps any tag, as in Word, which reads
/// text in its proofing language: English words in a run tagged `ru-RU` get the Russian voice.
pub fn speech_language(text: &str, explicit: Option<&str>, bidi: Option<&str>) -> Option<String> {
    let explicit = explicit.and_then(plain_tag);
    let Some(d) = detect_language(text) else { return explicit.map(str::to_string) };
    let fits = |tag: &&str| {
        let primary = tag.split('-').next().unwrap_or("").to_ascii_lowercase();
        let script = script_language(&primary);
        script == script_language(d) || (d == "zh" && matches!(script, Some("ja" | "ko")))
    };
    let bidi = bidi.and_then(plain_tag).filter(|_| matches!(d, "ar" | "he"));
    Some(bidi.into_iter().chain(explicit).find(fits).unwrap_or(d).to_string())
}

/// The primary language subtag of a plain tag, lowercased (`ru-RU` gives `ru`). `None` for tags
/// without a two- or three-letter language, such as Word's `x-none`.
#[cfg(any(unix, windows, test))]
fn primary_tag(tag: &str) -> Option<String> {
    let t = plain_tag(tag)?;
    t.split('-').next().filter(|p| (2..=3).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphabetic())).map(str::to_ascii_lowercase)
}

/// The language to ask the speech program for, or `None` to keep its default voice: only when
/// `lang` isn't the language the default voice speaks (`default`, a primary subtag) already.
#[cfg(any(unix, test))]
fn voice_language(lang: Option<&str>, default: &str) -> Option<String> {
    lang.and_then(primary_tag).filter(|p| !p.eq_ignore_ascii_case(default))
}

/// The arguments of `espeak-ng`/`espeak`: the speed, the voice for `lang` unless it's English
/// (espeak's default voice), and `--stdin`.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn espeak_args(wpm: &str, lang: Option<&str>) -> Vec<String> {
    let mut a = vec!["-s".to_string(), wpm.to_string()];
    if let Some(p) = voice_language(lang, "en") {
        a.push("-v".into());
        a.push(p);
    }
    a.push("--stdin".into());
    a
}

/// The voices in `say -v '?'`'s `listing`: each line is `Name   ll_RR   # sample sentence`, and
/// names may contain spaces. Gives (name, lowercased `ll_rr` code) pairs.
#[cfg(any(target_os = "macos", test))]
fn mac_voices(listing: &str) -> impl Iterator<Item = (String, String)> + '_ {
    listing.lines().filter_map(|line| {
        let line = line.split('#').next().unwrap_or("");
        let words: Vec<&str> = line.split_whitespace().collect();
        let at = words.iter().skip(1).position(|w| {
            let mut p = w.split('_');
            p.next().is_some_and(|l| (2..=3).contains(&l.len()) && l.chars().all(|c| c.is_ascii_lowercase()))
                && p.next().is_some_and(|r| !r.is_empty())
        })? + 1;
        Some((words.get(..at)?.join(" "), words.get(at)?.to_ascii_lowercase()))
    })
}

/// macOS's novelty voices (sound effects and singing), never chosen to read a document.
#[cfg(any(target_os = "macos", test))]
const MAC_NOVELTY_VOICES: &[&str] = &[
    "Albert",
    "Bad News",
    "Bahh",
    "Bells",
    "Boing",
    "Bubbles",
    "Cellos",
    "Deranged",
    "Good News",
    "Hysterical",
    "Jester",
    "Organ",
    "Pipe Organ",
    "Superstar",
    "Trinoids",
    "Whisper",
    "Wobble",
    "Zarvox",
];

/// The voice `say` should use for `lang`, or `None` to keep the system voice: `None` when the
/// system voice already speaks the language (`default`, a primary subtag) or no voice does, else
/// the first voice in `listing` for the language, one of the same region first, never a novelty voice.
#[cfg(any(target_os = "macos", test))]
fn mac_voice(lang: &str, listing: &str, default: &str) -> Option<String> {
    let primary = voice_language(Some(lang), default)?;
    let tag = plain_tag(lang)?.replace('-', "_").to_ascii_lowercase();
    let mut first = None;
    for (name, code) in mac_voices(listing) {
        if code.split('_').next() != Some(primary.as_str()) || MAC_NOVELTY_VOICES.contains(&name.as_str()) {
            continue;
        }
        if code == tag {
            return Some(name);
        }
        first.get_or_insert(name);
    }
    first
}

/// The language (primary subtag) of the system voice: the `selected` voice's in `listing`, else
/// the `locale`'s (`ru_RU`, `ru_RU.UTF-8`), else English.
#[cfg(any(target_os = "macos", test))]
fn mac_default_language(selected: Option<&str>, listing: &str, locale: Option<&str>) -> String {
    let voice = selected
        .and_then(|v| mac_voices(listing).find(|(name, _)| name == v.trim()))
        .and_then(|(_, code)| code.split('_').next().map(str::to_string));
    let locale = || locale.and_then(|l| l.split(['_', '-', '.', '@']).next()).and_then(primary_tag);
    voice.or_else(locale).unwrap_or_else(|| "en".into())
}

/// The language of the voice `say` uses by default (asked once).
#[cfg(target_os = "macos")]
fn mac_default_voice_language() -> &'static str {
    static LANG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LANG.get_or_init(|| {
        let read = |args: &[&str]| {
            std::process::Command::new("defaults")
                .args(args)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        let voice = read(&["read", "com.apple.speech.voice.prefs", "SelectedVoiceName"]);
        let locale = read(&["read", "-g", "AppleLocale"]).or_else(|| std::env::var("LANG").ok());
        mac_default_language(voice.as_deref(), mac_voice_listing(), locale.as_deref())
    })
}

/// Should `speak` say the sentence again with the default voice? When it gave the program a voice
/// and the program then exited with a failure: espeak-ng does that, silently, for a voice it
/// doesn't know, and a missing voice must never mean silence. `exited_ok` is `None` when the
/// program didn't exit on its own (stopped, or never started).
#[cfg(any(not(target_arch = "wasm32"), test))]
fn retry_without_voice(gave_voice: bool, exited_ok: Option<bool>) -> bool {
    gave_voice && exited_ok == Some(false)
}

/// The names `say` can voice, as `say -v '?'` lists them (asked once).
#[cfg(target_os = "macos")]
fn mac_voice_listing() -> &'static str {
    static LISTING: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LISTING.get_or_init(|| {
        std::process::Command::new("say").args(["-v", "?"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    })
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
    // No space to split at (a long URL, a run of letters or full stops): cut the span hard.
    while b - a > MAX_UTTERANCE {
        out.push((a, a + MAX_UTTERANCE));
        a += MAX_UTTERANCE;
        while a < b && chars.get(a).is_some_and(|c| c.is_whitespace()) {
            a += 1;
        }
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
/// One pass, linear in `s` (#3680): a closing bracket right after its opening one (or after it
/// and a space) removes the pair, so nested empty brackets "( [ ] )" go too.
fn tidy(s: &str) -> String {
    let mut t = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_whitespace() {
            if !t.is_empty() && !t.ends_with(' ') {
                t.push(' ');
            }
            continue;
        }
        let open = match c {
            ')' => Some('('),
            ']' => Some('['),
            _ => None,
        };
        if let Some(open) = open {
            let body = t.strip_suffix(' ').unwrap_or(&t);
            if let Some(rest) = body.strip_suffix(open) {
                let keep = rest.len();
                t.truncate(keep);
                continue;
            }
        }
        if matches!(c, '.' | ',' | ';' | ':') && t.ends_with(' ') {
            t.pop();
        }
        t.push(c);
    }
    if t.ends_with(' ') {
        t.pop();
    }
    t
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
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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
    /// The document being read ([`crate::Session::document_id`]): the sentences' positions are
    /// in it.
    pub document: Option<u64>,
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
            document: None,
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
        let (text, lang, generation, rate) = {
            let mut s = lock(m);
            while s.state == State::Paused {
                s = cv.wait(s).unwrap_or_else(PoisonError::into_inner);
            }
            if s.state == State::Stopped {
                s.running = false;
                return;
            }
            match s.queue.get(s.index) {
                Some(u) => (u.text.clone(), u.lang.clone(), s.generation, s.rate),
                None => {
                    s.state = State::Stopped;
                    s.running = false;
                    return;
                }
            }
        };
        let changed = || lock(m).generation != generation;
        let outcome = match backend {
            Backend::System => speak(&text, lang.as_deref(), rate, &changed),
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

/// Windows PowerShell by its full path, `<SystemRoot>\System32\WindowsPowerShell\v1.0\powershell.exe`
/// (`C:\Windows` when `SystemRoot` is unset), rather than whichever `powershell` the program search
/// finds first.
#[cfg(any(target_os = "windows", test))]
fn windows_powershell(system_root: Option<std::ffi::OsString>) -> std::path::PathBuf {
    let root = system_root.filter(|r| !r.is_empty()).unwrap_or_else(|| "C:\\Windows".into());
    std::path::Path::new(&root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe")
}

/// Win32 `CREATE_NO_WINDOW` process creation flag: a console program starts without a console window.
#[cfg(any(target_os = "windows", test))]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How Read Aloud starts Windows PowerShell to speak at `rate`: the program, its arguments and
/// its process creation flags. `speak` builds its `Command` from exactly this; std can't read
/// creation flags back from a `Command`, so the tests check this value instead.
#[cfg(any(target_os = "windows", test))]
struct WindowsSpeech {
    program: std::path::PathBuf,
    args: [String; 4],
    creation_flags: u32,
}

#[cfg(any(target_os = "windows", test))]
fn windows_speech(system_root: Option<std::ffi::OsString>, rate: f32, lang: Option<&str>) -> WindowsSpeech {
    // SAPI rate: -10 (a third of normal speed) … 10 (three times).
    let r = ((rate.ln() / 3f32.ln()) * 10.0).round().clamp(-10.0, 10.0) as i64;
    // The voice of the sentence's language, when one is installed (the default voice would spell
    // out Cyrillic, Greek… letters one by one), but only when the user's default voice doesn't
    // speak that language already, in any region. The tag is `primary_tag`-clean, so it can't end
    // the quote.
    let voice = lang.and_then(primary_tag).map_or_else(String::new, |p| {
        format!(
            "if ($s.Voice.Culture.TwoLetterISOLanguageName -ne '{p}') {{ $v = $s.GetInstalledVoices() | Where-Object {{ $_.Enabled -and $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq '{p}' }} | Select-Object -First 1; if ($v) {{ $s.SelectVoice($v.VoiceInfo.Name) }} }}; "
        )
    });
    // The sentence arrives on stdin as UTF-8 bytes; `[Console]::In` would decode them with the
    // console's code page (Cyrillic became `?` and the voice read single letters), so read the raw
    // stream and decode it as UTF-8. The text itself is never part of the script.
    let script = format!(
        "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Rate = {r}; {voice}$m = New-Object System.IO.MemoryStream; [Console]::OpenStandardInput().CopyTo($m); $s.Speak([System.Text.Encoding]::UTF8.GetString($m.ToArray()))"
    );
    WindowsSpeech {
        program: windows_powershell(system_root),
        args: ["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script],
        // PowerShell is a console program: without this, every sentence flashes a console window
        // over the document (#1122).
        creation_flags: CREATE_NO_WINDOW,
    }
}

/// Speak one sentence with the system's speech program, stopping early when `changed()`. When
/// the program fails with the sentence's voice, the default voice says it instead.
#[cfg(not(target_arch = "wasm32"))]
fn speak(text: &str, lang: Option<&str>, rate: f32, changed: &dyn Fn() -> bool) -> Spoken {
    let (spoken, gave_voice, exited_ok) = speak_with(text, lang, rate, changed);
    if retry_without_voice(gave_voice, exited_ok) {
        return speak_with(text, None, rate, changed).0;
    }
    spoken
}

/// [`speak`] in `lang`'s voice once: how it ended, whether the program was given a voice, and
/// whether it exited successfully (`None` when it didn't exit on its own).
#[cfg(not(target_arch = "wasm32"))]
fn speak_with(text: &str, lang: Option<&str>, rate: f32, changed: &dyn Fn() -> bool) -> (Spoken, bool, Option<bool>) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let wpm = ((BASE_WPM * rate).round() as i64).to_string();
    // The text goes in on stdin so it can never be taken for an option. Each try is a program
    // and whether it was given a voice.
    let mut tries: Vec<(Command, bool)> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("say");
        c.args(["-r", &wpm]);
        let voice = lang.and_then(|l| mac_voice(l, mac_voice_listing(), mac_default_voice_language()));
        if let Some(voice) = &voice {
            c.args(["-v", voice.as_str()]);
        }
        tries.push((c, voice.is_some()));
    }
    #[cfg(target_os = "windows")]
    {
        let w = windows_speech(std::env::var_os("SystemRoot"), rate, lang);
        let mut c = Command::new(&w.program);
        c.args(&w.args);
        std::os::windows::process::CommandExt::creation_flags(&mut c, w.creation_flags);
        tries.push((c, lang.and_then(primary_tag).is_some()));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for prog in ["espeak-ng", "espeak"] {
            let args = espeak_args(&wpm, lang);
            let voiced = args.iter().any(|a| a == "-v");
            let mut c = Command::new(prog);
            c.args(args);
            tries.push((c, voiced));
        }
    }
    let _ = (&wpm, lang);
    let mut child = None;
    let mut last_err = String::from("no speech program on this system");
    for (mut c, voiced) in tries {
        match c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(ch) => {
                child = Some((ch, voiced));
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let Some((mut child, voiced)) = child else {
        return (Spoken::Failed(format!("Read Aloud couldn't start the speech program: {last_err}")), false, None);
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
        // Dropping stdin closes it, which tells the program the text is complete.
    }
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (Spoken::Done, voiced, Some(status.success())),
            Ok(None) => {}
            Err(e) => return (Spoken::Failed(e.to_string()), voiced, None),
        }
        if changed() {
            let _ = child.kill();
            let _ = child.wait();
            return (Spoken::Interrupted, voiced, None);
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

#[cfg(test)]
mod tests;
