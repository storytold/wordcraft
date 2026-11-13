use super::autocorrect::{AUTOCORRECT, AutoCap, AutoCorrectPrefs, SMILEYS};
use super::{CmdError, Pos, Selection, Session};

/// Apply AutoCorrect to the word just before the caret (called after typing a space/punctuation):
/// replacements, then ordinal superscripts, web addresses as links and sentence capitals.
pub fn autocorrect(s: &mut Session) -> Result<(), CmdError> {
    autocorrect_word(s, false)
}

/// AutoCorrect the word ending at the caret; `on_enter` means Enter finished it (no trigger
/// character was typed).
pub fn autocorrect_word(s: &mut Session, on_enter: bool) -> Result<(), CmdError> {
    if !s.prefs.autocorrect.enabled {
        return Ok(());
    }
    // The word without its marks goes on through the steps below ("*teh* " → "The ").
    let outside = emphasis_autoformat(s, on_enter)?;
    let ac = s.prefs.autocorrect.clone();
    let mut f = s.sel.focus.clone();
    let Some(text) = s.doc.para_at(&f).map(|p| p.text.clone()) else { return Ok(()) };
    let Some(before) = text.get(..f.off) else { return Ok(()) };
    let Some(last) = before.chars().next_back() else { return Ok(()) };
    if !(on_enter || last == ' ' || ",.;:!?".contains(last)) {
        return Ok(());
    }
    let trigger = if on_enter { 0 } else { last.len_utf8() };
    let Some(body) = before.get(..before.len() - trigger) else { return Ok(()) };
    // The word starts after the last white-space character, whatever its width (a no-break
    // space is two bytes and an ideographic space is three).
    let mut start = body.char_indices().rev().find(|(_, c)| c.is_whitespace()).map(|(i, c)| i + c.len_utf8()).unwrap_or(0);
    let Some(mut word) = body.get(start..).map(str::to_string) else { return Ok(()) };
    if word.is_empty() {
        return Ok(());
    }
    // The opening mark of a `*bold*` or `_italic_` span is not part of the word, so the first
    // word of a span of several words is corrected as it is typed ("*teh end*" → "*the end*").
    let mark = word.chars().next().filter(|c| ac.bold_italic && (*c == '*' || *c == '_'));
    let mut opened = false;
    if let Some(rest) = mark.and_then(|m| word.get(1..).filter(|r| !r.is_empty() && !r.contains(m))) {
        start += 1;
        word = rest.to_string();
        opened = true;
    }
    // The word's language (run, paragraph style, document default) picks the replacement list
    // (#835) and the case mapping (#1923). Text with no language at all is in the editing
    // language (a Polish interface writes Polish), else English.
    let lang: Option<String> = s.doc.para_at(&f).and_then(|p| {
        let rc = s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(start));
        rc.lang.clone()
    });
    let lang = lang.or_else(|| s.editing_lang.clone());
    let turkic = is_turkic(lang.as_deref());
    // TWo INitial CApitals and cAPS LOCK come first, so "TEh" is still a replacement's typo.
    if let Some((at, reps)) = case_fix(&ac, &word, turkic) {
        let para = s.doc.para_mut(f.story, &f.path)?;
        let letters = word.get(at..).unwrap_or("").chars().take(reps.len()).collect::<String>();
        let mut end = start + at + letters.len();
        for (c, rep) in letters.chars().rev().zip(reps.iter().rev()) {
            let from = end.saturating_sub(c.len_utf8());
            if rep.chars().ne(std::iter::once(c)) {
                let props = para.props_of_char(from).clone().without_symbol();
                para.delete(from, end)?;
                para.insert_text(from, rep, &props)?;
            }
            end = from;
        }
        s.autocap = Some(AutoCap {
            depth: u64::MAX,
            story: f.story,
            path: f.path.clone(),
            start,
            prev: String::new(),
            two_caps: (ac.two_initial_caps
                && letters.chars().nth(1).is_some_and(char::is_uppercase)
                && letters.chars().next().is_some_and(char::is_uppercase))
            .then(|| letters.clone()),
            backspaced: false,
        });
        let fixed: String = reps.concat();
        f.off = (f.off + fixed.len()).saturating_sub(letters.len());
        s.sel = Selection::caret(f.clone());
        word = format!("{}{fixed}{}", word.get(..at).unwrap_or(""), word.get(at + letters.len()..).unwrap_or(""));
    }
    // A whole-word replacement, else "wait..." → "wait…". Proofing decides what is English the
    // same way (#835); every English (en-GB, en-CA…) counts (#1281).
    let english = s.doc.para_at(&f).is_none_or(|_| wordcraft_proof::ProofLang::from_tag(lang.as_deref()).is_english());
    let found = if ac.replace_text { ac.replacement_in(&word, english) } else { None };
    let (from, rep) = match found {
        Some(r) => (start, Some(r)),
        None if ac.replace_text && word.len() > 3 && word.ends_with("...") => (start + word.len() - 3, Some("…".to_string())),
        None if ac.replace_text => match SMILEYS.iter().find(|m| word.len() > m.len() && word.ends_with(**m)) {
            // "ok:)" → "ok☺": only the smiley at the end of the word is replaced.
            Some(m) => (start + word.len() - m.len(), AUTOCORRECT.iter().find(|(a, _)| a == m).map(|(_, b)| b.to_string())),
            None => (start, None),
        },
        None => (start, None),
    };
    let replaced = rep.is_some();
    let mut caret = f.off;
    if let Some(rep) = rep {
        let end = start + word.len();
        let para = s.doc.para_mut(f.story, &f.path)?;
        // The replacement is text: a symbol it replaces doesn't give it its symbol font (#2036).
        let props = para.props_of_char(from).clone().without_symbol();
        para.delete(from, end)?;
        para.insert_text(from, &rep, &props)?;
        caret = (f.off + rep.len()).saturating_sub(end - from);
        s.sel = Selection::caret(Pos { off: caret, ..f.clone() });
    }
    let end = caret.saturating_sub(trigger);
    let Some(word) = s.doc.para_at(&f).and_then(|p| p.text.get(start..end)).map(str::to_string) else { return Ok(()) };
    let at = |off: usize| Pos { off, ..f.clone() };
    // 1st, 22nd, 103rd → superscript suffix.
    if let Some(n) = ordinal_suffix(&word).filter(|_| ac.ordinals) {
        s.doc.format_range(&at(end - n), &at(end), &|c| c.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript))?;
        return Ok(());
    }
    // Web addresses become links (trailing punctuation stays outside).
    let lower = word.to_ascii_lowercase();
    if ac.links && (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")) && word.len() > 8 {
        let url = word.trim_end_matches(|c: char| ".,;:!?)]\"'”’".contains(c));
        let href = if lower.starts_with("www.") { format!("http://{url}") } else { url.to_string() };
        let url_end = start + url.len();
        s.doc.format_range(&at(start), &at(url_end), &|c| {
            c.link = Some(href.clone());
            c.style = Some("Hyperlink".into());
        })?;
        // Typing after the link isn't part of it (but keeps the text's other formatting, or
        // the formatting outside a `*bold*` or `_italic_` span).
        let mut after = outside.unwrap_or_else(|| s.doc.para_at(&f).map(|p| p.props_of_char(start).clone()).unwrap_or_default());
        after.link = None;
        after.style = None;
        s.pending = Some(after);
        return Ok(());
    }
    // A sentence starts before the opening mark, as for the word without it.
    let before_word = text.get(..start - usize::from(opened)).unwrap_or("");
    // The first word in a table cell follows its own option; other sentences the sentence one.
    let sentence_on = if before_word.trim().is_empty() && f.path.cell().is_some() { ac.cap_cells } else { ac.cap_sentences };
    let exceptions = ac.exception_list();
    // A replacement of several words ("brb" → "be right back") is capitalised by its first.
    let first = word.split_whitespace().next().unwrap_or("");
    let next = if first.len() < word.len() {
        ' '
    } else if on_enter {
        '\n'
    } else {
        last
    };
    let mut sentence = sentence_on && starts_sentence(before_word, &exceptions) && should_capitalize(first, next);
    let prev = if sentence { prev_token(before_word) } else { String::new() };
    // Typing the word again after backspacing an automatic capital: the user doesn't want it
    // (Word's "Automatically add words to list"); an exception, so no capital this time either.
    if sentence
        && !prev.is_empty()
        && s.autocap.take_if(|c| c.backspaced && c.story == f.story && c.path == f.path && c.start == start && c.prev == prev).is_some()
        && ac.auto_add_first_letter
    {
        s.prefs.autocorrect.add_exception(prev.clone());
        sentence = false;
    }
    let day = ac.cap_days && DAYS.contains(&first);
    if sentence || day {
        let para = s.doc.para_mut(f.story, &f.path)?;
        if let Some(c) = word.chars().next() {
            let up = upper_char(c, turkic);
            let props = para.props_of_char(start).clone().without_symbol();
            para.delete(start, start + c.len_utf8())?;
            para.insert_text(start, &up, &props)?;
            let off = (caret + up.len()).saturating_sub(c.len_utf8());
            s.sel = Selection::caret(Pos { off, ..f.clone() });
            if sentence && !prev.is_empty() && !replaced {
                match s.autocap.as_mut().filter(|c| c.depth == u64::MAX) {
                    Some(c) => c.prev = prev,
                    None => {
                        s.autocap = Some(AutoCap { depth: u64::MAX, story: f.story, path: f.path, start, prev, two_caps: None, backspaced: false })
                    }
                }
            }
        }
    }
    Ok(())
}

/// The lowercase token before a word that starts a sentence ("there." in "there. world"), as
/// [`starts_sentence`] reads it; empty at the start of a paragraph.
fn prev_token(before: &str) -> String {
    let t = before.trim_end().trim_end_matches(|c: char| "\"'”’)]".contains(c));
    // Only full stops: Word's exception was observed with "there." (`there?` isn't added).
    if !t.ends_with('.') {
        return String::new();
    }
    t.rsplit(char::is_whitespace).next().unwrap_or("").trim_start_matches(|c: char| "\"'“‘([".contains(c)).to_lowercase()
}

/// Turkish, Azerbaijani (and Crimean Tatar) map `i`/`İ` and `ı`/`I` as pairs (#1923).
pub(crate) fn is_turkic(lang: Option<&str>) -> bool {
    if wordcraft_proof::ProofLang::from_tag(lang) == wordcraft_proof::ProofLang::Tr {
        return true; // tr, tur, any region, same normalisation as proofing
    }
    let Some(tag) = lang.map(str::trim) else { return false };
    let primary = tag.split(['-', '_']).next().unwrap_or("");
    ["az", "aze", "crh"].iter().any(|c| primary.eq_ignore_ascii_case(c))
}

/// A letter in capitals: `i` → `İ` in Turkic languages, otherwise the Unicode default.
pub(crate) fn upper_char(c: char, turkic: bool) -> String {
    if turkic && c == 'i' {
        return "\u{130}".to_string();
    }
    c.to_uppercase().collect()
}

/// A letter in lowercase: `I` → `ı` in Turkic languages. `İ` is always the single letter `i`,
/// never `i` plus a combining dot.
pub(crate) fn lower_char(c: char, turkic: bool) -> String {
    match c {
        'I' if turkic => "\u{131}".to_string(),
        '\u{130}' => "i".to_string(),
        _ => c.to_lowercase().collect(),
    }
}

/// Built-in words that keep their two initial capitals (plurals of two-letter abbreviations).
pub const INITIAL_CAPS_EXCEPTIONS: &[&str] = &["CDs", "IDs", "PCs", "TVs", "DJs", "GBs", "KBs", "MBs", "TBs", "VMs"];

/// Words with a lowercase first letter and the rest in capitals that aren't cAPS LOCK slips.
const CAPS_LOCK_EXCEPTIONS: &[&str] = &["iOS", "mRNA", "tRNA", "rRNA", "cDNA", "eSIM", "eSATA", "iSCSI", "vCPU"];

/// The capitalisation fix for a finished word: TWo INitial CApitals ("WOrd" → "Word") or cAPS
/// LOCK ("wORD" → "Word"). Returns the byte offset of the word's letters and each letter's
/// replacement. Only words of three or more letters, optionally in quotes or brackets.
fn case_fix(caps: &AutoCorrectPrefs, word: &str, turkic: bool) -> Option<(usize, Vec<String>)> {
    if !caps.two_initial_caps && !caps.caps_lock {
        return None;
    }
    let at = word.find(char::is_alphabetic)?;
    let rest = word.get(at..)?;
    let letters = rest.get(..rest.find(|c: char| !c.is_alphabetic()).unwrap_or(rest.len()))?;
    let tail = rest.get(letters.len()..)?;
    if !word.get(..at)?.chars().all(|c| "\"'“‘([".contains(c)) || !tail.chars().all(|c| "\"'”’)]".contains(c)) {
        return None;
    }
    let cs: Vec<char> = letters.chars().collect();
    let (&a, &b, more) = match cs.as_slice() {
        [a, b, more @ ..] if !more.is_empty() => (a, b, more),
        _ => return None,
    };
    let one = |c: char| c.to_string();
    let lower = |c: char| lower_char(c, turkic);
    if caps.two_initial_caps
        && a.is_uppercase()
        && b.is_uppercase()
        && more.iter().all(|c| c.is_lowercase())
        && !INITIAL_CAPS_EXCEPTIONS.contains(&letters)
        && !caps.initial_caps_exceptions.iter().any(|e| e == letters)
    {
        return Some((at, std::iter::once(one(a)).chain(std::iter::once(lower(b))).chain(more.iter().map(|&c| one(c))).collect()));
    }
    if caps.caps_lock && a.is_lowercase() && b.is_uppercase() && more.iter().all(|c| c.is_uppercase()) && !CAPS_LOCK_EXCEPTIONS.contains(&letters) {
        let up = upper_char(a, turkic);
        return Some((at, std::iter::once(up).chain(std::iter::once(lower(b))).chain(more.iter().map(|&c| lower(c))).collect()));
    }
    None
}

/// *Bold* and _italic_ with real formatting, for the word ending at the caret (`on_enter` as for
/// [`autocorrect_word`]): the two marks are removed and the text between them is made bold or
/// italic, for plain and complex-script text. Returns the closing mark's formatting, which is
/// the formatting outside the span, when anything changed.
pub fn emphasis_autoformat(s: &mut Session, on_enter: bool) -> Result<Option<wordcraft_doc::props::CharProps>, CmdError> {
    let Some((open, close, bold)) = emphasis_at_caret(s, on_enter) else { return Ok(None) };
    let f = s.sel.focus.clone();
    let para = s.doc.para_mut(f.story, &f.path)?;
    let outside = para.props_of_char(close).clone();
    para.delete(close, close + 1)?;
    para.delete(open, open + 1)?;
    para.format(open, close - 1, &|c| {
        if bold {
            c.bold = Some(true);
            c.bold_cs = Some(true);
        } else {
            c.italic = Some(true);
            c.italic_cs = Some(true);
        }
    })?;
    s.sel = Selection::caret(Pos { off: f.off.saturating_sub(2), ..f });
    Ok(Some(outside))
}

/// Would [`emphasis_autoformat`] change the word ending at the caret?
pub fn has_emphasis(s: &Session, on_enter: bool) -> bool {
    emphasis_at_caret(s, on_enter).is_some()
}

/// The `*bold*` or `_italic_` span ending at the caret (see [`emphasis_span`]) when the option is
/// on. With Track Changes on, only the current author's tracked insertions are converted, since
/// removing the marks from other text would not be tracked. Nothing is converted while the
/// document limits formatting to its styles.
fn emphasis_at_caret(s: &Session, on_enter: bool) -> Option<(usize, usize, bool)> {
    // Formatting limited to the document's styles allows no direct bold or italic.
    let styles_only = crate::cmd::protect::enforced(&s.doc) && s.doc.settings.protect_formatting;
    if !(s.prefs.autocorrect.enabled && s.prefs.autocorrect.bold_italic) || styles_only {
        return None;
    }
    let f = &s.sel.focus;
    let para = s.doc.para_at(f)?;
    let before = para.text.get(..f.off)?;
    let trigger = match before.chars().next_back() {
        _ if on_enter => 0,
        Some(c) if c == ' ' || ",.;:!?".contains(c) => c.len_utf8(),
        _ => return None,
    };
    let span = before.get(..before.len() - trigger).and_then(emphasis_span)?;
    let tracked = |(open, close, _): &(usize, usize, bool)| {
        !s.doc.settings.track_changes
            || para
                .run_ranges()
                .filter(|(r, _)| r.start <= *close && r.end > *open)
                .all(|(_, p)| p.del.is_none() && p.ins.and_then(|i| s.doc.revisions.get(i as usize)).is_some_and(|rv| rv.author == s.author))
    };
    Some(span).filter(tracked)
}

/// The marks of a `*bold*` or `_italic_` span that `text` ends with: the byte offsets of the
/// opening and closing mark, and whether it is bold. The opening mark starts a word (it follows
/// the paragraph start, white space or an opening bracket or quote), the text between the marks
/// holds a letter or digit and neither starts nor ends with white space, so `2*3*`, `file_name_`
/// and `*?!*` stay as typed. Text holding an object (a picture, field or content control) is left
/// alone.
fn emphasis_span(text: &str) -> Option<(usize, usize, bool)> {
    let mark = text.chars().next_back().filter(|c| *c == '*' || *c == '_')?;
    let close = text.len() - 1;
    let open = text.get(..close)?.rfind(mark)?;
    let inner = text.get(open + 1..close)?;
    let edge_ok = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace());
    if !edge_ok(inner.chars().next())
        || !edge_ok(inner.chars().next_back())
        || !inner.chars().any(char::is_alphanumeric)
        || inner.contains(wordcraft_doc::para::OBJ)
    {
        return None;
    }
    let starts_word = text.get(..open)?.chars().next_back().is_none_or(|c| c.is_whitespace() || "([{\"'“‘".contains(c));
    starts_word.then_some((open, close, mark == '*'))
}

/// Byte length of an ordinal suffix to superscript ("1st" → 2), if `word` is a correct ordinal.
fn ordinal_suffix(word: &str) -> Option<usize> {
    let digits = word.find(|c: char| !c.is_ascii_digit())?;
    let (num, suffix) = word.split_at(digits);
    if num.is_empty() || num.len() > 9 {
        return None;
    }
    let n: u64 = num.parse().ok()?;
    let want = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    (suffix == want).then_some(2)
}

/// Abbreviations whose full stop doesn't end a sentence.
pub(super) const NOT_SENTENCE_END: &[&str] = &[
    "e.g.", "i.e.", "etc.", "vs.", "cf.", "mr.", "mrs.", "ms.", "dr.", "st.", "no.", "approx.", "fig.", "p.", "pp.", "jr.", "sr.", "ca.", "inc.",
    "ltd.", "co.", "vol.", "a.m.", "p.m.",
];

/// Names of days, capitalised as you type (Capitalize names of days).
const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// Does a word typed after `before` (the paragraph text up to the word) begin a sentence?
/// `exceptions` are the lowercase abbreviations whose full stop doesn't end one.
fn starts_sentence(before: &str, exceptions: &[String]) -> bool {
    let t = before.trim_end();
    if t.is_empty() {
        return true;
    }
    if t.len() == before.len() {
        return false; // no space between the previous text and the word
    }
    let t = t.trim_end_matches(|c: char| "\"'”’)]".contains(c));
    if !t.ends_with(['.', '!', '?']) || t.ends_with("..") || t.ends_with('…') {
        return false;
    }
    let prev = t.rsplit(char::is_whitespace).next().unwrap_or(t).trim_start_matches(|c: char| "\"'“‘([".contains(c));
    let lower = prev.to_lowercase();
    // "J." (an initial) or a known abbreviation.
    let initial = prev.chars().count() == 2 && prev.chars().next().is_some_and(char::is_uppercase);
    !initial && !exceptions.contains(&lower)
}

/// Only plain lowercase words are capitalised (not "iPhone", "x2", file names or addresses).
fn should_capitalize(word: &str, next: char) -> bool {
    // A single letter before a full stop is a list label or an initial ("a.", "j.").
    if (next == '.' || word.ends_with('.')) && word.trim_end_matches('.').chars().count() < 2 {
        return false;
    }
    if word == "e.g." || word == "i.e." {
        return true;
    }
    word.chars().next().is_some_and(char::is_lowercase)
        && word.chars().all(|c| c.is_alphabetic() || "'’-.".contains(c))
        && !word.chars().any(char::is_uppercase)
        && !word.trim_end_matches('.').contains('.')
}
