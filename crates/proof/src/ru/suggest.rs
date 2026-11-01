//! Russian spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with
//! Russian's own letter groups and confusions, since Russian Hunspell dictionaries carry no `MAP`
//! or `REP` tables (`е`/`ё` first, then unstressed vowels, voiced and voiceless pairs, `жы` →
//! `жи`, `тся`/`ться`, dropped or doubled consonants; up to three letters changed), and its rule
//! for a missing space (`незнаю` → `не знаю`, `вобщем` → `в общем`).

use crate::affix::AffixDict;
use crate::affix_suggest::{COST_OTHER_MAP, COST_SPLIT, COST_SPLIT_LONG, Split, Tuning};

/// Letters confused with each other: a slip inside a group is cheaper than another typo.
const MAP: &[&str] = &["её", "оа", "еия", "зс", "дт", "бп", "вф", "гк", "жш", "ьъ", "еэ", "ыи"];
/// Common confusions: a written sequence and what was meant.
const REP: &[(&str, &str)] = &[
    ("жы", "жи"),
    ("шы", "ши"),
    ("чя", "ча"),
    ("щя", "ща"),
    ("чю", "чу"),
    ("щю", "щу"),
    ("тся", "ться"),
    ("ться", "тся"),
    ("сч", "щ"),
    ("щ", "сч"),
    ("нн", "н"),
    ("н", "нн"),
    ("сс", "с"),
    ("с", "сс"),
    ("лл", "л"),
    ("л", "лл"),
    ("тт", "т"),
    ("т", "тт"),
    ("мм", "м"),
    ("м", "мм"),
    ("рр", "р"),
    ("р", "рр"),
    ("ье", "ъе"),
    ("ъе", "ье"),
    ("раст", "рост"),
    ("рост", "раст"),
    ("лож", "лаг"),
    ("лаг", "лож"),
    // Unpronounced consonants: `здраствуй`, `чуствовать`, `солнце`, `лесница`, `праздник`.
    ("аст", "авст"),
    ("уст", "увст"),
    ("лнц", "лц"),
    ("лц", "лнц"),
    ("сн", "стн"),
    ("стн", "сн"),
    ("зн", "здн"),
    ("здн", "зн"),
    ("ц", "тс"),
    ("тс", "ц"),
];
/// Short words often run into the next one (`не знаю`, `в общем`, `на верху`, `потому что`).
const LEADING_WORDS: &[&str] = &[
    "не",
    "ни",
    "в",
    "во",
    "на",
    "с",
    "со",
    "к",
    "ко",
    "о",
    "об",
    "обо",
    "от",
    "ото",
    "до",
    "из",
    "изо",
    "за",
    "по",
    "под",
    "над",
    "при",
    "про",
    "без",
    "для",
    "у",
    "а",
    "и",
    "но",
    "да",
    "же",
    "как",
    "так",
    "то",
    "что",
    "уже",
    "еще",
    "ещё",
    "все",
    "всё",
    "потому",
    "чтобы",
    "это",
];
/// Prepositions among [`LEADING_WORDS`].
const PREPOSITIONS: &[&str] = &[
    "в", "во", "на", "с", "со", "к", "ко", "о", "об", "обо", "от", "ото", "до", "из", "изо", "за", "по", "под", "над", "при", "про", "без", "для",
    "у",
];
/// Short words often run into the one before (`мог бы`, `он же`, `так ли`).
const TRAILING_WORDS: &[&str] = &["ли", "же", "бы", "ж", "б", "что"];
/// The Russian alphabet, when the dictionary has no `TRY` line.
const ALPHABET: &str = "оеаинтсрвлкмдпуяыьгзбчйхжшюцщэфъё";

/// `е` written for `ё`, or the other way round, is barely a slip.
const COST_YO: u32 = 3;
/// A one- or two-letter preposition run into the next word (`вдоме` → `в доме`).
const COST_SPLIT_PREPOSITION: u32 = 17;

/// What Russian adds to its dictionary's tables.
const TUNING: Tuning =
    Tuning { split: Split::Custom(split), alphabet: ALPHABET, map: MAP, rep: REP, rep_cost: 9, max_map_changes: 3, map_cost, ..Tuning::DEFAULT };

/// The cost of changing `from` to `to` within a letter group: `е`/`ё` cheapest.
fn map_cost(from: char, to: char) -> u32 {
    if matches!((from, to), ('е', 'ё') | ('ё', 'е')) { COST_YO } else { COST_OTHER_MAP }
}

/// A missing space: cheap after a short leading word or before a short trailing one, dearer after
/// a short preposition or between two longer words, never leaving a stray short half.
fn split(a: &str, b: &str) -> Option<u32> {
    let long = |p: &str| p.chars().count() >= 3;
    // A preposition takes a noun, not a verb (`с генерировать` isn't a fix for
    // `сгенерировать`); after one or two letters a split is likelier wrong than a typo.
    let preposition = PREPOSITIONS.contains(&a);
    if preposition && ["ть", "ться", "ти", "чь"].iter().any(|e| b.ends_with(e)) {
        return None;
    }
    if preposition && a.chars().count() <= 2 && LEADING_WORDS.contains(&a) && long(b) {
        Some(COST_SPLIT_PREPOSITION)
    } else if (LEADING_WORDS.contains(&a) && long(b)) || (long(a) && TRAILING_WORDS.contains(&b)) {
        Some(COST_SPLIT)
    } else if long(a) && long(b) {
        Some(COST_SPLIT_LONG)
    } else {
        None
    }
}

/// Spelling suggestions for a Russian word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &TUNING)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(word: &str, want: &str) {
        let s = suggest(word, 8);
        assert!(s.iter().any(|x| x == want), "{word} → {s:?}, want {want}");
    }

    /// Russian's own costs: `е`/`ё` is the cheapest slip, so a `ё` and one more group slip
    /// (made-up `нёснаю`) beat a missing space; a REP confusion beats a `ё` and a group slip;
    /// a preposition is never split off a verb, and after a one-letter one a typo is likelier.
    #[test]
    fn russian_costs_rank_its_slips() {
        let dict = |words: &[&str]| AffixDict::from_hunspell("", &format!("{}\n{}\n", words.len(), words.join("\n")), &[]).unwrap();
        assert_eq!(suggest_in(&dict(&["не", "знаю", "нёснаю"]), "незнаю", 1), ["нёснаю"]);
        assert_eq!(suggest_in(&dict(&["еча", "ёчи"]), "ечя", 1), ["еча"]);
        assert!(!suggest_in(&dict(&["с", "генерировать"]), "сгенерировать", 8).contains(&"с генерировать".into()));
        let s = suggest_in(&dict(&["в", "доме"]), "вдоме", 4);
        assert_eq!(s.first().map(String::as_str), Some("доме"), "{s:?}");
        assert!(s.contains(&"в доме".into()), "{s:?}");
    }

    /// A word the dictionary marks NOSUGGEST (a vulgarism, a rare form) is correct but never
    /// offered, in any round or letter case. Russian's MAP and REP tables are its own, so the
    /// shared Latin-script check doesn't reach it.
    #[test]
    fn nosuggest_words_are_never_offered() {
        let offered = |aff: &str, input: &str| {
            let d = AffixDict::from_hunspell(aff, "2\nпривет/N\nпредмет/N\n", &[]).unwrap();
            suggest_in(&d, input, 16)
        };
        assert!(offered("SET UTF-8\n", "превет").contains(&"привет".into()), "offered without the flag declared");
        for input in ["превет", "ПРЕВЕТ", "предмот", "приветт", "привте", "привет"] {
            let s = offered("SET UTF-8\nNOSUGGEST N\n", input);
            assert!(!s.iter().any(|w| w.to_lowercase().contains("привет") || w.to_lowercase().contains("предмет")), "{input}: {s:?}");
        }
    }

    #[test]
    fn russian_misspellings_get_the_right_suggestion() {
        crate::ru::test_dict::install();
        has("превет", "привет");
        has("сдесь", "здесь");
        has("жыть", "жить");
        has("чясто", "часто");
        has("незнаю", "не знаю");
        has("вобщем", "в общем");
        has("здраствуй", "здравствуй");
        has("москва", "Москва");
        has("Превет", "Привет");
        has("ПРЕВЕТ", "ПРИВЕТ");
        has("читаеться", "читается");
        has("краснного", "красного");
        has("западнный", "западный");
        assert!(!suggest("сгенерировать", 8).contains(&"с генерировать".to_string()));
    }

    #[test]
    fn the_likeliest_fix_comes_first() {
        crate::ru::test_dict::install();
        assert_eq!(suggest("превет", 3).first().map(String::as_str), Some("привет"));
        assert_eq!(suggest("незнаю", 3).first().map(String::as_str), Some("не знаю"));
        assert_eq!(suggest("жыть", 3).first().map(String::as_str), Some("жить"));
        assert_eq!(suggest("зеленый", 3).first().map(String::as_str), Some("зелёный"));
    }

    #[test]
    fn edge_cases_stay_empty_or_bounded() {
        crate::ru::test_dict::install();
        assert!(suggest("", 5).is_empty());
        assert!(suggest("дом", 0).is_empty());
        assert!(suggest(&"а".repeat(41), 5).is_empty());
        assert!(suggest("два слова", 5).is_empty());
        assert!(suggest("привет", 5).len() <= 5);
        assert!(!suggest("привет", 5).contains(&"привет".to_string()));
    }

    #[test]
    fn distance_weighs_letter_groups_half() {
        use crate::affix_suggest::{fold, weighted_distance};
        let map: Vec<Vec<char>> = MAP.iter().map(|g| g.chars().collect()).collect();
        let related = |a: char, b: char| fold(a, &map) == fold(b, &map);
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(weighted_distance(&c("сдесь"), &c("здесь"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("ёлка"), &c("елка"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("дом"), &c("дмо"), &related, 9), 2);
        assert_eq!(weighted_distance(&c("дом"), &c("домами"), &related, 9), 6);
        assert_eq!(weighted_distance(&c("дом"), &c("книга"), &related, 2), 3);
    }

    #[test]
    fn suggestions_are_fast_enough() {
        crate::ru::test_dict::install();
        let _ = suggest("дом", 1);
        let t = std::time::Instant::now();
        for w in ["ыкщзвотрусскогословаааа", "здраствуйте", "краснннного"] {
            let _ = suggest(w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }
}
