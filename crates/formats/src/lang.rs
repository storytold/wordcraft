//! Text languages in the text formats: BCP 47 tags (RFC 5646) checked and put in their usual
//! case, the Windows language IDs RTF names them by ([MS-LCID] §2.2), and how ODF 1.3 splits a
//! tag into `fo:language`, `fo:script` and `fo:country` (§20.188, §20.231, §20.177), keeping
//! the whole tag in `style:rfc-language-tag` (§19.871) when the split can't say it.
//!
//! Everything here reads hostile input: a tag longer than [`MAX_TAG`] bytes, or one that isn't
//! shaped like a language tag, is no language; a language ID not in the table is ignored.

use std::sync::Arc;

/// The longest language tag kept, in bytes (the .docx reader keeps tags under 64 bytes).
pub const MAX_TAG: usize = 63;

/// Which of a run's three languages: the one for Latin (and other simple-script) text, the one
/// for right-to-left / complex-script text (`w:lang/@w:bidi`), or the one for East Asian text
/// (`w:lang/@w:eastAsia`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Latin,
    Bidi,
    Ea,
}

/// A run's languages, one per [`Slot`]; `None` = inherited.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Langs {
    pub latin: Option<Arc<str>>,
    pub bidi: Option<Arc<str>>,
    pub ea: Option<Arc<str>>,
}

impl Langs {
    pub fn is_empty(&self) -> bool {
        self.latin.is_none() && self.bidi.is_none() && self.ea.is_none()
    }

    pub fn get(&self, s: Slot) -> Option<&Arc<str>> {
        match s {
            Slot::Latin => self.latin.as_ref(),
            Slot::Bidi => self.bidi.as_ref(),
            Slot::Ea => self.ea.as_ref(),
        }
    }

    pub fn slot_mut(&mut self, s: Slot) -> &mut Option<Arc<str>> {
        match s {
            Slot::Latin => &mut self.latin,
            Slot::Bidi => &mut self.bidi,
            Slot::Ea => &mut self.ea,
        }
    }

    /// `self` without the languages that equal `defaults`' (what a run inherits anyway).
    pub fn without(&self, defaults: &Langs) -> Langs {
        let keep = |v: &Option<Arc<str>>, d: &Option<Arc<str>>| v.clone().filter(|v| d.as_deref() != Some(&**v));
        Langs { latin: keep(&self.latin, &defaults.latin), bidi: keep(&self.bidi, &defaults.bidi), ea: keep(&self.ea, &defaults.ea) }
    }

    /// Each slot of `self`, else `defaults`'.
    pub fn or(&self, defaults: &Langs) -> Langs {
        Langs {
            latin: self.latin.clone().or_else(|| defaults.latin.clone()),
            bidi: self.bidi.clone().or_else(|| defaults.bidi.clone()),
            ea: self.ea.clone().or_else(|| defaults.ea.clone()),
        }
    }

    /// Bytes the tags hold.
    pub fn len(&self) -> usize {
        [&self.latin, &self.bidi, &self.ea].iter().map(|v| v.as_deref().map_or(0, str::len)).sum()
    }
}

/// `raw` as a language tag in its usual case (RFC 5646 §2.1.1: `sr-Cyrl-RS`, `en-US`), `_`
/// read as `-`; `None` when it isn't shaped like a BCP 47 tag (subtags of 1–8 letters and
/// digits, a primary subtag of 2–8 letters or the `x`/`i` singletons) or is over [`MAX_TAG`].
pub fn tag(raw: &str) -> Option<Arc<str>> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_TAG || !raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return None;
    }
    let mut out = String::with_capacity(raw.len());
    // After a singleton (an extension or private use), subtags are kept lowercase.
    let mut extension = false;
    for (i, sub) in raw.split(['-', '_']).enumerate() {
        if sub.is_empty() || sub.len() > 8 {
            return None;
        }
        if i == 0 {
            let private = sub.eq_ignore_ascii_case("x") || sub.eq_ignore_ascii_case("i");
            if !private && !(sub.len() >= 2 && sub.bytes().all(|b| b.is_ascii_alphabetic())) {
                return None;
            }
            extension = private;
            out.push_str(&sub.to_ascii_lowercase());
            continue;
        }
        out.push('-');
        if sub.len() == 1 {
            extension = true;
        }
        if !extension && sub.len() == 4 && sub.bytes().all(|b| b.is_ascii_alphabetic()) {
            // A script: title case.
            let lower = sub.to_ascii_lowercase();
            let mut cs = lower.chars();
            if let Some(c) = cs.next() {
                out.push(c.to_ascii_uppercase());
                out.push_str(cs.as_str());
            }
        } else if !extension && sub.len() == 2 && sub.bytes().all(|b| b.is_ascii_alphabetic()) {
            out.push_str(&sub.to_ascii_uppercase());
        } else {
            out.push_str(&sub.to_ascii_lowercase());
        }
    }
    // A singleton (`x`, an extension) is followed by its subtags.
    if raw.split(['-', '_']).next_back().is_some_and(|s| s.len() == 1) {
        return None;
    }
    Some(out.into())
}

/// The primary language subtag of `tag`, lowercase.
fn primary(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase()
}

/// The slot a language's text goes in when a format names one language per text (HTML `lang`):
/// right-to-left languages are the complex-script language, Chinese, Japanese and Korean the
/// East Asian one, every other the Latin one.
pub fn slot_of(tag: &str) -> Slot {
    match primary(tag).as_str() {
        "ar" | "he" | "iw" | "yi" | "fa" | "prs" | "ur" | "ps" | "sd" | "ug" | "ckb" | "syr" | "dv" => Slot::Bidi,
        "ku" if tag.to_ascii_lowercase().contains("-arab") => Slot::Bidi,
        "ja" | "zh" | "ko" | "yue" => Slot::Ea,
        _ => Slot::Latin,
    }
}

/// The slot `text`'s first letter belongs to: right-to-left letters are complex script,
/// kana, ideographs and hangul East Asian, every other letter Latin. Text without letters is Latin.
pub fn text_slot(text: &str) -> Slot {
    for c in text.chars() {
        let u = c as u32;
        let rtl = matches!(u, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF);
        if rtl {
            return Slot::Bidi;
        }
        let cjk = matches!(
            u,
            0x1100..=0x11FF
                | 0x2E80..=0x2FDF
                | 0x3040..=0x30FF
                | 0x3100..=0x31FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xA960..=0xA97F
                | 0xAC00..=0xD7FF
                | 0xF900..=0xFAFF
                | 0xFF66..=0xFF9F
                | 0x20000..=0x3FFFF
        );
        if cjk {
            return Slot::Ea;
        }
        if c.is_alphabetic() {
            return Slot::Latin;
        }
    }
    Slot::Latin
}

// ---------------------------------------------------------------------------------------------
// Windows language IDs (RTF `\lang`, `\langfe`, `\alang`)

/// Windows language IDs and their tags, from [MS-LCID] §2.2 (the language ID table). Where
/// several IDs name the same tag, the first listed is the one written. A language's first entry
/// is the one a tag naming only the language (`de`) or the language and script (`sr-Latn`) is
/// written as.
const LCIDS: &[(u16, &str)] = &[
    (0x0409, "en-US"),
    (0x0809, "en-GB"),
    (0x0C09, "en-AU"),
    (0x1009, "en-CA"),
    (0x1409, "en-NZ"),
    (0x1809, "en-IE"),
    (0x1C09, "en-ZA"),
    (0x2009, "en-JM"),
    (0x2409, "en-029"),
    (0x2809, "en-BZ"),
    (0x2C09, "en-TT"),
    (0x3009, "en-ZW"),
    (0x3409, "en-PH"),
    (0x4009, "en-IN"),
    (0x4409, "en-MY"),
    (0x4809, "en-SG"),
    (0x0407, "de-DE"),
    (0x0807, "de-CH"),
    (0x0C07, "de-AT"),
    (0x1007, "de-LU"),
    (0x1407, "de-LI"),
    (0x040C, "fr-FR"),
    (0x080C, "fr-BE"),
    (0x0C0C, "fr-CA"),
    (0x100C, "fr-CH"),
    (0x140C, "fr-LU"),
    (0x180C, "fr-MC"),
    (0x0C0A, "es-ES"),
    (0x040A, "es-ES"),
    (0x080A, "es-MX"),
    (0x100A, "es-GT"),
    (0x140A, "es-CR"),
    (0x180A, "es-PA"),
    (0x1C0A, "es-DO"),
    (0x200A, "es-VE"),
    (0x240A, "es-CO"),
    (0x280A, "es-PE"),
    (0x2C0A, "es-AR"),
    (0x300A, "es-EC"),
    (0x340A, "es-CL"),
    (0x380A, "es-UY"),
    (0x3C0A, "es-PY"),
    (0x400A, "es-BO"),
    (0x440A, "es-SV"),
    (0x480A, "es-HN"),
    (0x4C0A, "es-NI"),
    (0x500A, "es-PR"),
    (0x540A, "es-US"),
    (0x580A, "es-419"),
    (0x0416, "pt-BR"),
    (0x0816, "pt-PT"),
    (0x0410, "it-IT"),
    (0x0810, "it-CH"),
    (0x0413, "nl-NL"),
    (0x0813, "nl-BE"),
    (0x0415, "pl-PL"),
    (0x0405, "cs-CZ"),
    (0x041B, "sk-SK"),
    (0x040E, "hu-HU"),
    (0x0418, "ro-RO"),
    (0x0818, "ro-MD"),
    (0x0402, "bg-BG"),
    (0x041A, "hr-HR"),
    (0x101A, "hr-BA"),
    (0x281A, "sr-Cyrl-RS"),
    (0x0C1A, "sr-Cyrl-CS"),
    (0x1C1A, "sr-Cyrl-BA"),
    (0x301A, "sr-Cyrl-ME"),
    (0x241A, "sr-Latn-RS"),
    (0x081A, "sr-Latn-CS"),
    (0x181A, "sr-Latn-BA"),
    (0x2C1A, "sr-Latn-ME"),
    (0x141A, "bs-Latn-BA"),
    (0x201A, "bs-Cyrl-BA"),
    (0x0424, "sl-SI"),
    (0x042F, "mk-MK"),
    (0x041C, "sq-AL"),
    (0x0422, "uk-UA"),
    (0x0423, "be-BY"),
    (0x0419, "ru-RU"),
    (0x0819, "ru-MD"),
    (0x041F, "tr-TR"),
    (0x0408, "el-GR"),
    (0x0406, "da-DK"),
    (0x041D, "sv-SE"),
    (0x081D, "sv-FI"),
    (0x0414, "nb-NO"),
    (0x0814, "nn-NO"),
    (0x040B, "fi-FI"),
    (0x040F, "is-IS"),
    (0x0438, "fo-FO"),
    (0x0425, "et-EE"),
    (0x0426, "lv-LV"),
    (0x0427, "lt-LT"),
    (0x0403, "ca-ES"),
    (0x042D, "eu-ES"),
    (0x0456, "gl-ES"),
    (0x0452, "cy-GB"),
    (0x083C, "ga-IE"),
    (0x0491, "gd-GB"),
    (0x043A, "mt-MT"),
    (0x046E, "lb-LU"),
    (0x0462, "fy-NL"),
    (0x0417, "rm-CH"),
    (0x042E, "hsb-DE"),
    (0x047E, "br-FR"),
    (0x0482, "oc-FR"),
    (0x0483, "co-FR"),
    (0x0484, "gsw-FR"),
    (0x0436, "af-ZA"),
    (0x0435, "zu-ZA"),
    (0x0434, "xh-ZA"),
    (0x0432, "tn-ZA"),
    (0x0441, "sw-KE"),
    (0x045E, "am-ET"),
    (0x046A, "yo-NG"),
    (0x0470, "ig-NG"),
    (0x0468, "ha-Latn-NG"),
    (0x0488, "wo-SN"),
    (0x0487, "rw-RW"),
    (0x0477, "so-SO"),
    (0x0473, "ti-ET"),
    (0x0873, "ti-ER"),
    (0x042A, "vi-VN"),
    (0x0421, "id-ID"),
    (0x043E, "ms-MY"),
    (0x083E, "ms-BN"),
    (0x0464, "fil-PH"),
    (0x0411, "ja-JP"),
    (0x0412, "ko-KR"),
    (0x0804, "zh-CN"),
    (0x0404, "zh-TW"),
    (0x0C04, "zh-HK"),
    (0x1004, "zh-SG"),
    (0x1404, "zh-MO"),
    (0x0401, "ar-SA"),
    (0x0801, "ar-IQ"),
    (0x0C01, "ar-EG"),
    (0x1001, "ar-LY"),
    (0x1401, "ar-DZ"),
    (0x1801, "ar-MA"),
    (0x1C01, "ar-TN"),
    (0x2001, "ar-OM"),
    (0x2401, "ar-YE"),
    (0x2801, "ar-SY"),
    (0x2C01, "ar-JO"),
    (0x3001, "ar-LB"),
    (0x3401, "ar-KW"),
    (0x3801, "ar-AE"),
    (0x3C01, "ar-BH"),
    (0x4001, "ar-QA"),
    (0x040D, "he-IL"),
    (0x0429, "fa-IR"),
    (0x048C, "prs-AF"),
    (0x0420, "ur-PK"),
    (0x0820, "ur-IN"),
    (0x0463, "ps-AF"),
    (0x0859, "sd-Arab-PK"),
    (0x0480, "ug-CN"),
    (0x0492, "ku-Arab-IQ"),
    (0x045A, "syr-SY"),
    (0x0465, "dv-MV"),
    (0x0439, "hi-IN"),
    (0x0461, "ne-NP"),
    (0x0861, "ne-IN"),
    (0x044E, "mr-IN"),
    (0x044F, "sa-IN"),
    (0x0457, "kok-IN"),
    (0x0445, "bn-IN"),
    (0x0845, "bn-BD"),
    (0x044D, "as-IN"),
    (0x0446, "pa-IN"),
    (0x0846, "pa-Arab-PK"),
    (0x0447, "gu-IN"),
    (0x0448, "or-IN"),
    (0x0449, "ta-IN"),
    (0x0849, "ta-LK"),
    (0x044A, "te-IN"),
    (0x044B, "kn-IN"),
    (0x044C, "ml-IN"),
    (0x045B, "si-LK"),
    (0x041E, "th-TH"),
    (0x0453, "km-KH"),
    (0x0454, "lo-LA"),
    (0x0455, "my-MM"),
    (0x0451, "bo-CN"),
    (0x0450, "mn-MN"),
    (0x0437, "ka-GE"),
    (0x042B, "hy-AM"),
    (0x042C, "az-Latn-AZ"),
    (0x082C, "az-Cyrl-AZ"),
    (0x043F, "kk-KZ"),
    (0x0440, "ky-KG"),
    (0x0443, "uz-Latn-UZ"),
    (0x0843, "uz-Cyrl-UZ"),
    (0x0442, "tk-TM"),
    (0x0428, "tg-Cyrl-TJ"),
    (0x0444, "tt-RU"),
    (0x046D, "ba-RU"),
    (0x0485, "sah-RU"),
    (0x043B, "se-NO"),
    (0x046F, "kl-GL"),
    (0x045D, "iu-Cans-CA"),
    (0x085D, "iu-Latn-CA"),
    (0x047C, "moh-CA"),
    (0x047A, "arn-CL"),
    (0x0481, "mi-NZ"),
    (0x0478, "ii-CN"),
    (0x085F, "tzm-Latn-DZ"),
];

/// RTF's "no language" (`\lang1024`, [MS-LCID] `LANG_USER_DEFAULT` 0x0400 with no sublanguage).
pub const LCID_NONE: u16 = 0x0400;

/// The tag Windows language ID `id` names; `None` for an ID not in the table (ignored) and
/// for [`LCID_NONE`].
pub fn tag_of_lcid(id: i32) -> Option<Arc<str>> {
    let id = u16::try_from(id).ok()?;
    LCIDS.iter().find(|(n, _)| *n == id).map(|(_, t)| Arc::from(*t))
}

/// The Windows language ID `tag` is written as: the table's entry for the tag, else for its
/// language and script, else for its language. `None` for a language the table doesn't have.
pub fn lcid_of(tag: &str) -> Option<u16> {
    let t = tag.replace('_', "-");
    if let Some((n, _)) = LCIDS.iter().find(|(_, e)| e.eq_ignore_ascii_case(&t)) {
        return Some(*n);
    }
    let lower = t.to_ascii_lowercase();
    // Chinese named by script: Simplified is China's, Traditional Taiwan's.
    if lower == "zh-hans" || lower.starts_with("zh-hans-") {
        return Some(0x0804);
    }
    if lower == "zh-hant" || lower.starts_with("zh-hant-") {
        return Some(0x0404);
    }
    let mut parts = lower.split('-');
    let lang = parts.next().unwrap_or("");
    if lang.is_empty() {
        return None;
    }
    let script = parts.next().filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphabetic()));
    let of_lang = |e: &&(u16, &str)| primary(e.1) == lang;
    if let Some(s) = script {
        let with_script = format!("{lang}-{s}-");
        if let Some((n, _)) = LCIDS.iter().find(|(_, e)| e.to_ascii_lowercase().starts_with(&with_script)) {
            return Some(*n);
        }
    }
    LCIDS.iter().find(of_lang).map(|(n, _)| *n)
}

// ---------------------------------------------------------------------------------------------
// ODF 1.3 language attributes

/// The ODF attributes of one of a run's languages: their names (`fo:` for Latin text,
/// `style:…-asian`, `style:…-complex`).
pub struct OdfNames {
    pub language: &'static str,
    pub country: &'static str,
    pub script: &'static str,
    pub rfc: &'static str,
}

/// The attribute names for each slot (ODF 1.3 §20.188 and the `-asian` / `-complex` variants).
pub fn odf_names(s: Slot) -> OdfNames {
    match s {
        Slot::Latin => OdfNames { language: "fo:language", country: "fo:country", script: "fo:script", rfc: "style:rfc-language-tag" },
        Slot::Ea => OdfNames {
            language: "style:language-asian",
            country: "style:country-asian",
            script: "style:script-asian",
            rfc: "style:rfc-language-tag-asian",
        },
        Slot::Bidi => OdfNames {
            language: "style:language-complex",
            country: "style:country-complex",
            script: "style:script-complex",
            rfc: "style:rfc-language-tag-complex",
        },
    }
}

/// `tag` as the ODF attributes of slot `s` (each with a leading space): language, script and
/// country where the tag is just those (`sr-Cyrl-RS`), plus `style:rfc-language-tag…` with the
/// whole tag when it says more (`es-419`, `de-DE-1996`, `x-klingon`).
pub fn odf_attrs(s: Slot, tag: &str, esc: impl Fn(&str) -> String) -> String {
    let n = odf_names(s);
    let mut parts = tag.split('-');
    let mut out = String::new();
    let lang = parts.next().filter(|l| (2..=3).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_alphabetic()));
    let Some(lang) = lang else {
        // A private-use or long primary subtag: only the whole tag can say it.
        out.push_str(&format!(" {}=\"{}\"", n.rfc, esc(tag)));
        return out;
    };
    out.push_str(&format!(" {}=\"{}\"", n.language, esc(lang)));
    let rest: Vec<&str> = parts.collect();
    let mut i = 0;
    let mut script = None;
    if let Some(sc) = rest.get(i).filter(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphabetic())) {
        script = Some(*sc);
        i += 1;
    }
    let mut country = None;
    if let Some(c) = rest.get(i).filter(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic())) {
        country = Some(*c);
        i += 1;
    }
    if let Some(sc) = script {
        out.push_str(&format!(" {}=\"{}\"", n.script, esc(sc)));
    }
    if let Some(c) = country {
        out.push_str(&format!(" {}=\"{}\"", n.country, esc(c)));
    }
    if i < rest.len() {
        out.push_str(&format!(" {}=\"{}\"", n.rfc, esc(tag)));
    }
    out
}

/// The language a style's ODF attributes give slot `s`, read by `get`: the whole tag of
/// `style:rfc-language-tag…` when it is one, else language, script and country. `Some(None)`
/// is an explicit "no language" (`zxx`, `none`); `None` = the style doesn't say.
pub fn odf_read<'a>(s: Slot, get: impl Fn(&str) -> Option<&'a str>) -> Option<Option<Arc<str>>> {
    let n = odf_names(s);
    if let Some(t) = get(n.rfc).and_then(tag) {
        return Some(Some(t));
    }
    let lang = get(n.language)?.trim();
    if lang.is_empty() || lang.eq_ignore_ascii_case("zxx") || lang.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let mut t = lang.to_string();
    if let Some(sc) = get(n.script).map(str::trim).filter(|v| !v.is_empty()) {
        t.push('-');
        t.push_str(sc);
    }
    if let Some(c) = get(n.country).map(str::trim).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("none")) {
        t.push('-');
        t.push_str(c);
    }
    // A malformed value is no language at all, not "no language": the style says nothing.
    tag(&t).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_checked_and_put_in_their_usual_case() {
        assert_eq!(tag("sr-cyrl-rs").as_deref(), Some("sr-Cyrl-RS"));
        assert_eq!(tag(" EN_us ").as_deref(), Some("en-US"));
        assert_eq!(tag("es-419").as_deref(), Some("es-419"));
        assert_eq!(tag("x-Klingon").as_deref(), Some("x-klingon"));
        assert_eq!(tag("de-DE-u-co-phonebk").as_deref(), Some("de-DE-u-co-phonebk"));
        for bad in ["", "-", "e", "1a", "en--US", "en-US-", "en US", "en-<b>", "ab-123456789", "\"en\"", "ja-JP\u{0}", "ü-DE", "x", "en-x", "de-u"] {
            assert_eq!(tag(bad), None, "{bad:?}");
        }
        assert_eq!(tag(&"a".repeat(MAX_TAG + 1)), None);
        assert_eq!(tag(&format!("en{}", "-abcdefgh".repeat(20))), None);
    }

    #[test]
    fn language_ids_map_both_ways() {
        for (id, t) in [(0x0409, "en-US"), (0x0461, "ne-NP"), (0x0861, "ne-IN"), (0x281A, "sr-Cyrl-RS"), (0x241A, "sr-Latn-RS"), (0x0401, "ar-SA")] {
            assert_eq!(tag_of_lcid(id).as_deref(), Some(t));
            assert_eq!(lcid_of(t), Some(id as u16), "{t}");
        }
        assert_eq!(lcid_of("ja-JP"), Some(0x0411));
        assert_eq!(lcid_of("JA-jp"), Some(0x0411));
        // Spanish with the traditional sort reads as Spanish; Spanish is written as the modern sort.
        assert_eq!(tag_of_lcid(0x040A).as_deref(), Some("es-ES"));
        assert_eq!(lcid_of("es-ES"), Some(0x0C0A));
        // A tag the table lacks falls back to its language and script, then its language.
        assert_eq!(lcid_of("sr-Latn"), Some(0x241A));
        assert_eq!(lcid_of("sr"), Some(0x281A));
        assert_eq!(lcid_of("de"), Some(0x0407));
        assert_eq!(lcid_of("zh-Hant-HK"), Some(0x0404));
        assert_eq!(lcid_of("tlh"), None);
        assert_eq!(lcid_of(""), None);
        for junk in [i32::MIN, -1, 0, 1024, 0xFFFF, 0x10000, i32::MAX] {
            assert_eq!(tag_of_lcid(junk), None, "{junk}");
        }
        // Every entry is a well-formed tag in its usual case, and every ID is listed once.
        for (i, (n, t)) in LCIDS.iter().enumerate() {
            assert_eq!(tag(t).as_deref(), Some(*t));
            assert!(LCIDS.iter().skip(i + 1).all(|(m, _)| m != n), "{n:#06x} twice");
        }
    }

    #[test]
    fn odf_splits_what_it_can_and_keeps_the_rest_whole() {
        let id = |s: &str| s.to_string();
        assert_eq!(odf_attrs(Slot::Latin, "en-US", id), " fo:language=\"en\" fo:country=\"US\"");
        assert_eq!(odf_attrs(Slot::Latin, "sr-Cyrl-RS", id), " fo:language=\"sr\" fo:script=\"Cyrl\" fo:country=\"RS\"");
        assert_eq!(odf_attrs(Slot::Ea, "ja-JP", id), " style:language-asian=\"ja\" style:country-asian=\"JP\"");
        assert_eq!(odf_attrs(Slot::Bidi, "ar-SA", id), " style:language-complex=\"ar\" style:country-complex=\"SA\"");
        assert_eq!(odf_attrs(Slot::Latin, "es-419", id), " fo:language=\"es\" style:rfc-language-tag=\"es-419\"");
        assert_eq!(odf_attrs(Slot::Latin, "x-klingon", id), " style:rfc-language-tag=\"x-klingon\"");
        let attrs = |v: &'static [(&'static str, &'static str)]| move |k: &str| v.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        assert_eq!(odf_read(Slot::Latin, attrs(&[("fo:language", "sr"), ("fo:script", "Latn"), ("fo:country", "RS")])), Some(tag("sr-Latn-RS")));
        assert_eq!(odf_read(Slot::Latin, attrs(&[("fo:language", "es"), ("style:rfc-language-tag", "es-419")])), Some(tag("es-419")));
        assert_eq!(odf_read(Slot::Bidi, attrs(&[("style:language-complex", "zxx"), ("style:country-complex", "none")])), Some(None));
        assert_eq!(odf_read(Slot::Latin, attrs(&[("fo:country", "US")])), None);
        assert_eq!(odf_read(Slot::Latin, attrs(&[("fo:language", "<script>")])), None);
    }

    #[test]
    fn slots() {
        assert_eq!(slot_of("ar-SA"), Slot::Bidi);
        assert_eq!(slot_of("ja-JP"), Slot::Ea);
        assert_eq!(slot_of("ne-NP"), Slot::Latin);
        assert_eq!(text_slot("123 مرحبا"), Slot::Bidi);
        assert_eq!(text_slot("日本語"), Slot::Ea);
        assert_eq!(text_slot("नमस्ते"), Slot::Latin);
        assert_eq!(text_slot(""), Slot::Latin);
    }
}
