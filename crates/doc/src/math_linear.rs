//! The linear format (UnicodeMath, the plain text Word builds equations from) → math tree.
//!
//! Grammar, informally: an expression is a sequence of atoms and operators. Atoms are characters,
//! bracket groups (`(…)`, `[…]`, `{…}`, `|…|`, `〖…〗` invisible, `├…┤` one-sided, `│`
//! separators), prefix structures (`√x`, `∑_a^b▒x`, `█(…)`, `■(…)`, `▭x`, `□x`, `¯x`, `▁x`,
//! `⏞x`, `⏟x`, `⟡x`), function names (`sin x`, `log_2 x`, `lim┬(n→∞) x`) and placeholders (`⬚`).
//! Postfix operators apply to the atom before them: `^` `_` scripts, combining accents, `┬` `┴`
//! limits. `/` `⁄` `∕` `¦` take the operands on either side (an operand is a run of atoms up to
//! an operator or space; a parenthesised operand loses its parentheses). `^`/`_` with nothing
//! before them are pre-scripts. A top-level `#` numbers the equation (`E=mc^2#(1)`).

use crate::math::{Arg, FracKind, MAX_DEPTH, MClass, MNode, MRun, ScriptKind, is_nary_char, math_class, merge_runs};

/// Function names set upright and applied to the following operand.
pub const FUNCTION_NAMES: &[&str] = &[
    "arccos", "arccot", "arccsc", "arcsec", "arcsin", "arctan", "arg", "cos", "cosh", "cot", "coth", "csc", "csch", "deg", "det", "dim", "exp",
    "gcd", "hom", "inf", "ker", "lg", "lim", "liminf", "limsup", "ln", "log", "max", "min", "mod", "Pr", "sec", "sech", "sin", "sinh", "sup", "tan",
    "tanh",
];

/// Function names whose subscripts go underneath (`lim_(n→∞)`).
const LIMIT_NAMES: &[&str] = &["lim", "liminf", "limsup", "max", "min", "sup", "inf", "det", "gcd", "Pr"];

/// The empty-argument placeholder.
pub const PLACEHOLDER: char = '⬚';

/// Parse linear-format text. Never fails: what isn't understood stays as text.
pub fn parse(s: &str) -> Arg {
    let mut p = P { c: s.chars().take(20_000).collect(), i: 0, depth: 0, no_limits: 0, steps: 0 };
    let mut out = Vec::new();
    // Stray closers at the top level are text.
    while p.i < p.c.len() {
        out.extend(p.expr(&[]));
        if let Some(c) = p.peek() {
            p.i += 1;
            out.push(text(c));
        }
    }
    let mut out = clean(out);
    // `expr#(n)`: a numbered equation.
    if out.iter().any(|n| matches!(n, MNode::Run(r) if r.text.contains('#'))) {
        out = vec![MNode::EqArr { rows: vec![out] }];
    }
    out
}

fn text(c: char) -> MNode {
    MNode::Run(MRun::new(c.to_string()))
}

/// Closing bracket for an opening one.
pub fn closer(c: char) -> Option<char> {
    Some(match c {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '⟨' => '⟩',
        '〈' => '〉',
        '⌈' => '⌉',
        '⌊' => '⌋',
        '⟦' => '⟧',
        '〖' => '〗',
        _ => return None,
    })
}

fn is_closer(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '⟩' | '〉' | '⌉' | '⌋' | '⟧' | '〗' | '┤')
}

/// Prefix characters that make a structure from the operand after them.
fn is_prefix(c: char) -> bool {
    matches!(c, '√' | '∛' | '∜' | '█' | '■' | '▭' | '□' | '¯' | '▁' | '⟡' | '⏞' | '⏟' | '⏜' | '⏝' | '⎴' | '⎵') || is_nary_char(c)
}

/// Combining marks used as accents.
pub fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x300..=0x36F | 0x20D0..=0x20FF)
}

/// Characters that end an operand.
fn is_boundary(c: char) -> bool {
    matches!(c, ' ' | '&' | '@' | '#' | '▒' | '│' | '/' | '⁄' | '∕' | '¦' | '\u{2061}')
        || is_closer(c)
        || matches!(math_class(c), MClass::Bin | MClass::Rel | MClass::Punct)
}

struct P {
    c: Vec<char>,
    i: usize,
    depth: usize,
    /// Inside a brace's operand: limits are left for the brace.
    no_limits: usize,
    /// Work done (bounds hostile input).
    steps: usize,
}

const MAX_STEPS: usize = 400_000;

impl P {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn peek_at(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }
    fn tired(&mut self) -> bool {
        self.steps += 1;
        self.steps > MAX_STEPS || self.depth > MAX_DEPTH
    }
    /// The rest of the input as text (when too deep or too long).
    fn rest(&mut self) -> Arg {
        let t: String = self.c.get(self.i..).unwrap_or(&[]).iter().collect();
        self.i = self.c.len();
        if t.is_empty() { Vec::new() } else { vec![MNode::Run(MRun::new(t))] }
    }

    /// A sequence up to one of `stops` (not consumed) or the end.
    fn expr(&mut self, stops: &[char]) -> Arg {
        self.depth += 1;
        let mut out: Arg = Vec::new();
        if self.tired() {
            self.depth -= 1;
            return self.rest();
        }
        while let Some(c) = self.peek() {
            if stops.contains(&c) || (is_closer(c) && !stops.is_empty()) {
                break;
            }
            if self.tired() {
                out.extend(self.rest());
                break;
            }
            match c {
                '/' | '⁄' | '∕' | '¦' => {
                    self.i += 1;
                    let start = operand_start(&out);
                    let num = strip_parens(out.drain(start..).collect());
                    let den = self.operand(stops);
                    let kind = match c {
                        '⁄' => FracKind::Skewed,
                        '∕' => FracKind::Linear,
                        '¦' => FracKind::NoBar,
                        _ => FracKind::Bar,
                    };
                    out.push(MNode::Frac { kind, num, den });
                }
                '^' | '_' => {
                    let pre = match out.last() {
                        None => true,
                        Some(MNode::Run(r)) => r.text == " " || r.text.chars().all(|c| is_boundary(c) && c != '#'),
                        _ => false,
                    };
                    if pre {
                        out.push(self.prescript(stops));
                    } else {
                        self.i += 1;
                        let s = self.script_operand(stops);
                        let base = out.pop();
                        out.push(attach_script(base, c == '^', s));
                    }
                }
                '┬' | '┴' => {
                    self.i += 1;
                    let lim = self.script_operand(stops);
                    let e = out.pop().map(|n| vec![n]).unwrap_or_default();
                    out.push(MNode::Lim { upper: c == '┴', e: strip_parens(e), lim });
                }
                c if is_combining(c) => {
                    self.i += 1;
                    let e = out.pop().map(|n| strip_parens(vec![n])).unwrap_or_default();
                    out.push(MNode::Acc { chr: c, e });
                }
                '▒' | '\u{2061}' => self.i += 1,
                ' ' => {
                    self.i += 1;
                    out.push(text(' '));
                }
                _ => {
                    let atom = self.atom(stops);
                    out.extend(atom);
                }
            }
        }
        self.depth -= 1;
        out
    }

    /// One atom (a character, a bracket group, a structure or a function application).
    fn atom(&mut self, stops: &[char]) -> Arg {
        let Some(c) = self.peek() else { return Vec::new() };
        if let Some(close) = closer(c) {
            self.i += 1;
            return vec![self.group(Some(c), &[close])];
        }
        if c == '├' {
            self.i += 1;
            if let Some(d) = self.explicit_group() {
                return vec![d];
            }
            return vec![self.group(None, &[')', ']', '}', '⟩', '⌉', '⌋', '|', '‖'])];
        }
        if (c == '|' || c == '‖') && self.c.get(self.i + 1..).is_some_and(|r| r.contains(&c)) {
            self.i += 1;
            return vec![self.group(Some(c), &[c])];
        }
        if is_prefix(c) {
            self.i += 1;
            return vec![self.prefix(c, stops)];
        }
        if c.is_ascii_alphabetic() {
            return self.word(stops);
        }
        self.i += 1;
        vec![text(c)]
    }

    /// A bracket group after its opening character; `closers` end it.
    fn group(&mut self, open: Option<char>, closers: &[char]) -> MNode {
        let mut stops: Vec<char> = closers.to_vec();
        stops.extend(['┤', '│']);
        let mut elems = Vec::new();
        let mut end = None;
        loop {
            let e = self.expr(&stops);
            elems.push(e);
            match self.peek() {
                Some('│') => self.i += 1,
                Some('┤') => {
                    self.i += 1;
                    break;
                }
                Some(c) if closers.contains(&c) => {
                    self.i += 1;
                    end = Some(c);
                    break;
                }
                // Another closer: the group ends unclosed there.
                _ => break,
            }
            if self.tired() {
                break;
            }
        }
        let beg = open.filter(|c| *c != '〖');
        let end = end.filter(|c| *c != '〗');
        let sep = if elems.len() > 1 { Some('|') } else { None };
        MNode::Delim { beg, end, sep, grow: true, shp_match: false, elems }
    }

    /// `├X … Y┤`: brackets given explicitly (any bracket characters, either side optional), the
    /// cursor just after `├`. `None` (cursor unchanged) when there's no matching `┤`.
    fn explicit_group(&mut self) -> Option<MNode> {
        let mut level = 1usize;
        let mut k = self.i;
        while let Some(c) = self.c.get(k) {
            match c {
                '├' => level += 1,
                '┤' => {
                    level -= 1;
                    if level == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        if level != 0 {
            return None;
        }
        let bracket = |c: &char| matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '⟨' | '⟩' | '⌈' | '⌉' | '⌊' | '⌋' | '|' | '‖' | '⟦' | '⟧');
        let mut inner: Vec<char> = self.c.get(self.i..k).unwrap_or(&[]).to_vec();
        self.i = k + 1;
        let beg = if inner.first().is_some_and(bracket) { Some(inner.remove(0)) } else { None };
        // A closing bracket at the end that pairs with an opening one inside is content.
        let paired = |inner: &[char]| -> bool {
            let Some((last, body)) = inner.split_last() else { return false };
            let opener = match last {
                ')' => '(',
                ']' => '[',
                '}' => '{',
                '⟩' => '⟨',
                '⌉' => '⌈',
                '⌋' => '⌊',
                '⟧' => '⟦',
                _ => return false,
            };
            body.iter().filter(|c| **c == opener).count() > body.iter().filter(|c| *c == last).count()
        };
        let end = if !inner.is_empty() && inner.last().is_some_and(bracket) && !paired(&inner) { inner.pop() } else { None };
        let mut sub = P { c: inner, i: 0, depth: self.depth + 1, no_limits: 0, steps: self.steps };
        let mut elems = vec![Vec::new()];
        while sub.i < sub.c.len() {
            if let Some(e) = elems.last_mut() {
                e.extend(sub.expr(&['│']));
            }
            match sub.peek() {
                Some('│') => {
                    sub.i += 1;
                    elems.push(Vec::new());
                }
                Some(c) => {
                    sub.i += 1;
                    if let Some(e) = elems.last_mut() {
                        e.push(text(c));
                    }
                }
                None => {}
            }
            if sub.tired() {
                break;
            }
        }
        self.steps = sub.steps;
        let sep = if elems.len() > 1 { Some('|') } else { None };
        Some(MNode::Delim { beg, end, sep, grow: true, shp_match: false, elems })
    }

    /// A prefix structure after its character.
    fn prefix(&mut self, c: char, stops: &[char]) -> MNode {
        match c {
            '√' | '∛' | '∜' => {
                let mut deg = match c {
                    '∛' => vec![text('3')],
                    '∜' => vec![text('4')],
                    _ => Vec::new(),
                };
                let mut e = self.operand(stops);
                if let Some(k) = e.iter().position(|n| matches!(n, MNode::Run(r) if r.text == "&")) {
                    let rest = e.split_off(k + 1);
                    e.pop();
                    deg = e;
                    e = rest;
                }
                MNode::Rad { deg_hide: deg.is_empty(), deg, e }
            }
            '█' | '■' => {
                if self.peek() == Some('(') {
                    self.i += 1;
                    let inner = self.expr(&[')']);
                    if self.peek() == Some(')') {
                        self.i += 1;
                    }
                    grid(c == '■', inner)
                } else {
                    text(c)
                }
            }
            '▭' => MNode::BorderBox { hide: [false; 4], strike: [false; 4], e: self.operand(stops) },
            '□' => MNode::Boxed { e: self.operand(stops) },
            '¯' => MNode::Bar { top: true, e: self.operand(stops) },
            '▁' => MNode::Bar { top: false, e: self.operand(stops) },
            '⟡' => MNode::Phant { show: false, zero_wid: false, zero_asc: false, zero_desc: false, e: self.operand(stops) },
            // A label (`┴`/`┬`) after the operand belongs to the whole brace.
            '⏞' | '⏜' | '⎴' | '⏟' | '⏝' | '⎵' => {
                self.no_limits += 1;
                let e = self.operand(stops);
                self.no_limits -= 1;
                MNode::GroupChr { chr: c, top: matches!(c, '⏞' | '⏜' | '⎴'), e }
            }
            _ => {
                // N-ary operator: limits, then the operand.
                let (mut sub, mut sup) = (Vec::new(), Vec::new());
                let (mut has_sub, mut has_sup) = (false, false);
                loop {
                    match self.peek() {
                        Some('_') => {
                            self.i += 1;
                            sub = self.script_operand(stops);
                            has_sub = true;
                        }
                        Some('^') => {
                            self.i += 1;
                            sup = self.script_operand(stops);
                            has_sup = true;
                        }
                        _ => break,
                    }
                }
                if self.peek() == Some('▒') {
                    self.i += 1;
                }
                let e = if self.peek().is_some_and(|c| c == ' ') { Vec::new() } else { self.operand(stops) };
                MNode::Nary { chr: c, lim_loc: None, grow: false, sub_hide: !has_sub, sup_hide: !has_sup, sub, sup, e }
            }
        }
    }

    /// Letters: a function application when they spell a function name, else one letter.
    fn word(&mut self, stops: &[char]) -> Arg {
        let start = self.i;
        let mut end = start;
        while self.c.get(end).is_some_and(|c| c.is_ascii_alphabetic()) {
            end += 1;
        }
        let word: String = self.c.get(start..end).unwrap_or(&[]).iter().collect();
        if !FUNCTION_NAMES.contains(&word.as_str()) {
            self.i = start + 1;
            return vec![text(self.c.get(start).copied().unwrap_or(' '))];
        }
        self.i = end;
        let plain = vec![MNode::Run(MRun::plain(word.clone()))];
        // Limits and scripts on the name.
        let name = match self.peek() {
            Some('┬') | Some('┴') => {
                let upper = self.peek() == Some('┴');
                self.i += 1;
                let lim = self.script_operand(stops);
                vec![MNode::Lim { upper, e: plain, lim }]
            }
            Some('_') if LIMIT_NAMES.contains(&word.as_str()) => {
                self.i += 1;
                let lim = self.script_operand(stops);
                vec![MNode::Lim { upper: false, e: plain, lim }]
            }
            Some(c @ ('^' | '_')) => {
                self.i += 1;
                let s = self.script_operand(stops);
                let mut n = attach_script(Some(MNode::Run(MRun::plain(word.clone()))), c == '^', s);
                if let Some(c2 @ ('^' | '_')) = self.peek() {
                    self.i += 1;
                    let s2 = self.script_operand(stops);
                    n = attach_script(Some(n), c2 == '^', s2);
                }
                vec![n]
            }
            _ => plain,
        };
        // The operand, after an optional space or function-application mark.
        if matches!(self.peek(), Some(' ') | Some('\u{2061}') | Some('▒')) {
            self.i += 1;
        }
        let next = self.peek();
        let applies = next.is_some_and(|c| !stops.contains(&c) && !is_closer(c) && (!is_boundary(c) || c == '(' || matches!(c, '−' | '-' | '+')));
        if !applies {
            return name;
        }
        let e = self.operand(stops);
        if e.is_empty() {
            return name;
        }
        vec![MNode::Func { name, e }]
    }

    /// Pre-scripts (`_a^b x`): the scripts, then the base.
    fn prescript(&mut self, stops: &[char]) -> MNode {
        let (mut sub, mut sup) = (Vec::new(), Vec::new());
        while let Some(c @ ('^' | '_')) = self.peek() {
            self.i += 1;
            let s = self.script_operand(stops);
            if c == '^' {
                sup = s;
            } else {
                sub = s;
            }
        }
        if self.peek() == Some('▒') {
            self.i += 1;
        }
        let base = self.operand(stops);
        MNode::Script { kind: ScriptKind::Pre, base, sub, sup }
    }

    /// An operand: a parenthesised group (parentheses dropped) or a run of atoms with their
    /// postfix scripts, accents and limits, up to an operator or space.
    fn operand(&mut self, stops: &[char]) -> Arg {
        if self.tired() {
            return self.rest();
        }
        self.depth += 1;
        let mut out: Arg = Vec::new();
        if self.peek() == Some('(') {
            self.i += 1;
            let g = self.group(Some('('), &[')']);
            let inner = match g {
                MNode::Delim { elems, end: Some(')'), .. } if elems.len() == 1 => elems.into_iter().next().unwrap_or_default(),
                other => vec![other],
            };
            out = inner;
            self.postfix(&mut out, stops, true);
        } else if self.peek() == Some('〖') {
            self.i += 1;
            let g = self.group(Some('〖'), &['〗']);
            out = match g {
                MNode::Delim { elems, .. } if elems.len() == 1 => elems.into_iter().next().unwrap_or_default(),
                other => vec![other],
            };
            self.postfix(&mut out, stops, true);
        } else {
            // A signed operand (`e^-x`, `−b`).
            if let Some(c @ ('−' | '-' | '+' | '±' | '∓')) = self.peek()
                && self.peek_at(1).is_some_and(|n| !is_boundary(n))
            {
                self.i += 1;
                out.push(text(c));
            }
            while let Some(c) = self.peek() {
                if stops.contains(&c) || is_boundary(c) || self.tired() {
                    break;
                }
                let a = self.atom(stops);
                if a.is_empty() {
                    break;
                }
                out.extend(a);
                self.postfix(&mut out, stops, false);
            }
            if out.is_empty()
                && let Some(c) = self.peek()
                && c != ' '
                && !stops.contains(&c)
                && !is_closer(c)
                && !matches!(c, '/' | '⁄' | '∕' | '¦' | '&' | '@' | '#' | '│' | '▒')
            {
                // A lone operator as operand.
                self.i += 1;
                out.push(text(c));
            }
        }
        self.depth -= 1;
        out
    }

    /// Postfix operators on the last atom of `out` (`whole`: on all of `out` as one group).
    fn postfix(&mut self, out: &mut Arg, stops: &[char], whole: bool) {
        while let Some(c) = self.peek() {
            if self.tired() {
                break;
            }
            let base = |out: &mut Arg| -> Option<MNode> {
                if whole && out.len() != 1 {
                    if out.is_empty() {
                        return None;
                    }
                    return Some(MNode::Delim { beg: None, end: None, sep: None, grow: true, shp_match: false, elems: vec![std::mem::take(out)] });
                }
                out.pop()
            };
            match c {
                '^' | '_' => {
                    self.i += 1;
                    let s = self.script_operand(stops);
                    let b = base(out);
                    out.push(attach_script(b, c == '^', s));
                }
                '┬' | '┴' if self.no_limits == 0 => {
                    self.i += 1;
                    let lim = self.script_operand(stops);
                    let b = base(out);
                    out.push(MNode::Lim { upper: c == '┴', e: b.map(|n| vec![n]).unwrap_or_default(), lim });
                }
                c if is_combining(c) => {
                    self.i += 1;
                    let b = base(out);
                    out.push(MNode::Acc { chr: c, e: b.map(|n| strip_parens(vec![n])).unwrap_or_default() });
                }
                _ => break,
            }
        }
    }

    /// A script or limit: a parenthesised group, a structure, a number, a signed operand or one
    /// character.
    fn script_operand(&mut self, stops: &[char]) -> Arg {
        if self.tired() {
            return self.rest();
        }
        self.depth += 1;
        let out = match self.peek() {
            Some('(') => {
                self.i += 1;
                match self.group(Some('('), &[')']) {
                    MNode::Delim { elems, end: Some(')'), .. } if elems.len() == 1 => elems.into_iter().next().unwrap_or_default(),
                    other => vec![other],
                }
            }
            Some('〖') => {
                self.i += 1;
                match self.group(Some('〖'), &['〗']) {
                    MNode::Delim { elems, .. } if elems.len() == 1 => elems.into_iter().next().unwrap_or_default(),
                    other => vec![other],
                }
            }
            Some(c) if is_prefix(c) || closer(c).is_some() => self.atom(stops),
            Some(c) if c.is_ascii_digit() => {
                let start = self.i;
                while self.peek().is_some_and(|k| k.is_ascii_digit() || k == '.') {
                    self.i += 1;
                }
                let t: String = self.c.get(start..self.i).unwrap_or(&[]).iter().collect();
                vec![MNode::Run(MRun::new(t))]
            }
            Some(c @ ('−' | '-' | '+' | '±' | '∓')) if self.peek_at(1).is_some_and(|n| !is_boundary(n)) => {
                self.i += 1;
                let mut v = vec![text(c)];
                v.extend(self.script_operand(stops));
                v
            }
            Some(c) if c.is_ascii_alphabetic() => {
                // A function name as a script (`e^sin x` is rare): one letter.
                self.i += 1;
                vec![text(c)]
            }
            Some(c) if !stops.contains(&c) && c != ' ' && !is_closer(c) => {
                self.i += 1;
                vec![text(c)]
            }
            _ => Vec::new(),
        };
        self.depth -= 1;
        out
    }
}

/// Where the trailing operand of `out` starts (after the last operator or space).
fn operand_start(out: &[MNode]) -> usize {
    let mut k = out.len();
    while k > 0 {
        match out.get(k - 1) {
            Some(MNode::Run(r)) if r.text.chars().all(is_boundary) => break,
            _ => k -= 1,
        }
    }
    k
}

/// A single parenthesised group used as an operand loses its parentheses.
pub fn strip_parens(mut a: Arg) -> Arg {
    if a.len() == 1
        && let Some(MNode::Delim { beg: Some('('), end: Some(')'), elems, .. } | MNode::Delim { beg: None, end: None, elems, .. }) = a.first_mut()
        && elems.len() == 1
    {
        return elems.pop().unwrap_or_default();
    }
    a
}

fn attach_script(base: Option<MNode>, sup: bool, s: Arg) -> MNode {
    match base {
        Some(MNode::Script { kind: ScriptKind::Sub, base, sub, .. }) if sup => MNode::Script { kind: ScriptKind::SubSup, base, sub, sup: s },
        Some(MNode::Script { kind: ScriptKind::Sup, base, sup: sp, .. }) if !sup => MNode::Script { kind: ScriptKind::SubSup, base, sub: s, sup: sp },
        b => {
            let base = b.map(|b| vec![b]).unwrap_or_default();
            if sup {
                MNode::Script { kind: ScriptKind::Sup, base, sub: Vec::new(), sup: s }
            } else {
                MNode::Script { kind: ScriptKind::Sub, base, sub: s, sup: Vec::new() }
            }
        }
    }
}

/// `█(a@b)` equation array / `■(a&b@c&d)` matrix from the parsed inside.
fn grid(matrix: bool, inner: Arg) -> MNode {
    let mut rows: Vec<Vec<Arg>> = vec![vec![Vec::new()]];
    for n in inner {
        match &n {
            MNode::Run(r) if r.text == "@" => rows.push(vec![Vec::new()]),
            MNode::Run(r) if r.text == "&" && matrix => {
                if let Some(row) = rows.last_mut() {
                    row.push(Vec::new());
                }
            }
            _ => {
                if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                    cell.push(n);
                }
            }
        }
    }
    if matrix {
        MNode::Matrix { rows, col_jc: Vec::new() }
    } else {
        MNode::EqArr { rows: rows.into_iter().map(|r| r.into_iter().flatten().collect()).collect() }
    }
}

/// Drop placeholders and build-up spaces, merge runs; recursively.
fn clean(a: Arg) -> Arg {
    let mut out = Vec::with_capacity(a.len());
    for n in a {
        match n {
            MNode::Run(r) if r.text == " " || r.text.chars().all(|c| c == PLACEHOLDER) => {}
            other => out.push(map_args(other, &clean)),
        }
    }
    merge_runs(out)
}

/// Apply `f` to every argument of a node.
pub fn map_args(n: MNode, f: &dyn Fn(Arg) -> Arg) -> MNode {
    match n {
        MNode::Run(r) => MNode::Run(r),
        MNode::Frac { kind, num, den } => MNode::Frac { kind, num: f(num), den: f(den) },
        MNode::Script { kind, base, sub, sup } => MNode::Script { kind, base: f(base), sub: f(sub), sup: f(sup) },
        MNode::Rad { deg, deg_hide, e } => MNode::Rad { deg: f(deg), deg_hide, e: f(e) },
        MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub, sup, e } => {
            MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub: f(sub), sup: f(sup), e: f(e) }
        }
        MNode::Delim { beg, end, sep, grow, shp_match, elems } => {
            MNode::Delim { beg, end, sep, grow, shp_match, elems: elems.into_iter().map(f).collect() }
        }
        MNode::Func { name, e } => MNode::Func { name: f(name), e: f(e) },
        MNode::Lim { upper, e, lim } => MNode::Lim { upper, e: f(e), lim: f(lim) },
        MNode::Acc { chr, e } => MNode::Acc { chr, e: f(e) },
        MNode::Bar { top, e } => MNode::Bar { top, e: f(e) },
        MNode::BorderBox { hide, strike, e } => MNode::BorderBox { hide, strike, e: f(e) },
        MNode::Boxed { e } => MNode::Boxed { e: f(e) },
        MNode::GroupChr { chr, top, e } => MNode::GroupChr { chr, top, e: f(e) },
        MNode::EqArr { rows } => MNode::EqArr { rows: rows.into_iter().map(f).collect() },
        MNode::Matrix { rows, col_jc } => MNode::Matrix { rows: rows.into_iter().map(|r| r.into_iter().map(f).collect()).collect(), col_jc },
        MNode::Phant { show, zero_wid, zero_asc, zero_desc, e } => MNode::Phant { show, zero_wid, zero_asc, zero_desc, e: f(e) },
    }
}
