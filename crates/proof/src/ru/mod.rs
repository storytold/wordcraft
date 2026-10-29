//! Russian proofing.
//!
//! - **Spelling** against a Russian Hunspell dictionary installed on the system ([`dictionary`],
//!   found by [`crate::hunspell`]: `ru_RU.aff`/`ru_RU.dic` from the OS, a package manager or
//!   LibreOffice's `dict-ru`; it isn't bundled). Without it Russian text gets no spelling marks.
//!   `ё` is optional, as in ordinary Russian writing: `елка` and `ёлка`, `все` and `всё` are
//!   accepted whichever one the dictionary lists. Short words in capitals are abbreviations
//!   (`ООО`, `НДС`). Hyphenated words are checked whole first, then by their parts, knowing the
//!   particles (`кто-то`, `где-нибудь`, `давай-ка`, `всё-таки`), `кое-`/`кой-`, adverbs with
//!   `по-` (`по-русски`, `по-новому`) and compound adjectives whose first part ends in a linking
//!   vowel (`северо-западный`, `сине-зелёный`). Compounds written solid are checked the same way
//!   (`высокоэластичный`, `газоиспользующий`), and so are adjectives, participles and verbal nouns
//!   with `не` (`невскрытый`, `непредоставление`; not verbs: `незнаю` stays wrong). Our supplement
//!   adds words older dictionaries lack (`маркетплейс`, `промокод`, `интеграция`: [`extra`]).
//!   Words in another script (`iPhone`), with digits or superscripts (`м²`), in camel case
//!   (`ВКонтакте`) or abbreviations with an ending (`ГИПа`, `ПДн`) are left alone.
//! - **Suggestions** tuned for Russian ([`suggest()`]): `е`/`ё`, the common confusions (`тся`/
//!   `ться`, `жи`/`ши` written with `ы`, `ча`/`ща` with `я`, voiced and voiceless consonant pairs,
//!   doubled consonants), a missing space (`незнаю` → `не знаю`, `вобщем` → `в общем`), one-letter
//!   typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Russian messages.
//! - **Hyphenation** ([`hyphen`]): the rules of Russian word division, written as code.

mod extra;
pub mod grammar;
pub mod hyphen;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Russian dictionary and whether it came from [`install_dictionary`] rather than a file on
/// disk (it then has no file of its own to report from [`installed_path`], even if a matching
/// file also happens to exist): one lock, so a reload and an install can't interleave their two
/// fields into an inconsistent combination.
static DICT: RwLock<DictState> = RwLock::new(DictState { dict: None, in_memory: false });

struct DictState {
    dict: Option<Arc<AffixDict>>,
    in_memory: bool,
}

/// File names of a Russian Hunspell dictionary.
const NAMES: &[&str] = &["ru_RU", "ru-RU", "ru"];

/// The Russian dictionary: the installed Hunspell `ru_RU` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Russian spelling is off then;
/// see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).dict.as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.dict.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Russian spelling: no Hunspell ru_RU dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Russian spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Russian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8),
/// for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.dict = Some(Arc::new(d));
    slot.in_memory = true;
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.dict = None;
    slot.in_memory = false;
}

/// The `.dic` file of the installed Russian dictionary, searching the dictionaries folder and
/// the system locations now (File › Options › Proofing shows it, #3226); `None` when there is
/// none. A dictionary given with [`install_dictionary`] has no file and isn't reported, even if a
/// matching file also happens to exist.
pub fn installed_path() -> Option<std::path::PathBuf> {
    if DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).in_memory {
        return None;
    }
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Russian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Cyrillic script (the only one the Russian dictionary covers)?
pub(crate) fn is_cyrillic(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{052F}')
}

/// Particles joined to a word with a hyphen (`кто-то`, `где-нибудь`, `давай-ка`, `всё-таки`,
/// `он-де`).
const PARTICLES: &[&str] = &["то", "либо", "нибудь", "ка", "таки", "де", "с"];

/// Prefixes joined to a pronoun with a hyphen (`кое-кто`, `кое-где`, `кой-какой`).
const PRONOUN_PREFIXES: &[&str] = &["кое", "кой"];

/// Words of fewer letters than this written in capitals are taken as abbreviations (`ООО`, `НДС`,
/// `МГУ`, `ГОСТ`).
const ABBREVIATION_LETTERS: usize = 6;

/// Is `word` spelled correctly in Russian? Numbers, single letters, short words in capitals,
/// words with digits and words in other scripts are accepted, and so is every word when no
/// Russian dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    if d.is_empty() {
        return true; // no Russian dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
    let w = w.trim_matches('-');
    let n = w.chars().count();
    if n <= 1 || w.chars().any(char::is_numeric) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && n <= ABBREVIATION_LETTERS {
        return true;
    }
    // Camel case (`ВКонтакте`, `МирКрасок`): a brand or an identifier, not a dictionary word.
    let cs: Vec<char> = w.chars().collect();
    if cs.windows(2).any(|p| matches!(p, [a, b] if a.is_lowercase() && b.is_uppercase())) {
        return true;
    }
    // An abbreviation with lower-case letters or an ending (`ГИПа`, `ПДн`, `ИСПДн`).
    if n <= 10 && cs.iter().take(2).all(|c| c.is_uppercase()) && cs.iter().any(|c| c.is_lowercase()) {
        return true;
    }
    // Another script, or a mix of scripts (`iPhone`, `SMS-ка`): not this dictionary's to judge.
    if w.chars().any(|c| c.is_alphabetic() && !is_cyrillic(c)) {
        return true;
    }
    if known(d, w) {
        return true;
    }
    if w.contains('-') {
        return hyphenated(d, w);
    }
    solid_compound(d, w)
}

/// Endings of adjectives, participles and nouns that are written together with `не`
/// (`невскрытый`, `неисполнение`, `неготовность`). Verb endings aren't among them: `не` with a
/// verb is written apart (`не знаю`).
const NE_ENDINGS: &[&str] = &[
    "ый",
    "ий",
    "ой",
    "ая",
    "яя",
    "ое",
    "ее",
    "ые",
    "ие",
    "ого",
    "его",
    "ому",
    "ему",
    "ым",
    "ую",
    "юю",
    "ых",
    "их",
    "ыми",
    "ими",
    "ние",
    "ния",
    "нию",
    "нием",
    "нии",
    "ость",
    "ости",
    "остью",
    "остей",
    "остям",
    "остями",
    "остях",
];

/// Prefixes Russian joins to any word (`ультрастойкий`, `супермаркет`, `экопакет`).
const WORD_PREFIXES: &[&str] = &[
    "ультра",
    "супер",
    "гипер",
    "мега",
    "микро",
    "макро",
    "мульти",
    "мини",
    "макси",
    "анти",
    "псевдо",
    "квази",
    "контр",
    "сверх",
    "полу",
    "само",
    "авто",
    "видео",
    "аудио",
    "фото",
    "радио",
    "теле",
    "эко",
    "био",
    "нано",
    "агро",
    "медиа",
    "интернет",
    "веб",
    "бизнес",
    "онлайн",
    "спец",
    "гос",
    "мед",
    "юр",
    "орг",
    "проф",
    "соц",
];

/// Prefixes that make a verb perfective (`с-генерировать`, `от-модерировать`, `пере-загрузить`).
const VERB_PREFIXES: &[&str] = &[
    "с", "со", "за", "по", "про", "на", "вы", "от", "ото", "пере", "под", "подо", "при", "раз", "рас", "из", "ис", "у", "о", "об", "обо", "до",
    "недо", "пред", "в", "во", "вз", "вс",
];

/// Endings of verb forms (infinitive, present, past, reflexive).
const VERB_ENDINGS: &[&str] = &[
    "ть", "ться", "ти", "чь", "ю", "ешь", "ет", "ем", "ете", "ют", "ишь", "ит", "им", "ите", "ят", "ал", "ала", "ало", "али", "ил", "ила", "ило",
    "или", "ся", "сь",
];

/// A word written solid from known parts: `не` with an adjective, participle or verbal noun
/// (`невскрытой`), or two stems joined by a linking vowel (`высокоэластичная`, `газоиспользующего`).
fn solid_compound(d: &AffixDict, w: &str) -> bool {
    let lower = w.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let n = chars.len();
    if !(7..=40).contains(&n) {
        return false;
    }
    if let Some(rest) = lower.strip_prefix("не")
        && rest.chars().count() >= 4
        && NE_ENDINGS.iter().any(|e| rest.ends_with(e))
        && known(d, rest)
    {
        return true;
    }
    // A productive prefix (`ультрастойкий`, `экопакет`, `микроцемент`) on a known word.
    if WORD_PREFIXES.iter().any(|p| lower.strip_prefix(p).is_some_and(|rest| rest.chars().count() >= 4 && known(d, rest))) {
        return true;
    }
    // A verb with a perfective prefix (`сгенерировать`, `отмодерировали`) whose unprefixed form is
    // known.
    if VERB_PREFIXES.iter().any(|p| {
        lower.strip_prefix(p).is_some_and(|rest| rest.chars().count() >= 5 && VERB_ENDINGS.iter().any(|e| rest.ends_with(e)) && known(d, rest))
    }) {
        return true;
    }
    (4..=n.saturating_sub(4)).any(|i| {
        let (Some(left), Some(right)) = (chars.get(..i), chars.get(i..)) else { return false };
        if !matches!(left.last(), Some('о' | 'е')) {
            return false;
        }
        let left: String = left.iter().collect();
        let right: String = right.iter().collect();
        known(d, &right) && (known(d, &left) || linking_stem(d, &left))
    })
}

/// Is `part`, ending in a linking vowel, a known word's stem (`северо` → `север`, `сине` →
/// `синий`, `газо` → `газ`)?
fn linking_stem(d: &AffixDict, part: &str) -> bool {
    let Some(stem) = part.strip_suffix(['о', 'е', 'О', 'Е']) else { return false };
    stem.chars().count() >= 2 && ["", "ый", "ий", "ой", "ь"].iter().any(|end| known(d, &format!("{stem}{end}")))
}

/// Is one word in the dictionary or the user dictionary, with `ё` optional?
fn known(d: &AffixDict, w: &str) -> bool {
    if d.check(w) || crate::in_user_dictionary(&w.to_lowercase()) || extra::contains(w) {
        return true;
    }
    // `ё` written as `е` is ordinary Russian: try the dictionary's `ё` spelling…
    let without = w.replace('ё', "е").replace('Ё', "Е");
    if without != w && (d.check(&without) || crate::in_user_dictionary(&without.to_lowercase())) {
        return true;
    }
    // …and `е` where the dictionary has `ё` (one or two of them: `Черемушки`, `еще`).
    let e_positions: Vec<usize> = without.char_indices().filter(|(_, c)| matches!(c, 'е' | 'Е')).map(|(i, _)| i).collect();
    if e_positions.is_empty() || e_positions.len() > 4 {
        return false;
    }
    let swap = |s: &str, i: usize| -> Option<String> {
        let c = s.get(i..)?.chars().next()?;
        let yo = if c == 'Е' { "Ё" } else { "ё" };
        Some(format!("{}{}{}", s.get(..i)?, yo, s.get(i + c.len_utf8()..)?))
    };
    for (k, &i) in e_positions.iter().enumerate() {
        let Some(one) = swap(&without, i) else { continue };
        if d.check(&one) {
            return true;
        }
        for &j in e_positions.get(k + 1..).unwrap_or_default() {
            // `ё` is two bytes like `е`, so later offsets still hold.
            if swap(&one, j).is_some_and(|two| d.check(&two)) {
                return true;
            }
        }
    }
    false
}

/// A hyphenated word that isn't in the dictionary whole: its parts, with the particles,
/// prefixes and linking vowels Russian joins with a hyphen.
fn hyphenated(d: &AffixDict, w: &str) -> bool {
    let parts: Vec<&str> = w.split('-').collect();
    if parts.iter().any(|p| p.is_empty()) || parts.len() > 4 {
        return false;
    }
    let lower = w.to_lowercase();
    let lp: Vec<&str> = lower.split('-').collect();
    // `кто-то`, `где-нибудь`, `всё-таки`: a word and its particles.
    let first_particle = lp.iter().skip(1).position(|p| PARTICLES.contains(p)).map(|i| i + 1);
    if let Some(k) = first_particle
        && lp.iter().skip(k).all(|p| PARTICLES.contains(p))
    {
        let base = parts.get(..k).map(|b| b.join("-")).unwrap_or_default();
        return known(d, &base) || (base.contains('-') && hyphenated(d, &base));
    }
    match lp.as_slice() {
        // `кое-кто`, `кой-какой`.
        [prefix, rest] if PRONOUN_PREFIXES.contains(prefix) => return parts.get(1).is_some_and(|r| known(d, r)),
        // `по-русски` (from `русский`), `по-новому`, `по-моему`.
        ["по", rest] => {
            let Some(r) = parts.get(1) else { return false };
            return known(d, r) || ((rest.ends_with("ски") || rest.ends_with("цки") || rest.ends_with("ьи")) && known(d, &format!("{r}й")));
        }
        _ => {}
    }
    // Compounds part by part: `северо-западный`, `сине-зелёный`, `интернет-магазин`, `из-за`.
    let last = parts.len() - 1;
    parts.iter().enumerate().all(|(i, p)| {
        // An abbreviation part (`ИИ-консультант`, `SMS-рассылка`) or a known word.
        if known(d, p) || (p.chars().all(|c| !c.is_lowercase()) && p.chars().count() <= ABBREVIATION_LETTERS) {
            return true;
        }
        // A first part with a linking vowel: the stem (`северо` → `север`) or its adjective
        // (`сине` → `синий`, `бело` → `белый`).
        i != last && linking_stem(d, p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 50, "{}", d.len());
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged_and_nothing_suggested() {
        let none = AffixDict::default();
        for w in ["превет", "сдесь", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(suggest::suggest_in(&none, "превет", 5).is_empty());
    }

    #[test]
    fn russian_words_are_correct() {
        test_dict::install();
        for w in [
            "привет",
            "Привет",
            "ПРИВЕТ",
            "дом",
            "дома",
            "домами",
            "красный",
            "красного",
            "красной",
            "читать",
            "читаем",
            "читаются",
            "ёлка",
            "елка",
            "всё",
            "все",
            "ещё",
            "еще",
            "Москва",
            "Москве",
            "кто-то",
            "где-нибудь",
            "что-либо",
            "давай-ка",
            "всё-таки",
            "все-таки",
            "кое-кто",
            "по-русски",
            "по-новому",
            "северо-западный",
            "сине-зелёный",
            "сине-зеленый",
            "из-за",
            "ООО",
            "НДС",
            "2026",
            "5-й",
            "iPhone",
            "e-mail",
            "в",
            "м²",
            "ВКонтакте",
            "МирКрасок",
            "ГИПа",
            "ПДн",
            "ИИ-консультант",
            "маркетплейсов",
            "промокода",
            "невскрытой",
            "некрасного",
            "непредоставления",
            "высокоэластичная",
            "краснозелёный",
            "Пн-Пт",
            "ультракрасный",
            "суперновый",
            "прочитать",
            "администрируется",
            "юрлица",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in [
            "превет",
            "сдесь",
            "жыть",
            "чясто",
            "незнаю",
            "нечитаю",
            "вобщем",
            "москва",
            "здраствуй",
            "кто-нибуть",
            "по-руски",
            "северо-зопадный",
            "ПРИВЕТТТ",
            "высокозопадный",
        ] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("зкщфрусскоеслово"));
        crate::add_word("Зкщфрусскоеслово");
        assert!(is_correct("зкщфрусскоеслово"));
    }
}
