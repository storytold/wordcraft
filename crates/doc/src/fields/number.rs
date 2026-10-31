//! Shared field-number formatting for layout and file exporters.
use crate::section::NumFormat;

/// `\* FORMAT` switch value (Arabic, roman, ROMAN, alphabetic, ALPHABETIC, CardText, Ordinal,
/// ArabicDash, DollarText, Hex…). `{ PAGE \* ArabicDash }` shows "- 31 -".
pub fn format_switch(instr: &str) -> Option<NumFormat> {
    let mut it = instr.split_whitespace();
    while let Some(w) = it.next() {
        if let Some(rest) = w.strip_prefix("\\*") {
            let v = if rest.is_empty() { it.next()? } else { rest };
            let v = v.split('\\').next().unwrap_or(v);
            let fmt = match v {
                "roman" => NumFormat::LowerRoman,
                "ROMAN" | "Roman" => NumFormat::UpperRoman,
                "alphabetic" => NumFormat::LowerLetter,
                "ALPHABETIC" | "Alphabetic" => NumFormat::UpperLetter,
                "CardText" | "cardtext" => NumFormat::CardinalText,
                "OrdText" | "ordtext" => NumFormat::OrdinalText,
                "Ordinal" | "ordinal" => NumFormat::Ordinal,
                "Arabic" | "arabic" | "ARABIC" => NumFormat::Decimal,
                "ArabicDash" | "arabicdash" | "ARABICDASH" => NumFormat::NumberInDash,
                "DollarText" | "dollartext" | "DOLLARTEXT" => NumFormat::DollarText,
                "Hex" | "hex" | "HEX" => NumFormat::Hex,
                // Chinese numerals: counting (一百零一), legal (壹佰零壹), digit by digit (一〇一).
                v if v.eq_ignore_ascii_case("ChineseNum1") => NumFormat::ChineseCountingThousand,
                v if v.eq_ignore_ascii_case("ChineseNum2") => NumFormat::ChineseLegalSimplified,
                v if v.eq_ignore_ascii_case("ChineseNum3") => NumFormat::IdeographDigital,
                _ => continue,
            };
            return Some(fmt);
        }
    }
    None
}

/// A field's capitalization switch (`\* Upper`, `\* Lower`, `\* Caps`, `\* FirstCap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseSwitch {
    Upper,
    Lower,
    Caps,
    FirstCap,
}

/// The capitalization switches in a field code, in the order they appear. Quoted text in the code
/// is not a switch.
fn case_switches(instr: &str) -> Vec<CaseSwitch> {
    let mut chars = instr.chars().peekable();
    let mut quoted = false;
    let mut cases = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted => {
                chars.next();
            }
            '"' => quoted = !quoted,
            '\\' if chars.peek() == Some(&'*') => {
                chars.next();
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                let mut word = String::new();
                while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '\\' && *c != '"') {
                    word.push(c);
                }
                let case = match word.to_ascii_lowercase().as_str() {
                    "upper" => CaseSwitch::Upper,
                    "lower" => CaseSwitch::Lower,
                    "caps" => CaseSwitch::Caps,
                    "firstcap" => CaseSwitch::FirstCap,
                    _ => continue,
                };
                cases.push(case);
            }
            _ => {}
        }
    }
    cases
}

/// `text` with the field's capitalization switches applied in the order they appear, so
/// `\* Lower \* Caps` shows "WEEKLY REPORT" as "Weekly Report". `\* Upper` capitalizes every
/// letter, `\* Lower` makes every letter lowercase, `\* Caps` capitalizes the first letter of each
/// word, and `\* FirstCap` capitalizes the first letter of the first word. A word's first letter is
/// its first letter or digit, after any leading quote, bracket or other punctuation. Without a
/// switch, the function returns `text` unchanged.
pub fn apply_case_switch(instr: &str, text: String) -> String {
    case_switches(instr).into_iter().fold(text, apply_case)
}

/// `text` with one capitalization switch applied (see [`apply_case_switch`]).
fn apply_case(text: String, case: CaseSwitch) -> String {
    match case {
        CaseSwitch::Upper => text.to_uppercase(),
        CaseSwitch::Lower => text.to_lowercase(),
        CaseSwitch::Caps | CaseSwitch::FirstCap => {
            let mut out = String::with_capacity(text.len());
            let mut word_start = true;
            let mut done = false;
            for c in text.chars() {
                if c.is_whitespace() {
                    word_start = true;
                    out.push(c);
                } else if word_start && !done && c.is_alphanumeric() {
                    out.extend(c.to_uppercase());
                    word_start = false;
                    done = case == CaseSwitch::FirstCap;
                } else {
                    if c.is_alphanumeric() {
                        word_start = false;
                    }
                    out.push(c);
                }
            }
            out
        }
    }
}

/// Formats `n` in `fmt` as a field shows it. Cardinal, ordinal and dollar text is lowercase
/// (`\* CardText` shows "twenty-one", `\* DollarText` shows "twenty-one and 00/100") and writes
/// numbers up to 999,999; a larger number keeps its digits. `\* Hex` writes uppercase hexadecimal
/// digits (`1CA`). The field's capitalization switches apply after that (`\* CardText \* Caps`
/// shows "Twenty-one").
pub fn field_number(instr: &str, fmt: NumFormat, n: u32) -> String {
    let text = number_text(fmt, n);
    apply_case_switch(instr, text)
}

fn number_text(fmt: NumFormat, n: u32) -> String {
    match fmt {
        NumFormat::CardinalText | NumFormat::OrdinalText | NumFormat::DollarText if n > MAX_TEXT_NUMBER => n.to_string(),
        NumFormat::CardinalText | NumFormat::OrdinalText => fmt.format(n).to_lowercase(),
        NumFormat::DollarText => dollar_text(f64::from(n)).unwrap_or_else(|| n.to_string()),
        NumFormat::Hex => format!("{n:X}"),
        _ => fmt.format(n),
    }
}

/// The largest number cardinal, ordinal and dollar text write in words.
const MAX_TEXT_NUMBER: u32 = 999_999;

/// `value` as `\* DollarText` writes it, before any capitalization switch: the whole part in
/// lowercase cardinal text, "and", and the first two decimals (rounded) over 100
/// (14.55 gives "fourteen and 55/100"). `None` for NaN, or for a value that rounds below 0 or past
/// 999,999.99.
pub fn dollar_text(value: f64) -> Option<String> {
    let cents = (value * 100.0).round();
    if !(0.0..=f64::from(MAX_TEXT_NUMBER) * 100.0 + 99.0).contains(&cents) {
        return None;
    }
    let cents = cents as u32;
    Some(format!("{} and {:02}/100", NumFormat::CardinalText.format(cents / 100).to_lowercase(), cents % 100))
}

/// A page field's number, label and case switches, applied in that order.
pub fn labelled_number(instr: &str, label: Option<&crate::page_label::PageLabel>, fmt: NumFormat, n: u32) -> String {
    use crate::page_label::PageLabelMode;
    let Some(label) = label else { return field_number(instr, fmt, n) };
    let text = if !label.counts()
        || (n == 0 && matches!(fmt, NumFormat::UpperRoman | NumFormat::LowerRoman | NumFormat::UpperLetter | NumFormat::LowerLetter))
    {
        label.format(fmt, n)
    } else {
        let number = number_text(fmt, n);
        if label.mode == PageLabelMode::TextNumber && !label.text.is_empty() { format!("{}{}{number}", label.text, label.separator) } else { number }
    };
    apply_case_switch(instr, text)
}
