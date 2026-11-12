use super::limits::MAX_NEST;

// ---------------------------------------------------------------------------------------------
// Equations: WordCraft keeps them in the linear format (`x=(-b±√(b^2-4ac))/2a`).

/// Symbols with a LaTeX command: (character, command without the backslash).
const MATH_SYMBOLS: &[(char, &str)] = &[
    ('α', "alpha"),
    ('β', "beta"),
    ('γ', "gamma"),
    ('δ', "delta"),
    ('ϵ', "epsilon"),
    ('ε', "varepsilon"),
    ('ζ', "zeta"),
    ('η', "eta"),
    ('θ', "theta"),
    ('ϑ', "vartheta"),
    ('ι', "iota"),
    ('κ', "kappa"),
    ('λ', "lambda"),
    ('μ', "mu"),
    ('ν', "nu"),
    ('ξ', "xi"),
    ('π', "pi"),
    ('ϖ', "varpi"),
    ('ρ', "rho"),
    ('ϱ', "varrho"),
    ('σ', "sigma"),
    ('ς', "varsigma"),
    ('τ', "tau"),
    ('υ', "upsilon"),
    ('ϕ', "phi"),
    ('φ', "varphi"),
    ('χ', "chi"),
    ('ψ', "psi"),
    ('ω', "omega"),
    ('Γ', "Gamma"),
    ('Δ', "Delta"),
    ('Θ', "Theta"),
    ('Λ', "Lambda"),
    ('Ξ', "Xi"),
    ('Π', "Pi"),
    ('Σ', "Sigma"),
    ('Υ', "Upsilon"),
    ('Φ', "Phi"),
    ('Ψ', "Psi"),
    ('Ω', "Omega"),
    ('±', "pm"),
    ('∓', "mp"),
    ('×', "times"),
    ('÷', "div"),
    ('⋅', "cdot"),
    ('·', "cdot"),
    ('∘', "circ"),
    ('∗', "ast"),
    ('≤', "leq"),
    ('≥', "geq"),
    ('≠', "neq"),
    ('≈', "approx"),
    ('≡', "equiv"),
    ('∼', "sim"),
    ('≃', "simeq"),
    ('≅', "cong"),
    ('∝', "propto"),
    ('≪', "ll"),
    ('≫', "gg"),
    ('∈', "in"),
    ('∉', "notin"),
    ('∋', "ni"),
    ('⊂', "subset"),
    ('⊃', "supset"),
    ('⊆', "subseteq"),
    ('⊇', "supseteq"),
    ('∪', "cup"),
    ('∩', "cap"),
    ('∖', "setminus"),
    ('∅', "emptyset"),
    ('∀', "forall"),
    ('∃', "exists"),
    ('¬', "neg"),
    ('∧', "wedge"),
    ('∨', "vee"),
    ('⊕', "oplus"),
    ('⊗', "otimes"),
    ('→', "to"),
    ('←', "leftarrow"),
    ('↔', "leftrightarrow"),
    ('⇒', "Rightarrow"),
    ('⇐', "Leftarrow"),
    ('⇔', "Leftrightarrow"),
    ('↦', "mapsto"),
    ('↑', "uparrow"),
    ('↓', "downarrow"),
    ('∑', "sum"),
    ('∏', "prod"),
    ('∐', "coprod"),
    ('∫', "int"),
    ('∬', "iint"),
    ('∭', "iiint"),
    ('∮', "oint"),
    ('⋃', "bigcup"),
    ('⋂', "bigcap"),
    ('∞', "infty"),
    ('∂', "partial"),
    ('∇', "nabla"),
    ('ℏ', "hbar"),
    ('ℓ', "ell"),
    ('ℵ', "aleph"),
    ('ℜ', "Re"),
    ('ℑ', "Im"),
    ('∠', "angle"),
    ('⊥', "perp"),
    ('∥', "parallel"),
    ('′', "prime"),
    ('…', "ldots"),
    ('⋯', "cdots"),
    ('⋮', "vdots"),
    ('⋱', "ddots"),
    ('⟨', "langle"),
    ('⟩', "rangle"),
    ('⌊', "lfloor"),
    ('⌋', "rfloor"),
    ('⌈', "lceil"),
    ('⌉', "rceil"),
    ('°', "circ"),
    ('•', "bullet"),
    ('⊤', "top"),
];

/// Extra input spellings mapped to the same symbols.
const MATH_ALIASES: &[(&str, char)] = &[
    ("le", '≤'),
    ("ge", '≥'),
    ("ne", '≠'),
    ("rightarrow", '→'),
    ("gets", '←'),
    ("land", '∧'),
    ("lor", '∨'),
    ("lnot", '¬'),
    ("dots", '…'),
    ("dotsc", '…'),
    ("dotsb", '⋯'),
    ("varnothing", '∅'),
    ("implies", '⇒'),
    ("iff", '⇔'),
    ("degree", '°'),
    ("lbrace", '{'),
    ("rbrace", '}'),
    ("vert", '|'),
    ("mid", '|'),
    ("Vert", '‖'),
    ("lvert", '|'),
    ("rvert", '|'),
    ("bot", '⊥'),
    ("hslash", 'ℏ'),
    ("colon", ':'),
];

/// Operator names written upright in both formats.
const MATH_FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "log", "ln", "lg", "exp", "lim",
    "liminf", "limsup", "min", "max", "sup", "inf", "det", "dim", "gcd", "deg", "arg", "ker", "Pr", "hom",
];

/// Control words an exported equation may carry besides the symbols and functions above (font
/// switches, delimiters, spacing).
const MATH_COMMANDS: &[&str] = &[
    "mathbb",
    "mathbf",
    "mathcal",
    "mathfrak",
    "mathit",
    "mathrm",
    "mathsf",
    "mathtt",
    "boldsymbol",
    "left",
    "right",
    "quad",
    "qquad",
    "displaystyle",
    "textstyle",
    "limits",
    "nolimits",
    "not",
];

/// May `\\name` from an equation's text be written to the exported file as a command? Only the
/// names known here. An equation's text comes from the document (a file someone else may have
/// made), and `\\input`, `\\write` or `\\def` in it would act on the machine of whoever compiles
/// the export.
fn known_math_command(name: &str) -> bool {
    math_symbol(name).is_some() || MATH_FUNCTIONS.contains(&name) || MATH_COMMANDS.contains(&name)
}

fn math_symbol(name: &str) -> Option<char> {
    MATH_SYMBOLS.iter().find(|(_, n)| *n == name).map(|(c, _)| *c).or_else(|| MATH_ALIASES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c))
}

/// For every `l` in `chars`, the index of the `r` that closes it, found in one stack pass.
/// Looking groups up here keeps scanning linear: rescanning to the end of the input for every
/// unmatched `(` made long equations quadratic.
pub(super) fn closers(chars: &[char], l: char, r: char) -> Vec<Option<usize>> {
    let mut out = vec![None; chars.len()];
    let mut open = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        if *c == l {
            open.push(i);
        } else if *c == r
            && let Some(o) = open.pop()
            && let Some(slot) = out.get_mut(o)
        {
            *slot = Some(i);
        }
    }
    out
}

/// The balanced group opening at `open` (see [`closers`]): its content and the index after it.
pub(super) fn balanced(chars: &[char], closers: &[Option<usize>], open: usize) -> Option<(String, usize)> {
    let close = (*closers.get(open)?)?;
    Some((chars.get(open + 1..close).unwrap_or_default().iter().collect(), close + 1))
}

/// A linear-format equation as LaTeX math.
pub fn linear_to_latex(linear: &str) -> String {
    linear_to_latex_at(linear, 0)
}

fn linear_to_latex_at(linear: &str, depth: usize) -> String {
    let chars: Vec<char> = linear.chars().collect();
    let parens = closers(&chars, '(', ')');
    let mut o = String::new();
    let mut i = 0;
    let sub = |s: &str| if depth < MAX_NEST { linear_to_latex_at(s, depth + 1) } else { escape_math_text(s) };
    while let Some(&c) = chars.get(i) {
        // (a)/(b) → \frac{a}{b}
        // (The `/` is looked for before the group is copied out: copying every group of a deeply
        // nested equation only to find no `/` after it was quadratic.)
        if c == '('
            && let Some(Some(close)) = parens.get(i)
            && chars.get(close + 1) == Some(&'/')
            && let Some((num, after)) = balanced(&chars, &parens, i)
            && let Some((den, end)) = balanced(&chars, &parens, after + 1)
        {
            o.push_str(&format!("\\frac{{{}}}{{{}}}", sub(&num), sub(&den)));
            i = end;
            continue;
        }
        match c {
            '^' | '_' => {
                // Never `^^`: TeX reads `^^5c` as a backslash, which would start a command.
                if c == '^' && o.ends_with('^') {
                    o.push_str("{}");
                }
                o.push(c);
                match balanced(&chars, &parens, i + 1) {
                    Some((inner, end)) => {
                        o.push_str(&format!("{{{}}}", sub(&inner)));
                        i = end;
                    }
                    None => i += 1,
                }
                continue;
            }
            '√' => {
                match balanced(&chars, &parens, i + 1) {
                    Some((inner, end)) => {
                        match inner.split_once('&') {
                            Some((n, x)) => o.push_str(&format!("\\sqrt[{}]{{{}}}", sub(n), sub(x))),
                            None => o.push_str(&format!("\\sqrt{{{}}}", sub(&inner))),
                        }
                        i = end;
                    }
                    None => {
                        o.push_str("\\surd ");
                        i += 1;
                    }
                }
                continue;
            }
            '\\' => {
                // Linear format control words (`\alpha`) are LaTeX's too.
                let name: String = chars.iter().skip(i + 1).take_while(|c| c.is_ascii_alphabetic()).collect();
                if name.is_empty() {
                    o.push_str("\\backslash ");
                    i += 1;
                } else if !known_math_command(&name) {
                    // Not a command we know: written as the text it is.
                    o.push_str("\\backslash ");
                    o.push_str(&name);
                    i += 1 + name.len();
                } else {
                    o.push('\\');
                    o.push_str(&name);
                    i += 1 + name.len();
                    if chars.get(i).is_some_and(char::is_ascii_alphabetic) {
                        o.push(' ');
                    }
                }
                continue;
            }
            '#'..='&' => {
                o.push('\\');
                o.push(c);
            }
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            '~' => o.push_str("\\sim "),
            c => match MATH_SYMBOLS.iter().find(|(s, _)| *s == c) {
                Some((_, name)) => {
                    o.push('\\');
                    o.push_str(name);
                    // A control word needs a space only before a letter.
                    if chars.get(i + 1).is_some_and(char::is_ascii_alphabetic) {
                        o.push(' ');
                    }
                }
                None if c.is_control() => {}
                None => o.push(c),
            },
        }
        i += 1;
    }
    o.trim().to_string()
}

fn escape_math_text(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars().filter(|c| !matches!(c, '{' | '}' | '\\' | '$' | '%' | '#' | '&') && !c.is_control()) {
        // Never `^^` (see `linear_to_latex_at`).
        if c == '^' && o.ends_with('^') {
            continue;
        }
        o.push(c);
    }
    o
}

/// LaTeX math as a linear-format equation.
pub fn latex_to_linear(src: &str) -> String {
    let toks = math_tokens(src);
    let mut pos = 0;
    let s = math_seq(&toks, &mut pos, 0, false);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, PartialEq)]
enum MTok {
    Cmd(String),
    Open,
    Close,
    OptOpen,
    OptClose,
    Char(char),
}

fn math_tokens(src: &str) -> Vec<MTok> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            '\\' => {
                let name: String = chars.iter().skip(i + 1).take_while(|c| c.is_ascii_alphabetic()).collect();
                if name.is_empty() {
                    if let Some(&n) = chars.get(i + 1) {
                        out.push(MTok::Cmd(n.to_string()));
                    }
                    i += 2;
                } else {
                    i += 1 + name.len();
                    out.push(MTok::Cmd(name));
                }
            }
            '{' => {
                out.push(MTok::Open);
                i += 1;
            }
            '}' => {
                out.push(MTok::Close);
                i += 1;
            }
            '[' => {
                out.push(MTok::OptOpen);
                i += 1;
            }
            ']' => {
                out.push(MTok::OptClose);
                i += 1;
            }
            '%' => {
                // A comment, to the end of the line.
                while chars.get(i).is_some_and(|c| *c != '\n') {
                    i += 1;
                }
            }
            c => {
                out.push(MTok::Char(if c.is_whitespace() { ' ' } else { c }));
                i += 1;
            }
        }
    }
    out
}

/// One argument: a `{…}` group or a single token.
fn math_arg(t: &[MTok], pos: &mut usize, depth: usize) -> String {
    while t.get(*pos) == Some(&MTok::Char(' ')) {
        *pos += 1;
    }
    match t.get(*pos) {
        Some(MTok::Open) => {
            *pos += 1;
            let s = math_seq(t, pos, depth + 1, false);
            if t.get(*pos) == Some(&MTok::Close) {
                *pos += 1;
            }
            s
        }
        Some(MTok::Close) | None => String::new(),
        Some(_) => {
            let end = *pos + 1;
            let mut p = *pos;
            let s = math_seq(t.get(..end).unwrap_or_default(), &mut p, depth + 1, false);
            *pos = end;
            s
        }
    }
}

/// An optional `[…]` argument.
fn math_opt(t: &[MTok], pos: &mut usize, depth: usize) -> Option<String> {
    if t.get(*pos) != Some(&MTok::OptOpen) {
        return None;
    }
    *pos += 1;
    let s = math_seq(t, pos, depth + 1, true);
    if t.get(*pos) == Some(&MTok::OptClose) {
        *pos += 1;
    }
    Some(s)
}

/// Parenthesise a fraction part or script unless it is a single symbol.
fn wrap_part(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() == 1 || (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())) { s.to_string() } else { format!("({s})") }
}

fn math_seq(t: &[MTok], pos: &mut usize, depth: usize, in_opt: bool) -> String {
    let mut o = String::new();
    while let Some(tok) = t.get(*pos) {
        match tok {
            MTok::Close => break,
            MTok::OptClose if in_opt => break,
            MTok::Open => {
                *pos += 1;
                if depth < MAX_NEST {
                    o.push_str(&math_seq(t, pos, depth + 1, false));
                }
                if t.get(*pos) == Some(&MTok::Close) {
                    *pos += 1;
                }
                continue;
            }
            MTok::OptOpen => o.push('['),
            MTok::OptClose => o.push(']'),
            MTok::Char('^') | MTok::Char('_') => {
                let c = if *tok == MTok::Char('^') { '^' } else { '_' };
                *pos += 1;
                let a = if depth < MAX_NEST { math_arg(t, pos, depth) } else { String::new() };
                o.push(c);
                o.push_str(&wrap_part(&a));
                continue;
            }
            MTok::Char('&') => o.push(' '),
            MTok::Char('~') => o.push(' '),
            MTok::Char(c) => o.push(*c),
            MTok::Cmd(name) => {
                *pos += 1;
                let name = name.as_str();
                if depth >= MAX_NEST {
                    continue;
                }
                match name {
                    "frac" | "dfrac" | "tfrac" | "cfrac" => {
                        let a = math_arg(t, pos, depth);
                        let b = math_arg(t, pos, depth);
                        o.push_str(&format!("{}/{}", wrap_part(&a), wrap_part(&b)));
                    }
                    "binom" | "dbinom" | "tbinom" => {
                        let a = math_arg(t, pos, depth);
                        let b = math_arg(t, pos, depth);
                        o.push_str(&format!("({a}¦{b})"));
                    }
                    "sqrt" => {
                        let n = math_opt(t, pos, depth);
                        let a = math_arg(t, pos, depth);
                        match n {
                            Some(n) => o.push_str(&format!("√({n}&{a})")),
                            None => o.push_str(&format!("√({a})")),
                        }
                    }
                    "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "middle" => {
                        // Keep the delimiter, drop the sizing; `\left.` is no delimiter.
                        if t.get(*pos) == Some(&MTok::Char('.')) {
                            *pos += 1;
                        }
                    }
                    "text" | "textrm" | "textit" | "textbf" | "mathrm" | "mathit" | "mathbf" | "mathsf" | "mathtt" | "mathcal" | "mathbb"
                    | "mathfrak" | "boldsymbol" | "bm" | "pmb" | "operatorname" | "mbox" | "hbox" | "displaystyle" | "textstyle" | "scriptstyle" => {
                        if t.get(*pos) == Some(&MTok::Open) || !matches!(name, "displaystyle" | "textstyle" | "scriptstyle") {
                            let a = math_arg(t, pos, depth);
                            o.push_str(&a);
                        }
                    }
                    "overline" | "bar" => o.push_str(&format!("{}¯", wrap_part(&math_arg(t, pos, depth)))),
                    "hat" | "widehat" => o.push_str(&format!("{}̂", math_arg(t, pos, depth))),
                    "tilde" | "widetilde" => o.push_str(&format!("{}̃", math_arg(t, pos, depth))),
                    "vec" => o.push_str(&format!("{}⃗", math_arg(t, pos, depth))),
                    "dot" => o.push_str(&format!("{}̇", math_arg(t, pos, depth))),
                    "ddot" => o.push_str(&format!("{}̈", math_arg(t, pos, depth))),
                    "acute" => o.push_str(&format!("{}́", math_arg(t, pos, depth))),
                    "grave" => o.push_str(&format!("{}̀", math_arg(t, pos, depth))),
                    "breve" => o.push_str(&format!("{}̆", math_arg(t, pos, depth))),
                    "check" => o.push_str(&format!("{}̌", math_arg(t, pos, depth))),
                    "pmod" => {
                        let a = math_arg(t, pos, depth);
                        o.push_str(&format!(" (mod {a})"));
                    }
                    "bmod" => o.push_str(" mod "),
                    "begin" | "end" => {
                        // Inner environments (`matrix`, `cases`, `aligned`…): keep the content.
                        let _ = math_arg(t, pos, depth);
                        if name == "begin" {
                            let _ = math_opt(t, pos, depth);
                        }
                    }
                    "," | ";" | ":" | "quad" | "qquad" | " " | "enspace" | "thinspace" => o.push(' '),
                    "!" | "limits" | "nolimits" | "nonumber" | "notag" => {}
                    "\\" | "cr" => o.push(' '),
                    "{" | "}" | "%" | "#" | "&" | "$" | "_" => o.push_str(name),
                    "label" | "tag" => {
                        let _ = math_arg(t, pos, depth);
                    }
                    n if MATH_FUNCTIONS.contains(&n) => {
                        o.push_str(n);
                        o.push(' ');
                    }
                    n => match math_symbol(n) {
                        Some(c) => o.push(c),
                        // Linear format knows LaTeX-style control words too.
                        None => {
                            o.push('\\');
                            o.push_str(n);
                            o.push(' ');
                        }
                    },
                }
                continue;
            }
        }
        *pos += 1;
    }
    o
}
