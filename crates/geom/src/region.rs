//! The conventions of the user's region that new documents and the measurement fields follow
//! (#838): the paper a new blank document starts on, the unit lengths are shown in, and the
//! decimal separator. Word takes them from the system region, not from the interface language
//! (an English interface in Germany still gets A4 and centimetres).
//!
//! The territory lists come from Unicode CLDR's supplemental `measurementData` (Unicode License):
//! `paperSize` US-Letter and `measurementSystem` US. The decimal separators follow CLDR's number
//! symbols for the common locales. The front end detects the region (it knows the platform); the
//! engine only sees this plain value.

use serde::{Deserialize, Serialize};

use crate::Unit;

/// Territories whose default paper is US Letter (CLDR `paperSize type="US-Letter"`).
const LETTER: &[&str] = &["BZ", "CA", "CL", "CO", "CR", "GT", "MX", "NI", "PA", "PH", "PR", "SV", "US", "VE"];

/// Territories that measure in inches (CLDR `measurementSystem type="US"`).
const INCHES: &[&str] = &["LR", "MM", "US"];

/// Languages whose numbers use a decimal comma in most of their territories.
const COMMA_LANGUAGES: &[&str] = &[
    "af", "az", "be", "bg", "bs", "ca", "cs", "da", "de", "el", "es", "et", "eu", "fi", "fo", "fr", "gl", "hr", "hu", "hy", "id", "is", "it", "ka",
    "kk", "ky", "lb", "lt", "lv", "mk", "nb", "nl", "nn", "no", "pl", "pt", "ro", "ru", "sk", "sl", "sq", "sr", "sv", "tr", "uk", "uz", "vi",
];

/// Territories where those languages use a decimal point after all.
const POINT_EXCEPTIONS: &[(&str, &[&str])] =
    &[("de", &["CH", "LI"]), ("it", &["CH"]), ("es", &["CU", "DO", "GT", "HN", "MX", "NI", "PA", "PE", "PR", "SV", "US", "419"])];

/// Territories where English (and other decimal-point languages) is written with a decimal
/// comma: most of continental Europe, South Africa and much of South America.
const COMMA_TERRITORIES: &[&str] = &[
    "AR", "AT", "BE", "BG", "BO", "BR", "CL", "CO", "CZ", "DE", "DK", "EC", "EE", "ES", "FI", "FR", "GR", "HR", "HU", "ID", "IS", "IT", "LT", "LU",
    "LV", "NL", "NO", "PL", "PT", "PY", "RO", "RS", "RU", "SE", "SI", "SK", "TR", "UA", "UY", "VE", "VN", "ZA",
];

/// A region: a language and a territory, read from a locale tag like `pl-PL`, `de_AT.UTF-8` or
/// `en-US`. The default is `en-US` (Letter, inches, decimal point).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    /// Lower-case language subtag (`pl`); empty when the tag has none.
    pub language: String,
    /// Upper-case territory (`PL`, or a UN M.49 number like `419`); empty when the tag has none.
    pub territory: String,
    /// US Letter or not, when a separate paper locale says so (Unix `LC_PAPER`); otherwise the
    /// territory decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter: Option<bool>,
    /// Inches or not, when a separate measurement locale says so (Unix `LC_MEASUREMENT`);
    /// otherwise the territory decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inches: Option<bool>,
}

impl Default for Region {
    fn default() -> Self {
        Region { language: "en".into(), territory: "US".into(), letter: None, inches: None }
    }
}

impl Region {
    /// The region a BCP 47 or POSIX locale tag names (`pl-PL`, `pl_PL.UTF-8@euro`,
    /// `zh-Hans-CN`, `en-US-u-ms-metric`). An empty or unreadable tag, `C` and `POSIX` are
    /// `en-US`; a bare `en` is the United States, any other bare language has no territory.
    pub fn from_tag(tag: &str) -> Region {
        // Locale tags are short; anything long is junk and only its start is read.
        let tag: String = tag.chars().take(64).collect();
        let tag = tag.split(['.', '@']).next().unwrap_or_default().trim();
        if tag.is_empty() || tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix") {
            return Region::default();
        }
        let mut parts = tag.split(['-', '_']);
        let language = parts.next().unwrap_or_default();
        if !(2..=3).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic()) {
            return Region::default();
        }
        let language = language.to_ascii_lowercase();
        let mut territory = String::new();
        for p in parts {
            // A singleton starts the extensions (`-u-…`, `-x-…`): no territory after it.
            if p.len() == 1 {
                break;
            }
            let alpha = p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic());
            let digits = p.len() == 3 && p.chars().all(|c| c.is_ascii_digit());
            if alpha || digits {
                territory = p.to_ascii_uppercase();
                break;
            }
        }
        if territory.is_empty() && language == "en" {
            territory = "US".into();
        }
        Region { language, territory, letter: None, inches: None }
    }

    /// This region with another territory (from a platform setting that names only the
    /// territory, like Windows' home location). A value that isn't a territory code is ignored.
    pub fn with_territory(mut self, territory: &str) -> Region {
        let t = territory.trim();
        let ok = (t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic())) || (t.len() == 3 && t.chars().all(|c| c.is_ascii_digit()));
        if ok {
            self.territory = t.to_ascii_uppercase();
        }
        self
    }

    /// This region with the paper and the measurement system taken from their own locale tags,
    /// when the platform sets them apart (Unix `LC_PAPER`, `LC_MEASUREMENT`): `LANG=pl_PL.UTF-8`
    /// with `LC_PAPER=en_US.UTF-8` writes `2,54 cm` on Letter.
    pub fn with_paper_and_measurement(mut self, paper: Option<&str>, measurement: Option<&str>) -> Region {
        if let Some(t) = paper {
            self.letter = Some(Region::from_tag(t).uses_letter());
        }
        if let Some(t) = measurement {
            self.inches = Some(Region::from_tag(t).units() == Unit::Inches);
        }
        self
    }

    /// The system region from the locale variables `var` reads (the process environment) and,
    /// when none is set, the platform's first preferred locale `platform` gives. One tag for
    /// everything first (`WORDCRAFT_REGION`, then `LC_ALL`); otherwise Unix sets each convention
    /// on its own: numbers (`LC_NUMERIC`, else `LANG`), paper (`LC_PAPER`) and measurement
    /// (`LC_MEASUREMENT`). Empty, `C` and `POSIX` values count as unset.
    pub fn from_locale_vars(var: impl Fn(&str) -> Option<String>, platform: impl FnOnce() -> Option<String>) -> Region {
        let var = |name: &str| var(name).filter(|v| usable_locale(v));
        if let Some(tag) = var("WORDCRAFT_REGION").or_else(|| var("LC_ALL")) {
            return Region::from_tag(&tag);
        }
        let base = var("LC_NUMERIC").or_else(|| var("LANG")).unwrap_or_else(|| platform().unwrap_or_default());
        Region::from_tag(&base).with_paper_and_measurement(var("LC_PAPER").as_deref(), var("LC_MEASUREMENT").as_deref())
    }

    /// Does a new document start on US Letter here (otherwise A4)?
    pub fn uses_letter(&self) -> bool {
        self.letter.unwrap_or_else(|| LETTER.contains(&self.territory.as_str()))
    }

    /// The default paper's name and portrait size in points: Letter (612 × 792) or A4.
    pub fn paper(&self) -> (&'static str, f32, f32) {
        if self.uses_letter() { ("Letter", 612.0, 792.0) } else { ("A4", 595.28, 841.89) }
    }

    /// The unit lengths are shown in by default: inches in the US, centimetres elsewhere.
    pub fn units(&self) -> Unit {
        if self.inches.unwrap_or_else(|| INCHES.contains(&self.territory.as_str())) { Unit::Inches } else { Unit::Centimeters }
    }

    /// The decimal separator numbers are written with here (`,` or `.`).
    pub fn decimal(&self) -> char {
        let lang = self.language.as_str();
        let t = self.territory.as_str();
        if COMMA_LANGUAGES.contains(&lang) {
            let point = POINT_EXCEPTIONS.iter().any(|(l, ts)| *l == lang && ts.contains(&t));
            return if point { '.' } else { ',' };
        }
        if lang == "en" && COMMA_TERRITORIES.contains(&t) { ',' } else { '.' }
    }

    /// `pt` in `unit` written the way this region writes numbers (`2,54 cm` in Poland).
    pub fn format(&self, unit: Unit, pt: f32) -> String {
        unit.format_with(pt, self.decimal())
    }
}

/// A locale variable that names a locale (not empty, `C` or `POSIX`).
fn usable_locale(v: &str) -> bool {
    let v = v.trim();
    !v.is_empty() && !v.eq_ignore_ascii_case("c") && !v.eq_ignore_ascii_case("posix") && !v.starts_with("C.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_locale_tags() {
        assert_eq!(Region::from_tag("pl-PL"), Region { language: "pl".into(), territory: "PL".into(), ..Region::default() });
        assert_eq!(Region::from_tag("de_AT.UTF-8@euro").territory, "AT");
        assert_eq!(Region::from_tag("zh-Hans-CN").territory, "CN");
        assert_eq!(Region::from_tag("es-419").territory, "419");
        assert_eq!(Region::from_tag("en-u-ms-metric").territory, "US", "bare English is the US");
        assert_eq!(Region::from_tag("fr").territory, "");
        for junk in ["", "C", "POSIX", "1234", "x", "--", "ünïcode-ÄÄ", &"a".repeat(10_000)] {
            assert_eq!(Region::from_tag(junk), Region::default(), "{junk:?}");
        }
        assert_eq!(Region::from_tag("en-US").with_territory("de").territory, "DE");
        assert_eq!(Region::from_tag("en-US").with_territory("Germany").territory, "US");
    }

    /// Paper and units follow the territory, not the language (#838).
    #[test]
    fn paper_and_units_follow_the_territory() {
        let pl = Region::from_tag("pl-PL");
        assert_eq!((pl.paper().0, pl.units(), pl.decimal()), ("A4", Unit::Centimeters, ','));
        let us = Region::from_tag("en-US");
        assert_eq!((us.paper().0, us.units(), us.decimal()), ("Letter", Unit::Inches, '.'));
        let ca = Region::from_tag("fr-CA");
        assert_eq!((ca.paper().0, ca.units(), ca.decimal()), ("Letter", Unit::Centimeters, ','));
        let en_de = Region::from_tag("en-DE");
        assert_eq!((en_de.paper().0, en_de.units(), en_de.decimal()), ("A4", Unit::Centimeters, ','));
        let gb = Region::from_tag("en-GB");
        assert_eq!((gb.paper().0, gb.units(), gb.decimal()), ("A4", Unit::Centimeters, '.'));
        assert_eq!(Region::from_tag("de-CH").decimal(), '.');
        assert_eq!(Region::from_tag("es-MX").paper().0, "Letter");
        assert_eq!(Region::from_tag("ja-JP").paper().0, "A4");
        assert_eq!(pl.format(Unit::Centimeters, 72.0), "2,54 cm");
        // Unix sets paper and measurement apart from the number locale.
        let mixed = Region::from_tag("pl_PL.UTF-8").with_paper_and_measurement(Some("en_US.UTF-8"), None);
        assert_eq!((mixed.paper().0, mixed.units(), mixed.decimal()), ("Letter", Unit::Centimeters, ','));
        let mixed = Region::from_tag("en_US.UTF-8").with_paper_and_measurement(Some("de_DE.UTF-8"), Some("de_DE.UTF-8"));
        assert_eq!((mixed.paper().0, mixed.units(), mixed.decimal()), ("A4", Unit::Centimeters, '.'));
        assert_eq!(Region::from_tag("pl-PL").with_paper_and_measurement(Some(""), Some("junk")).paper().0, "Letter", "junk is en-US");
        assert_eq!(us.format(Unit::Inches, 90.0), "1.25\"");
    }

    #[test]
    fn region_from_locale_variables() {
        // #1381: the CLI and the app read the system region the same way.
        let env = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        let none = || None;
        assert_eq!(Region::from_locale_vars(env(&[("LANG", "it_IT.UTF-8")]), none).paper().0, "A4");
        assert_eq!(Region::from_locale_vars(env(&[("LANG", "en_US.UTF-8")]), none).paper().0, "Letter");
        assert_eq!(Region::from_locale_vars(env(&[("LANG", "it_IT.UTF-8"), ("LC_ALL", "en_US")]), none).paper().0, "Letter");
        assert_eq!(Region::from_locale_vars(env(&[("LANG", "en_US"), ("WORDCRAFT_REGION", "de-DE")]), none).territory, "DE");
        assert_eq!(Region::from_locale_vars(env(&[("LANG", "en_US"), ("LC_PAPER", "it_IT")]), none).paper().0, "A4");
        assert_eq!(Region::from_locale_vars(env(&[("LANG", "C.UTF-8")]), || Some("it-IT".into())).territory, "IT");
        assert_eq!(Region::from_locale_vars(env(&[]), none), Region::default());
    }
}
