//! Numbers written the way a language writes them: `1 250,50` in Polish, `1.250,50` in German,
//! `1,250.50` in English. Used by table sort and table formulas.

use crate::collate::language;

/// The decimal separator of language `lang` (a BCP 47 tag; `None` means English).
pub fn decimal_separator(lang: Option<&str>) -> char {
    let tag = lang.unwrap_or("").trim().to_ascii_lowercase().replace('_', "-");
    // Regions that write a decimal point where the language usually writes a comma.
    const POINT_REGIONS: &[&str] =
        &["de-ch", "de-li", "it-ch", "es-mx", "es-us", "es-419", "es-pr", "es-do", "es-gt", "es-hn", "es-ni", "es-pa", "es-sv", "es-cu", "es-pe"];
    if POINT_REGIONS.iter().any(|r| tag == *r || tag.starts_with(&format!("{r}-"))) {
        return '.';
    }
    const COMMA: &[&str] = &[
        "af", "az", "be", "bg", "bs", "ca", "cs", "da", "de", "el", "es", "et", "eu", "fi", "fo", "fr", "gl", "hr", "hu", "hy", "id", "is", "it",
        "ka", "kk", "ky", "lb", "lt", "lv", "mk", "mn", "nb", "nl", "nn", "no", "pl", "pt", "ro", "ru", "sk", "sl", "sq", "sr", "sv", "tr", "uk",
        "uz", "vi",
    ];
    if COMMA.contains(&language(lang).as_str()) { ',' } else { '.' }
}

const SPACES: &[char] = &[' ', '\u{a0}', '\u{202f}', '\u{2009}'];
const CURRENCY: &[char] = &['$', '€', '£', '¥', '¢', '₽', '₴', '₺', '₹', '₩', '₪', '₫', '฿', '₱', '₸', '₼', '₾'];
/// Currency signs spelled with more than one character, longest first, and whether the sign may
/// touch the number. Word reads `R$1,250` and `US$ 1,250`, but needs a space in `Bs. 1,250`,
/// `S/. 1,250` and `B/. 1,250.50`. ISO codes (`ARS 1,250`) are read by the three-letter currency
/// word below.
const CURRENCY_SYMBOLS: &[(&str, bool)] = &[
    ("Bs.S", false), // Venezuelan bolívar
    ("Bs.", false),  // Bolivian boliviano; also the Venezuelan bolívar
    ("Bs", false),   // Bolivian boliviano, as CLDR writes it
    ("US$", true),   // United States dollar, used in Ecuador, El Salvador and Puerto Rico
    ("Arg$", true),  // Argentine peso
    ("CUP$", true),  // Cuban peso
    ("RD$", true),   // Dominican peso
    ("MX$", true),   // Mexican peso
    ("C$", true),    // Nicaraguan córdoba
    ("R$", true),    // Brazilian real
    ("S/.", false),  // Peruvian sol
    ("S/", false),   // Peruvian sol, as CLDR writes it
    ("$U", false),   // Uruguayan peso
    ("Q", false),    // Guatemalan quetzal
    ("L", false),    // Honduran lempira
    ("B/.", false),  // Panamanian balboa
];

/// Trim spaces and single-character currency signs from either end.
fn trim_currency(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || CURRENCY.contains(&c))
}

/// Strip a multi-character currency sign from one end. A sign that may not touch the number must
/// be followed (as a prefix) or preceded (as a suffix) by whitespace, as Word reads it.
fn strip_symbol(s: &str, suffix: bool) -> Option<&str> {
    CURRENCY_SYMBOLS.iter().find_map(|(sign, may_touch)| {
        let rest = if suffix { s.strip_suffix(*sign)? } else { s.strip_prefix(*sign)? };
        let boundary = if suffix { rest.chars().next_back() } else { rest.chars().next() };
        (*may_touch || boundary.is_some_and(char::is_whitespace)).then_some(rest)
    })
}

/// Strip a currency symbol or a short currency word (`zł`, `kr`, `PLN`, `R$`, `Bs.`) from either
/// end.
fn strip_currency(s: &str) -> &str {
    let mut s = s.trim();
    if let Some(rest) = strip_symbol(s, false) {
        s = trim_currency(rest);
    }
    if let Some(rest) = strip_symbol(s, true) {
        s = trim_currency(rest);
    }
    s = trim_currency(s);
    if let Some((head, word)) = s.rsplit_once(SPACES)
        && (1..=3).contains(&word.chars().count())
        && word.chars().all(|c| c.is_alphabetic() || c == '.')
    {
        s = head;
    }
    if let Some((word, tail)) = s.split_once(SPACES)
        && word.chars().count() == 3
        && word.chars().all(|c| c.is_ascii_uppercase())
    {
        s = tail;
    }
    s.trim_matches(|c: char| c.is_whitespace() || CURRENCY.contains(&c))
}

/// Digits of an integer part with group separators `groups`: the first group has 1–3 digits,
/// every later one exactly 3.
fn grouped_digits(int: &str, groups: &[char]) -> Option<String> {
    if int.chars().all(|c| c.is_ascii_digit()) {
        return Some(int.to_string());
    }
    let parts: Vec<&str> = int.split(groups).collect();
    let (first, rest) = parts.split_first()?;
    // A first group of `0` is never grouping (`0.125` is a fraction, not 125).
    let ok = (1..=3).contains(&first.len())
        && !first.starts_with('0')
        && first.chars().all(|c| c.is_ascii_digit())
        && rest.iter().all(|g| g.len() == 3 && g.chars().all(|c| c.is_ascii_digit()));
    ok.then(|| parts.concat())
}

/// Parse `s` as a number written in language `lang`, or `None` when it isn't one. The decimal
/// separator is the language's; group separators are spaces (and `.` or `'` where the decimal
/// separator is a comma, `,` or `'` where it is a point) between groups of three digits. A
/// currency sign or short currency word is ignored, and so is a trailing `%` (`12 %` is 12, as
/// table sort reads it). In comma languages a lone `.` that cannot be a group separator (`1.5`)
/// is read as a decimal point. A trailing separator with no digits after it (`1.`, an ordinal
/// in a numbered column) is the whole number. A minus may be `-`, `−` or an en dash.
pub fn parse_number(s: &str, lang: Option<&str>) -> Option<f64> {
    let dec = decimal_separator(lang);
    let s = s.trim();
    let s = s.strip_suffix('%').map_or(s, str::trim_end);
    let mut body = strip_currency(s);
    let mut neg = false;
    if let Some(rest) = body.strip_prefix(['-', '\u{2212}', '\u{2013}']) {
        neg = true;
        body = strip_currency(rest);
    } else if let Some(rest) = body.strip_prefix('+') {
        body = strip_currency(rest);
    }
    if body.is_empty() || body.len() > 400 {
        return None;
    }
    let mut groups: Vec<char> = SPACES.to_vec();
    groups.extend(['\'', '\u{2019}']);
    groups.push(if dec == ',' { '.' } else { ',' });
    let (int, frac) = match body.split_once(dec) {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let int_digits = match grouped_digits(int, &groups) {
        Some(d) => d,
        None if dec == ',' && frac.is_none() && int.matches('.').count() == 1 => {
            return parse_number(&format!("{}{}", if neg { "-" } else { "" }, int.replace('.', ",")), lang);
        }
        None => return None,
    };
    let frac_digits = match frac {
        Some(f) if f.chars().all(|c| c.is_ascii_digit()) => f,
        Some(_) => return None,
        None => "",
    };
    if int_digits.is_empty() && frac_digits.is_empty() {
        return None;
    }
    let v: f64 = format!("0{int_digits}.{frac_digits}0").parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(if neg { -v } else { v })
}

/// Write `v` the way language `lang` writes a formula result: whole numbers without decimals,
/// others with two decimals and the language's decimal separator (`5,75` in Polish).
pub fn format_number(v: f64, lang: Option<&str>) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 9.2e18 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.2}");
    let s = if s == "-0.00" { "0.00".to_string() } else { s };
    if decimal_separator(lang) == ',' { s.replace('.', ",") } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polish_numbers() {
        let pl = Some("pl-PL");
        assert_eq!(parse_number("1,5", pl), Some(1.5));
        assert_eq!(parse_number("3,50", pl), Some(3.5));
        assert_eq!(parse_number("1 250,00", pl), Some(1250.0));
        assert_eq!(parse_number("1\u{a0}250,00 zł", pl), Some(1250.0));
        assert_eq!(parse_number("1.250,00", pl), Some(1250.0));
        assert_eq!(parse_number("1.5", pl), Some(1.5));
        assert_eq!(parse_number("-2,25", pl), Some(-2.25));
        assert_eq!(parse_number("PLN 12", pl), Some(12.0));
        assert_eq!(parse_number("-B/. 1 250,50", pl), Some(-1250.5));
        assert_eq!(parse_number("abc", pl), None);
        assert_eq!(parse_number("1,2,3", pl), None);
        // From review: ordinals, a leading zero, percentages and an en dash minus.
        assert_eq!(parse_number("1.", pl), Some(1.0), "a numbered column: 1.");
        assert_eq!(parse_number("10.", pl), Some(10.0));
        assert_eq!(parse_number("0.125", pl), Some(0.125), "a leading 0 is never a group");
        assert_eq!(parse_number("0,125", pl), Some(0.125));
        assert_eq!(parse_number("12 %", pl), Some(12.0));
        assert_eq!(parse_number("12%", pl), Some(12.0));
        assert_eq!(parse_number("\u{2013}2,5", pl), Some(-2.5));
        assert_eq!(parse_number("%", pl), None);
        assert_eq!(parse_number(".", pl), None);
        assert_eq!(format_number(5.75, pl), "5,75");
        assert_eq!(format_number(1350.0, pl), "1350");
        assert_eq!(format_number(-0.001, pl), "0,00", "a rounded zero has no sign");
    }

    #[test]
    fn overflowing_digit_strings_are_not_numbers() {
        assert_eq!(parse_number(&"1".repeat(350), None), None);
        assert_eq!(parse_number(&"9".repeat(400), Some("pl-PL")), None);
        assert_eq!(parse_number(&format!("-{}", "1".repeat(350)), None), None);
        let big = parse_number(&"1".repeat(300), None);
        assert!(big.is_some_and(f64::is_finite), "{big:?}");
    }

    #[test]
    fn cuban_and_peruvian_numbers_use_decimal_points() {
        for lang in [Some("es-CU"), Some("es-PE")] {
            assert_eq!(parse_number("1.234", lang), Some(1.234));
            assert_eq!(parse_number("1,234.50", lang), Some(1234.5));
            assert_eq!(format_number(5.75, lang), "5.75");
        }
        assert_eq!(parse_number("1.234,50", Some("es-ES")), Some(1234.5));
        assert_eq!(format_number(5.75, Some("es-ES")), "5,75");
    }

    #[test]
    fn balboa_numbers() {
        let en = Some("en-US");
        // Word 16.0 reads `B/.` as a word of its own, before or after the number.
        assert_eq!(parse_number("B/. 1,234.50", en), Some(1234.5));
        assert_eq!(parse_number("1,234.50 B/.", en), Some(1234.5));
        assert_eq!(parse_number("B/.\u{a0}1,234.50", en), Some(1234.5), "a no-break space after the sign");
        assert_eq!(parse_number("B/. 1,234.50 B/.", en), Some(1234.5), "the sign at both ends");
        assert_eq!(parse_number("-B/. 1,234.50", en), Some(-1234.5), "a minus before the sign");
        assert_eq!(parse_number("B/. 1,234.50", Some("es-PA")), Some(1234.5), "the balboa's own region");
        // The sign has no space of its own in Word: touching the number is text.
        assert_eq!(parse_number("B/.1,234.50", en), None);
        assert_eq!(parse_number("1,234.50B/.", en), None);
        assert_eq!(parse_number("B/ 1,234.50", en), None, "the balboa is B/. in Word");
        assert_eq!(parse_number("B/.", en), None);
        // The slash and the full stop make `B/.` ambiguous with a fraction: the sign is matched at
        // the ends of the text only, so a fraction is never read as a balboa amount and a sign in
        // the middle is not a sign.
        assert_eq!(parse_number("1/.5", en), None, "a fraction is not a price");
        assert_eq!(parse_number("1/.5", Some("es-PA")), None);
        assert_eq!(parse_number("1/.5 B/.", en), None, "the sign at the end does not rescue a fraction");
        assert_eq!(parse_number("B/. 1/.5", en), None);
        assert_eq!(parse_number("2 B/. 1,234.50", en), None, "the sign sits before the number");
        assert_eq!(parse_number("1,234.50 B/. 1,234.50", en), None, "the sign sits between two numbers");
        assert_eq!(parse_number("1,234.50/2", en), None, "a division is not a balboa amount");
        assert_eq!(parse_number("B/. 1,234.50,50", en), None, "a second decimal separator");
    }

    #[test]
    fn english_numbers() {
        let en = Some("en-US");
        assert_eq!(parse_number("1,250.50", en), Some(1250.5));
        assert_eq!(parse_number("$1,250", en), Some(1250.0));
        assert_eq!(parse_number("PAB 1,250.50", en), Some(1250.5), "the ISO code");
        assert_eq!(parse_number("1.5", en), Some(1.5));
        assert_eq!(parse_number("12", None), Some(12.0));
        assert_eq!(parse_number(".5", None), Some(0.5));
        assert_eq!(parse_number("1,5", en), None);
        assert_eq!(parse_number("1.", en), Some(1.0));
        assert_eq!(parse_number("0,125", en), None, "never 125");
        assert_eq!(format_number(5.75, en), "5.75");
        assert_eq!(format_number(-0.001, en), "0.00", "a rounded zero has no sign");
        assert_eq!(format_number(-0.0, None), "0");
        assert_eq!(decimal_separator(Some("de-CH")), '.');
        assert_eq!(decimal_separator(Some("de-DE")), ',');
    }

    #[test]
    fn latin_american_currencies() {
        let en = Some("en-US");
        // Signs that may touch the number, as Word reads them.
        for s in [
            "US$1,250.50",
            "US$ 1,250.50",
            "Arg$1,250.50",
            "Arg$ 1,250.50",
            "R$1,250.50",
            "R$ 1,250.50",
            "CUP$1,250.50",
            "CUP$ 1,250.50",
            "RD$1,250.50",
            "RD$ 1,250.50",
            "MX$1,250.50",
            "MX$ 1,250.50",
            "C$1,250.50",
            "C$ 1,250.50",
        ] {
            assert_eq!(parse_number(s, en), Some(1250.5), "{s}");
        }
        // Signs that are a word of their own, as Word reads them.
        for s in ["Bs. 1,250.50", "Bs 1,250.50", "Bs.S 1,250.50", "S/. 1,250.50", "S/ 1,250.50", "$U 1,250.50", "Q 1,250.50", "L 1,250.50"] {
            assert_eq!(parse_number(s, en), Some(1250.5), "{s}");
        }
        for s in ["Bs.1,250.50", "Bs1,250.50", "Bs.S1,250.50", "S/.1,250.50", "S/1,250.50", "$U1,250.50", "Q1,250.50", "L1,250.50"] {
            assert_eq!(parse_number(s, en), None, "{s}");
        }
        // Suffixes.
        for s in [
            "1,250.50 Bs.",
            "1,250.50 R$",
            "1,250.50 S/.",
            "1,250.50 S/",
            "1,250.50 Q",
            "1,250.50 L",
            "1,250.50 $U",
            "1,250.50 C$",
            "1,250.50 RD$",
            "1,250.50 MX$",
            "1,250.50 US$",
            "1,250.50 Arg$",
            "1,250.50 CUP$",
        ] {
            assert_eq!(parse_number(s, en), Some(1250.5), "{s}");
        }
        // The ISO codes were already read as three-letter currency words.
        for code in ["ARS", "BOB", "BRL", "CLP", "COP", "CUP", "DOP", "GTQ", "HNL", "MXN", "NIO", "PEN", "UYU", "VES", "USD"] {
            assert_eq!(parse_number(&format!("{code} 1,250.50"), en), Some(1250.5), "{code}");
        }
        // Locale separators and a sign before the currency sign.
        assert_eq!(parse_number("R$ 1.250,50", Some("pt-BR")), Some(1250.5));
        assert_eq!(parse_number("Bs. 1 250,50", Some("es-BO")), Some(1250.5));
        assert_eq!(parse_number("-R$ 1,250.50", en), Some(-1250.5));
        for s in ["Bs.", "Bs", "R$", "US$", "S/.", "S/", "$U", "Q", "L"] {
            assert_eq!(parse_number(s, en), None, "{s}");
        }
    }

    #[test]
    fn regional_separators_and_whole_amounts() {
        // Latin American regions that write a decimal point (CLDR) versus the rest of Spanish America and Brazil.
        for (s, lang) in [
            ("MX$ 1,250.50", "es-MX"),
            ("US$ 1,250.50", "es-PA"),
            ("S/. 1,250.50", "es-PE"),
            ("CUP$ 1,250.50", "es-CU"),
            ("Q 1,250.50", "es-GT"),
            ("L 1,250.50", "es-HN"),
            ("C$ 1,250.50", "es-NI"),
            ("RD$ 1,250.50", "es-DO"),
        ] {
            assert_eq!(parse_number(s, Some(lang)), Some(1250.5), "{lang}: {s}");
        }
        for (s, lang) in [
            ("R$ 1.250,50", "pt-BR"),
            ("Arg$ 1.250,50", "es-AR"),
            ("Bs. 1.250,50", "es-BO"),
            ("CLP 1.250,50", "es-CL"),
            ("COP 1.250,50", "es-CO"),
            ("CRC 1.250,50", "es-CR"),
            ("Bs. 1.250,50", "es-VE"),
            ("$U 1.250,50", "es-UY"),
            ("PYG 1.250,50", "es-PY"),
        ] {
            assert_eq!(parse_number(s, Some(lang)), Some(1250.5), "{lang}: {s}");
        }
        // Whole amounts without cents: the region's group separator, no decimal part.
        for (s, lang) in [
            ("MX$ 1,250", "es-MX"),
            ("US$ 1,250", "es-PA"),
            ("S/. 1,250", "es-PE"),
            ("CUP$ 1,250", "es-CU"),
            ("Q 1,250", "es-GT"),
            ("L 1,250", "es-HN"),
            ("C$ 1,250", "es-NI"),
            ("RD$ 1,250", "es-DO"),
        ] {
            assert_eq!(parse_number(s, Some(lang)), Some(1250.0), "{lang}: {s}");
        }
        for (s, lang) in [
            ("R$ 1.250", "pt-BR"),
            ("Arg$ 1.250", "es-AR"),
            ("Bs. 1.250", "es-BO"),
            ("Bs. 1.250", "es-VE"),
            ("$U 1.250", "es-UY"),
            ("PYG 1.250", "es-PY"),
        ] {
            assert_eq!(parse_number(s, Some(lang)), Some(1250.0), "{lang}: {s}");
        }
        // The other form is not that region's number when the separators change meaning.
        assert_eq!(parse_number("R$ 1,250.50", Some("pt-BR")), None);
        assert_eq!(parse_number("S/. 1.250,50", Some("es-PE")), None);
        // Whole results show no decimals; fractional results use the region's separator.
        assert_eq!(format_number(1250.0, Some("pt-BR")), "1250");
        assert_eq!(format_number(1250.0, Some("es-PE")), "1250");
        assert_eq!(format_number(1250.5, Some("pt-BR")), "1250,50");
        assert_eq!(format_number(1250.5, Some("es-PE")), "1250.50");
    }

    #[test]
    fn hostile_numbers_never_panic() {
        for s in ["", "-", "+", ",", ".", "1e400", "inf", "NaN", "--1", "₽", "1 ,", " zł", "\u{2212}", "9".repeat(500).as_str()] {
            for l in [None, Some("pl"), Some("en"), Some("")] {
                let _ = parse_number(s, l);
            }
        }
        assert_eq!(format_number(f64::NAN, None), "0");
    }

    #[test]
    fn large_whole_numbers_have_no_decimals() {
        assert_eq!(format_number(1e15, None), "1000000000000000");
        assert_eq!(format_number(2.5e15, None), "2500000000000000");
        assert_eq!(format_number(-1e15, None), "-1000000000000000");
        assert_eq!(format_number(9.0e18, None), "9000000000000000000");
        assert_eq!(format_number(1e15 + 0.5, None), "1000000000000000.50");
    }
}
