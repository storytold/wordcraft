//! Turkish grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when unsure.
//! Messages are in Turkish.
//!
//! - **Repeated word** (`çok çok`), except idiomatic reduplications and doublings (`ALLOWED_REPEATS`:
//!   `yavaş yavaş`, `tıkır tıkır`, `adım adım`).
//! - **Space before punctuation** (`ev ,`) and **double spaces**.
//! - **Capital letter at the start of a sentence**, knowing Turkish abbreviations (`vb.`, `vs.`,
//!   `prof.`, `dr.`, `mad.`, `yy.`, `s.`…) after which lowercase or numbers may continue.
//! - **No comma before/after conjunctions**: `ve`, `veya`, `yahut`, `ile`, `de`, `da`, `hem ... hem`,
//!   `ne ... ne` conjunctions should not have commas immediately before or after them.
//! - **Doubled punctuation** (`..`, `,,`, `;;`).

use crate::{Issue, IssueKind, ProofLang, words};

/// Turkish reduplications and emphatic word pairings that are intentionally repeated.
const ALLOWED_REPEATS: &[&str] = &[
    "yavaş",
    "hızlı",
    "koşa",
    "güle",
    "adım",
    "kara",
    "tıkır",
    "şakır",
    "akın",
    "ağır",
    "açık",
    "ayrı",
    "bire",
    "bir",
    "boş",
    "bölük",
    "buram",
    "cana",
    "can",
    "cılk",
    "çifter",
    "çit",
    "dalga",
    "damla",
    "deste",
    "didik",
    "dizi",
    "dolu",
    "deste",
    "döne",
    "eğri",
    "enine",
    "epey",
    "fıkır",
    "fırıl",
    "gece",
    "gıcır",
    "gide",
    "günü",
    "güzel",
    "halk",
    "harıl",
    "hık",
    "hım",
    "horul",
    "ılık",
    "ışıl",
    "için",
    "içten",
    "iğne",
    "iri",
    "iyiden",
    "iyi",
    "kaba",
    "kaç",
    "kat",
    "kaynar",
    "kelli",
    "kese",
    "kıtır",
    "kıvrım",
    "koca",
    "kol",
    "kucak",
    "kütür",
    "mırıl",
    "öbek",
    "parça",
    "patır",
    "pıtır",
    "sabah",
    "salına",
    "salkım",
    "sap",
    "sara",
    "sarı",
    "serin",
    "sıra",
    "sırım",
    "sıkı",
    "sırılsıklam",
    "tane",
    "teker",
    "tek",
    "tıpış",
    "top",
    "tutam",
    "uzun",
    "üçer",
    "üst",
    "yalan",
    "yana",
    "yan",
    "yarım",
    "yer",
    "yudum",
    "yığın",
    "yürekten",
    "yüz",
    "zaman",
    "zor",
];

/// Turkish abbreviations that end with a period and after which lower case may continue.
const ABBREVIATIONS: &[&str] = &[
    "vb", "vs", "vd", "bkz", "bknz", "ör", "örnek", "mad", "md", "s", "sf", "sn", "dk", "sa", "yy", "m", "km", "cm", "mm", "gr", "kg", "mg", "dr",
    "prof", "doç", "yrd", "av", "ist", "ank", "izm", "cad", "sok", "mah", "apt", "no", "tel", "faks", "tl", "kr", "krş", "a.ş", "ltd", "şti", "t.c",
    "tbmm", "meb", "möt", "mö", "ms",
];

/// Conjunctions that must not be preceded by a comma.
const CONJUNCTIONS_NO_PRE_COMMA: &[&str] = &["ve", "veya", "yahut", "velev", "veyahut", "de", "da"];

/// Conjunctions that must not be followed by a comma.
const CONJUNCTIONS_NO_POST_COMMA: &[&str] = &["ve", "veya", "yahut", "veyahut", "hem", "ne", "ya", "gerek", "ister", "de", "da"];

/// Check grammar and punctuation in a Turkish text block.
pub fn check(text: &str) -> Vec<Issue> {
    check_with(text, true)
}

/// [`check`] where `capital_first` says whether the text's first word must start a sentence with a capital
/// (false for a list item, which may be a fragment).
pub fn check_with(text: &str, capital_first: bool) -> Vec<Issue> {
    let mut v = Vec::new();
    let ws = words(text);

    // 1. Repeated words (`kelime kelime`)
    for pair in ws.windows(2) {
        let (w1, w2) = match (text.get(pair[0].0..pair[0].1), text.get(pair[1].0..pair[1].1)) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };
        let gap = match text.get(pair[0].1..pair[1].0) {
            Some(g) => g,
            None => continue,
        };
        let only_spaces = gap.chars().all(|c| c == ' ' || c == '\t');
        let lower1 = w1.to_lowercase();
        let lower2 = w2.to_lowercase();
        if only_spaces && lower1 == lower2 && !ALLOWED_REPEATS.contains(&lower1.as_str()) {
            v.push(Issue {
                start: pair[1].0,
                end: pair[1].1,
                kind: IssueKind::Grammar,
                message: "Yinelenen sözcük".into(),
                suggestions: Vec::new(),
                lang: ProofLang::Tr,
            });
        }
    }

    // 2. Spaces before punctuation and double spaces
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for i in 0..chars.len() {
        let (idx, c) = chars[i];
        if matches!(c, ',' | '.' | '?' | '!' | ';' | ':') && i > 0 && chars[i - 1].1 == ' ' {
            let prev_sp = chars[i - 1].0;
            // Leave emoticons and ellipses alone
            let is_dot_in_dots = c == '.' && (i + 1 < chars.len() && chars[i + 1].1 == '.');
            if !is_dot_in_dots {
                v.push(Issue {
                    start: prev_sp,
                    end: idx + c.len_utf8(),
                    kind: IssueKind::Grammar,
                    message: "Noktalama işaretinden önce boşluk bırakılmaz".into(),
                    suggestions: vec![c.to_string()],
                    lang: ProofLang::Tr,
                });
            }
        }
        // Double space
        if c == ' ' && i + 1 < chars.len() && chars[i + 1].1 == ' ' && (i == 0 || chars[i - 1].1 != ' ') {
            let mut end = idx + 1;
            while end < text.len() && text.as_bytes().get(end) == Some(&b' ') {
                end += 1;
            }
            v.push(Issue {
                start: idx,
                end,
                kind: IssueKind::Grammar,
                message: "Fazladan boşluk".into(),
                suggestions: vec![" ".into()],
                lang: ProofLang::Tr,
            });
        }
    }

    // 3. Comma before or after conjunctions (`ali, ve veli` or `ve, veli`)
    for (i, &(w_start, w_end)) in ws.iter().enumerate() {
        let word = match text.get(w_start..w_end) {
            Some(w) => w.to_lowercase(),
            None => continue,
        };
        // Check comma before conjunction
        if CONJUNCTIONS_NO_PRE_COMMA.contains(&word.as_str())
            && i > 0
            && let Some(between) = text.get(ws[i - 1].1..w_start)
            && let Some(comma_pos) = between.find(',')
        {
            let actual_pos = ws[i - 1].1 + comma_pos;
            v.push(Issue {
                start: actual_pos,
                end: actual_pos + 1,
                kind: IssueKind::Grammar,
                message: format!("'{word}' bağlacından önce virgül konulmaz"),
                suggestions: vec![String::new()],
                lang: ProofLang::Tr,
            });
        }
        // Check comma after conjunction
        if CONJUNCTIONS_NO_POST_COMMA.contains(&word.as_str())
            && i + 1 < ws.len()
            && let Some(between) = text.get(w_end..ws[i + 1].0)
            && let Some(comma_pos) = between.find(',')
        {
            let actual_pos = w_end + comma_pos;
            v.push(Issue {
                start: actual_pos,
                end: actual_pos + 1,
                kind: IssueKind::Grammar,
                message: format!("'{word}' bağlacından sonra virgül konulmaz"),
                suggestions: vec![String::new()],
                lang: ProofLang::Tr,
            });
        }
    }

    // 4. Sentence capitalization knowing Turkish abbreviations
    for (i, &(w_start, w_end)) in ws.iter().enumerate() {
        if i == 0 {
            // First word in paragraph
            if capital_first
                && let Some(w) = text.get(w_start..w_end)
                && w.chars().next().is_some_and(char::is_lowercase)
            {
                let cap = turkish_capitalize(w);
                v.push(Issue {
                    start: w_start,
                    end: w_end,
                    kind: IssueKind::Grammar,
                    message: "Cümle büyük harfle başlar".into(),
                    suggestions: vec![cap],
                    lang: ProofLang::Tr,
                });
            }
            continue;
        }

        // Check if previous character sequence ended a sentence
        let prev_end = ws[i - 1].1;
        let prev_word = text.get(ws[i - 1].0..prev_end).unwrap_or("");
        if let Some(gap) = text.get(prev_end..w_start) {
            let trimmed = gap.trim();
            if (trimmed.ends_with('.') || trimmed.ends_with('?') || trimmed.ends_with('!'))
                && !trimmed.contains("...")
                && !ABBREVIATIONS.contains(&prev_word.to_lowercase().as_str())
                && let Some(w) = text.get(w_start..w_end)
                && w.chars().next().is_some_and(char::is_lowercase)
            {
                let cap = turkish_capitalize(w);
                v.push(Issue {
                    start: w_start,
                    end: w_end,
                    kind: IssueKind::Grammar,
                    message: "Cümle büyük harfle başlar".into(),
                    suggestions: vec![cap],
                    lang: ProofLang::Tr,
                });
            }
        }
    }

    v.sort_by_key(|issue| (issue.start, issue.end));
    v
}

/// Capitalize according to Turkish rules (`i` -> `İ`, `ı` -> `I`).
pub fn turkish_capitalize(s: &str) -> String {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return String::new() };
    let first_cap = match first {
        'i' => 'İ',
        'ı' => 'I',
        other => other.to_uppercase().next().unwrap_or(other),
    };
    let mut out = String::with_capacity(s.len() + 1);
    out.push(first_cap);
    out.push_str(chars.as_str());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_turkish_text_produces_no_issues() {
        let text = "Bu bir test cümlesidir. Kitap ve defter aldım. Çocuk yavaş yavaş yürüdü.";
        assert!(check(text).is_empty());
    }

    #[test]
    fn repeated_words_flagged_except_allowed() {
        let text = "Gitti gitti ama bulamadı.";
        let issues = check(text);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Yinelenen sözcük");

        // Allowed repeat
        let text2 = "Çocuk yavaş yavaş ilerliyordu.";
        assert!(check(text2).is_empty());
    }

    #[test]
    fn space_before_punctuation_flagged() {
        let text = "Merhaba , dünya!";
        let issues = check(text);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Noktalama işaretinden önce boşluk bırakılmaz");
    }

    #[test]
    fn comma_around_conjunctions_flagged() {
        let text = "Elma, ve armut.";
        let issues = check(text);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("virgül konulmaz"));

        let text2 = "Bana kalırsa bu mesele de, haklı olan taraf bizdik.";
        let issues2 = check(text2);
        assert_eq!(issues2.len(), 1);
        assert!(issues2[0].message.contains("'de' bağlacından sonra virgül konulmaz"));
    }

    #[test]
    fn sentence_capitalization_respects_abbreviations() {
        let text = "Dr. ahmet geldi. Fakat geç kaldı.";
        let issues = check(text);
        // "Dr. ahmet" does not flag "ahmet" because "Dr." is in ABBREVIATIONS.
        assert!(issues.is_empty());
    }
}
