//! LaTeX ↔ math tree (Word's LaTeX equation input and conversion).
//!
//! Input covers the commands people type in equations: `\frac`, `\sqrt[n]{}`, scripts, big
//! operators with limits, `\left…\middle…\right`, matrix and `cases`/`aligned` environments,
//! accents, `\overbrace`/`\underbrace`, `\text`/`\mathrm`/`\mathbf`/`\mathbb`…, function names,
//! spacing and every named symbol in [`crate::math_symbols`]. Unknown commands stay as text.

use crate::math::{Arg, FracKind, MAX_DEPTH, MNode, MRun, MScr, MSty, ScriptKind, is_nary_char, math_class, merge_runs};
use crate::math_linear::{FUNCTION_NAMES, map_args};
use crate::math_symbols::{name_of, symbol};

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Cmd(String),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
    Row,
    Ch(char),
}

fn tokenize(s: &str) -> Vec<Tok> {
    let cs: Vec<char> = s.chars().take(20_000).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = cs.get(i) {
        i += 1;
        match c {
            '\\' => {
                let start = i;
                while cs.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
                    i += 1;
                }
                if i == start {
                    match cs.get(i) {
                        Some('\\') => {
                            i += 1;
                            out.push(Tok::Row);
                        }
                        Some(&c) => {
                            i += 1;
                            out.push(Tok::Cmd(c.to_string()));
                        }
                        None => {}
                    }
                } else {
                    out.push(Tok::Cmd(cs.get(start..i).unwrap_or(&[]).iter().collect()));
                }
            }
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '^' => out.push(Tok::Sup),
            '_' => out.push(Tok::Sub),
            '&' => out.push(Tok::Amp),
            ' ' | '\t' | '\n' | '\r' => {}
            '~' => out.push(Tok::Ch('\u{A0}')),
            c => out.push(Tok::Ch(c)),
        }
    }
    out
}

/// Parse LaTeX math into nodes. Never fails.
pub fn parse_latex(s: &str) -> Arg {
    let mut p = Lp { t: tokenize(s), i: 0, depth: 0 };
    let mut out = Vec::new();
    while p.i < p.t.len() {
        out.extend(p.seq(false));
        // A stray `}` or `&`/`\\` at the top level.
        match p.t.get(p.i) {
            Some(Tok::Amp) => out.push(run("&")),
            Some(Tok::Row) => {}
            Some(Tok::Close) => {}
            _ => {}
        }
        p.i += 1;
    }
    let out = merge_runs(out);
    if out.iter().any(|n| matches!(n, MNode::Run(r) if r.text.contains('#'))) {
        return vec![MNode::EqArr { rows: vec![out] }];
    }
    out
}

fn run(t: &str) -> MNode {
    MNode::Run(MRun::new(t))
}

struct Lp {
    t: Vec<Tok>,
    i: usize,
    depth: usize,
}

impl Lp {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    /// A sequence up to `}` (in a group), `&`, `\\`, `\right`, `\middle` or `\end`.
    fn seq(&mut self, in_group: bool) -> Arg {
        self.depth += 1;
        let mut out: Arg = Vec::new();
        if self.depth > MAX_DEPTH {
            self.i = self.t.len();
            self.depth -= 1;
            return out;
        }
        while let Some(tok) = self.peek().cloned() {
            match tok {
                Tok::Close if in_group => break,
                Tok::Close | Tok::Amp | Tok::Row => break,
                Tok::Cmd(ref c) if matches!(c.as_str(), "right" | "middle" | "end") => break,
                Tok::Sup | Tok::Sub => {
                    self.i += 1;
                    let s = self.arg();
                    let base = out.pop();
                    out.push(script(base, tok == Tok::Sup, s));
                }
                Tok::Cmd(ref c) if c == "limits" || c == "nolimits" || c == "displaystyle" || c == "textstyle" => {
                    self.i += 1;
                    if c == "nolimits"
                        && let Some(MNode::Nary { lim_loc, .. }) = out.last_mut()
                    {
                        *lim_loc = Some(crate::math::LimLoc::SubSup);
                    }
                    if c == "limits"
                        && let Some(MNode::Nary { lim_loc, .. }) = out.last_mut()
                    {
                        *lim_loc = Some(crate::math::LimLoc::UndOvr);
                    }
                }
                _ => {
                    let a = self.atom();
                    out.extend(a);
                }
            }
        }
        self.depth -= 1;
        // Big operators take the following term as their operand.
        nary_operands(out)
    }

    /// A command argument: a `{group}` or one token.
    fn arg(&mut self) -> Arg {
        match self.peek() {
            Some(Tok::Open) => {
                self.i += 1;
                let g = self.seq(true);
                if self.peek() == Some(&Tok::Close) {
                    self.i += 1;
                }
                g
            }
            Some(_) => self.atom(),
            None => Vec::new(),
        }
    }

    /// `[optional]` argument.
    fn opt(&mut self) -> Option<Arg> {
        if self.peek() != Some(&Tok::Ch('[')) {
            return None;
        }
        self.i += 1;
        let mut toks = Vec::new();
        let mut level = 0i32;
        while let Some(t) = self.peek().cloned() {
            self.i += 1;
            match t {
                Tok::Ch(']') if level == 0 => break,
                Tok::Open => level += 1,
                Tok::Close => level -= 1,
                _ => {}
            }
            toks.push(t);
        }
        let mut p = Lp { t: toks, i: 0, depth: self.depth };
        Some(p.seq(false))
    }

    /// The text of a `{…}` argument, verbatim.
    fn raw_arg(&mut self) -> String {
        let mut s = String::new();
        if self.peek() != Some(&Tok::Open) {
            if let Some(Tok::Ch(c)) = self.peek().cloned() {
                self.i += 1;
                s.push(c);
            }
            return s;
        }
        self.i += 1;
        let mut level = 0i32;
        while let Some(t) = self.peek().cloned() {
            self.i += 1;
            match t {
                Tok::Close if level == 0 => break,
                Tok::Open => {
                    level += 1;
                    s.push('{');
                }
                Tok::Close => {
                    level -= 1;
                    s.push('}');
                }
                Tok::Ch(c) => s.push(c),
                Tok::Cmd(c) => {
                    if c.chars().all(|c| c.is_ascii_alphabetic()) {
                        match symbol(&c) {
                            Some(sym) => s.push(sym),
                            None => s.push_str(&c),
                        }
                    } else {
                        s.push_str(&c);
                    }
                    if c == " " {
                        s.push(' ');
                    }
                }
                Tok::Sup => s.push('^'),
                Tok::Sub => s.push('_'),
                Tok::Amp => s.push('&'),
                Tok::Row => s.push('\n'),
            }
        }
        s
    }

    /// A delimiter after `\left`, `\right`, `\middle`, `\big…`.
    fn delim(&mut self) -> Option<char> {
        let t = self.peek().cloned()?;
        self.i += 1;
        match t {
            Tok::Ch('.') => None,
            Tok::Ch(c) => Some(c),
            Tok::Cmd(c) => match c.as_str() {
                "{" => Some('{'),
                "}" => Some('}'),
                "|" => Some('‖'),
                other => symbol(other),
            },
            _ => None,
        }
    }

    fn atom(&mut self) -> Arg {
        if self.depth > MAX_DEPTH {
            self.i = self.t.len();
            return Vec::new();
        }
        self.depth += 1;
        let out = self.atom_inner();
        self.depth -= 1;
        out
    }

    fn atom_inner(&mut self) -> Arg {
        let Some(t) = self.peek().cloned() else { return Vec::new() };
        self.i += 1;
        match t {
            Tok::Open => {
                let g = self.seq(true);
                if self.peek() == Some(&Tok::Close) {
                    self.i += 1;
                }
                g
            }
            Tok::Ch(c) => vec![run(&c.to_string())],
            Tok::Close | Tok::Amp | Tok::Row | Tok::Sup | Tok::Sub => Vec::new(),
            Tok::Cmd(c) => self.command(&c),
        }
    }

    fn command(&mut self, c: &str) -> Arg {
        let styled = |a: Arg, sty: Option<MSty>, scr: MScr, nor: bool| -> Arg { restyle(a, sty, scr, nor) };
        let one = |n: MNode| vec![n];
        match c {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.arg();
                let den = self.arg();
                one(MNode::Frac { kind: FracKind::Bar, num, den })
            }
            "binom" | "dbinom" | "tbinom" => {
                let n = self.arg();
                let k = self.arg();
                one(MNode::Delim {
                    beg: Some('('),
                    end: Some(')'),
                    sep: None,
                    grow: true,
                    shp_match: false,
                    elems: vec![vec![MNode::Frac { kind: FracKind::NoBar, num: n, den: k }]],
                })
            }
            "sqrt" => {
                let deg = self.opt().unwrap_or_default();
                let e = self.arg();
                one(MNode::Rad { deg_hide: deg.is_empty(), deg, e })
            }
            "left" => {
                let beg = self.delim();
                let mut elems = vec![self.seq(false)];
                let mut sep = None;
                let mut end = None;
                while let Some(Tok::Cmd(k)) = self.peek().cloned() {
                    self.i += 1;
                    if k == "middle" {
                        sep = self.delim().or(Some('|'));
                        elems.push(self.seq(false));
                    } else {
                        if k == "right" {
                            end = self.delim();
                        }
                        break;
                    }
                }
                one(MNode::Delim { beg, end, sep, grow: true, shp_match: false, elems })
            }
            "bigl" | "Bigl" | "biggl" | "Biggl" | "bigr" | "Bigr" | "biggr" | "Biggr" | "big" | "Big" | "bigg" | "Bigg" => {
                self.delim().map(|d| vec![run(&d.to_string())]).unwrap_or_default()
            }
            "begin" => {
                let env = self.raw_arg();
                self.environment(env.trim())
            }
            "text" | "textrm" | "mbox" | "textnormal" => {
                let t = self.raw_arg();
                one(MNode::Run(MRun { text: t, nor: true, sty: Some(MSty::Plain), ..Default::default() }))
            }
            "textbf" => {
                let t = self.raw_arg();
                one(MNode::Run(MRun { text: t, nor: true, sty: Some(MSty::Bold), ..Default::default() }))
            }
            "textit" => {
                let t = self.raw_arg();
                one(MNode::Run(MRun { text: t, nor: true, sty: Some(MSty::Italic), ..Default::default() }))
            }
            "operatorname" => {
                let t = self.raw_arg();
                let name = vec![MNode::Run(MRun::plain(t))];
                self.apply(name)
            }
            "mathrm" | "rm" => {
                let a = self.arg();
                styled(a, Some(MSty::Plain), MScr::Roman, false)
            }
            "mathbf" | "bf" => {
                let a = self.arg();
                styled(a, Some(MSty::Bold), MScr::Roman, false)
            }
            "mathit" => {
                let a = self.arg();
                styled(a, Some(MSty::Italic), MScr::Roman, false)
            }
            "boldsymbol" | "bm" => {
                let a = self.arg();
                styled(a, Some(MSty::BoldItalic), MScr::Roman, false)
            }
            "mathbb" => {
                let a = self.arg();
                styled(a, None, MScr::DoubleStruck, false)
            }
            "mathcal" | "mathscr" => {
                let a = self.arg();
                styled(a, None, MScr::Script, false)
            }
            "mathfrak" => {
                let a = self.arg();
                styled(a, None, MScr::Fraktur, false)
            }
            "mathsf" => {
                let a = self.arg();
                styled(a, None, MScr::SansSerif, false)
            }
            "mathtt" => {
                let a = self.arg();
                styled(a, None, MScr::Monospace, false)
            }
            "overline" => one(MNode::Bar { top: true, e: self.arg() }),
            "underline" => one(MNode::Bar { top: false, e: self.arg() }),
            "boxed" | "fbox" => one(MNode::BorderBox { hide: [false; 4], strike: [false; 4], e: self.arg() }),
            "phantom" => one(MNode::Phant { show: false, zero_wid: false, zero_asc: false, zero_desc: false, e: self.arg() }),
            "overbrace" | "underbrace" | "overparen" | "underparen" => {
                let top = c.starts_with("over");
                let chr = match c {
                    "overbrace" => '⏞',
                    "underbrace" => '⏟',
                    "overparen" => '⏜',
                    _ => '⏝',
                };
                let e = self.arg();
                let g = MNode::GroupChr { chr, top, e };
                let want = if top { Tok::Sup } else { Tok::Sub };
                if self.peek() == Some(&want) {
                    self.i += 1;
                    let lim = self.arg();
                    one(MNode::Lim { upper: top, e: vec![g], lim })
                } else {
                    one(g)
                }
            }
            "stackrel" | "overset" => {
                let over = self.arg();
                let base = self.arg();
                one(MNode::Lim { upper: true, e: base, lim: over })
            }
            "underset" => {
                let under = self.arg();
                let base = self.arg();
                one(MNode::Lim { upper: false, e: base, lim: under })
            }
            "hat" | "widehat" | "tilde" | "widetilde" | "dot" | "ddot" | "dddot" | "check" | "breve" | "acute" | "grave" | "vec" | "bar"
            | "overrightarrow" | "overleftarrow" | "overleftrightarrow" | "mathring" => {
                let chr = match c {
                    "hat" | "widehat" => '\u{302}',
                    "tilde" | "widetilde" => '\u{303}',
                    "dot" => '\u{307}',
                    "ddot" => '\u{308}',
                    "dddot" => '\u{20DB}',
                    "check" => '\u{30C}',
                    "breve" => '\u{306}',
                    "acute" => '\u{301}',
                    "grave" => '\u{300}',
                    "bar" => '\u{305}',
                    "overleftarrow" => '\u{20D6}',
                    "overleftrightarrow" => '\u{20E1}',
                    "mathring" => '\u{30A}',
                    _ => '\u{20D7}',
                };
                one(MNode::Acc { chr, e: self.arg() })
            }
            "," | "thinspace" => vec![run("\u{2009}")],
            ":" | ">" | "medspace" => vec![run("\u{205F}")],
            ";" | "thickspace" => vec![run("\u{2005}")],
            "!" | "negthinspace" => Vec::new(),
            " " => vec![run(" ")],
            "qquad" => vec![run("\u{2003}\u{2003}")],
            "{" | "}" | "%" | "$" | "#" | "&" | "_" => {
                // Escaped characters (a literal `#` must not number the equation).
                let mut r = MRun::new(c);
                r.lit = c == "#";
                vec![MNode::Run(r)]
            }
            "|" => vec![run("‖")],
            name if FUNCTION_NAMES.contains(&name) => {
                let plain = vec![MNode::Run(MRun::plain(name))];
                let limits = matches!(name, "lim" | "liminf" | "limsup" | "max" | "min" | "sup" | "inf" | "det" | "gcd" | "Pr");
                let named = if limits && self.peek() == Some(&Tok::Sub) {
                    self.i += 1;
                    let lim = self.arg();
                    vec![MNode::Lim { upper: false, e: plain, lim }]
                } else if matches!(self.peek(), Some(Tok::Sup) | Some(Tok::Sub)) {
                    let sup = self.peek() == Some(&Tok::Sup);
                    self.i += 1;
                    let s = self.arg();
                    vec![script(Some(MNode::Run(MRun::plain(name))), sup, s)]
                } else {
                    plain
                };
                self.apply(named)
            }
            name => match symbol(name) {
                Some(ch) if is_nary_char(ch) => vec![MNode::Nary {
                    chr: ch,
                    lim_loc: None,
                    grow: false,
                    sub_hide: true,
                    sup_hide: true,
                    sub: Vec::new(),
                    sup: Vec::new(),
                    e: Vec::new(),
                }],
                Some(ch) => vec![run(&ch.to_string())],
                None => vec![run(&format!("\\{name}"))],
            },
        }
    }

    /// A function name applied to the next atom (with its scripts).
    fn apply(&mut self, name: Arg) -> Arg {
        let mut e = self.atom();
        while let Some(t @ (Tok::Sup | Tok::Sub)) = self.peek().cloned() {
            self.i += 1;
            let s = self.arg();
            let base = if e.len() == 1 { e.pop() } else { Some(group(std::mem::take(&mut e))) };
            e.push(script(base, t == Tok::Sup, s));
        }
        if e.is_empty() {
            return name;
        }
        vec![MNode::Func { name, e }]
    }

    fn environment(&mut self, env: &str) -> Arg {
        // Rows of `&`-separated cells up to `\end{…}`.
        let mut rows: Vec<Vec<Arg>> = vec![vec![]];
        loop {
            let cell = self.seq(false);
            if let Some(r) = rows.last_mut() {
                r.push(cell);
            }
            match self.peek().cloned() {
                Some(Tok::Amp) => self.i += 1,
                Some(Tok::Row) => {
                    self.i += 1;
                    rows.push(vec![]);
                }
                Some(Tok::Cmd(k)) if k == "end" => {
                    self.i += 1;
                    let _ = self.raw_arg();
                    break;
                }
                Some(Tok::Close) => self.i += 1,
                Some(_) => self.i += 1,
                None => break,
            }
        }
        // A trailing `\\` leaves an empty last row.
        if rows.len() > 1 && rows.last().is_some_and(|r| r.iter().all(|c| c.is_empty())) {
            rows.pop();
        }
        let base = env.trim_end_matches('*');
        let delims = match base {
            "pmatrix" => Some((Some('('), Some(')'))),
            "bmatrix" => Some((Some('['), Some(']'))),
            "Bmatrix" => Some((Some('{'), Some('}'))),
            "vmatrix" => Some((Some('|'), Some('|'))),
            "Vmatrix" => Some((Some('‖'), Some('‖'))),
            _ => None,
        };
        let wrap = |inner: MNode, beg: Option<char>, end: Option<char>| MNode::Delim {
            beg,
            end,
            sep: None,
            grow: true,
            shp_match: false,
            elems: vec![vec![inner]],
        };
        match base {
            "matrix" | "pmatrix" | "bmatrix" | "Bmatrix" | "vmatrix" | "Vmatrix" | "smallmatrix" | "array" => {
                let m = MNode::Matrix { rows, col_jc: Vec::new() };
                match delims {
                    Some((b, e)) => vec![wrap(m, b, e)],
                    None => vec![m],
                }
            }
            "cases" | "dcases" => {
                let eq = MNode::EqArr { rows: rows.into_iter().map(join_cells).collect() };
                vec![wrap(eq, Some('{'), None)]
            }
            "rcases" => {
                let eq = MNode::EqArr { rows: rows.into_iter().map(join_cells).collect() };
                vec![wrap(eq, None, Some('}'))]
            }
            _ => vec![MNode::EqArr { rows: rows.into_iter().map(join_cells).collect() }],
        }
    }
}

/// Cells of an aligned row joined with `&` alignment marks.
fn join_cells(cells: Vec<Arg>) -> Arg {
    let mut out = Vec::new();
    for (i, c) in cells.into_iter().enumerate() {
        if i > 0 {
            out.push(run("&"));
        }
        out.extend(c);
    }
    merge_runs(out)
}

fn group(a: Arg) -> MNode {
    MNode::Delim { beg: None, end: None, sep: None, grow: true, shp_match: false, elems: vec![a] }
}

fn script(base: Option<MNode>, sup: bool, s: Arg) -> MNode {
    match base {
        Some(MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub, sup: sp, e }) if e.is_empty() => {
            if sup {
                MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide: false, sub, sup: s, e }
            } else {
                MNode::Nary { chr, lim_loc, grow, sub_hide: false, sup_hide, sub: s, sup: sp, e }
            }
        }
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

/// Big operators without an operand take the atoms after them up to a relation or binary operator.
fn nary_operands(a: Arg) -> Arg {
    let mut out: Arg = Vec::with_capacity(a.len());
    let mut it = a.into_iter().peekable();
    while let Some(n) = it.next() {
        match n {
            MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub, sup, e } if e.is_empty() => {
                let mut body = Vec::new();
                while let Some(next) = it.peek() {
                    let stop = match next {
                        MNode::Run(r) => r.text.chars().next().is_some_and(|c| {
                            matches!(math_class(c), crate::math::MClass::Rel | crate::math::MClass::Bin | crate::math::MClass::Punct) || c == '&'
                        }),
                        _ => false,
                    };
                    if stop {
                        break;
                    }
                    if let Some(x) = it.next() {
                        body.push(x);
                    }
                }
                let body = nary_operands(body);
                out.push(MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub, sup, e: merge_runs(body) });
            }
            other => out.push(other),
        }
    }
    out
}

/// Set a style on every run inside.
fn restyle(a: Arg, sty: Option<MSty>, scr: MScr, nor: bool) -> Arg {
    a.into_iter()
        .map(|n| match n {
            MNode::Run(mut r) => {
                if sty.is_some() {
                    r.sty = sty;
                }
                if scr != MScr::Roman {
                    r.scr = scr;
                }
                r.nor |= nor;
                MNode::Run(r)
            }
            other => map_args(other, &|a| restyle(a, sty, scr, nor)),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Out.

/// The equation as LaTeX.
pub fn to_latex(nodes: &[MNode]) -> String {
    let mut s = String::new();
    arg(nodes, &mut s, 0);
    s.trim().to_string()
}

fn arg(a: &[MNode], s: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for n in a {
        node(n, s, depth + 1);
    }
}

fn braced(a: &[MNode], s: &mut String, depth: usize) {
    s.push('{');
    arg(a, s, depth);
    s.push('}');
}

fn chars(text: &str, s: &mut String) {
    for c in text.chars() {
        match c {
            '{' | '}' | '%' | '$' | '#' | '&' | '_' => {
                s.push('\\');
                s.push(c);
            }
            '\\' => s.push_str("\\backslash "),
            '−' => s.push('-'),
            c if c.is_ascii() => s.push(c),
            c => match name_of(c) {
                Some(n) if !matches!(n, "sqrt" | "matrix" | "of" | "above" | "below" | "box" | "rect") => {
                    s.push('\\');
                    s.push_str(n);
                    s.push(' ');
                }
                _ => s.push(c),
            },
        }
    }
}

fn delim_tex(c: Option<char>) -> String {
    match c {
        None => ".".into(),
        Some('{') => "\\{".into(),
        Some('}') => "\\}".into(),
        Some('‖') => "\\|".into(),
        Some(c) if c.is_ascii() => c.to_string(),
        Some(c) => name_of(c).map(|n| format!("\\{n} ")).unwrap_or_else(|| c.to_string()),
    }
}

fn node(n: &MNode, s: &mut String, depth: usize) {
    match n {
        MNode::Run(r) => {
            let (open, close) = if r.nor {
                ("\\text{", "}")
            } else {
                match (r.sty, r.scr) {
                    (_, MScr::DoubleStruck) => ("\\mathbb{", "}"),
                    (_, MScr::Script) => ("\\mathcal{", "}"),
                    (_, MScr::Fraktur) => ("\\mathfrak{", "}"),
                    (_, MScr::SansSerif) => ("\\mathsf{", "}"),
                    (_, MScr::Monospace) => ("\\mathtt{", "}"),
                    (Some(MSty::Bold), _) => ("\\mathbf{", "}"),
                    (Some(MSty::BoldItalic), _) => ("\\boldsymbol{", "}"),
                    (Some(MSty::Plain), _) if r.text.chars().any(|c| c.is_alphabetic()) => {
                        if FUNCTION_NAMES.contains(&r.text.as_str()) {
                            s.push('\\');
                            s.push_str(&r.text);
                            s.push(' ');
                            return;
                        }
                        ("\\mathrm{", "}")
                    }
                    _ => ("", ""),
                }
            };
            s.push_str(open);
            if r.nor {
                s.push_str(&r.text);
            } else {
                chars(&r.text, s);
            }
            s.push_str(close);
        }
        MNode::Frac { kind, num, den } => match kind {
            FracKind::NoBar => {
                s.push_str("\\genfrac{}{}{0pt}{}");
                braced(num, s, depth);
                braced(den, s, depth);
            }
            FracKind::Linear | FracKind::Skewed => {
                braced(num, s, depth);
                s.push('/');
                braced(den, s, depth);
            }
            FracKind::Bar => {
                s.push_str("\\frac");
                braced(num, s, depth);
                braced(den, s, depth);
            }
        },
        MNode::Script { kind, base, sub, sup } => {
            if *kind == ScriptKind::Pre {
                s.push_str("{}_");
                braced(sub, s, depth);
                s.push('^');
                braced(sup, s, depth);
                braced(base, s, depth);
                return;
            }
            braced(base, s, depth);
            if matches!(kind, ScriptKind::Sub | ScriptKind::SubSup) {
                s.push('_');
                braced(sub, s, depth);
            }
            if matches!(kind, ScriptKind::Sup | ScriptKind::SubSup) {
                s.push('^');
                braced(sup, s, depth);
            }
        }
        MNode::Rad { deg, deg_hide, e } => {
            s.push_str("\\sqrt");
            if !deg_hide && !deg.is_empty() {
                s.push('[');
                arg(deg, s, depth);
                s.push(']');
            }
            braced(e, s, depth);
        }
        MNode::Nary { chr, lim_loc, sub_hide, sup_hide, sub, sup, e, .. } => {
            match name_of(*chr) {
                Some(n) => {
                    s.push('\\');
                    s.push_str(n);
                }
                None => s.push(*chr),
            }
            match lim_loc {
                Some(crate::math::LimLoc::UndOvr) => s.push_str("\\limits"),
                Some(crate::math::LimLoc::SubSup) if !crate::math::is_integral(*chr) => s.push_str("\\nolimits"),
                _ => {}
            }
            if !sub_hide && !sub.is_empty() {
                s.push('_');
                braced(sub, s, depth);
            }
            if !sup_hide && !sup.is_empty() {
                s.push('^');
                braced(sup, s, depth);
            }
            s.push(' ');
            braced(e, s, depth);
        }
        MNode::Delim { beg, end, elems, .. } => {
            s.push_str("\\left");
            s.push_str(&delim_tex(*beg));
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    s.push_str("\\middle|");
                }
                arg(e, s, depth);
            }
            s.push_str("\\right");
            s.push_str(&delim_tex(*end));
        }
        MNode::Func { name, e } => {
            arg(name, s, depth);
            braced(e, s, depth);
        }
        MNode::Lim { upper, e, lim } => {
            // `\lim_{…}` keeps its name; other bases stack.
            let simple = matches!(e.as_slice(), [MNode::Run(r)] if r.sty == Some(MSty::Plain) && FUNCTION_NAMES.contains(&r.text.as_str()));
            if simple && !upper {
                arg(e, s, depth);
                s.push('_');
                braced(lim, s, depth);
            } else if let [MNode::GroupChr { chr, top, e: inner }] = e.as_slice() {
                s.push_str(if *top { "\\overbrace" } else { "\\underbrace" });
                braced(inner, s, depth);
                s.push(if *upper { '^' } else { '_' });
                braced(lim, s, depth);
                let _ = chr;
            } else {
                s.push_str(if *upper { "\\overset" } else { "\\underset" });
                braced(lim, s, depth);
                braced(e, s, depth);
            }
        }
        MNode::Acc { chr, e } => {
            let cmd = match chr {
                '\u{302}' => "hat",
                '\u{303}' => "tilde",
                '\u{307}' => "dot",
                '\u{308}' => "ddot",
                '\u{20DB}' => "dddot",
                '\u{30C}' => "check",
                '\u{306}' => "breve",
                '\u{301}' => "acute",
                '\u{300}' => "grave",
                '\u{305}' | '\u{AF}' => "bar",
                '\u{20D6}' => "overleftarrow",
                '\u{20E1}' => "overleftrightarrow",
                _ => "vec",
            };
            s.push('\\');
            s.push_str(cmd);
            braced(e, s, depth);
        }
        MNode::Bar { top, e } => {
            s.push_str(if *top { "\\overline" } else { "\\underline" });
            braced(e, s, depth);
        }
        MNode::BorderBox { e, .. } => {
            s.push_str("\\boxed");
            braced(e, s, depth);
        }
        MNode::Boxed { e } => braced(e, s, depth),
        MNode::GroupChr { top, e, .. } => {
            s.push_str(if *top { "\\overbrace" } else { "\\underbrace" });
            braced(e, s, depth);
        }
        MNode::EqArr { rows } => {
            if rows.len() == 1
                && let Some(r) = rows.first()
                && crate::math::has_number_mark(r)
            {
                arg(r, s, depth);
                return;
            }
            s.push_str("\\begin{aligned}");
            for (i, r) in rows.iter().enumerate() {
                if i > 0 {
                    s.push_str("\\\\");
                }
                for n in r {
                    match n {
                        MNode::Run(run) if !run.lit && run.text.contains('&') => {
                            for (k, part) in run.text.split('&').enumerate() {
                                if k > 0 {
                                    s.push('&');
                                }
                                let mut p = run.clone();
                                p.text = part.to_string();
                                node(&MNode::Run(p), s, depth + 1);
                            }
                        }
                        other => node(other, s, depth + 1),
                    }
                }
            }
            s.push_str("\\end{aligned}");
        }
        MNode::Matrix { rows, .. } => {
            s.push_str("\\begin{matrix}");
            for (i, r) in rows.iter().enumerate() {
                if i > 0 {
                    s.push_str("\\\\");
                }
                for (j, c) in r.iter().enumerate() {
                    if j > 0 {
                        s.push('&');
                    }
                    arg(c, s, depth);
                }
            }
            s.push_str("\\end{matrix}");
        }
        MNode::Phant { e, .. } => {
            s.push_str("\\phantom");
            braced(e, s, depth);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{LimLoc, MathJc};

    #[test]
    fn latex_structures() {
        let n = parse_latex(r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}");
        assert_eq!(crate::math::to_linear(&n), "x=(-b±√(b^2-4ac))/2a");
        let n = parse_latex(r"\sum_{i=1}^{n} i^2 = \frac{n(n+1)(2n+1)}{6}");
        let MNode::Nary { chr, sub, sup, e, .. } = &n[0] else { panic!("{n:#?}") };
        assert_eq!((*chr, sub.len(), sup.len()), ('∑', 1, 1));
        assert!(matches!(e.first(), Some(MNode::Script { .. })), "{e:#?}");
        let n = parse_latex(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}");
        assert!(
            matches!(&n[0], MNode::Delim { beg: Some('('), elems, .. } if matches!(elems[0][0], MNode::Matrix { ref rows, .. } if rows.len() == 2))
        );
        let n = parse_latex(r"f(x)=\begin{cases} x & x\ge 0\\ -x & x<0\end{cases}");
        assert!(n.iter().any(|m| matches!(m, MNode::Delim { beg: Some('{'), end: None, .. })), "{n:#?}");
        let n = parse_latex(r"\lim_{n\to\infty}\left(1+\frac{1}{n}\right)^n");
        assert!(matches!(&n[0], MNode::Func { .. }), "{n:#?}");
        let n = parse_latex(r"\mathbb{R}\hat{x}\text{ if }\overbrace{a+b}^{k}");
        assert!(matches!(&n[0], MNode::Run(r) if r.scr == MScr::DoubleStruck));
        let _ = (LimLoc::UndOvr, MathJc::Center);
    }

    #[test]
    fn latex_round_trip() {
        for lin in ["x=(-b±√(b^2-4ac))/2a", "∑_(i=1)^n▒i^2", "√(3&x)", "a⃗+x̂", "■(a&b@c&d)", "sin⁡x+cos⁡y", "e^(-x^2)"] {
            let n = crate::math::parse_linear(lin);
            let tex = to_latex(&n);
            let back = parse_latex(&tex);
            assert_eq!(crate::math::to_linear(&back), crate::math::to_linear(&n), "{lin} → {tex}");
        }
    }

    #[test]
    fn hostile_latex_never_panics() {
        for s in ["", "\\", "{", "}", "^", "_", "\\frac", "\\sqrt[", "\\left(", "\\right)", "\\begin{matrix", "&&\\\\", "{{{{", "\\end{x}", "a^^b__c"]
        {
            let _ = to_latex(&parse_latex(s));
        }
        let deep = "{".repeat(5000);
        let _ = parse_latex(&deep);
        let deep = "\\frac{".repeat(3000);
        let _ = parse_latex(&deep);
        let deep = "\\sqrt".repeat(5000);
        let _ = parse_latex(&deep);
        let deep = "\\hat".repeat(5000) + "x";
        let _ = parse_latex(&deep);
    }
}
