//! Language-aware sorting of text: a small, pure-Rust collation.
//!
//! Every letter gets a *primary* weight (its place in the alphabet) and a *secondary* one (its
//! diacritic). Strings compare by primary weights first, then by secondary ones, so `é` sorts
//! with `e` (`resume` < `résumé` < `rex`) unless the language makes it a letter of its own:
//! Polish puts `ą` right after `a` and `ż` after `ź` after `z`, Swedish puts `å ä ö` after `z`,
//! Czech puts `ch` after `h`, Spanish `ñ` after `n`, Hungarian `cs`, `gy`, `sz`… after their
//! first letter, Vietnamese `ă â` after `a`, `đ` after `d`, `ô ơ` after `o`, `ch kh ng nh…` after
//! their first letter, with the tones as accents. The per-language tables follow the published alphabets of those languages.
//! Text without diacritics sorts exactly as a case-insensitive code-point compare.

use std::cmp::Ordering;

/// The diacritic of a precomposed lowercase Latin letter: `(base letters, mark)`. Marks: 1 acute,
/// 2 grave, 3 circumflex, 4 diaeresis, 5 tilde, 6 ring, 7 caron, 8 cedilla, 9 ogonek, 10 dot,
/// 11 stroke, 12 breve, 13 macron, 14 double acute, 15 comma below, 16 ligature, 17 dotless,
/// 18 eth.
const DECOMP: &[(char, &str, u16)] = &[
    ('à', "a", 2),
    ('á', "a", 1),
    ('â', "a", 3),
    ('ã', "a", 5),
    ('ä', "a", 4),
    ('å', "a", 6),
    ('ā', "a", 13),
    ('ă', "a", 12),
    ('ą', "a", 9),
    ('æ', "ae", 16),
    ('ç', "c", 8),
    ('ć', "c", 1),
    ('ĉ', "c", 3),
    ('ċ', "c", 10),
    ('č', "c", 7),
    ('ď', "d", 7),
    ('đ', "d", 11),
    ('ð', "d", 18),
    ('è', "e", 2),
    ('é', "e", 1),
    ('ê', "e", 3),
    ('ë', "e", 4),
    ('ē', "e", 13),
    ('ĕ', "e", 12),
    ('ė', "e", 10),
    ('ę', "e", 9),
    ('ě', "e", 7),
    ('ĝ', "g", 3),
    ('ğ', "g", 12),
    ('ġ', "g", 10),
    ('ģ', "g", 8),
    ('ĥ', "h", 3),
    ('ħ', "h", 11),
    ('ì', "i", 2),
    ('í', "i", 1),
    ('î', "i", 3),
    ('ï', "i", 4),
    ('ĩ', "i", 5),
    ('ī', "i", 13),
    ('ĭ', "i", 12),
    ('į', "i", 9),
    ('ı', "i", 17),
    ('ĳ', "ij", 16),
    ('ĵ', "j", 3),
    ('ķ', "k", 8),
    ('ĺ', "l", 1),
    ('ļ', "l", 8),
    ('ľ', "l", 7),
    ('ŀ', "l", 10),
    ('ł', "l", 11),
    ('ñ', "n", 5),
    ('ń', "n", 1),
    ('ņ', "n", 8),
    ('ň', "n", 7),
    ('ò', "o", 2),
    ('ó', "o", 1),
    ('ô', "o", 3),
    ('õ', "o", 5),
    ('ö', "o", 4),
    ('ø', "o", 11),
    ('ō', "o", 13),
    ('ŏ', "o", 12),
    ('ő', "o", 14),
    ('œ', "oe", 16),
    ('ŕ', "r", 1),
    ('ŗ', "r", 8),
    ('ř', "r", 7),
    ('ś', "s", 1),
    ('ŝ', "s", 3),
    ('ş', "s", 8),
    ('š', "s", 7),
    ('ș', "s", 15),
    ('ß', "ss", 16),
    ('ţ', "t", 8),
    ('ť', "t", 7),
    ('ŧ', "t", 11),
    ('ț', "t", 15),
    ('ù', "u", 2),
    ('ú', "u", 1),
    ('û', "u", 3),
    ('ü', "u", 4),
    ('ũ', "u", 5),
    ('ū', "u", 13),
    ('ŭ', "u", 12),
    ('ů', "u", 6),
    ('ű', "u", 14),
    ('ų', "u", 9),
    ('ŵ', "w", 3),
    ('ý', "y", 1),
    ('ÿ', "y", 4),
    ('ŷ', "y", 3),
    ('ź', "z", 1),
    ('ż', "z", 10),
    ('ž', "z", 7),
];

/// A letter (or letter group) a language sorts as a letter of its own: right after `after`
/// (`rank` places after it), with secondary weight `sec` among letters of the same place.
struct Tailor {
    seq: &'static str,
    after: char,
    rank: u32,
    sec: u16,
}

const fn t(seq: &'static str, after: char, rank: u32) -> Tailor {
    Tailor { seq, after, rank, sec: 0 }
}

const fn ts(seq: &'static str, after: char, rank: u32, sec: u16) -> Tailor {
    Tailor { seq, after, rank, sec }
}

const PL: &[Tailor] =
    &[t("ą", 'a', 1), t("ć", 'c', 1), t("ę", 'e', 1), t("ł", 'l', 1), t("ń", 'n', 1), t("ó", 'o', 1), t("ś", 's', 1), t("ź", 'z', 1), t("ż", 'z', 2)];
const CS: &[Tailor] = &[t("č", 'c', 1), t("ch", 'h', 1), t("ř", 'r', 1), t("š", 's', 1), t("ž", 'z', 1)];
const SK: &[Tailor] = &[t("ä", 'a', 1), t("č", 'c', 1), t("ch", 'h', 1), t("ô", 'o', 1), t("š", 's', 1), t("ž", 'z', 1)];
const HU: &[Tailor] = &[
    t("cs", 'c', 1),
    t("dz", 'd', 1),
    t("dzs", 'd', 2),
    t("gy", 'g', 1),
    t("ly", 'l', 1),
    t("ny", 'n', 1),
    t("ö", 'o', 1),
    ts("ő", 'o', 1, 14),
    t("sz", 's', 1),
    t("ty", 't', 1),
    t("ü", 'u', 1),
    ts("ű", 'u', 1, 14),
    t("zs", 'z', 1),
];
const SV: &[Tailor] = &[t("å", 'z', 1), t("ä", 'z', 2), ts("æ", 'z', 2, 16), t("ö", 'z', 3), ts("ø", 'z', 3, 11)];
const DA: &[Tailor] = &[t("æ", 'z', 1), ts("ä", 'z', 1, 4), t("ø", 'z', 2), ts("ö", 'z', 2, 4), t("å", 'z', 3)];
const ES: &[Tailor] = &[t("ñ", 'n', 1)];
// Russian ё shares е's primary weight; the diaeresis breaks ties (CLDR Russian standard).
const RU: &[Tailor] = &[ts("ё", 'е', 0, 4)];
const TR: &[Tailor] = &[t("ç", 'c', 1), t("ğ", 'g', 1), t("ı", 'h', 1), t("ö", 'o', 1), t("ş", 's', 1), t("ü", 'u', 1)];
const HR: &[Tailor] =
    &[t("č", 'c', 1), t("ć", 'c', 2), t("dž", 'd', 1), t("đ", 'd', 2), t("lj", 'l', 1), t("nj", 'n', 1), t("š", 's', 1), t("ž", 'z', 1)];
const RO: &[Tailor] = &[t("ă", 'a', 1), t("â", 'a', 2), t("î", 'i', 1), t("ș", 's', 1), ts("ş", 's', 1, 8), t("ț", 't', 1), ts("ţ", 't', 1, 8)];
const ET: &[Tailor] = &[t("š", 's', 1), t("z", 's', 2), t("ž", 's', 3), t("õ", 'w', 1), t("ä", 'w', 2), t("ö", 'w', 3), t("ü", 'w', 4)];
const LT: &[Tailor] = &[t("č", 'c', 1), t("š", 's', 1), t("ž", 'z', 1)];
const LV: &[Tailor] = &[t("č", 'c', 1), t("ģ", 'g', 1), t("ķ", 'k', 1), t("ļ", 'l', 1), t("ņ", 'n', 1), t("š", 's', 1), t("ž", 'z', 1)];
const IS: &[Tailor] = &[
    t("á", 'a', 1),
    t("ð", 'd', 1),
    t("é", 'e', 1),
    t("í", 'i', 1),
    t("ó", 'o', 1),
    t("ú", 'u', 1),
    t("ý", 'y', 1),
    t("þ", 'z', 1),
    t("æ", 'z', 2),
    t("ö", 'z', 3),
];
// Vietnamese digraphs are letters of their own after their first letter, as Word sorts them
// (`ca co cu chi`, `ta tu tha tra`, `na no nga nha`, `ga gha gu gi`); CLDR `vi` sorts them letter by
// letter. A tone on the `i` of `gi` stays a tone of that letter (`gì` files under `gi`), in
// `VI_VOWELS` order.
const VI: &[Tailor] = &[
    t("ch", 'c', 1),
    t("đ", 'd', 1),
    t("gi", 'g', 1),
    ts("gì", 'g', 1, 1),
    ts("gỉ", 'g', 1, 2),
    ts("gĩ", 'g', 1, 3),
    ts("gí", 'g', 1, 4),
    ts("gị", 'g', 1, 5),
    t("kh", 'k', 1),
    t("ng", 'n', 1),
    t("nh", 'n', 2),
    t("ph", 'p', 1),
    t("th", 't', 1),
    t("tr", 't', 2),
];

/// Vietnamese vowels (CLDR `vi`): the Latin letter each one sorts after, its rank there, and the
/// letter followed by its toned forms in dictionary order of the tones: grave (huyền), hook above
/// (hỏi), tilde (ngã), acute (sắc), dot below (nặng). The tone is an accent: `ma` < `mà` < `mả`
/// < `mã` < `má` < `mạ`, and all of them sort before `mai`.
const VI_VOWELS: &[(char, u16, &str)] = &[
    ('a', 0, "aàảãáạ"),
    ('a', 1, "ăằẳẵắặ"),
    ('a', 2, "âầẩẫấậ"),
    ('e', 0, "eèẻẽéẹ"),
    ('e', 1, "êềểễếệ"),
    ('i', 0, "iìỉĩíị"),
    ('o', 0, "oòỏõóọ"),
    ('o', 1, "ôồổỗốộ"),
    ('o', 2, "ơờởỡớợ"),
    ('u', 0, "uùủũúụ"),
    ('u', 1, "ưừửữứự"),
    ('y', 0, "yỳỷỹýỵ"),
];

/// Where languages other than Vietnamese put a toned or modified Vietnamese vowel among the accents
/// of its Latin letter: after the [`DECOMP`] marks and the combining marks.
const VI_FOLD: u16 = 160;

/// A lowercase Vietnamese vowel: `(Latin letter, rank after it, the letter without its tone, tone)`
/// with tone 0 for none and 1–5 in the order of [`VI_VOWELS`].
fn vi_vowel(c: char) -> Option<(char, u16, &'static str, u16)> {
    VI_VOWELS.iter().find_map(|(after, rank, forms)| {
        let tone = forms.chars().position(|x| x == c)?;
        let letter = forms.get(..forms.chars().next()?.len_utf8())?;
        Some((*after, *rank, letter, u16::try_from(tone).ok()?))
    })
}

/// The primary language subtag of a BCP 47 tag (`pl-PL` → `pl`), lowercased.
pub fn language(lang: Option<&str>) -> String {
    lang.and_then(|l| l.split(['-', '_']).next()).unwrap_or("").trim().to_ascii_lowercase()
}

fn tailoring(lang: Option<&str>) -> &'static [Tailor] {
    match language(lang).as_str() {
        "pl" => PL,
        "cs" => CS,
        "sk" => SK,
        "hu" => HU,
        "sv" | "fi" => SV,
        "da" | "nb" | "nn" | "no" => DA,
        "es" | "gl" => ES,
        "ru" => RU,
        "tr" | "az" => TR,
        "hr" | "bs" | "sr" => HR,
        "ro" => RO,
        "et" => ET,
        "lt" => LT,
        "lv" => LV,
        "is" => IS,
        "sl" => LT,
        "vi" => VI,
        _ => &[],
    }
}

/// A lowercase Latin or Russian letter followed by a combining mark, and the single letter Unicode
/// composes them to (NFC; generated from the Unicode Character Database). Text pasted from
/// decomposed sources (macOS file names: `z` + U+0307) collates like the composed letter.
const COMPOSE: &[(char, char, char)] = &[
    ('a', '\u{300}', 'à'),
    ('a', '\u{301}', 'á'),
    ('a', '\u{302}', 'â'),
    ('a', '\u{303}', 'ã'),
    ('a', '\u{304}', 'ā'),
    ('a', '\u{306}', 'ă'),
    ('a', '\u{307}', 'ȧ'),
    ('a', '\u{308}', 'ä'),
    ('a', '\u{30a}', 'å'),
    ('a', '\u{30c}', 'ǎ'),
    ('a', '\u{328}', 'ą'),
    ('b', '\u{307}', 'ḃ'),
    ('c', '\u{301}', 'ć'),
    ('c', '\u{302}', 'ĉ'),
    ('c', '\u{307}', 'ċ'),
    ('c', '\u{30c}', 'č'),
    ('c', '\u{327}', 'ç'),
    ('d', '\u{307}', 'ḋ'),
    ('d', '\u{30c}', 'ď'),
    ('d', '\u{327}', 'ḑ'),
    ('e', '\u{300}', 'è'),
    ('e', '\u{301}', 'é'),
    ('e', '\u{302}', 'ê'),
    ('e', '\u{303}', 'ẽ'),
    ('e', '\u{304}', 'ē'),
    ('e', '\u{306}', 'ĕ'),
    ('e', '\u{307}', 'ė'),
    ('e', '\u{308}', 'ë'),
    ('e', '\u{30c}', 'ě'),
    ('e', '\u{327}', 'ȩ'),
    ('e', '\u{328}', 'ę'),
    ('f', '\u{307}', 'ḟ'),
    ('g', '\u{301}', 'ǵ'),
    ('g', '\u{302}', 'ĝ'),
    ('g', '\u{304}', 'ḡ'),
    ('g', '\u{306}', 'ğ'),
    ('g', '\u{307}', 'ġ'),
    ('g', '\u{30c}', 'ǧ'),
    ('g', '\u{327}', 'ģ'),
    ('h', '\u{302}', 'ĥ'),
    ('h', '\u{307}', 'ḣ'),
    ('h', '\u{308}', 'ḧ'),
    ('h', '\u{30c}', 'ȟ'),
    ('h', '\u{327}', 'ḩ'),
    ('i', '\u{300}', 'ì'),
    ('i', '\u{301}', 'í'),
    ('i', '\u{302}', 'î'),
    ('i', '\u{303}', 'ĩ'),
    ('i', '\u{304}', 'ī'),
    ('i', '\u{306}', 'ĭ'),
    ('i', '\u{308}', 'ï'),
    ('i', '\u{30c}', 'ǐ'),
    ('i', '\u{328}', 'į'),
    ('j', '\u{302}', 'ĵ'),
    ('j', '\u{30c}', 'ǰ'),
    ('k', '\u{301}', 'ḱ'),
    ('k', '\u{30c}', 'ǩ'),
    ('k', '\u{327}', 'ķ'),
    ('l', '\u{301}', 'ĺ'),
    ('l', '\u{30c}', 'ľ'),
    ('l', '\u{327}', 'ļ'),
    ('m', '\u{301}', 'ḿ'),
    ('m', '\u{307}', 'ṁ'),
    ('n', '\u{300}', 'ǹ'),
    ('n', '\u{301}', 'ń'),
    ('n', '\u{303}', 'ñ'),
    ('n', '\u{307}', 'ṅ'),
    ('n', '\u{30c}', 'ň'),
    ('n', '\u{327}', 'ņ'),
    ('o', '\u{300}', 'ò'),
    ('o', '\u{301}', 'ó'),
    ('o', '\u{302}', 'ô'),
    ('o', '\u{303}', 'õ'),
    ('o', '\u{304}', 'ō'),
    ('o', '\u{306}', 'ŏ'),
    ('o', '\u{307}', 'ȯ'),
    ('o', '\u{308}', 'ö'),
    ('o', '\u{30b}', 'ő'),
    ('o', '\u{30c}', 'ǒ'),
    ('o', '\u{328}', 'ǫ'),
    ('p', '\u{301}', 'ṕ'),
    ('p', '\u{307}', 'ṗ'),
    ('r', '\u{301}', 'ŕ'),
    ('r', '\u{307}', 'ṙ'),
    ('r', '\u{30c}', 'ř'),
    ('r', '\u{327}', 'ŗ'),
    ('s', '\u{301}', 'ś'),
    ('s', '\u{302}', 'ŝ'),
    ('s', '\u{307}', 'ṡ'),
    ('s', '\u{30c}', 'š'),
    ('s', '\u{326}', 'ș'),
    ('s', '\u{327}', 'ş'),
    ('t', '\u{307}', 'ṫ'),
    ('t', '\u{308}', 'ẗ'),
    ('t', '\u{30c}', 'ť'),
    ('t', '\u{326}', 'ț'),
    ('t', '\u{327}', 'ţ'),
    ('u', '\u{300}', 'ù'),
    ('u', '\u{301}', 'ú'),
    ('u', '\u{302}', 'û'),
    ('u', '\u{303}', 'ũ'),
    ('u', '\u{304}', 'ū'),
    ('u', '\u{306}', 'ŭ'),
    ('u', '\u{308}', 'ü'),
    ('u', '\u{30a}', 'ů'),
    ('u', '\u{30b}', 'ű'),
    ('u', '\u{30c}', 'ǔ'),
    ('u', '\u{328}', 'ų'),
    ('v', '\u{303}', 'ṽ'),
    ('w', '\u{300}', 'ẁ'),
    ('w', '\u{301}', 'ẃ'),
    ('w', '\u{302}', 'ŵ'),
    ('w', '\u{307}', 'ẇ'),
    ('w', '\u{308}', 'ẅ'),
    ('w', '\u{30a}', 'ẘ'),
    ('x', '\u{307}', 'ẋ'),
    ('x', '\u{308}', 'ẍ'),
    ('y', '\u{300}', 'ỳ'),
    ('y', '\u{301}', 'ý'),
    ('y', '\u{302}', 'ŷ'),
    ('y', '\u{303}', 'ỹ'),
    ('y', '\u{304}', 'ȳ'),
    ('y', '\u{307}', 'ẏ'),
    ('y', '\u{308}', 'ÿ'),
    ('y', '\u{30a}', 'ẙ'),
    ('z', '\u{301}', 'ź'),
    ('z', '\u{302}', 'ẑ'),
    ('z', '\u{307}', 'ż'),
    ('z', '\u{30c}', 'ž'),
    // Vietnamese: the hook above, dot below and horn, then the tones of `ă â ê ô ơ ư`. Decomposed
    // text puts a dot below before a circumflex or breve (`ậ` is `a` U+0323 U+0302), typed text
    // often after it.
    ('a', '\u{309}', 'ả'),
    ('a', '\u{323}', 'ạ'),
    ('e', '\u{309}', 'ẻ'),
    ('e', '\u{323}', 'ẹ'),
    ('i', '\u{309}', 'ỉ'),
    ('i', '\u{323}', 'ị'),
    ('o', '\u{309}', 'ỏ'),
    ('o', '\u{31b}', 'ơ'),
    ('o', '\u{323}', 'ọ'),
    ('u', '\u{309}', 'ủ'),
    ('u', '\u{31b}', 'ư'),
    ('u', '\u{323}', 'ụ'),
    ('y', '\u{309}', 'ỷ'),
    ('y', '\u{323}', 'ỵ'),
    ('ạ', '\u{302}', 'ậ'),
    ('ạ', '\u{306}', 'ặ'),
    ('ẹ', '\u{302}', 'ệ'),
    ('ọ', '\u{302}', 'ộ'),
    ('ă', '\u{300}', 'ằ'),
    ('ă', '\u{309}', 'ẳ'),
    ('ă', '\u{303}', 'ẵ'),
    ('ă', '\u{301}', 'ắ'),
    ('ă', '\u{323}', 'ặ'),
    ('â', '\u{300}', 'ầ'),
    ('â', '\u{309}', 'ẩ'),
    ('â', '\u{303}', 'ẫ'),
    ('â', '\u{301}', 'ấ'),
    ('â', '\u{323}', 'ậ'),
    ('ê', '\u{300}', 'ề'),
    ('ê', '\u{309}', 'ể'),
    ('ê', '\u{303}', 'ễ'),
    ('ê', '\u{301}', 'ế'),
    ('ê', '\u{323}', 'ệ'),
    ('ô', '\u{300}', 'ồ'),
    ('ô', '\u{309}', 'ổ'),
    ('ô', '\u{303}', 'ỗ'),
    ('ô', '\u{301}', 'ố'),
    ('ô', '\u{323}', 'ộ'),
    ('ơ', '\u{300}', 'ờ'),
    ('ơ', '\u{309}', 'ở'),
    ('ơ', '\u{303}', 'ỡ'),
    ('ơ', '\u{301}', 'ớ'),
    ('ơ', '\u{323}', 'ợ'),
    ('ư', '\u{300}', 'ừ'),
    ('ư', '\u{309}', 'ử'),
    ('ư', '\u{303}', 'ữ'),
    ('ư', '\u{301}', 'ứ'),
    ('ư', '\u{323}', 'ự'),
    ('е', '\u{308}', 'ё'),
    ('и', '\u{306}', 'й'),
];

/// `s` lowercased, with each letter + combining mark that has a single-letter form composed.
fn lower_composed(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        if let Some(prev) = out.chars().next_back()
            && let Some((_, _, composed)) = COMPOSE.iter().find(|(b, m, _)| *b == prev && *m == c)
        {
            out.pop();
            out.push(*composed);
        } else {
            out.push(c);
        }
    }
    out
}

fn weight(c: char, rank: u32) -> u32 {
    ((c as u32) << 4).saturating_add(rank)
}

/// One collation element: primary weight, secondary weight, the text it covers.
struct Elem<'a> {
    primary: Vec<u32>,
    sec: u16,
    text: &'a str,
    /// Set when the text is a letter of its own in the language (shown as is in headings).
    own: bool,
}

/// The next collation element of `rest` (lowercase), and its length in bytes. `vi` sorts
/// Vietnamese vowels as Vietnamese does.
fn next_elem<'a>(rest: &'a str, rules: &[Tailor], vi: bool) -> Option<(Elem<'a>, usize)> {
    if let Some(r) = rules.iter().filter(|r| rest.starts_with(r.seq)).max_by_key(|r| r.seq.len()) {
        let mut text = rest.get(..r.seq.len())?;
        // A toned Vietnamese letter shows without its tone: `gì` files under `Gi`.
        if vi
            && r.sec > 0
            && let Some(base) = rules.iter().find(|b| b.after == r.after && b.rank == r.rank && b.sec == 0)
        {
            text = base.seq;
        }
        return Some((Elem { primary: vec![weight(r.after, r.rank)], sec: r.sec, text, own: true }, r.seq.len()));
    }
    let c = rest.chars().next()?;
    let n = c.len_utf8();
    let text = rest.get(..n)?;
    let vowel = vi_vowel(c);
    if vi {
        // The text is the letter without its tone: `ấm` files under `Â` in an index.
        if let Some((after, rank, letter, tone)) = vowel {
            return Some((Elem { primary: vec![weight(after, u32::from(rank))], sec: tone, text: letter, own: true }, n));
        }
    }
    if let Some((_, base, mark)) = DECOMP.iter().find(|(x, _, _)| *x == c) {
        return Some((Elem { primary: base.chars().map(|b| weight(b, 0)).collect(), sec: *mark, text, own: false }, n));
    }
    // A combining mark adds a secondary difference to the letter before it.
    if ('\u{300}'..='\u{36f}').contains(&c) {
        return Some((Elem { primary: Vec::new(), sec: 32 + (c as u32 - 0x300) as u16, text, own: false }, n));
    }
    // Other languages sort a Vietnamese vowel with its Latin letter, as an accented form of it.
    if let Some((after, rank, _, tone)) = vowel.filter(|(_, rank, _, tone)| *rank > 0 || *tone > 0) {
        return Some((Elem { primary: vec![weight(after, 0)], sec: VI_FOLD + rank * 8 + tone, text, own: false }, n));
    }
    Some((Elem { primary: vec![weight(c, 0)], sec: 0, text, own: false }, n))
}

/// A sort key: compare keys to sort strings in a language's order. Equal keys mean the strings
/// differ at most in letter case.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SortKey {
    primary: Vec<u32>,
    secondary: Vec<u16>,
    text: String,
}

/// The sort key of `s` for language `lang` (a BCP 47 tag such as `pl-PL`; `None` sorts like
/// English).
pub fn sort_key(s: &str, lang: Option<&str>) -> SortKey {
    let text = lower_composed(s);
    let rules = tailoring(lang);
    let vi = language(lang) == "vi";
    let mut key = SortKey::default();
    let mut i = 0;
    while let Some(rest) = text.get(i..).filter(|r| !r.is_empty()) {
        let Some((e, n)) = next_elem(rest, rules, vi) else { break };
        i += n.max(1);
        if e.primary.is_empty() {
            if let Some(last) = key.secondary.last_mut() {
                *last = last.saturating_add(e.sec);
            }
            continue;
        }
        for (k, w) in e.primary.iter().enumerate() {
            key.primary.push(*w);
            key.secondary.push(if k == 0 { e.sec } else { 0 });
        }
    }
    key.text = text;
    key
}

/// Compare two strings in the order of language `lang`, ignoring case.
pub fn collate(a: &str, b: &str, lang: Option<&str>) -> Ordering {
    sort_key(a, lang).cmp(&sort_key(b, lang))
}

/// The index heading letter `s` files under in language `lang`: `Ł` for `Łódź` in Polish, `L`
/// in English; `Ch` for `chata` in Czech; `A` for `Äpfel` in German, `Ä` in Swedish; `Â` for
/// `ấm` in Vietnamese, `A` in English.
pub fn heading_letter(s: &str, lang: Option<&str>) -> Option<String> {
    let low = lower_composed(s.trim_start());
    let (e, _) = next_elem(&low, tailoring(lang), language(lang) == "vi")?;
    let shown = if e.own || e.sec == 0 {
        e.text.to_string()
    } else {
        DECOMP
            .iter()
            .find(|(x, _, _)| e.text.starts_with(*x))
            .map(|(_, b, _)| b.chars().take(1).collect())
            .or_else(|| e.text.chars().next().and_then(vi_vowel).map(|(after, ..)| after.to_string()))
            .unwrap_or_else(|| e.text.to_string())
    };
    let mut cs = shown.chars();
    let first = cs.next()?;
    Some(first.to_uppercase().chain(cs).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(words: &[&str], lang: &str) -> Vec<String> {
        let mut v: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        v.sort_by(|a, b| collate(a, b, Some(lang)));
        v
    }

    #[test]
    fn polish_letters_follow_their_base_letter() {
        assert_eq!(sorted(&["Zakopane", "Łódź", "Kraków"], "pl-PL"), ["Kraków", "Łódź", "Zakopane"]);
        assert_eq!(sorted(&["zebra", "ćma", "dom"], "pl-PL"), ["ćma", "dom", "zebra"]);
        assert_eq!(sorted(&["żaba", "źle", "zebra", "ząb", "lato", "łapa", "mama"], "pl"), ["lato", "łapa", "mama", "ząb", "zebra", "źle", "żaba"]);
        assert_eq!(sorted(&["b", "ą", "a"], "pl"), ["a", "ą", "b"]);
        // Every Polish letter is a letter of its own, after its base letter (review of #844).
        assert_eq!(sorted(&["Malbork", "Łódź", "Lublin"], "pl"), ["Lublin", "Łódź", "Malbork"]);
        assert_eq!(sorted(&["ósmy", "owca", "pies"], "pl"), ["owca", "ósmy", "pies"]);
        assert_eq!(sorted(&["ściana", "sum", "tak"], "pl"), ["sum", "ściana", "tak"]);
        assert_eq!(sorted(&["ęsi", "ezop", "fala"], "pl"), ["ezop", "ęsi", "fala"]);
        assert_eq!(sorted(&["ńa", "nz", "oko"], "pl"), ["nz", "ńa", "oko"]);
        assert_eq!(heading_letter("ósmy", Some("pl")).as_deref(), Some("Ó"));
        assert_eq!(heading_letter("ńa", Some("pl")).as_deref(), Some("Ń"));
        // Decomposed letters (`z` + combining dot above, as macOS file names write `ż`) sort like
        // the composed ones.
        assert_eq!(collate("z\u{307}aba", "żaba", Some("pl-PL")), Ordering::Equal);
        assert_eq!(collate("z\u{307}aba", "zebra", Some("pl-PL")), Ordering::Greater);
        assert_eq!(heading_letter("Z\u{307}aba", Some("pl")).as_deref(), Some("Ż"));
        assert_eq!(collate("a\u{328}", "ą", Some("pl")), Ordering::Equal);
    }

    #[test]
    fn swedish_puts_a_ring_and_umlauts_after_z() {
        assert_eq!(sorted(&["ödla", "zebra", "äpple", "åka", "apa"], "sv-SE"), ["apa", "zebra", "åka", "äpple", "ödla"]);
    }

    #[test]
    fn german_sorts_umlauts_with_their_base_letter() {
        assert_eq!(sorted(&["Zug", "Äpfel", "Apfel", "Bach"], "de-DE"), ["Apfel", "Äpfel", "Bach", "Zug"]);
        assert_eq!(sorted(&["Straße", "Strasse", "Strand"], "de"), ["Strand", "Strasse", "Straße"]);
    }

    #[test]
    fn russian_yo_sorts_with_e_instead_of_after_ya() {
        for lang in ["ru", "ru-RU", "RU_ru"] {
            assert_eq!(sorted(&["яблоко", "ёлка", "жук", "ель", "ёж", "еж"], lang), ["еж", "ёж", "ёлка", "ель", "жук", "яблоко"]);
            assert_eq!(collate("ЁЛКА", "ёлка", Some(lang)), Ordering::Equal);
            assert_eq!(heading_letter("Ёлка", Some(lang)).as_deref(), Some("Ё"));
        }
    }

    #[test]
    fn russian_decomposed_letters_match_their_composed_forms() {
        for (composed, decomposed) in [("ёлка", "е\u{308}лка"), ("Ёлка", "Е\u{308}лка"), ("йод", "и\u{306}од"), ("Йод", "И\u{306}од")]
        {
            assert_eq!(collate(composed, decomposed, Some("ru-RU")), Ordering::Equal);
            assert_eq!(heading_letter(composed, Some("ru")), heading_letter(decomposed, Some("ru")));
        }
        assert_eq!(sorted(&["к", "и\u{306}", "и"], "ru"), ["и", "и\u{306}", "к"]);
    }

    #[test]
    fn spanish_n_tilde_after_n() {
        assert_eq!(sorted(&["oso", "ñu", "nube"], "es-ES"), ["nube", "ñu", "oso"]);
    }

    #[test]
    fn czech_slovak_hungarian_alphabets() {
        assert_eq!(sorted(&["chata", "hrad", "ihla", "cesta", "čaj", "dům"], "cs-CZ"), ["cesta", "čaj", "dům", "hrad", "chata", "ihla"]);
        assert_eq!(sorted(&["ôsmy", "oko", "pes", "ä", "b"], "sk"), ["ä", "b", "oko", "ôsmy", "pes"]);
        assert_eq!(sorted(&["csak", "cukor", "dob", "szó", "sűrű", "tál"], "hu-HU"), ["cukor", "csak", "dob", "sűrű", "szó", "tál"]);
        // ö and ő are one letter after o, told apart by the accent only.
        assert_eq!(sorted(&["ötös", "őr", "pálya", "olló", "ör"], "hu"), ["olló", "ör", "őr", "ötös", "pálya"]);
    }

    #[test]
    fn english_keeps_case_insensitive_order_and_folds_accents() {
        assert_eq!(sorted(&["rex", "résumé", "resume", "Apple", "banana"], "en-US"), ["Apple", "banana", "resume", "résumé", "rex"]);
        assert_eq!(collate("Łódź", "Lodz", None), Ordering::Greater);
        assert_eq!(collate("Łódź", "Lodzia", None), Ordering::Less);
        assert_eq!(collate("ABC", "abc", None), Ordering::Equal);
    }

    #[test]
    fn vietnamese_alphabet_and_tones() {
        // #4317: Vietnamese letters sorted after `z`.
        assert_eq!(sorted(&["An", "Dung", "Khoa", "Kim", "Ăn", "Đang", "Ẩn"], "vi-VN"), ["An", "Ăn", "Ẩn", "Dung", "Đang", "Kim", "Khoa"]);
        let alphabet = [
            "a", "ă", "â", "b", "c", "d", "đ", "e", "ê", "g", "h", "i", "k", "l", "m", "n", "o", "ô", "ơ", "p", "q", "r", "s", "t", "u", "ư", "v",
            "x", "y",
        ];
        let mut shuffled = alphabet;
        shuffled.reverse();
        assert_eq!(sorted(&shuffled, "vi"), alphabet);
        // Tones are accents, in dictionary order: none, grave, hook, tilde, acute, dot below.
        assert_eq!(sorted(&["mạ", "má", "mã", "mả", "mà", "ma"], "vi"), ["ma", "mà", "mả", "mã", "má", "mạ"]);
        assert_eq!(sorted(&["mát", "màn", "mạ"], "vi"), ["mạ", "màn", "mát"]);
        assert_eq!(sorted(&["ba", "ấm", "âm", "ăn", "an"], "vi"), ["an", "ăn", "âm", "ấm", "ba"]);
        assert_eq!(sorted(&["ước", "ung", "ở", "ôm", "ổn", "ong"], "vi"), ["ong", "ôm", "ổn", "ở", "ung", "ước"]);
        assert_eq!(collate("ĐẶNG", "đặng", Some("vi")), Ordering::Equal);
    }

    #[test]
    fn vietnamese_digraphs_follow_their_first_letter_as_in_word() {
        // Word's Vietnamese sort, observed in review of #4483; `gh` is not a letter of its own.
        assert_eq!(sorted(&["chi", "cu", "co", "ca"], "vi"), ["ca", "co", "cu", "chi"]);
        assert_eq!(sorted(&["kha", "ko", "ki", "ka"], "vi"), ["ka", "ki", "ko", "kha"]);
        assert_eq!(sorted(&["pha", "po", "pi", "pa"], "vi"), ["pa", "pi", "po", "pha"]);
        assert_eq!(sorted(&["tra", "tha", "tu", "to", "ta"], "vi"), ["ta", "to", "tu", "tha", "tra"]);
        assert_eq!(sorted(&["nha", "nga", "no", "ni", "na"], "vi"), ["na", "ni", "no", "nga", "nha"]);
        assert_eq!(sorted(&["gi", "gu", "go", "gha", "ga"], "vi"), ["ga", "gha", "go", "gu", "gi"]);
        assert_eq!(heading_letter("Khoa", Some("vi")).as_deref(), Some("Kh"));
        assert_eq!(heading_letter("nghĩa", Some("vi")).as_deref(), Some("Ng"));
        // A tone on the `i` of `gi` is a tone of the letter `gi` (review of #4483).
        assert_eq!(sorted(&["gia", "gí", "gu", "gì", "gi", "ga"], "vi"), ["ga", "gu", "gi", "gì", "gí", "gia"]);
        assert_eq!(heading_letter("gì", Some("vi")).as_deref(), Some("Gi"));
        assert_eq!(heading_letter("Gị", Some("vi")).as_deref(), Some("Gi"));
        assert_eq!(collate("gi\u{300}", "gì", Some("vi")), Ordering::Equal);
        // Other languages keep letter-by-letter order.
        assert_eq!(sorted(&["Kim", "Khoa"], "en"), ["Khoa", "Kim"]);
    }

    #[test]
    fn vietnamese_decomposed_letters_match_their_composed_forms() {
        // A toneless letter + combining tone (Windows "Unicode tổ hợp" input), and full NFD, where
        // a dot below comes before the circumflex and after the horn.
        for (composed, decomposed) in [
            ("ấm", "â\u{301}m"),
            ("ấm", "a\u{302}\u{301}m"),
            ("Ậu", "A\u{323}\u{302}u"),
            ("ợ", "o\u{31b}\u{323}"),
            ("ửa", "u\u{31b}\u{309}a"),
            ("mả", "ma\u{309}"),
        ] {
            assert_eq!(collate(composed, decomposed, Some("vi-VN")), Ordering::Equal, "{composed}");
            assert_eq!(heading_letter(composed, Some("vi")), heading_letter(decomposed, Some("vi")), "{composed}");
        }
    }

    #[test]
    fn other_languages_sort_vietnamese_vowels_with_their_latin_letter() {
        assert_eq!(sorted(&["zebra", "Ẩn", "bánh", "Ước", "vợ"], "en-US"), ["Ẩn", "bánh", "Ước", "vợ", "zebra"]);
        assert_eq!(sorted(&["mo", "mợ", "mỏ", "mơ"], "en"), ["mo", "mỏ", "mơ", "mợ"]);
        assert_eq!(heading_letter("Ẩn", Some("en")).as_deref(), Some("A"));
        assert_eq!(heading_letter("ước", None).as_deref(), Some("U"));
    }

    #[test]
    fn heading_letters() {
        assert_eq!(heading_letter("ấm", Some("vi")).as_deref(), Some("Â"));
        assert_eq!(heading_letter("Ánh", Some("vi")).as_deref(), Some("A"));
        assert_eq!(heading_letter("đường", Some("vi")).as_deref(), Some("Đ"));
        assert_eq!(heading_letter("ợ", Some("vi")).as_deref(), Some("Ơ"));
        assert_eq!(heading_letter("Łódź", Some("pl-PL")).as_deref(), Some("Ł"));
        assert_eq!(heading_letter("Łódź", Some("en-US")).as_deref(), Some("L"));
        assert_eq!(heading_letter("chata", Some("cs")).as_deref(), Some("Ch"));
        assert_eq!(heading_letter("Äpfel", Some("de")).as_deref(), Some("A"));
        assert_eq!(heading_letter("Äpple", Some("sv")).as_deref(), Some("Ä"));
        assert_eq!(heading_letter("", Some("sv")), None);
    }

    #[test]
    fn hostile_input_never_panics() {
        for s in
            ["", "\u{301}", "a\u{301}\u{301}\u{301}", "dzs", "\u{10FFFF}", "ẞ", "İstanbul", "\u{323}\u{302}", "ậ\u{301}\u{323}\u{309}", "đ\u{31b}"]
        {
            for l in [None, Some("hu"), Some(""), Some("-"), Some("tr-TR"), Some("vi")] {
                let _ = sort_key(s, l);
                let _ = heading_letter(s, l);
            }
        }
    }
}
