//! Russian grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when unsure
//! (a missed comma costs less than a wrong mark on correct text). Messages are in Russian.
//!
//! - **Repeated word** (`в в`), except words Russian doubles on purpose (`да да`, `так так`,
//!   `ну ну`, `ха ха`, `очень очень`).
//! - **Space before punctuation** (`да ,`) and **double spaces**; emoticons (` :)`) and ellipses
//!   are left alone.
//! - **Capital letter at the start of a sentence**, knowing Russian abbreviations (`т. е.`, `г.`,
//!   `руб.`, `ул.`, `и т. д.`), initials, numbers, ellipses and dialogue dashes, after which lower
//!   case is correct.
//! - **Missing comma** before conjunctions and relative pronouns that always open a clause:
//!   `что`, `чтобы`, `поскольку`, `если`, `хотя` (not `хотя бы`) and the forms of `который`, and
//!   before `потому`/`оттого` in `потому что`. Not after a conjunction or particle (`и что`,
//!   `только что`, `даже если`, `так что`, `для того чтобы`), not after a preposition (`за что`,
//!   `в котором`: the comma goes before it) and not inside `что-то`.
//! - **Doubled punctuation** (`,,` — often a typed opening quote — `..`, `;;`).
//!
//! No English rule (such as `a`/`an`) applies to Russian text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Russian repeats on purpose: interjections, onomatopoeia and emphatic doubling.
const ALLOWED_REPEATS: &[&str] = &[
    "да",
    "нет",
    "так",
    "ну",
    "ха",
    "хи",
    "хе",
    "ой",
    "ай",
    "эй",
    "ах",
    "ох",
    "эх",
    "ага",
    "угу",
    "тук",
    "кап",
    "бах",
    "бум",
    "тик",
    "так",
    "ля",
    "очень",
    "много",
    "давно",
    "далеко",
    "долго",
    "еле",
    "чуть",
    "тихо",
    "быстро",
    "скорей",
    "скорее",
    "иди",
    "давай",
    "пока",
    "ладно",
    "вот",
    "ещё",
    "еще",
    "всё",
    "все",
    "кто",
    "что",
    "то",
];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "т",
    "е",
    "д",
    "п",
    "др",
    "пр",
    "см",
    "ср",
    "напр",
    "т.е",
    "т.д",
    "т.п",
    "т.к",
    "т.н",
    "т.о",
    "г",
    "гг",
    "в",
    "вв",
    "н.э",
    "до н.э",
    "руб",
    "коп",
    "тыс",
    "млн",
    "млрд",
    "трлн",
    "ул",
    "пр-т",
    "пер",
    "наб",
    "пл",
    "ш",
    "д",
    "кв",
    "корп",
    "стр",
    "оф",
    "эт",
    "под",
    "обл",
    "р-н",
    "пос",
    "дер",
    "им",
    "св",
    "проф",
    "доц",
    "акад",
    "канд",
    "наук",
    "зав",
    "зам",
    "ред",
    "изд",
    "сост",
    "пер",
    "рис",
    "табл",
    "гл",
    "разд",
    "ст",
    "ч",
    "п",
    "пп",
    "абз",
    "прим",
    "прил",
    "илл",
    "с",
    "сс",
    "т",
    "тт",
    "вып",
    "кн",
    "экз",
    "шт",
    "кг",
    "мин",
    "сек",
    "час",
    "мес",
    "нед",
    "янв",
    "фев",
    "февр",
    "мар",
    "апр",
    "авг",
    "сент",
    "окт",
    "нояб",
    "дек",
    "пн",
    "вт",
    "чт",
    "пт",
    "сб",
    "вс",
    "англ",
    "нем",
    "фр",
    "франц",
    "лат",
    "рус",
    "греч",
    "итал",
    "исп",
    "букв",
    "перен",
    "разг",
    "устар",
    "ок",
    "ок",
    "прибл",
    "мл",
    "ст",
    "тел",
    "моб",
    "факс",
    "эл",
    "почт",
    "вкл",
    "искл",
    "мм",
    "см",
    "дм",
    "км",
    "м",
    "л",
    "мл",
    "т",
    "ц",
    "млн",
    "долл",
    "евро",
    "др",
    "проч",
    "etc",
    "vs",
];

/// Conjunctions and relative pronouns that open a clause: a comma comes before them.
const CLAUSE_OPENERS: &[&str] = &[
    "что",
    "чтобы",
    "поскольку",
    "если",
    "хотя",
    "который",
    "которая",
    "которое",
    "которые",
    "которого",
    "которой",
    "которому",
    "которым",
    "котором",
    "которую",
    "которых",
    "которыми",
];

/// Words after which a clause opener takes no comma of its own: conjunctions (`и что`, `а если`),
/// particles (`только что`, `даже если`, `лишь бы`), interrogatives and the first halves of
/// compound conjunctions (`потому что`, `так что`, `для того чтобы`, `с тем чтобы`, `несмотря на
/// то что`, `тогда как`, `как будто`).
const NO_COMMA_AFTER: &[&str] = &[
    "и",
    "а",
    "но",
    "да",
    "или",
    "либо",
    "ни",
    "не",
    "же",
    "ли",
    "бы",
    "б",
    "то",
    "только",
    "лишь",
    "именно",
    "даже",
    "особенно",
    "потому",
    "оттого",
    "поэтому",
    "так",
    "затем",
    "тем",
    "того",
    "ввиду",
    "вследствие",
    "несмотря",
    "благодаря",
    "прежде",
    "раньше",
    "мере",
    "пор",
    "чем",
    "будто",
    "словно",
    "точно",
    "как",
    "если",
    "хотя",
    "хоть",
    "чтобы",
    "что",
    "кто",
    "где",
    "когда",
    "почему",
    "зачем",
    "куда",
    "откуда",
    "сколько",
    "разве",
    "неужели",
    "вот",
    "вон",
    "ну",
    "уже",
    "это",
    "просто",
    "чуть",
    "мол",
    "ведь",
    "вроде",
    "тоже",
    "также",
    "ещё",
    "еще",
    "всё",
    "все",
    "почти",
    "вовсе",
    "ой",
    "ах",
    "эх",
    "ох",
    "ага",
    "нет",
    "конечно",
    "наверное",
    "может",
    "пусть",
    "лучше",
    "условии",
    "случае",
    "того",
    "ради",
    "раз",
    "покуда",
    "пока",
    "после",
    "перед",
    "лишь",
    "едва",
    "чуть",
    "дабы",
    "кабы",
    "коли",
    "ежели",
    "абы",
    "вдруг",
    "иначе",
    "причём",
    "причем",
    "притом",
    "зато",
    "однако",
    "тогда",
    "сейчас",
    "теперь",
    "сначала",
    "сперва",
    "иногда",
    "часто",
    "всегда",
    "никогда",
    "т",
    "др",
];

/// Prepositions: a relative pronoun or `что` after one takes the comma before the preposition
/// instead (`дом, в котором`, `то, за что`).
const PREPOSITIONS: &[&str] = &[
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
    "подо",
    "над",
    "надо",
    "при",
    "про",
    "без",
    "безо",
    "для",
    "у",
    "через",
    "чрез",
    "между",
    "меж",
    "среди",
    "из-за",
    "из-под",
    "около",
    "возле",
    "вокруг",
    "мимо",
    "против",
    "вместо",
    "кроме",
    "сквозь",
    "вдоль",
    "внутри",
    "вне",
    "после",
    "перед",
    "пред",
    "ради",
    "благодаря",
    "согласно",
    "вопреки",
    "навстречу",
    "насчёт",
    "насчет",
    "ввиду",
    "вследствие",
    "посредством",
    "путём",
    "путем",
    "относительно",
    "касательно",
    "вслед",
    "ко",
    "подле",
    "близ",
    "внутрь",
    "поверх",
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Ru }
}

fn capitalize(w: &str) -> String {
    crate::affix::capitalize(w)
}

/// Grammar issues in a paragraph of Russian text, in order.
pub fn check(text: &str) -> Vec<Issue> {
    check_with(text, true)
}

/// [`check`] where `capital_first` says whether the text's first word must start a sentence with a capital
/// (false for a list item, which may be a fragment).
pub fn check_with(text: &str, capital_first: bool) -> Vec<Issue> {
    let ws: Vec<(usize, usize)> = words(text);
    let mut v = Vec::new();
    repeated_words(text, &ws, &mut v);
    spaces(text, &mut v);
    sentence_capitals(text, &ws, capital_first, &mut v);
    missing_commas(text, &ws, &mut v);
    doubled_punctuation(text, &mut v);
    v.sort_by_key(|i| (i.start, i.end));
    v
}

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(char::is_alphabetic)
            && w0.to_lowercase() == w1.to_lowercase()
            && !ALLOWED_REPEATS.contains(&w0.to_lowercase().as_str())
        {
            v.push(issue(*a0, *b1, format!("Повтор слова: «{w1}»"), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        if c != ' ' {
            continue;
        }
        let rest = text.get(i + 1..).unwrap_or("");
        let mut it = rest.chars();
        let Some(p) = it.next() else { continue };
        let after = it.next();
        let flag = match p {
            ',' => true,
            // `;)` and `;-)` are emoticons.
            ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
            // ` ...` is an ellipsis, ` .5` a number.
            '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
            // ` :)`, ` :-)`, ` :D` and similar are emoticons; ` ?!` is still a stray space.
            ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '»' | '”' | '’' | '!' | '?')),
            _ => false,
        };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "Лишний пробел перед знаком препинания".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Двойной пробел".into(), vec![" ".into()]));
    }
}

/// Is `prev`, before a period, an abbreviation, an initial, a number or a Roman numeral (after
/// which a lower-case word doesn't start a sentence)?
fn abbreviation_or_number(prev: &str) -> bool {
    prev.chars().count() <= 1
        || prev.chars().any(|c| c.is_ascii_digit())
        || (prev.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')))
        || ABBREVIATIONS.contains(&prev.to_lowercase().as_str())
}

const CLOSING: [char; 8] = ['"', '»', '”', '’', '\'', ')', ']', '\u{1}'];
const OPENING: [char; 8] = ['«', '"', '„', '“', '(', '[', '\'', '‚'];

/// Does the gap between two words end a sentence: a `.`, `!` or `?` (not an ellipsis, not after
/// an abbreviation), then space, with nothing but quotes and brackets around?
fn ends_sentence(gap: &str, prev: &str) -> bool {
    let t = gap.trim_start_matches(CLOSING);
    let Some(end) = t.chars().next() else { return false };
    if !matches!(end, '.' | '!' | '?') || t.starts_with("..") {
        return false;
    }
    if end == '.' && abbreviation_or_number(prev) {
        return false;
    }
    let rest = t.trim_start_matches(['.', '!', '?']).trim_start_matches(CLOSING);
    rest.starts_with(char::is_whitespace) && rest.trim_start().chars().all(|c| OPENING.contains(&c))
}

fn sentence_capitals(text: &str, ws: &[(usize, usize)], capital_first: bool, v: &mut Vec<Issue>) {
    // List items (`– яблоки,`) may start in lower case.
    let list_item = !capital_first || text.trim_end().ends_with([',', ';']);
    for (k, &(a, b)) in ws.iter().enumerate() {
        let Some(w) = text.get(a..b) else { continue };
        let starts = if k == 0 {
            let before = text.get(..a).unwrap_or("");
            !list_item && before.chars().all(|c| c.is_whitespace() || OPENING.contains(&c))
        } else {
            let Some(&(pa, pb)) = ws.get(k - 1) else { continue };
            ends_sentence(text.get(pb..a).unwrap_or(""), text.get(pa..pb).unwrap_or(""))
        };
        if starts && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            v.push(issue(a, b, "Предложение начинается со строчной буквы".into(), vec![capitalize(w)]));
        }
    }
}

fn missing_commas(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for k in 1..ws.len() {
        let (Some(&(pa, pb)), Some(&(a, b))) = (ws.get(k - 1), ws.get(k)) else { continue };
        let (Some(prev), Some(w), Some(gap)) = (text.get(pa..pb), text.get(a..b), text.get(pb..a)) else { continue };
        if gap.is_empty() || !gap.chars().all(|c| c == ' ' || c == '\u{a0}') || !prev.chars().all(char::is_alphabetic) {
            continue;
        }
        let lw = w.to_lowercase();
        let next = ws.get(k + 1).and_then(|&(na, nb)| text.get(na..nb)).map(str::to_lowercase);
        let next_gap_plain = ws.get(k + 1).and_then(|&(na, _)| text.get(b..na)).is_some_and(|g| g.chars().all(|c| c == ' ' || c == '\u{a0}'));
        // `потому что`, `оттого что`: the comma goes before the first word.
        let compound = matches!(lw.as_str(), "потому" | "оттого") && next.as_deref() == Some("что") && next_gap_plain;
        if !compound && !CLAUSE_OPENERS.contains(&lw.as_str()) {
            continue;
        }
        // `хотя бы` is a particle (`хотя бы раз`).
        if lw == "хотя" && next.as_deref() == Some("бы") {
            continue;
        }
        let p = prev.to_lowercase();
        let p = p.as_str();
        if NO_COMMA_AFTER.contains(&p) || PREPOSITIONS.contains(&p) {
            continue;
        }
        v.push(issue(pb, b, format!("Возможно, пропущена запятая перед «{w}»"), vec![format!(", {w}")]));
    }
}

fn doubled_punctuation(text: &str, v: &mut Vec<Issue>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if !matches!(b, b',' | b'.' | b';') {
            i += 1;
            continue;
        }
        let run = bytes.get(i..).map_or(0, |r| r.iter().take_while(|&&x| x == b).count());
        let flag = match b {
            b'.' => run == 2,
            _ => run >= 2,
        };
        if flag {
            let end = i + run;
            let suggestions: Vec<String> = match b {
                // `,,` before a word, after a space: a typed opening quote.
                b',' if text.get(end..).and_then(|t| t.chars().next()).is_some_and(char::is_alphabetic)
                    && (i == 0 || text.get(..i).is_some_and(|t| t.ends_with(char::is_whitespace))) =>
                {
                    vec!["«".into()]
                }
                b'.' => vec![".".into(), "…".into()],
                _ => vec![char::from(b).to_string()],
            };
            v.push(issue(i, end, "Повтор знака препинания".into(), suggestions));
        }
        i += run.max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(t: &str) -> Vec<String> {
        check(t).into_iter().map(|i| format!("{}@{}", i.message, &t[i.start..i.end])).collect()
    }

    fn clean(t: &str) {
        let m = messages(t);
        assert!(m.is_empty(), "{t:?}: {m:?}");
    }

    #[test]
    fn repeated_words() {
        let v = check("Я пошёл в в магазин и и купил хлеб.");
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[0].message, "Повтор слова: «в»");
        assert_eq!(v[0].suggestions, ["в"]);
        assert!(v.iter().all(|i| i.lang == ProofLang::Ru && i.kind == IssueKind::Grammar));
        clean("Да да, так так. Ха ха! Очень очень давно.");
    }

    #[test]
    fn spaces_and_punctuation() {
        let m = messages("Мы пришли , а он ушёл .  Правда ?");
        assert!(m.contains(&"Лишний пробел перед знаком препинания@ ,".to_string()), "{m:?}");
        assert!(m.contains(&"Лишний пробел перед знаком препинания@ .".to_string()), "{m:?}");
        assert!(m.contains(&"Лишний пробел перед знаком препинания@ ?".to_string()), "{m:?}");
        assert!(m.contains(&"Двойной пробел@  ".to_string()), "{m:?}");
        clean("Отлично :) Правда :-) Число .5 и многоточие ...");
        let m = messages("Он сказал ,,да» и ушёл.. Потом;; конец.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Повтор знака")).count(), 3, "{m:?}");
        assert_eq!(check("Он сказал ,,да».").iter().find(|i| i.message.starts_with("Повтор знака")).unwrap().suggestions, ["«"]);
        clean("Ждал… и ждал... до утра.");
    }

    #[test]
    fn sentence_start_capitals() {
        let m = messages("это первое предложение. а это второе! правда? да.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Предложение начинается")).count(), 4, "{m:?}");
        assert_eq!(check("мама мыла раму.")[0].suggestions, ["Мама"]);
        // Abbreviations, initials, numbers, ellipses and dialogue dashes.
        clean("Купили фрукты: яблоки, груши и т. д. и овощи.");
        clean("В 1990 г. открылась фирма, т. е. наша компания.");
        clean("Цена 500 руб. за штуку, адрес ул. Ленина, д. 5, кв. 3.");
        clean("Встретил А. С. Пушкина и проф. Иванова.");
        clean("Ждал... и наконец дождался.");
        clean("– Правда? – спросил он. – Да, – ответила она.");
        clean("Он сказал: «Вернусь». Потом ушёл.");
        clean("яблоки,");
    }

    #[test]
    fn missing_commas() {
        let m = messages(
            "Я знаю что он придёт. Это дом который построил Джек. Пришёл чтобы помочь. Ушёл потому что устал. Приходи если сможешь. Пошёл хотя устал.",
        );
        for want in [
            "Возможно, пропущена запятая перед «что»@ что",
            "Возможно, пропущена запятая перед «который»@ который",
            "Возможно, пропущена запятая перед «чтобы»@ чтобы",
            "Возможно, пропущена запятая перед «потому»@ потому",
            "Возможно, пропущена запятая перед «если»@ если",
            "Возможно, пропущена запятая перед «хотя»@ хотя",
        ] {
            assert!(m.contains(&want.to_string()), "{want}: {m:?}");
        }
        assert_eq!(check("Знаю что да.")[0].suggestions, [", что"]);
        // Conjunctions, particles, compound conjunctions and prepositions take no comma there.
        clean("Я знаю, что он придёт, и что останется.");
        clean("Он только что пришёл, так что всё в порядке.");
        clean("Это дом, в котором я живу, и сад, через который иду.");
        clean("Я сделаю это, даже если не успею, лишь бы ты был рад.");
        clean("Пришёл для того чтобы помочь, а не чтобы мешать.");
        clean("Ушёл, потому что устал, и не потому что обиделся.");
        clean("Расскажи хотя бы что-нибудь, за что тебя хвалят.");
        clean("Что ты делаешь? Который час?");
        clean("Кто-то что-то сказал.");
    }

    #[test]
    fn correct_russian_text_is_clean() {
        clean("Съешь же ещё этих мягких французских булок, да выпей чаю. Мы пришли, когда уже стемнело.");
        clean("Книга, которую я читаю, очень интересная, поэтому я её советую.");
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check(&s);
        }

        #[test]
        fn never_panics_on_russian_like_text(s in "[а-яёА-ЯЁ ,.;:!?«»…\\-]{0,60}") {
            for i in check(&s) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(i.start) && s.is_char_boundary(i.end));
            }
        }
    }
}
