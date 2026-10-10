//! The EQ field's overstrike switch (ECMA-376 Part 1 §17.16.5.20, `\o`): its arguments are drawn
//! on top of one another, centred (`\ac`, the default), left-aligned (`\al`) or right-aligned
//! (`\ar`). Home › Font › Enclose Characters stores a character in a circle, square, triangle or
//! diamond as one: ` eq \o\ac(○,字)`.
//!
//! Which of the two enclosure styles a field has is kept in the document model as a trailing
//! `\* enlarge` (WordCraft's own; nothing is written for the default "shrink text" style). In a
//! `.docx` the style is Word's: the font sizes of the field code's runs (the text smaller than the
//! symbol, or the symbol larger than the text), so the marker never reaches a file.

/// How the overstruck arguments line up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// An `EQ \o(…)` field: the arguments' text (nested switches flattened to their text).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overstrike {
    pub align: OAlign,
    pub args: Vec<String>,
}

/// Most arguments read from one field (the rest are ignored).
const MAX_ARGS: usize = 16;

/// Parse an `EQ \o` field code; `None` for any other field (or EQ switch).
pub fn parse_overstrike(instr: &str) -> Option<Overstrike> {
    let t = instr.trim_start();
    let name_end = t.find(|c: char| c.is_whitespace() || c == '\\').unwrap_or(t.len());
    if !t.get(..name_end)?.eq_ignore_ascii_case("eq") {
        return None;
    }
    let mut rest = t.get(name_end..)?.trim_start();
    // `\o`, then its alignment options, then the argument list.
    rest = rest.strip_prefix('\\')?;
    let o = rest.chars().next()?;
    if !o.eq_ignore_ascii_case(&'o') {
        return None;
    }
    rest = rest.get(1..)?;
    let mut align = OAlign::Center;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix('(') {
            rest = r;
            break;
        }
        let r = rest.strip_prefix('\\')?;
        let sw = r.get(..2)?;
        align = match sw.to_ascii_lowercase().as_str() {
            "al" => OAlign::Left,
            "ac" => OAlign::Center,
            "ar" => OAlign::Right,
            _ => return None,
        };
        rest = r.get(2..)?;
    }
    // Arguments up to the matching `)`, split at top-level `,` or `;` (the list separator).
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut chars = rest.chars();
    let mut closed = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '(' => {
                depth = depth.saturating_add(1);
                cur.push(c);
            }
            ')' if depth == 0 => {
                closed = true;
                break;
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' | ';' if depth == 0 => {
                if args.len() < MAX_ARGS {
                    args.push(arg_text(&cur));
                }
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    if !closed {
        return None;
    }
    if args.len() < MAX_ARGS {
        args.push(arg_text(&cur));
    }
    Some(Overstrike { align, args })
}

/// An argument's text: escapes (`\,` `\(` `\)` `\\`) become the character, nested switches
/// (`\s\up4(…)`) and their brackets are dropped, leaving what they show.
fn arg_text(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.peek().copied() {
                Some(n @ (',' | '(' | ')' | '\\' | ';')) => {
                    out.push(n);
                    it.next();
                }
                // A switch: its letters, then an optional (signed) number.
                Some(_) => {
                    while it.next_if(|n| n.is_ascii_alphabetic()).is_some() {}
                    while it.next_if(|n| *n == ' ').is_some() {}
                    while it.next_if(|n| n.is_ascii_digit() || *n == '-' || *n == '.').is_some() {}
                }
                None => {}
            },
            '(' | ')' => {}
            c => out.push(c),
        }
    }
    out
}

/// Escape text for an EQ argument.
fn escape_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, ',' | '(' | ')' | '\\' | ';') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The enclosure drawn around enclosed characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EncloseShape {
    Circle,
    Square,
    Triangle,
    Diamond,
}

impl EncloseShape {
    pub const ALL: [EncloseShape; 4] = [EncloseShape::Circle, EncloseShape::Square, EncloseShape::Triangle, EncloseShape::Diamond];

    /// The symbol in the field code.
    pub fn symbol(self) -> char {
        match self {
            EncloseShape::Circle => '○',
            EncloseShape::Square => '□',
            EncloseShape::Triangle => '△',
            EncloseShape::Diamond => '◇',
        }
    }
    /// The shape a field-code symbol stands for (the usual symbols and their large or small
    /// variants).
    pub fn from_symbol(s: &str) -> Option<EncloseShape> {
        let mut it = s.trim().chars();
        let c = it.next()?;
        if it.next().is_some() {
            return None;
        }
        Some(match c {
            '○' | '◯' | '⭘' | '◦' | '〇' => EncloseShape::Circle,
            '□' | '◻' | '⬜' | '▢' | '◽' => EncloseShape::Square,
            '△' | '▵' | '⊿' => EncloseShape::Triangle,
            '◇' | '◊' | '⬦' | '⋄' => EncloseShape::Diamond,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            EncloseShape::Circle => "circle",
            EncloseShape::Square => "square",
            EncloseShape::Triangle => "triangle",
            EncloseShape::Diamond => "diamond",
        }
    }
    pub fn parse(s: &str) -> Option<EncloseShape> {
        EncloseShape::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// Shrink the text to fit the shape, or enlarge the shape around the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EncloseStyle {
    #[default]
    Shrink,
    Enlarge,
}

impl EncloseStyle {
    pub fn name(self) -> &'static str {
        match self {
            EncloseStyle::Shrink => "shrink",
            EncloseStyle::Enlarge => "enlarge",
        }
    }
    pub fn parse(s: &str) -> Option<EncloseStyle> {
        match s.trim().to_ascii_lowercase().as_str() {
            "shrink" => Some(EncloseStyle::Shrink),
            "enlarge" => Some(EncloseStyle::Enlarge),
            _ => None,
        }
    }
}

/// Enclosed characters: an `EQ \o` field with an enclosure symbol and the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Enclosure {
    pub shape: EncloseShape,
    pub style: EncloseStyle,
    pub text: String,
}

/// The model's marker for the "enlarge symbol" style (see the module docs).
const ENLARGE: &str = "\\* enlarge";

impl Enclosure {
    /// Enclosed characters from a field code: `\o` with exactly two arguments, one of them an
    /// enclosure symbol and the other non-empty text.
    pub fn parse(instr: &str) -> Option<Enclosure> {
        let o = parse_overstrike(instr)?;
        let [a, b] = o.args.as_slice() else { return None };
        let (shape, text) = match (EncloseShape::from_symbol(a), EncloseShape::from_symbol(b)) {
            (Some(s), _) => (s, b),
            (None, Some(s)) => (s, a),
            _ => return None,
        };
        if text.trim().is_empty() {
            return None;
        }
        let close = instr.rfind(')')?;
        let tail = instr.get(close + 1..).unwrap_or("").trim();
        let style = if tail.eq_ignore_ascii_case(ENLARGE) { EncloseStyle::Enlarge } else { EncloseStyle::Shrink };
        Some(Enclosure { shape, style, text: text.clone() })
    }

    /// The field code in the model (with the style marker; see [`Enclosure::word_instr`]).
    pub fn instr(&self) -> String {
        match self.style {
            EncloseStyle::Shrink => self.word_instr(),
            EncloseStyle::Enlarge => format!("{} {ENLARGE}", self.word_instr()),
        }
    }

    /// The field code as Word writes it: `eq \o\ac(○,字)`.
    pub fn word_instr(&self) -> String {
        format!("eq \\o\\ac({},{})", self.shape.symbol(), escape_arg(&self.text))
    }
}

/// Characters Word counts as full width (East Asian wide or fullwidth); two half-width characters
/// or one full-width character fit in an enclosure.
pub fn is_full_width(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD)
}

/// Whether `text` fits in an enclosure: one full-width character or one or two half-width ones.
pub fn fits_enclosure(text: &str) -> bool {
    let n = text.chars().count();
    let wide = text.chars().filter(|c| is_full_width(*c)).count();
    !text.chars().any(|c| c.is_control() || c == crate::para::OBJ) && (n == 1 || (n == 2 && wide == 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_word_enclosures_and_general_overstrike() {
        let e = Enclosure::parse(" eq \\o\\ac(○,字)").unwrap();
        assert_eq!((e.shape, e.style, e.text.as_str()), (EncloseShape::Circle, EncloseStyle::Shrink, "字"));
        // Nested switches flatten to their text; the list separator may be `;`.
        let e = Enclosure::parse("EQ \\o\\ac(□;\\s\\up 1(12))").unwrap();
        assert_eq!((e.shape, e.text.as_str()), (EncloseShape::Square, "12"));
        let e = Enclosure { shape: EncloseShape::Diamond, style: EncloseStyle::Enlarge, text: "a,".into() };
        assert_eq!(Enclosure::parse(&e.instr()), Some(e.clone()));
        assert_eq!(e.word_instr(), "eq \\o\\ac(◇,a\\,)");
        let o = parse_overstrike("eq \\o \\al(=,/)").unwrap();
        assert_eq!((o.align, o.args), (OAlign::Left, vec!["=".to_string(), "/".to_string()]));
        assert!(parse_overstrike("eq \\f(1,2)").is_none());
        assert!(parse_overstrike("PAGE").is_none());
        assert!(parse_overstrike("eq \\o(unclosed").is_none());
        assert!(Enclosure::parse("eq \\o(a,b)").is_none());
        assert!(fits_enclosure("字") && fits_enclosure("12") && !fits_enclosure("字字") && !fits_enclosure("123") && !fits_enclosure(""));
    }
}
