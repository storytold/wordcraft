//! Shared character classification and language/script compatibility for speech and proofing.
use unicode_script::UnicodeScript;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum Script {
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Thai,
    Hangul,
    Kana,
    Han,
    Latin,
    Other,
}

/// The script of `c`. Non-letters (spaces, digits, punctuation) are [`Script::Other`].
pub(crate) fn of_char(c: char) -> Script {
    if !c.is_alphabetic() {
        return Script::Other;
    }
    match c {
        '\u{0400}'..='\u{052F}' | '\u{1C80}'..='\u{1C8F}' | '\u{2DE0}'..='\u{2DFF}' | '\u{A640}'..='\u{A69F}' => Script::Cyrillic,
        '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}' => Script::Greek,
        '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}' => {
            Script::Arabic
        }
        '\u{0590}'..='\u{05FF}' | '\u{FB1D}'..='\u{FB4F}' => Script::Hebrew,
        '\u{0E00}'..='\u{0E7F}' => Script::Thai,
        '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}' => Script::Hangul,
        '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' => Script::Kana,
        '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{2FFFF}' => Script::Han,
        _ if c.script() == unicode_script::Script::Latin => Script::Latin,
        _ => Script::Other,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CyrillicLetter {
    Russian,
    Ukrainian,
    Belarusian,
    SharedI,
    Candidates(&'static [&'static str]),
}

/// Shared letter evidence; unknown Cyrillic letters have no safe default-language candidate.
pub(crate) fn cyrillic_letter(c: char) -> Option<CyrillicLetter> {
    use CyrillicLetter::*;
    Some(match c.to_lowercase().next().unwrap_or(c) {
        'ї' | 'є' | 'ґ' => Ukrainian,
        'ў' => Belarusian,
        'і' => SharedI,
        'ђ' | 'ћ' => Candidates(&["sr"]),
        'ѓ' | 'ѕ' | 'ќ' => Candidates(&["mk"]),
        'ј' | 'љ' | 'њ' | 'џ' => Candidates(&["sr", "mk"]),
        'ә' | 'қ' | 'ұ' => Candidates(&["kk"]),
        'ғ' => Candidates(&["kk", "tg"]),
        'ң' | 'ө' | 'ү' => Candidates(&["kk", "ky", "mn"]),
        'һ' => Candidates(&["kk", "mn"]),
        'ҷ' | 'ҳ' | 'ӯ' | 'ӣ' => Candidates(&["tg"]),
        'а'..='я' | 'ё' => Russian,
        _ if of_char(c) == Script::Cyrillic => Candidates(&[]),
        _ => return None,
    })
}

/// Read Aloud's Ukrainian hint: a Ukrainian-only letter or the shared `і`.
pub(crate) fn ukrainian_letter(c: char) -> bool {
    matches!(cyrillic_letter(c), Some(CyrillicLetter::Ukrainian | CyrillicLetter::SharedI))
}

/// Reuse proofing's existing aliases rather than keeping another ISO 639 alias table.
pub(crate) fn primary_language(tag: &str) -> String {
    let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    let known = wordcraft_proof::ProofLang::from_tag(Some(&primary)).code();
    if !known.is_empty() {
        return known.to_string();
    }
    match primary.as_str() {
        "ukr" => "uk",
        "bel" => "be",
        "srp" => "sr",
        "mkd" | "mac" => "mk",
        "kaz" => "kk",
        "kir" => "ky",
        "mon" => "mn",
        "tgk" => "tg",
        _ => return primary,
    }
    .to_string()
}

/// The script a language tag is written in: an explicit script subtag (`sr-Latn`) wins over the
/// language's usual script. `None` for a language this table doesn't know.
pub(crate) fn of_language(tag: &str) -> Option<Script> {
    let mut parts = tag.split(['-', '_']);
    let primary = primary_language(parts.next()?);
    if let Some(script) = parts.next().filter(|p| p.len() == 4) {
        return match script.to_ascii_lowercase().as_str() {
            "latn" => Some(Script::Latin),
            "cyrl" => Some(Script::Cyrillic),
            "arab" => Some(Script::Arabic),
            "hebr" => Some(Script::Hebrew),
            "grek" => Some(Script::Greek),
            "thai" => Some(Script::Thai),
            "hang" | "kore" => Some(Script::Hangul),
            "jpan" | "kana" | "hira" => Some(Script::Kana),
            "hani" | "hans" | "hant" => Some(Script::Han),
            _ => None,
        };
    }
    Some(match primary.as_str() {
        "ru" | "uk" | "be" | "bg" | "sr" | "mk" | "kk" | "ky" | "mn" | "tg" => Script::Cyrillic,
        "el" => Script::Greek,
        "ar" | "fa" | "ur" | "ps" | "ku" | "sd" | "ug" => Script::Arabic,
        "he" | "yi" => Script::Hebrew,
        "th" => Script::Thai,
        "ko" => Script::Hangul,
        "ja" => Script::Kana,
        "zh" => Script::Han,
        "en" | "pl" | "pt" | "it" | "hr" | "tr" | "cs" | "id" | "in" | "es" | "de" | "fr" | "nl" | "da" | "sv" | "nb" | "nn" | "no" | "fi" | "et"
        | "lv" | "lt" | "ro" | "hu" | "sk" | "sl" | "vi" | "az" | "sq" | "is" | "ga" | "cy" | "mt" | "ms" | "sw" | "ca" | "eu" | "gl" | "af"
        | "bs" | "fil" => Script::Latin,
        _ => return None,
    })
}

/// Preserve Read Aloud's existing grouping and defaults.
pub(crate) fn speech_group(script: Script) -> Option<&'static str> {
    Some(match script {
        Script::Cyrillic => "ru",
        Script::Greek => "el",
        Script::Arabic => "ar",
        Script::Hebrew => "he",
        Script::Thai => "th",
        Script::Hangul => "ko",
        Script::Kana => "ja",
        Script::Han => "zh",
        _ => return None,
    })
}
