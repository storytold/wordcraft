//! The EQ field (ECMA-376 Part 1 §17.16.5.20): Word draws the formula the field code describes,
//! so the code is the content — there is usually no stored result to show.
//!
//! - `\o`: its arguments are drawn on top of one another, centred (`\ac`, the default),
//!   left-aligned (`\al`) or right-aligned (`\ar`). Home › Font › Enclose Characters stores a
//!   character in a circle, square, triangle or diamond as one: ` eq \o\ac(○,字)`.
//! - `\f(a,b)`: a fraction, `\r(x)` or `\r(n,x)`: a radical (with a degree), `\s\upN(…)` and
//!   `\s\doN(…)`: raised and lowered text — read into the math tree by [`parse_math`].
//!
//! Which of the two enclosure styles a field has is kept in the document model as a trailing
//! `\* enlarge` (WordCraft's own; nothing is written for the default "shrink text" style). In a
//! `.docx` the style is Word's: the font sizes of the field code's runs (the text smaller than the
//! symbol, or the symbol larger than the text), so the marker never reaches a file. Superscript
//! and subscript runs in the code are the same: Word draws them raised in the formula, so
//! [`parse_math_with`] takes their byte ranges.

use serde::{Deserialize, Serialize};

use crate::math::{Arg, FracKind, MNode, MRun, Math, ScriptKind};

/// How the overstruck arguments line up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// An `EQ \o(…)` field: the arguments' text (nested switches flattened to their text), and the
/// arguments as written (for [`overstrike_math`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overstrike {
    pub align: OAlign,
    pub args: Vec<String>,
    pub raw: Vec<String>,
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
    let mut raw = Vec::new();
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
                    raw.push(std::mem::take(&mut cur));
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
        raw.push(cur);
    }
    Some(Overstrike { align, args, raw })
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
                    while it.next_if(char::is_ascii_alphabetic).is_some() {}
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

/// An `EQ` field's formula as read from its code: the tree to draw, and the code's runs with
/// their formatting, so a save writes the code as the file had it (the `2` in `a2` keeps the
/// superscript run Word draws it with).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Formula {
    pub math: Math,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<(usize, usize, crate::props::CharProps)>,
}

/// How a piece of an EQ field's code is drawn: Word raises or lowers the formula's text where
/// the code's runs are superscript or subscript (`w:vertAlign`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raise {
    Sup,
    Sub,
}

/// Longest field code read as a formula. An EQ formula larger than this is passed over (the
/// field keeps its stored result): it isn't a formula anyone writes, and a hostile file could
/// otherwise make the reader read megabytes of switches.
const MAX_EQ_CODE: usize = 32 * 1024;
/// Deepest EQ structure read; deeper code keeps the field's stored result.
const MAX_EQ_DEPTH: usize = 32;
/// Most nodes one formula may hold.
const MAX_EQ_NODES: usize = 8192;

/// Parse an `EQ` field's formula into the math tree [OMML equations](crate::math) use: `\f`
/// fractions, `\r`/`\R` radicals, `\s` raised and lowered text, and `\o` overstrikes nested in
/// them. `None` for any other field, and for code with a switch this doesn't read (the field
/// keeps its stored result then).
pub fn parse_math(instr: &str) -> Option<Math> {
    parse_math_with(instr, &[])
}

/// Parse an EQ field's formula; `raises` gives the byte ranges of the code whose runs are
/// superscript or subscript, which Word draws raised or lowered in the formula.
pub fn parse_math_with(instr: &str, raises: &[(usize, usize, Raise)]) -> Option<Math> {
    if instr.len() > MAX_EQ_CODE {
        return None;
    }
    let mut p = EqParser { s: instr, i: 0, raises, nodes: 0 };
    p.name()?;
    let nodes = p.seq(0, false)?;
    if !p.rest().trim().is_empty() || nodes.is_empty() {
        return None;
    }
    Some(Math { nodes, ..Default::default() })
}

/// Parse an EQ expression on its own — a switch and its text, without the field's name — as an
/// overstrike argument may be a formula (`\f(5,2)`, `\r(5)`).
pub fn parse_expr(s: &str) -> Option<Math> {
    if s.len() > MAX_EQ_CODE {
        return None;
    }
    let mut p = EqParser { s, i: 0, raises: &[], nodes: 0 };
    let nodes = p.seq(0, false)?;
    if !p.rest().trim().is_empty() || nodes.is_empty() {
        return None;
    }
    Some(Math { nodes, ..Default::default() })
}

/// The formula an overstrike shows when exactly one argument is not blank and it is a formula:
/// some generators overstrike a space with a fraction (`\o\ac(　,\f(5,2))`), which Word draws as
/// the fraction. `None` for an overstrike of plain text, and for an enclosure's symbol and text,
/// which keep the overstrike drawing.
pub fn overstrike_math(o: &Overstrike) -> Option<Math> {
    if o.args.iter().any(|a| EncloseShape::from_symbol(a).is_some()) {
        return None;
    }
    let mut visible = o.raw.iter().filter(|a| !a.trim().is_empty());
    let only = visible.next()?;
    if visible.next().is_some() {
        return None;
    }
    let math = parse_expr(only)?;
    math.nodes.iter().any(|n| !matches!(n, MNode::Run(_))).then_some(math)
}

/// A parser over one field code.
struct EqParser<'a> {
    s: &'a str,
    i: usize,
    raises: &'a [(usize, usize, Raise)],
    nodes: usize,
}

impl<'a> EqParser<'a> {
    fn rest(&self) -> &'a str {
        self.s.get(self.i..).unwrap_or("")
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i = self.i.checked_add(c.len_utf8())?;
        Some(c)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if !c.is_whitespace() {
                break;
            }
            self.i = self.i.saturating_add(c.len_utf8());
        }
    }

    fn raise_at(&self, pos: usize) -> Option<Raise> {
        self.raises.iter().find(|(start, end, _)| *start <= pos && pos < *end).map(|(_, _, r)| *r)
    }

    /// The field's name: `EQ`, also with the stray backslash some generators write (`\eq`).
    fn name(&mut self) -> Option<()> {
        self.skip_ws();
        if self.peek() == Some('\\') {
            self.bump()?;
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.bump()?;
        }
        self.s.get(start..self.i)?.eq_ignore_ascii_case("eq").then_some(())
    }

    /// The code's items: text, structures and escapes, up to the end, or in an argument up to the
    /// `,`/`;` or `)` that closes it.
    fn seq(&mut self, depth: usize, arg: bool) -> Option<Arg> {
        if depth > MAX_EQ_DEPTH {
            return None;
        }
        let mut out: Arg = Vec::new();
        let mut text = String::new();
        let mut raised: Option<(Raise, String, Arg)> = None;
        let mut paren = 0usize;
        loop {
            self.skip_ws();
            let Some(c) = self.peek() else { break };
            // Arguments end at a top-level `,`, `;` or `)`: a literal parenthesis's depth keeps
            // their contents together (`\r((−2)2)`).
            if arg && paren == 0 && matches!(c, ')' | ',' | ';') {
                break;
            }
            match c {
                ')' => {
                    paren = paren.saturating_sub(1);
                    self.bump()?;
                    self.push_char(&mut out, &mut text, &mut raised, c);
                }
                '(' => {
                    paren = paren.saturating_add(1);
                    self.bump()?;
                    self.push_char(&mut out, &mut text, &mut raised, c);
                }
                '\\' => {
                    let next = self.rest().get(1..).and_then(|r| r.chars().next());
                    match next {
                        Some(n) if n.is_ascii_alphabetic() => {
                            Self::flush_raise(&mut out, &mut raised);
                            Self::flush_text(&mut out, &mut text);
                            let nodes = self.switch(depth)?;
                            self.nodes = self.nodes.checked_add(nodes.len())?;
                            if self.nodes > MAX_EQ_NODES {
                                return None;
                            }
                            out.extend(nodes);
                        }
                        // A general switch (`\* MERGEFORMAT`): not part of the formula.
                        Some('*') => {
                            self.bump()?;
                            self.bump()?;
                            self.skip_ws();
                            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                                self.bump()?;
                            }
                        }
                        Some(n) if matches!(n, '\\' | '(' | ')' | ',' | ';' | ':' | '/') => {
                            self.bump()?;
                            self.bump()?;
                            self.push_char(&mut out, &mut text, &mut raised, n);
                        }
                        _ => return None,
                    }
                }
                _ => {
                    self.bump()?;
                    self.push_char(&mut out, &mut text, &mut raised, c);
                }
            }
        }
        Self::flush_raise(&mut out, &mut raised);
        Self::flush_text(&mut out, &mut text);
        Some(out)
    }

    /// One character into the formula: text, or the content of a superscript/subscript run.
    fn push_char(&self, out: &mut Arg, text: &mut String, raised: &mut Option<(Raise, String, Arg)>, c: char) {
        let Some(raise) = self.raise_at(self.i.saturating_sub(c.len_utf8())) else {
            Self::flush_raise(out, raised);
            text.push(c);
            return;
        };
        match raised {
            Some((kind, s, _)) if *kind == raise => s.push(c),
            _ => {
                Self::flush_raise(out, raised);
                Self::flush_text(out, text);
                let base = Self::take_base(out);
                *raised = Some((raise, String::new(), base));
                if let Some((_, s, _)) = raised {
                    s.push(c);
                }
            }
        }
    }

    /// End the pending superscript/subscript run: its text off the base captured when it began.
    fn flush_raise(out: &mut Arg, raised: &mut Option<(Raise, String, Arg)>) {
        let Some((raise, s, base)) = raised.take() else { return };
        let (kind, sub, sup) = match raise {
            Raise::Sup => (ScriptKind::Sup, Vec::new(), vec![MNode::Run(MRun::new(s))]),
            Raise::Sub => (ScriptKind::Sub, vec![MNode::Run(MRun::new(s))], Vec::new()),
        };
        out.push(MNode::Script { kind, base, sub, sup });
    }

    fn flush_text(out: &mut Arg, text: &mut String) {
        if !text.is_empty() {
            out.push(MNode::Run(MRun::new(std::mem::take(text))));
        }
    }

    /// The base a superscript attaches to: the last character of the last run, or the last
    /// structure whole.
    fn take_base(out: &mut Arg) -> Arg {
        match out.last_mut() {
            Some(MNode::Run(r)) => {
                let Some((at, _)) = r.text.char_indices().last() else { return Vec::new() };
                let base = r.text.get(at..).unwrap_or("").to_string();
                r.text.truncate(at);
                if r.text.is_empty() {
                    out.pop();
                }
                vec![MNode::Run(MRun::new(base))]
            }
            Some(_) => out.pop().map(|n| vec![n]).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// A switch after its backslash: `\f`, `\r`/`\R`, `\s`, `\o`.
    fn switch(&mut self, depth: usize) -> Option<Arg> {
        self.bump()?;
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.bump()?;
        }
        match self.s.get(start..self.i)?.to_ascii_lowercase().as_str() {
            "f" => {
                let args = self.args(depth)?;
                let [num, den] = args.as_slice() else { return None };
                Some(vec![MNode::Frac { kind: FracKind::Bar, num: num.clone(), den: den.clone() }])
            }
            "r" => {
                let args = self.args(depth)?;
                let (deg, e) = match args.as_slice() {
                    [e] => (Vec::new(), e.clone()),
                    [deg, e] => (deg.clone(), e.clone()),
                    _ => return None,
                };
                Some(vec![MNode::Rad { deg_hide: deg.is_empty(), deg, e }])
            }
            "o" => {
                // Its arguments, after any alignment option: an overstruck pair shows the one
                // with content (an overstruck space and formula is how some generators write it).
                loop {
                    self.skip_ws();
                    match self.peek()? {
                        '(' => break,
                        '\\' => {
                            self.bump()?;
                            let option = self.take_letters(2)?;
                            if !matches!(option.to_ascii_lowercase().as_str(), "al" | "ac" | "ar") {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                let args = self.args(depth)?;
                // Word overstrikes every visible argument; the math tree has no overstrike, so
                // drawing only one of several would be wrong. Leave the field's stored result.
                if args.iter().filter(|a| !arg_blank(a)).count() > 1 {
                    return None;
                }
                Some(args.into_iter().find(|a| !arg_blank(a)).unwrap_or_default())
            }
            "s" => {
                let mut kind = None;
                loop {
                    self.skip_ws();
                    match self.peek()? {
                        '(' => break,
                        '\\' => {
                            self.bump()?;
                            kind = Some(match self.take_letters(2)?.to_ascii_lowercase().as_str() {
                                "up" => ScriptKind::Sup,
                                "do" => ScriptKind::Sub,
                                _ => return None,
                            });
                            // The raise in points: the math layout places scripts itself.
                            self.skip_ws();
                            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '.') {
                                self.bump()?;
                            }
                        }
                        _ => return None,
                    }
                }
                let kind = kind?;
                let args = self.args(depth)?;
                let [e] = args.as_slice() else { return None };
                let (sub, sup) = match kind {
                    ScriptKind::Sup => (Vec::new(), e.clone()),
                    _ => (e.clone(), Vec::new()),
                };
                Some(vec![MNode::Script { kind, base: Vec::new(), sub, sup }])
            }
            _ => None,
        }
    }

    fn take_letters(&mut self, n: usize) -> Option<String> {
        let start = self.i;
        let mut count = 0;
        while count < n && self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.bump()?;
            count += 1;
        }
        let s = self.s.get(start..self.i)?;
        (!s.is_empty()).then(|| s.to_string())
    }

    /// A switch's argument list: `(` arguments separated by `,` or `;` `)`.
    fn args(&mut self, depth: usize) -> Option<Vec<Arg>> {
        self.skip_ws();
        if self.bump()? != '(' {
            return None;
        }
        let mut args = Vec::new();
        loop {
            if args.len() > MAX_ARGS {
                return None;
            }
            args.push(self.seq(depth.saturating_add(1), true)?);
            self.skip_ws();
            match self.peek()? {
                ',' | ';' => {
                    self.bump()?;
                }
                ')' => {
                    self.bump()?;
                    break;
                }
                _ => return None,
            }
        }
        Some(args)
    }
}

/// Whether an argument shows nothing (an overstruck space, say).
fn arg_blank(arg: &[MNode]) -> bool {
    arg.iter().all(|n| match n {
        MNode::Run(r) => r.text.trim().is_empty(),
        _ => false,
    })
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

    fn run(s: &str) -> MNode {
        MNode::Run(MRun::new(s))
    }

    fn frac(num: MNode, den: MNode) -> MNode {
        MNode::Frac { kind: FracKind::Bar, num: vec![num], den: vec![den] }
    }

    #[test]
    fn eq_math_reads_fractions_radicals_and_nesting() {
        assert_eq!(parse_math(" EQ \\f(1,2)").map(|m| m.nodes), Some(vec![frac(run("1"), run("2"))]));
        // The radicand keeps what follows it: text, then a nested fraction.
        let m = parse_math(" EQ \\R(,1\\f(1,2))").unwrap();
        assert_eq!(m.nodes, vec![MNode::Rad { deg_hide: true, deg: Vec::new(), e: vec![run("1"), frac(run("1"), run("2"))] }]);
        // A fraction of radicals, as question 2 of the 11_題目 exam writes it.
        let m = parse_math(" eq \\f(\\R(,6),2)").unwrap();
        assert_eq!(
            m.nodes,
            vec![MNode::Frac {
                kind: FracKind::Bar,
                num: vec![MNode::Rad { deg_hide: true, deg: Vec::new(), e: vec![run("6")] }],
                den: vec![run("2")]
            }]
        );
        // A degree, `\r(3,8)`: the cube root of 8.
        let m = parse_math("EQ \\r(3,8)").unwrap();
        assert_eq!(m.nodes, vec![MNode::Rad { deg_hide: false, deg: vec![run("3")], e: vec![run("8")] }]);
        // Parentheses in an argument are its text, not its end.
        let m = parse_math("EQ \\r((－2)2)").unwrap();
        assert_eq!(m.nodes, vec![MNode::Rad { deg_hide: true, deg: Vec::new(), e: vec![run("(－2)2")] }]);
        // Some generators write a stray backslash before the field name.
        assert!(parse_math(" \\eq \\f(1,10)").is_some());
        // A general switch after the formula is not part of it.
        assert_eq!(parse_math("EQ \\f(1,2) \\* MERGEFORMAT").map(|m| m.nodes), Some(vec![frac(run("1"), run("2"))]));
    }

    #[test]
    fn eq_math_overstrikes_inside_a_radical_show_their_content() {
        let m = parse_math("EQ \\r(\\o\\ac(　,\\F(5,2)))").unwrap();
        assert_eq!(m.nodes, vec![MNode::Rad { deg_hide: true, deg: Vec::new(), e: vec![frac(run("5"), run("2"))] }]);
    }

    #[test]
    fn eq_math_skips_an_overstrike_with_several_visible_arguments() {
        // The math tree has no overstrike, so a nested overstrike of two visible arguments can't
        // be drawn: the field keeps its stored result instead of showing only one of them.
        assert!(parse_math("EQ \\r(\\o(1,2))").is_none());
        assert!(parse_expr("\\o(=,/)").is_none());
        // One visible argument is a formula overstruck with a blank, which is a formula.
        assert!(parse_math("EQ \\r(\\o(　,2))").is_some());
    }

    #[test]
    fn eq_math_reads_raised_text() {
        let m = parse_math("EQ \\s\\up6(2)").unwrap();
        assert_eq!(m.nodes, vec![MNode::Script { kind: ScriptKind::Sup, base: Vec::new(), sub: Vec::new(), sup: vec![run("2")] }]);
        let m = parse_math("EQ \\s\\do 6(2)").unwrap();
        assert_eq!(m.nodes, vec![MNode::Script { kind: ScriptKind::Sub, base: Vec::new(), sub: vec![run("2")], sup: Vec::new() }]);
    }

    #[test]
    fn eq_math_raises_the_superscript_runs_of_the_code() {
        // `a2－2ab＋b2` with the two 2s typed as superscript runs: a² − 2ab + b².
        let s = "EQ \\R(,a2－2ab＋b2)";
        let at = |c: char| s.match_indices(c).map(|(i, _)| i).collect::<Vec<_>>();
        let a = at('a')[0] + 1;
        let b = *at('b').last().unwrap_or(&0) + 1;
        let m = parse_math_with(s, &[(a, a + 1, Raise::Sup), (b, b + 1, Raise::Sup)]).unwrap();
        let e = m.nodes.iter().find_map(|n| match n {
            MNode::Rad { e, .. } => Some(e.clone()),
            _ => None,
        });
        let sup = |base: &str| MNode::Script { kind: ScriptKind::Sup, base: vec![run(base)], sub: Vec::new(), sup: vec![run("2")] };
        assert_eq!(e, Some(vec![sup("a"), run("－2ab＋"), sup("b")]));
    }

    #[test]
    fn eq_math_passes_over_huge_and_deeply_nested_code() {
        // A hostile file can't make the reader walk megabytes of switches, nor a structure deeper
        // than any real formula.
        let huge = format!("EQ \\f(1,2){}", "x".repeat(64 * 1024));
        assert!(parse_math(&huge).is_none());
        let mut deep = String::from("EQ ");
        for _ in 0..200 {
            deep.push_str("\\f(1,");
        }
        deep.push('2');
        for _ in 0..200 {
            deep.push(')');
        }
        assert!(parse_math(&deep).is_none());
    }

    #[test]
    fn eq_math_leaves_other_fields_and_unreadable_code_to_the_cached_result() {
        for code in ["PAGE", "EQ", "EQ 1\\f(1,2", "EQ \\a(1,2)", "EQ \\f(1)", "EQ \\f(1,2,3)", "EQ \\x(2)", "EQ \\s(2)"] {
            assert!(parse_math(code).is_none(), "{code}");
        }
        assert!(parse_overstrike("eq \\f(1,2)").is_none());
    }
}
