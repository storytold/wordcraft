//! LaTeX (`.tex`) import and export.
//!
//! **Export** writes a standalone LaTeX 2ε `article` that compiles with pdfLaTeX, XeLaTeX or
//! LuaLaTeX using only packages every TeX distribution ships (`fontenc`, `ulem`, `xcolor`,
//! `multirow`, `amsmath`, `hyperref`). Headings become unnumbered sectioning commands (Word headings
//! carry no numbers), lists `itemize`/`enumerate`, tables `tabular` with the document's column
//! widths, equations `$…$` / `\[…\]`, and character formatting the matching text commands. Pictures
//! can't live inside a single `.tex` file, so each is replaced by its alternative text.
//!
//! **Import** reads the everyday subset of LaTeX that documents are written in: the preamble's
//! `\title`/`\author`, sectioning commands, paragraphs, lists, `quote`, `center`/`flush…`,
//! `verbatim`-like environments, `tabular` (with `\multicolumn`/`\multirow`), inline and display math
//! (kept as equations), text formatting commands and declarations, colours, links, accents and
//! TeX's special characters and ligatures. It is not a TeX engine: macros aren't expanded. Unknown
//! commands are dropped while their braced arguments are kept as text, so no words are lost; layout
//! commands (`\vspace`, `\setlength`, `\usepackage`…) are dropped with their arguments.
//!
//! Like the other text formats, the parser is lenient and never panics on hostile input: group and
//! environment nesting is depth-limited and the input size is capped.

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Para};
use wordcraft_doc::{Align, Document, Rgb};

/// Largest `.tex` source imported (bytes); longer input is cut off.
pub const MAX_SOURCE: usize = 32 << 20;
/// Nesting limit for groups, environments and arguments (hostile input).
const MAX_NEST: usize = 32;
/// Longest `tabular` column spec read (characters); the rest is ignored.
const MAX_SPEC: usize = 4096;
/// Most bookmarks kept in a row with no text between them.
const MAX_ANCHOR_RUN: usize = 64;
/// Text width of the exported article in points (`\linewidth` when reading widths).
const LINE_WIDTH: f32 = 345.0;
/// LaTeX's `\tabcolsep` (both sides of a cell), points.
const TABCOLSEP: f32 = 6.0;

// =============================================================================================
// Export

/// Export `doc` as a LaTeX source file.
pub fn export(doc: &Document) -> String {
    let flow = model::from_doc(doc);
    let mut w = Writer::default();
    let title_at = flow.blocks.iter().position(|b| matches!(b, FBlock::Para(p) if p.kind == Kind::Title && !p.is_empty()));
    let mut title = String::new();
    if let Some(FBlock::Para(p)) = title_at.and_then(|i| flow.blocks.get(i)) {
        title = w.inlines(&p.inlines);
    }
    w.title_at = title_at;
    w.blocks(&flow.blocks);
    let body = std::mem::take(&mut w.out);

    let meta = &flow.meta;
    let mut o = String::new();
    o.push_str("% Written by WordCraft (https://getartcraft.com/apps/wordcraft).\n");
    o.push_str("\\documentclass{article}\n");
    o.push_str("\\usepackage[T1]{fontenc}\n");
    if w.uses.ulem {
        o.push_str("\\usepackage[normalem]{ulem}\n");
    }
    if w.uses.color {
        o.push_str("\\usepackage[table]{xcolor}\n");
    }
    if w.uses.multirow {
        o.push_str("\\usepackage{multirow}\n");
    }
    if w.uses.math {
        o.push_str("\\usepackage{amsmath}\n");
        o.push_str("\\usepackage{amssymb}\n");
    }
    o.push_str("\\usepackage{hyperref}\n");
    let pdf = [("pdftitle", &meta.title), ("pdfauthor", &meta.author), ("pdfsubject", &meta.subject), ("pdfkeywords", &meta.keywords)];
    let set: Vec<String> = pdf.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| format!("{k}={{{}}}", escape_text(v.trim()))).collect();
    if !set.is_empty() {
        o.push_str(&format!("\\hypersetup{{{}}}\n", set.join(", ")));
    }
    if title_at.is_some() {
        o.push_str(&format!("\\title{{{title}}}\n"));
        // `\maketitle` shows only the title: the document didn't show an author or a date.
        o.push_str("\\author{}\n\\date{}\n");
    }
    o.push_str("\n\\begin{document}\n\n");
    o.push_str(body.trim_end());
    o.push_str("\n\n\\end{document}\n");
    o
}

#[derive(Default)]
struct Uses {
    ulem: bool,
    color: bool,
    multirow: bool,
    math: bool,
}

#[derive(Default)]
struct Writer {
    out: String,
    uses: Uses,
    /// Index of the paragraph `\maketitle` stands for (top level only).
    title_at: Option<usize>,
    /// Inside a table cell: no `verbatim`, paragraphs separated by `\par`.
    in_cell: bool,
    depth: usize,
}

const SECTIONS: [&str; 6] = ["section", "subsection", "subsubsection", "paragraph", "subparagraph", "subparagraph"];

impl Writer {
    fn blocks(&mut self, blocks: &[FBlock]) {
        let mut lists: Vec<bool> = Vec::new();
        let mut i = 0;
        while let Some(b) = blocks.get(i) {
            let list = if let FBlock::Para(p) = b { p.list } else { None };
            match list {
                Some(li) => {
                    let depth = (li.level.min(3) + 1) as usize;
                    while lists.len() > depth || (lists.len() == depth && lists.last() != Some(&li.ordered)) {
                        self.close_list(&mut lists);
                    }
                    while lists.len() < depth {
                        let env = if li.ordered { "enumerate" } else { "itemize" };
                        self.out.push_str(&format!("{}\\begin{{{env}}}\n", "  ".repeat(lists.len())));
                        lists.push(li.ordered);
                    }
                }
                None => {
                    while !lists.is_empty() {
                        self.close_list(&mut lists);
                    }
                }
            }
            match b {
                FBlock::Para(p) if p.list.is_some() => {
                    let text = self.inlines(&p.inlines);
                    self.page_break(p);
                    self.out.push_str(&format!("{}\\item {}\n", "  ".repeat(lists.len()), text.trim()));
                }
                FBlock::Para(p) if p.kind == Kind::Code => {
                    let mut j = i;
                    let mut lines = Vec::new();
                    while let Some(FBlock::Para(q)) = blocks.get(j) {
                        if q.kind != Kind::Code || q.list.is_some() {
                            break;
                        }
                        lines.push(q.text());
                        j += 1;
                    }
                    self.page_break(p);
                    self.code(&lines);
                    i = j.max(i + 1);
                    continue;
                }
                FBlock::Para(p) if p.kind == Kind::Quote => {
                    let mut j = i;
                    let mut paras = Vec::new();
                    while let Some(FBlock::Para(q)) = blocks.get(j) {
                        if q.kind != Kind::Quote || q.list.is_some() {
                            break;
                        }
                        paras.push(self.inlines(&q.inlines));
                        j += 1;
                    }
                    self.page_break(p);
                    let body: Vec<&str> = paras.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                    let sep = if self.in_cell { "\\par\n" } else { "\n\n" };
                    self.out.push_str(&format!("\\begin{{quote}}\n{}\n\\end{{quote}}\n\n", body.join(sep)));
                    i = j.max(i + 1);
                    continue;
                }
                FBlock::Para(p) => {
                    let top_title = self.depth == 0 && self.title_at == Some(i);
                    self.para(p, top_title);
                }
                FBlock::Table(t) => self.table(t),
            }
            i += 1;
        }
        while !lists.is_empty() {
            self.close_list(&mut lists);
        }
    }

    fn close_list(&mut self, lists: &mut Vec<bool>) {
        if let Some(ordered) = lists.pop() {
            let env = if ordered { "enumerate" } else { "itemize" };
            self.out.push_str(&format!("{}\\end{{{env}}}\n", "  ".repeat(lists.len())));
            if lists.is_empty() && !self.in_cell {
                self.out.push('\n');
            }
        }
    }

    fn page_break(&mut self, p: &Para) {
        if p.page_break && !self.in_cell {
            self.out.push_str("\\clearpage\n\n");
        }
    }

    fn para(&mut self, p: &Para, top_title: bool) {
        self.page_break(p);
        if p.kind == Kind::Rule {
            self.out.push_str("\\noindent\\rule{\\linewidth}{0.4pt}\n\n");
            return;
        }
        if top_title {
            self.out.push_str("\\maketitle\n\n");
            return;
        }
        let text = self.inlines(&p.inlines);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let end = if self.in_cell { "\\par\n" } else { "\n\n" };
        match p.kind {
            Kind::Heading(n) if !self.in_cell => {
                let cmd = SECTIONS.get(usize::from(n.clamp(1, 6)) - 1).copied().unwrap_or("section");
                // `\\` (a line break) isn't allowed in a heading.
                let text = text.replace("\\\\\n", " ");
                self.out.push_str(&format!("\\{cmd}*{{{text}}}\n\n"));
            }
            Kind::Title => self.out.push_str(&format!("\\begin{{center}}\n{{\\LARGE {text}\\par}}\n\\end{{center}}\n\n")),
            Kind::Heading(_) => self.out.push_str(&format!("\\textbf{{{text}}}{end}")),
            _ => match p.align {
                Some(Align::Center) => self.out.push_str(&format!("\\begin{{center}}\n{text}\n\\end{{center}}\n\n")),
                Some(Align::Right) => self.out.push_str(&format!("\\begin{{flushright}}\n{text}\n\\end{{flushright}}\n\n")),
                _ => {
                    self.out.push_str(text);
                    self.out.push_str(end);
                }
            },
        }
    }

    fn code(&mut self, lines: &[String]) {
        if self.in_cell {
            // `verbatim` can't go in a table cell: typewriter lines instead.
            let v: Vec<String> = lines.iter().map(|l| format!("\\texttt{{{}}}", escape_text(&l.replace(' ', "\u{A0}")))).collect();
            self.out.push_str(&v.join("\\\\\n"));
            self.out.push_str("\\par\n");
            return;
        }
        self.out.push_str("\\begin{verbatim}\n");
        for l in lines {
            // The environment ends at the first `\end{verbatim}`, so keep it from appearing.
            let l: String = l.replace("\\end{verbatim}", "\\end {verbatim}").chars().filter(|c| *c == '\t' || !c.is_control()).collect();
            self.out.push_str(&l);
            self.out.push('\n');
        }
        self.out.push_str("\\end{verbatim}\n\n");
    }

    /// LaTeX for a run of inlines.
    fn inlines(&mut self, inlines: &[Inline]) -> String {
        let mut o = String::new();
        for i in inlines {
            match i {
                Inline::Text(t, f) => {
                    let s = self.span(t, f);
                    o.push_str(&s);
                }
                Inline::Image(img) => {
                    let alt = if img.alt.trim().is_empty() { "Picture".to_string() } else { format!("Picture: {}", img.alt.trim()) };
                    o.push_str(&format!("\\textit{{[{}]}}", escape_text(&alt)));
                }
                Inline::Anchor(name) => o.push_str(&format!("\\hypertarget{{{}}}{{}}", escape_url(name))),
                Inline::Figure(_) => {}
                Inline::Equation { linear, display } => {
                    self.uses.math = true;
                    let src = linear_to_latex(linear);
                    if *display && !self.in_cell {
                        o.push_str(&format!("\n\\[ {src} \\]\n"));
                    } else {
                        o.push_str(&format!("${src}$"));
                    }
                }
            }
        }
        o
    }

    fn span(&mut self, text: &str, f: &Fmt) -> String {
        let mut s = if f.code { escape_text(&text.replace(' ', "\u{A0}")) } else { escape_text(text) };
        if s.is_empty() {
            return s;
        }
        let wrap = |s: String, cmd: &str| format!("\\{cmd}{{{s}}}");
        if f.code {
            s = wrap(s, "texttt");
        }
        if f.sup {
            s = wrap(s, "textsuperscript");
        } else if f.sub {
            s = wrap(s, "textsubscript");
        }
        if f.bold {
            s = wrap(s, "textbf");
        }
        if f.italic {
            s = wrap(s, "textit");
        }
        if f.underline {
            self.uses.ulem = true;
            s = wrap(s, "uline");
        }
        if f.strike {
            self.uses.ulem = true;
            s = wrap(s, "sout");
        }
        if let Some(c) = f.color.filter(|c| *c != Rgb(0, 0, 0)) {
            self.uses.color = true;
            s = format!("\\textcolor[HTML]{{{}}}{{{s}}}", hex(c));
        }
        if let Some(size) = f.size.filter(|v| v.is_finite() && *v >= 1.0 && *v <= 1638.0) {
            s = format!("{{\\fontsize{{{}}}{{{}}}\\selectfont {s}}}", num(size), num(size * 1.2));
        }
        if let Some(l) = f.link.as_deref().filter(|l| !l.is_empty()) {
            s = match l.strip_prefix('#') {
                Some(target) => format!("\\hyperlink{{{}}}{{{s}}}", escape_url(target)),
                None => format!("\\href{{{}}}{{{s}}}", escape_url(l)),
            };
        }
        s
    }

    fn table(&mut self, t: &FTable) {
        let cols = t.cols().clamp(1, wordcraft_doc::table::MAX_COLS);
        let widths: Vec<f32> = if t.widths.len() == cols && t.widths.iter().all(|w| w.is_finite() && *w > 1.0) {
            let total: f32 = t.widths.iter().sum();
            // Scale a table wider than the page to fit.
            let k = if total > LINE_WIDTH { LINE_WIDTH / total } else { 1.0 };
            t.widths.iter().map(|w| w * k).collect()
        } else {
            vec![LINE_WIDTH / cols as f32; cols]
        };
        let col = |w: f32| format!("p{{{}pt}}", num((w - 2.0 * TABCOLSEP).max(12.0)));
        let spec: String = widths.iter().map(|w| format!("{}|", col(*w))).collect();
        let mut o = format!("\\noindent\\begin{{tabular}}{{|{spec}}}\n\\hline\n");
        for row in &t.rows {
            let mut cells = Vec::new();
            let mut g = 0usize;
            for c in row {
                let span = (c.colspan.clamp(1, 63) as usize).min(cols.saturating_sub(g).max(1));
                let w: f32 = widths.get(g..(g + span).min(widths.len())).map(|s| s.iter().sum()).unwrap_or(72.0);
                let mut body = if c.covered { String::new() } else { self.cell(c) };
                if c.rowspan > 1 && !c.covered {
                    self.uses.multirow = true;
                    body = format!("\\multirow{{{}}}{{=}}{{{body}}}", c.rowspan.min(1000));
                }
                if let Some(sh) = c.shading {
                    self.uses.color = true;
                    body = format!("\\cellcolor[HTML]{{{}}}{body}", hex(sh));
                }
                if span > 1 {
                    let left = if g == 0 { "|" } else { "" };
                    body = format!("\\multicolumn{{{span}}}{{{left}{}|}}{{{body}}}", col(w));
                }
                cells.push(body);
                g += span;
            }
            if cells.is_empty() {
                continue;
            }
            o.push_str(&cells.join(" & "));
            o.push_str(" \\\\\n\\hline\n");
        }
        o.push_str("\\end{tabular}\n");
        self.out.push_str(&o);
        self.out.push_str(if self.in_cell { "\\par\n" } else { "\n" });
    }

    fn cell(&mut self, c: &Cell) -> String {
        let mut sub = Writer { in_cell: true, depth: self.depth + 1, ..Default::default() };
        if self.depth < model::MAX_DEPTH {
            sub.blocks(&c.blocks);
        } else {
            sub.out = escape_text(&c.text());
        }
        self.uses.ulem |= sub.uses.ulem;
        self.uses.color |= sub.uses.color;
        self.uses.multirow |= sub.uses.multirow;
        self.uses.math |= sub.uses.math;
        let s = sub.out.trim().trim_end_matches("\\par").trim_end().to_string();
        s.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n")
    }
}

fn hex(c: Rgb) -> String {
    format!("{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// A length without needless decimals.
fn num(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r}") }
}

/// Text with LaTeX's special characters escaped.
pub fn escape_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    let mut prev = '\0';
    let mut line_has_text = false;
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\textbackslash{}"),
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            '$' | '&' | '#' | '%' | '_' => {
                o.push('\\');
                o.push(c);
            }
            '~' => o.push_str("\\textasciitilde{}"),
            '^' => o.push_str("\\textasciicircum{}"),
            '<' => o.push_str("\\textless{}"),
            '>' => o.push_str("\\textgreater{}"),
            '|' => o.push_str("\\textbar{}"),
            '\u{A0}' => o.push('~'),
            // Keep TeX from joining `--`, ``` `` ``` and `''` into dashes and quotes.
            '-' if prev == '-' => o.push_str("{}-"),
            '`' => o.push_str("\\textasciigrave{}"),
            '\'' if prev == '\'' => o.push_str("{}'"),
            '\t' => o.push_str("\\quad{}"),
            '\n' | '\u{000B}' => {
                // A line break; LaTeX refuses one with nothing before it on the line.
                o.push_str(if line_has_text { "\\\\\n" } else { "\\mbox{}\\\\\n" });
                line_has_text = false;
                prev = c;
                continue;
            }
            c if c.is_control() => {}
            c => o.push(c),
        }
        if !c.is_whitespace() {
            line_has_text = true;
        }
        prev = c;
    }
    o
}

/// A URL or anchor name for `\href`/`\hyperlink`: `%`, `#`, `\`, braces escaped.
fn escape_url(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars().filter(|c| !c.is_control()) {
        // Never `^^`: TeX reads `^^5c` as a backslash, which would start a command.
        if c == '^' && o.ends_with('^') {
            continue;
        }
        if matches!(c, '%' | '#' | '\\' | '{' | '}') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

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
fn closers(chars: &[char], l: char, r: char) -> Vec<Option<usize>> {
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
fn balanced(chars: &[char], closers: &[Option<usize>], open: usize) -> Option<(String, usize)> {
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
                    if chars.get(i).is_some_and(|n| n.is_ascii_alphabetic()) {
                        o.push(' ');
                    }
                }
                continue;
            }
            '%' | '#' | '&' | '$' => {
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
                    if chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic()) {
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
                    | "mathfrak" | "boldsymbol" | "operatorname" | "mbox" | "hbox" | "displaystyle" | "textstyle" | "scriptstyle" => {
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

// =============================================================================================
// Import

/// Import a LaTeX source file.
pub fn import(bytes: &[u8]) -> Document {
    let mut d = model::to_doc(&parse(&crate::txt::decode(bytes.get(..MAX_SOURCE).unwrap_or(bytes))));
    d.ensure_nonempty();
    d
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    /// `\name` (a starred form keeps its `*`) or a control symbol like `\&`.
    Cmd(String),
    Open,
    Close,
    /// Text (spaces collapsed to one).
    Text(String),
    /// A blank line.
    Par,
    Amp,
    /// `~`: a non-breaking space.
    Tie,
    /// Math source, and whether it is displayed.
    Math(String, bool),
    /// The content of a verbatim-like environment.
    Verbatim(String),
    /// `\verb|…|`.
    Verb(String),
    /// `\begin{name}` and `\end{name}`.
    Begin(String),
    End(String),
}

const VERBATIM_ENVS: &[&str] = &["verbatim", "verbatim*", "Verbatim", "lstlisting", "minted", "alltt", "comment"];
const MATH_ENVS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "alignat",
    "alignat*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "flalign",
    "flalign*",
    "displaymath",
    "math",
    "eqnarray",
    "eqnarray*",
];

struct Lexer<'a> {
    s: &'a str,
    i: usize,
    out: Vec<Tok>,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i..).and_then(|r| r.chars().next())
    }
    fn rest(&self) -> &str {
        self.s.get(self.i..).unwrap_or("")
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }
    fn text(&mut self, t: &str) {
        if let Some(Tok::Text(s)) = self.out.last_mut() {
            s.push_str(t);
        } else {
            self.out.push(Tok::Text(t.to_string()));
        }
    }
    /// Skip the rest of a line (a comment), the line end and the next line's leading blanks.
    fn comment(&mut self) {
        while let Some(c) = self.bump() {
            if c == '\n' {
                break;
            }
        }
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
    }
    /// Read until `end` (not included; consumed) and return what came before.
    fn until(&mut self, end: &str) -> String {
        match self.rest().find(end) {
            Some(at) => {
                let s = self.rest().get(..at).unwrap_or("").to_string();
                self.i += at + end.len();
                s
            }
            None => {
                let s = self.rest().to_string();
                self.i = self.s.len();
                s
            }
        }
    }
    /// After `\begin` / `\end`: the `{name}`, if well formed.
    fn env_name(&mut self) -> Option<String> {
        let save = self.i;
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
        if self.peek() != Some('{') {
            self.i = save;
            return None;
        }
        self.bump();
        // Only the next 65 bytes are searched: looking through the rest of the file for a `}`
        // made a run of unclosed `\\begin{` quadratic.
        let Some(end) = self.rest().bytes().take(65).position(|b| b == b'}') else {
            // Unclosed or overlong: leave the `{` to be read as ordinary input.
            self.i = save;
            return None;
        };
        let r = self.rest();
        let name = r.get(..end).unwrap_or("").trim().to_string();
        self.i += end + 1;
        Some(name)
    }

    fn run(mut self) -> Vec<Tok> {
        while let Some(c) = self.peek() {
            match c {
                '\\' => {
                    self.bump();
                    let r = self.rest();
                    let n = r.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(r.len());
                    if n == 0 {
                        let Some(sym) = self.bump() else { break };
                        match sym {
                            '(' => {
                                let m = self.until("\\)");
                                self.out.push(Tok::Math(m, false));
                            }
                            '[' => {
                                let m = self.until("\\]");
                                self.out.push(Tok::Math(m, true));
                            }
                            _ => self.out.push(Tok::Cmd(sym.to_string())),
                        }
                        continue;
                    }
                    let mut name = r.get(..n).unwrap_or("").to_string();
                    self.i += n;
                    if self.peek() == Some('*') {
                        self.bump();
                        name.push('*');
                    }
                    match name.as_str() {
                        "begin" => match self.env_name() {
                            Some(env) if VERBATIM_ENVS.contains(&env.as_str()) => {
                                // `minted{lang}` / `lstlisting[options]`: skip to the line end.
                                let _options = self.until("\n");
                                let v = self.until(&format!("\\end{{{env}}}"));
                                if env != "comment" {
                                    self.out.push(Tok::Verbatim(v));
                                }
                            }
                            Some(env) if MATH_ENVS.contains(&env.as_str()) => {
                                let m = self.until(&format!("\\end{{{env}}}"));
                                self.out.push(Tok::Math(m, env != "math"));
                            }
                            Some(env) => self.out.push(Tok::Begin(env)),
                            None => self.out.push(Tok::Cmd(name)),
                        },
                        "end" => match self.env_name() {
                            Some(env) => self.out.push(Tok::End(env)),
                            None => self.out.push(Tok::Cmd(name)),
                        },
                        "verb" | "verb*" => match self.bump() {
                            Some(delim) if !delim.is_alphabetic() && !delim.is_whitespace() => {
                                let v = self.until(&delim.to_string());
                                self.out.push(Tok::Verb(v));
                            }
                            _ => {}
                        },
                        _ => {
                            self.out.push(Tok::Cmd(name));
                            // A control word swallows the spaces (and one line end) after it.
                            let mut newline = false;
                            while let Some(c) = self.peek() {
                                if c == ' ' || c == '\t' || (c == '\n' && !newline) {
                                    newline |= c == '\n';
                                    self.bump();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
                '%' => self.comment(),
                '{' => {
                    self.bump();
                    self.out.push(Tok::Open);
                }
                '}' => {
                    self.bump();
                    self.out.push(Tok::Close);
                }
                '&' => {
                    self.bump();
                    self.out.push(Tok::Amp);
                }
                '~' => {
                    self.bump();
                    self.out.push(Tok::Tie);
                }
                '$' => {
                    self.bump();
                    if self.peek() == Some('$') {
                        self.bump();
                        let m = self.until("$$");
                        self.out.push(Tok::Math(m, true));
                    } else {
                        let m = self.until("$");
                        self.out.push(Tok::Math(m, false));
                    }
                }
                '\n' | '\r' | ' ' | '\t' => {
                    // Whitespace: a blank line ends the paragraph, anything else is one space.
                    let mut lines = 0;
                    while let Some(c) = self.peek() {
                        match c {
                            '\n' => lines += 1,
                            ' ' | '\t' | '\r' => {}
                            _ => break,
                        }
                        self.bump();
                    }
                    if lines >= 2 {
                        self.out.push(Tok::Par);
                    } else {
                        self.text(" ");
                    }
                }
                _ => {
                    let r = self.rest();
                    let n = r.find(['\\', '%', '{', '}', '&', '~', '$', '\n', '\r', ' ', '\t']).unwrap_or(r.len()).max(c.len_utf8());
                    let t = r.get(..n).unwrap_or("").to_string();
                    self.i += n;
                    self.text(&t);
                }
            }
        }
        self.out
    }
}

fn tokenize(src: &str) -> Vec<Tok> {
    Lexer { s: src, i: 0, out: Vec::new() }.run()
}

/// TeX's text ligatures: dashes and quotes.
fn ligatures(s: &str) -> String {
    s.replace("---", "—").replace("--", "–").replace("``", "“").replace("''", "”").replace('`', "‘").replace("<<", "«").replace(">>", "»")
}

/// Character for a symbol command, if it is one.
fn text_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "&" => "&",
        "%" => "%",
        "$" => "$",
        "#" => "#",
        "_" => "_",
        "{" => "{",
        "}" => "}",
        " " => " ",
        "," | ";" | ":" | "enspace" | "thinspace" => " ",
        "quad" => "\u{2003}",
        "qquad" => "\u{2003}\u{2003}",
        "-" | "/" | "@" | "!" | "noindent" | "indent" | "relax" | "protect" | "selectfont" | "nobreak" | "sloppy" | "fussy" | "centering"
        | "raggedright" | "raggedleft" | "normalfont" | "item" => "",
        "textbackslash" | "backslash" => "\\",
        "textasciitilde" => "~",
        "textasciicircum" => "^",
        "textless" => "<",
        "textgreater" => ">",
        "textbar" => "|",
        "textbraceleft" => "{",
        "textbraceright" => "}",
        "textunderscore" => "_",
        "textasciigrave" => "`",
        "textquotesingle" => "'",
        "textquotedbl" => "\"",
        "ldots" | "dots" | "textellipsis" => "…",
        "textendash" => "–",
        "textemdash" => "—",
        "textbullet" => "•",
        "textperiodcentered" => "·",
        "textquoteleft" => "‘",
        "textquoteright" => "’",
        "textquotedblleft" => "“",
        "textquotedblright" => "”",
        "guillemotleft" | "guillemetleft" => "«",
        "guillemotright" | "guillemetright" => "»",
        "copyright" | "textcopyright" => "©",
        "textregistered" => "®",
        "texttrademark" => "™",
        "S" | "textsection" => "§",
        "P" | "textparagraph" => "¶",
        "dag" | "textdagger" => "†",
        "ddag" | "textdaggerdbl" => "‡",
        "pounds" | "textsterling" => "£",
        "euro" | "texteuro" => "€",
        "textyen" => "¥",
        "textcent" => "¢",
        "textdegree" | "degree" => "°",
        "textmu" => "µ",
        "texttimes" => "×",
        "textdiv" => "÷",
        "textpm" => "±",
        "textonehalf" => "½",
        "textonequarter" => "¼",
        "textthreequarters" => "¾",
        "textexclamdown" => "¡",
        "textquestiondown" => "¿",
        "ss" => "ß",
        "SS" => "SS",
        "ae" => "æ",
        "AE" => "Æ",
        "oe" => "œ",
        "OE" => "Œ",
        "o" => "ø",
        "O" => "Ø",
        "aa" => "å",
        "AA" => "Å",
        "l" => "ł",
        "L" => "Ł",
        "i" => "ı",
        "j" => "ȷ",
        "LaTeX" => "LaTeX",
        "LaTeXe" => "LaTeX2ε",
        "TeX" => "TeX",
        "XeTeX" => "XeTeX",
        "LuaTeX" => "LuaTeX",
        "BibTeX" => "BibTeX",
        _ => return None,
    })
}

/// Combining mark for an accent command.
fn accent_mark(name: &str) -> Option<char> {
    Some(match name {
        "'" => '\u{301}',
        "`" => '\u{300}',
        "^" => '\u{302}',
        "\"" => '\u{308}',
        "~" => '\u{303}',
        "=" => '\u{304}',
        "." => '\u{307}',
        "u" => '\u{306}',
        "v" => '\u{30C}',
        "H" => '\u{30B}',
        "c" => '\u{327}',
        "k" => '\u{328}',
        "r" => '\u{30A}',
        "d" => '\u{323}',
        "b" => '\u{331}',
        _ => return None,
    })
}

/// Precomposed letters for the common accents (base + combining mark otherwise).
fn compose(base: char, mark: char) -> String {
    const TABLE: &[(char, char, char)] = &[
        ('a', '\u{301}', 'á'),
        ('e', '\u{301}', 'é'),
        ('i', '\u{301}', 'í'),
        ('ı', '\u{301}', 'í'),
        ('o', '\u{301}', 'ó'),
        ('u', '\u{301}', 'ú'),
        ('y', '\u{301}', 'ý'),
        ('c', '\u{301}', 'ć'),
        ('n', '\u{301}', 'ń'),
        ('s', '\u{301}', 'ś'),
        ('z', '\u{301}', 'ź'),
        ('A', '\u{301}', 'Á'),
        ('E', '\u{301}', 'É'),
        ('I', '\u{301}', 'Í'),
        ('O', '\u{301}', 'Ó'),
        ('U', '\u{301}', 'Ú'),
        ('Y', '\u{301}', 'Ý'),
        ('a', '\u{300}', 'à'),
        ('e', '\u{300}', 'è'),
        ('i', '\u{300}', 'ì'),
        ('ı', '\u{300}', 'ì'),
        ('o', '\u{300}', 'ò'),
        ('u', '\u{300}', 'ù'),
        ('A', '\u{300}', 'À'),
        ('E', '\u{300}', 'È'),
        ('I', '\u{300}', 'Ì'),
        ('O', '\u{300}', 'Ò'),
        ('U', '\u{300}', 'Ù'),
        ('a', '\u{302}', 'â'),
        ('e', '\u{302}', 'ê'),
        ('i', '\u{302}', 'î'),
        ('ı', '\u{302}', 'î'),
        ('o', '\u{302}', 'ô'),
        ('u', '\u{302}', 'û'),
        ('A', '\u{302}', 'Â'),
        ('E', '\u{302}', 'Ê'),
        ('I', '\u{302}', 'Î'),
        ('O', '\u{302}', 'Ô'),
        ('U', '\u{302}', 'Û'),
        ('a', '\u{308}', 'ä'),
        ('e', '\u{308}', 'ë'),
        ('i', '\u{308}', 'ï'),
        ('ı', '\u{308}', 'ï'),
        ('o', '\u{308}', 'ö'),
        ('u', '\u{308}', 'ü'),
        ('y', '\u{308}', 'ÿ'),
        ('A', '\u{308}', 'Ä'),
        ('E', '\u{308}', 'Ë'),
        ('I', '\u{308}', 'Ï'),
        ('O', '\u{308}', 'Ö'),
        ('U', '\u{308}', 'Ü'),
        ('a', '\u{303}', 'ã'),
        ('n', '\u{303}', 'ñ'),
        ('o', '\u{303}', 'õ'),
        ('A', '\u{303}', 'Ã'),
        ('N', '\u{303}', 'Ñ'),
        ('O', '\u{303}', 'Õ'),
        ('c', '\u{327}', 'ç'),
        ('C', '\u{327}', 'Ç'),
        ('s', '\u{327}', 'ş'),
        ('S', '\u{327}', 'Ş'),
        ('a', '\u{30A}', 'å'),
        ('A', '\u{30A}', 'Å'),
        ('u', '\u{30A}', 'ů'),
        ('c', '\u{30C}', 'č'),
        ('d', '\u{30C}', 'ď'),
        ('e', '\u{30C}', 'ě'),
        ('n', '\u{30C}', 'ň'),
        ('r', '\u{30C}', 'ř'),
        ('s', '\u{30C}', 'š'),
        ('t', '\u{30C}', 'ť'),
        ('z', '\u{30C}', 'ž'),
        ('C', '\u{30C}', 'Č'),
        ('D', '\u{30C}', 'Ď'),
        ('E', '\u{30C}', 'Ě'),
        ('N', '\u{30C}', 'Ň'),
        ('R', '\u{30C}', 'Ř'),
        ('S', '\u{30C}', 'Š'),
        ('T', '\u{30C}', 'Ť'),
        ('Z', '\u{30C}', 'Ž'),
        ('g', '\u{306}', 'ğ'),
        ('G', '\u{306}', 'Ğ'),
        ('o', '\u{30B}', 'ő'),
        ('u', '\u{30B}', 'ű'),
        ('O', '\u{30B}', 'Ő'),
        ('U', '\u{30B}', 'Ű'),
        ('a', '\u{328}', 'ą'),
        ('e', '\u{328}', 'ę'),
        ('A', '\u{328}', 'Ą'),
        ('E', '\u{328}', 'Ę'),
        ('z', '\u{307}', 'ż'),
        ('Z', '\u{307}', 'Ż'),
        ('a', '\u{304}', 'ā'),
        ('e', '\u{304}', 'ē'),
        ('i', '\u{304}', 'ī'),
        ('o', '\u{304}', 'ō'),
        ('u', '\u{304}', 'ū'),
    ];
    match TABLE.iter().find(|(b, m, _)| *b == base && *m == mark) {
        Some((_, _, c)) => c.to_string(),
        None => format!("{base}{mark}"),
    }
}

/// Commands dropped together with their arguments (layout and definitions, not text).
const DROP_WITH_ARGS: &[&str] = &[
    "documentclass",
    "usepackage",
    "RequirePackage",
    "newcommand",
    "renewcommand",
    "providecommand",
    "newcommand*",
    "renewcommand*",
    "providecommand*",
    "newenvironment",
    "renewenvironment",
    "DeclareMathOperator",
    "DeclareMathOperator*",
    "def",
    "let",
    "setlength",
    "addtolength",
    "setcounter",
    "addtocounter",
    "stepcounter",
    "refstepcounter",
    "vspace",
    "vspace*",
    "hspace",
    "hspace*",
    "vskip",
    "hskip",
    "pagestyle",
    "thispagestyle",
    "pagenumbering",
    "bibliographystyle",
    "bibliography",
    "addbibresource",
    "printbibliography",
    "input",
    "include",
    "includeonly",
    "graphicspath",
    "hypersetup",
    "definecolor",
    "geometry",
    "numberwithin",
    "newtheorem",
    "newtheorem*",
    "theoremstyle",
    "phantom",
    "hphantom",
    "vphantom",
    "index",
    "nocite",
    "addcontentsline",
    "markboth",
    "markright",
    "linespread",
    "setstretch",
    "fancyhf",
    "fancyhead",
    "fancyfoot",
    "lstset",
    "captionsetup",
    "setmainfont",
    "setsansfont",
    "setmonofont",
    "newfontfamily",
    "date",
    "thanks",
    "maketitle*",
    "tableofcontents",
    "listoffigures",
    "listoftables",
    "appendix",
    "frontmatter",
    "mainmatter",
    "backmatter",
    "cline",
    "hline",
    "toprule",
    "midrule",
    "bottomrule",
    "cmidrule",
    "addlinespace",
    "endhead",
    "endfirsthead",
    "endfoot",
    "endlastfoot",
    "rowcolor",
    "arraystretch",
    "smallskip",
    "medskip",
    "bigskip",
    "today",
    "and",
];

/// Font size declarations (points, for the default 10 pt article).
fn size_of(name: &str) -> Option<f32> {
    Some(match name {
        "tiny" => 5.0,
        "scriptsize" => 7.0,
        "footnotesize" => 8.0,
        "small" => 9.0,
        "normalsize" => 0.0,
        "large" => 12.0,
        "Large" => 14.4,
        "LARGE" => 17.28,
        "huge" => 20.74,
        "Huge" => 24.88,
        _ => return None,
    })
}

/// A named colour (`xcolor`'s base names).
fn named_color(name: &str) -> Option<Rgb> {
    Some(match name.trim() {
        "black" => Rgb(0, 0, 0),
        "white" => Rgb(255, 255, 255),
        "red" => Rgb(255, 0, 0),
        "green" => Rgb(0, 255, 0),
        "blue" => Rgb(0, 0, 255),
        "cyan" => Rgb(0, 255, 255),
        "magenta" => Rgb(255, 0, 255),
        "yellow" => Rgb(255, 255, 0),
        "gray" => Rgb(128, 128, 128),
        "darkgray" => Rgb(64, 64, 64),
        "lightgray" => Rgb(191, 191, 191),
        "brown" => Rgb(191, 128, 64),
        "lime" => Rgb(191, 255, 0),
        "olive" => Rgb(128, 128, 0),
        "orange" => Rgb(255, 128, 0),
        "pink" => Rgb(255, 191, 191),
        "purple" => Rgb(191, 0, 64),
        "teal" => Rgb(0, 128, 128),
        "violet" => Rgb(128, 0, 128),
        _ => return None,
    })
}

/// `\textcolor[model]{spec}`: HTML, RGB (0–255) and rgb (0–1) models, or a name.
fn parse_color(model: Option<&str>, spec: &str) -> Option<Rgb> {
    let spec = spec.trim();
    match model.map(str::trim) {
        Some("HTML") => crate::model::parse_color(&format!("#{spec}")),
        Some("RGB") => {
            let v: Vec<u8> = spec.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).map(|x| x.clamp(0.0, 255.0) as u8).collect();
            match v.as_slice() {
                [r, g, b] => Some(Rgb(*r, *g, *b)),
                _ => None,
            }
        }
        Some("rgb") => {
            let v: Vec<u8> = spec
                .split(',')
                .filter_map(|x| x.trim().parse::<f32>().ok())
                .filter(|x| x.is_finite())
                .map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();
            match v.as_slice() {
                [r, g, b] => Some(Rgb(*r, *g, *b)),
                _ => None,
            }
        }
        Some("gray") => spec.parse::<f32>().ok().filter(|x| x.is_finite()).map(|x| {
            let g = (x.clamp(0.0, 1.0) * 255.0).round() as u8;
            Rgb(g, g, g)
        }),
        _ => {
            // `red!50` (a tint) and `red!50!blue` (a mix) keep the first colour.
            named_color(spec.split('!').next().unwrap_or(spec))
        }
    }
}

/// A TeX length in points (`3cm`, `0.4\linewidth`), if it is one.
fn length_pt(s: &str) -> Option<f32> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: f32 = if n.is_empty() { 1.0 } else { n.parse().ok()? };
    let k = match unit.trim() {
        "pt" => 1.0,
        "bp" => 72.27 / 72.0,
        "in" => 72.27,
        "cm" => 28.45,
        "mm" => 2.845,
        "em" => 10.0,
        "ex" => 4.3,
        "pc" => 12.0,
        "\\linewidth" | "\\textwidth" | "\\columnwidth" | "\\hsize" => LINE_WIDTH,
        _ => return None,
    };
    let v = n * k;
    (v.is_finite() && v > 0.0 && v < 10_000.0).then_some(v)
}

/// Column widths of a `tabular` column spec, when every column has one (`p{3cm}`).
fn spec_widths(spec: &str) -> (usize, Vec<f32>) {
    let (cols, widths) = spec_widths_at(spec, 0);
    (cols.min(wordcraft_doc::table::MAX_COLS), if widths.len() == cols { widths } else { Vec::new() })
}

/// [`spec_widths`] at nesting `depth` of `*{n}{…}` repeats. The column count saturates and the
/// widths stop growing past [`wordcraft_doc::table::MAX_COLS`], so nested repeats can't overflow
/// or exhaust memory; the returned widths are empty unless every column has one.
fn spec_widths_at(spec: &str, depth: usize) -> (usize, Vec<f32>) {
    let max_cols = wordcraft_doc::table::MAX_COLS;
    let chars: Vec<char> = spec.chars().collect();
    let braces = closers(&chars, '{', '}');
    let mut cols = 0usize;
    let mut widths = Vec::new();
    let mut all = true;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            'l' | 'c' | 'r' | 'X' | 'S' | 'L' | 'C' | 'R' | 'J' => {
                cols = cols.saturating_add(1);
                all = false;
            }
            'p' | 'm' | 'b' => {
                cols = cols.saturating_add(1);
                match balanced(&chars, &braces, i + 1) {
                    Some((w, end)) => {
                        match length_pt(&w) {
                            Some(v) if widths.len() < max_cols => widths.push(v + 2.0 * TABCOLSEP),
                            Some(_) => {}
                            None => all = false,
                        }
                        i = end;
                        continue;
                    }
                    None => all = false,
                }
            }
            '*' => {
                // `*{3}{l}`: repeat. Past the nesting limit the repeat is skipped, unmeasured.
                if let Some((n, end)) = balanced(&chars, &braces, i + 1)
                    && let Some((inner, end2)) = balanced(&chars, &braces, end)
                {
                    if depth < MAX_NEST {
                        let n: usize = n.trim().parse().unwrap_or(1).min(64);
                        let (c2, w2) = spec_widths_at(&inner, depth + 1);
                        cols = cols.saturating_add(c2.saturating_mul(n));
                        if w2.len() == c2 {
                            for _ in 0..n {
                                if widths.len() >= max_cols {
                                    break;
                                }
                                widths.extend(w2.iter().take(max_cols - widths.len()));
                            }
                        } else {
                            all = false;
                        }
                    } else {
                        all = false;
                    }
                    i = end2;
                    continue;
                }
            }
            '@' | '!' | '>' | '<' => {
                // `@{…}`, `>{…}`: skip the argument.
                if let Some((_, end)) = balanced(&chars, &braces, i + 1) {
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (cols, if all && widths.len() == cols { widths } else { Vec::new() })
}

/// Paragraph-level context from the enclosing environments.
#[derive(Clone, Copy, Default)]
struct Ctx {
    kind: Kind,
    align: Option<Align>,
}

struct Parser<'a> {
    t: &'a [Tok],
    pos: usize,
    blocks: Vec<FBlock>,
    para: Para,
    /// Character formatting in effect (groups save and restore it).
    fmt: Fmt,
    /// Paragraph context of the enclosing environments.
    ctx: Ctx,
    /// Text that followed an optional argument's `]` in the same token, emitted next.
    pending: Option<String>,
    /// Open lists: ordered?
    lists: Vec<bool>,
    page_break: bool,
    depth: usize,
    meta: model::Meta,
    /// Sectioning levels in use: `\part`/`\chapter` push `\section` down.
    top: u8,
}

impl<'a> Parser<'a> {
    fn new(t: &'a [Tok], depth: usize) -> Parser<'a> {
        Parser {
            t,
            pos: 0,
            blocks: Vec::new(),
            para: Para::default(),
            fmt: Fmt::default(),
            ctx: Ctx::default(),
            pending: None,
            lists: Vec::new(),
            page_break: false,
            depth,
            meta: Default::default(),
            top: 0,
        }
    }

    fn fmt(&self) -> Fmt {
        self.fmt.clone()
    }
    fn fmt_mut(&mut self) -> &mut Fmt {
        &mut self.fmt
    }
    fn ctx(&self) -> Ctx {
        self.ctx
    }

    /// A bookmark here. `text` looks back past the bookmarks that end the paragraph to see what
    /// the last text was, so a run of them with nothing between is kept short: after
    /// [`MAX_ANCHOR_RUN`] in a row further ones are left out (thousands of labels with no text
    /// between them made reading a file take time quadratic in its size).
    fn anchor(&mut self, name: String) {
        let run = self.para.inlines.iter().rev().take(MAX_ANCHOR_RUN).take_while(|i| matches!(i, Inline::Anchor(_))).count();
        if run < MAX_ANCHOR_RUN {
            self.para.inlines.push(Inline::Anchor(name));
        }
    }

    fn text(&mut self, s: &str) {
        // Spaces collapse, and none start a paragraph (as in TeX).
        let after_space = match self.para.inlines.iter().rev().find(|i| !matches!(i, Inline::Anchor(_))) {
            Some(Inline::Text(t, _)) => t.is_empty() || t.ends_with([' ', '\n']),
            Some(_) => false,
            None => true,
        };
        let s = if after_space { s.trim_start_matches(' ') } else { s };
        if s.is_empty() {
            return;
        }
        let f = self.fmt();
        self.para.push_text(s, &f);
    }

    /// End the current paragraph (if it has content).
    fn flush(&mut self) {
        let mut p = std::mem::take(&mut self.para);
        p.trim();
        if p.is_empty() && !p.inlines.iter().any(|i| matches!(i, Inline::Image(_) | Inline::Equation { .. })) {
            // Keep a list item's position even when empty.
            if p.list.is_none() {
                return;
            }
        }
        let c = self.ctx();
        if p.kind == Kind::Normal {
            p.kind = c.kind;
        }
        if p.align.is_none() {
            p.align = c.align;
        }
        if self.page_break {
            p.page_break = true;
            self.page_break = false;
        }
        self.blocks.push(FBlock::Para(p));
    }

    fn skip_spaces(&mut self) {
        while let Some(Tok::Text(s)) = self.t.get(self.pos) {
            if !s.trim().is_empty() {
                break;
            }
            self.pos += 1;
        }
    }

    /// The tokens of an optional `[…]` argument, if one follows.
    fn opt_arg(&mut self) -> Option<String> {
        let save = self.pos;
        self.skip_spaces();
        let Some(Tok::Text(s)) = self.t.get(self.pos) else {
            self.pos = save;
            return None;
        };
        if !s.starts_with('[') {
            self.pos = save;
            return None;
        }
        // Collect raw text up to the matching `]` (no nested brackets across tokens).
        let mut out = String::new();
        let mut first = true;
        while let Some(tok) = self.t.get(self.pos) {
            match tok {
                Tok::Text(s) => {
                    let s = if first { s.get(1..).unwrap_or("") } else { s.as_str() };
                    first = false;
                    if let Some(end) = s.find(']') {
                        out.push_str(s.get(..end).unwrap_or(""));
                        let rest = s.get(end + 1..).unwrap_or("").to_string();
                        self.pos += 1;
                        if !rest.trim().is_empty() {
                            // The text after `]` comes next (tokens are shared, so it waits here).
                            self.pending = Some(rest);
                        }
                        return Some(out);
                    }
                    out.push_str(s);
                }
                Tok::Open => out.push('{'),
                Tok::Close => out.push('}'),
                Tok::Cmd(c) => {
                    out.push('\\');
                    out.push_str(c);
                }
                Tok::Par => break,
                _ => {}
            }
            self.pos += 1;
        }
        Some(out)
    }

    /// The tokens of a `{…}` argument (or a single token), consumed.
    fn arg_tokens(&mut self) -> &'a [Tok] {
        self.skip_spaces();
        match self.t.get(self.pos) {
            Some(Tok::Open) => {
                let start = self.pos + 1;
                let mut depth = 0usize;
                let mut i = self.pos;
                while let Some(tok) = self.t.get(i) {
                    match tok {
                        Tok::Open => depth += 1,
                        Tok::Close => {
                            depth -= 1;
                            if depth == 0 {
                                self.pos = i + 1;
                                return self.t.get(start..i).unwrap_or_default();
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                self.pos = self.t.len();
                self.t.get(start..).unwrap_or_default()
            }
            Some(Tok::Text(_)) => {
                // A single character argument (`\textbf x`) is rare: take the whole token.
                let i = self.pos;
                self.pos += 1;
                self.t.get(i..i + 1).unwrap_or_default()
            }
            Some(Tok::Cmd(_)) => {
                let i = self.pos;
                self.pos += 1;
                self.t.get(i..i + 1).unwrap_or_default()
            }
            _ => &[],
        }
    }

    /// An argument as plain text (no formatting): labels, URLs, colour names.
    fn arg_raw(&mut self) -> String {
        raw_text(self.arg_tokens())
    }

    /// An argument as LaTeX source (braces kept): column specs, lengths.
    fn arg_source(&mut self) -> String {
        source_text(self.arg_tokens())
    }

    /// An argument's text as it would be shown (formatting dropped): titles and authors.
    fn arg_text(&mut self) -> String {
        let toks = self.arg_tokens();
        let mut sub = Parser::new(toks, self.depth + 1);
        if self.depth < MAX_NEST {
            sub.run(None);
        }
        let (blocks, _) = sub.finish();
        let text: Vec<String> = blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.text()) } else { None }).collect();
        squash(&text.join(" "))
    }

    /// Parse `toks` inline into the current paragraph with formatting `f` applied.
    fn inline_with(&mut self, toks: &'a [Tok], f: impl FnOnce(&mut Fmt)) {
        if self.depth >= MAX_NEST {
            let s = raw_text(toks);
            self.text(&s);
            return;
        }
        let saved_fmt = self.fmt.clone();
        f(&mut self.fmt);
        let (saved_t, saved_pos) = (self.t, self.pos);
        self.t = toks;
        self.pos = 0;
        self.depth += 1;
        self.run(None);
        self.flush_pending();
        self.depth -= 1;
        self.t = saved_t;
        self.pos = saved_pos;
        self.fmt = saved_fmt;
    }

    /// Skip every `[…]` and `{…}` argument that follows (and, for definitions, the macro name).
    fn drop_args(&mut self, name: &str) {
        if matches!(name, "newcommand" | "renewcommand" | "providecommand" | "newcommand*" | "renewcommand*" | "providecommand*" | "def" | "let") {
            self.skip_spaces();
            if matches!(self.t.get(self.pos), Some(Tok::Cmd(_))) {
                self.pos += 1;
            }
            if name == "let" {
                self.skip_spaces();
                if matches!(self.t.get(self.pos), Some(Tok::Cmd(_))) {
                    self.pos += 1;
                }
                return;
            }
            if name == "def" {
                // `\def\x#1{…}`: the parameter text, then the body.
                while matches!(self.t.get(self.pos), Some(Tok::Text(_))) {
                    self.pos += 1;
                }
            }
        }
        loop {
            let save = self.pos;
            self.skip_spaces();
            match self.t.get(self.pos) {
                Some(Tok::Open) => {
                    let _ = self.arg_tokens();
                }
                Some(Tok::Text(s)) if s.starts_with('[') => {
                    let _ = self.opt_arg();
                    if self.pending.is_some() {
                        return;
                    }
                }
                _ => {
                    self.pos = save;
                    return;
                }
            }
        }
    }

    fn heading(&mut self, level: u8) {
        let _ = self.opt_arg();
        let toks = self.arg_tokens();
        self.flush();
        self.para = Para::new(Kind::Heading(level.clamp(1, 6)));
        self.inline_with(toks, |_| {});
        self.flush();
    }

    /// Parse until `\end{env}` (when given), a closing brace at this level, or the end.
    fn flush_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            self.text(&ligatures(&p));
        }
    }

    fn run(&mut self, env: Option<&str>) {
        loop {
            self.flush_pending();
            let Some(tok) = self.t.get(self.pos) else { break };
            self.pos += 1;
            match tok {
                Tok::Text(s) => {
                    let s = ligatures(s);
                    self.text(&s);
                }
                Tok::Par => self.flush(),
                Tok::Tie => self.text("\u{A0}"),
                Tok::Amp => self.text(" "),
                Tok::Open => {
                    // A group scopes declarations (`{\bfseries …}`).
                    let toks = {
                        self.pos -= 1;
                        self.arg_tokens()
                    };
                    self.inline_with(toks, |_| {});
                }
                Tok::Close => {}
                Tok::Math(m, display) => {
                    let linear = latex_to_linear(m);
                    if linear.is_empty() {
                        continue;
                    }
                    if *display {
                        self.flush();
                        self.para.inlines.push(Inline::Equation { linear, display: true });
                        self.flush();
                    } else {
                        self.para.inlines.push(Inline::Equation { linear, display: false });
                    }
                }
                Tok::Verbatim(v) => {
                    self.flush();
                    let v = v.strip_prefix('\n').unwrap_or(v);
                    let v = v.strip_suffix('\n').unwrap_or(v);
                    for line in v.lines() {
                        let mut p = Para::new(Kind::Code);
                        p.push_text(line.trim_end_matches('\r'), &Fmt::default());
                        if line.trim().is_empty() {
                            p.inlines.push(Inline::Text(String::new(), Fmt::default()));
                        }
                        self.blocks.push(FBlock::Para(p));
                    }
                }
                Tok::Verb(v) => {
                    let mut f = self.fmt();
                    f.code = true;
                    self.para.push_text(v, &f);
                }
                // Too deep: the environment's content goes on at this level.
                Tok::Begin(_) if self.depth >= MAX_NEST => {}
                Tok::Begin(name) => self.environment(name),
                Tok::End(name) => {
                    if env == Some(name.as_str()) {
                        return;
                    }
                    // A stray `\end`: ignore.
                }
                Tok::Cmd(name) => self.command(name),
            }
        }
    }

    fn command(&mut self, name: &str) {
        if let Some(sym) = text_symbol(name) {
            if name == "item" {
                self.item();
            } else if matches!(name, "centering" | "raggedleft" | "raggedright") {
                let a = match name {
                    "centering" => Align::Center,
                    "raggedleft" => Align::Right,
                    _ => Align::Left,
                };
                self.ctx.align = Some(a);
            } else {
                self.text(sym);
            }
            return;
        }
        if let Some(mark) = accent_mark(name) {
            // `\"o` takes one letter of the text after it; `\"{o}` the group.
            self.skip_spaces();
            let arg = match self.t.get(self.pos) {
                Some(Tok::Text(t)) => {
                    self.pos += 1;
                    let mut chars = t.chars();
                    let first = chars.next().map(String::from).unwrap_or_default();
                    if !chars.as_str().is_empty() {
                        self.pending = Some(chars.as_str().to_string());
                    }
                    first
                }
                _ => self.arg_raw(),
            };
            let mut chars = arg.chars();
            let s = match chars.next() {
                Some(b) => format!("{}{}", compose(b, mark), chars.as_str()),
                None => mark.to_string(),
            };
            self.text(&s);
            return;
        }
        if let Some(size) = size_of(name) {
            self.fmt_mut().size = (size > 0.0).then_some(size);
            return;
        }
        match name {
            "\\" | "newline" | "linebreak" | "break" => {
                let _ = self.opt_arg();
                self.text("\n");
            }
            "part" | "part*" => self.heading(1),
            "chapter" | "chapter*" => self.heading(self.top.max(1)),
            "section" | "section*" => self.heading(1 + self.top),
            "subsection" | "subsection*" => self.heading(2 + self.top),
            "subsubsection" | "subsubsection*" => self.heading(3 + self.top),
            "paragraph" | "paragraph*" => self.heading(4 + self.top),
            "subparagraph" | "subparagraph*" => self.heading(5 + self.top),
            "title" => self.meta.title = self.arg_text(),
            "author" => self.meta.author = self.arg_text(),
            "maketitle" => {
                self.flush();
                if !self.meta.title.is_empty() {
                    let mut p = Para::new(Kind::Title);
                    p.push_text(&self.meta.title, &Fmt::default());
                    self.blocks.push(FBlock::Para(p));
                }
                if !self.meta.author.is_empty() {
                    let mut p = Para { align: Some(Align::Center), ..Default::default() };
                    p.push_text(&self.meta.author, &Fmt::default());
                    self.blocks.push(FBlock::Para(p));
                }
            }
            "newpage" | "clearpage" | "cleardoublepage" | "pagebreak" | "eject" => {
                let _ = self.opt_arg();
                self.flush();
                self.page_break = true;
            }
            "par" => self.flush(),
            "hrule" | "hrulefill" | "rule" => {
                let _ = self.opt_arg();
                let full = if name == "rule" {
                    let w = self.arg_source();
                    let _ = self.arg_raw();
                    w.contains("linewidth") || w.contains("textwidth") || w.contains("columnwidth")
                } else {
                    true
                };
                if full {
                    self.flush();
                    self.blocks.push(FBlock::Para(Para::new(Kind::Rule)));
                }
            }
            "textbf" => self.with_arg(|f| f.bold = true),
            "textit" | "textsl" => self.with_arg(|f| f.italic = true),
            "emph" => self.with_arg(|f| f.italic = !f.italic),
            "underline" | "uline" | "ul" | "uuline" => self.with_arg(|f| f.underline = true),
            "sout" | "st" | "xout" | "cancel" => self.with_arg(|f| f.strike = true),
            "texttt" | "code" | "path" => self.with_arg(|f| f.code = true),
            "textsuperscript" => self.with_arg(|f| f.sup = true),
            "textsubscript" => self.with_arg(|f| f.sub = true),
            "textmd" | "textup" | "textrm" | "textsf" | "textsc" | "textnormal" | "mbox" | "makebox" | "fbox" | "framebox" | "hbox" | "text"
            | "textls" | "foreignlanguage" | "enquote" | "MakeUppercase" | "MakeLowercase" | "uppercase" | "lowercase" => {
                if name == "foreignlanguage" {
                    let _ = self.arg_raw();
                }
                if matches!(name, "makebox" | "framebox") {
                    let _ = self.opt_arg();
                    let _ = self.opt_arg();
                }
                let quote = name == "enquote";
                if quote {
                    self.text("“");
                }
                self.with_arg(|f| {
                    if name == "textmd" {
                        f.bold = false;
                    }
                    if name == "textup" {
                        f.italic = false;
                    }
                    if name == "textnormal" {
                        *f = Fmt { link: f.link.clone(), ..Default::default() };
                    }
                });
                if quote {
                    self.text("”");
                }
            }
            "bfseries" | "bf" => self.fmt_mut().bold = true,
            "itshape" | "it" | "slshape" | "sl" | "em" => self.fmt_mut().italic = true,
            "ttfamily" | "tt" => self.fmt_mut().code = true,
            "mdseries" => self.fmt_mut().bold = false,
            "upshape" => self.fmt_mut().italic = false,
            "rmfamily" | "sffamily" | "scshape" | "sc" | "rm" | "sf" | "normalfont*" => {}
            "fontsize" => {
                let s = self.arg_source();
                let _ = self.arg_raw();
                self.fmt_mut().size = length_pt(&s).or_else(|| s.trim().parse().ok()).filter(|v: &f32| *v >= 1.0 && *v <= 1638.0);
            }
            "textcolor" | "color" | "colorbox" | "highlight" | "hl" | "cellcolor" => {
                let model = self.opt_arg();
                let spec = if matches!(name, "hl" | "highlight") { "yellow".to_string() } else { self.arg_raw() };
                let c = parse_color(model.as_deref(), &spec);
                match name {
                    "color" => self.fmt_mut().color = c,
                    "cellcolor" => {}
                    "textcolor" => self.with_arg(move |f| f.color = c.or(f.color)),
                    _ => self.with_arg(move |f| f.background = c.or(f.background)),
                }
            }
            "href" => {
                let url = self.arg_raw();
                self.with_arg(move |f| f.link = Some(url));
            }
            "url" | "nolinkurl" => {
                let url = self.arg_raw();
                let mut f = self.fmt();
                if name == "url" {
                    f.link = Some(url.clone());
                }
                self.para.push_text(&url, &f);
            }
            "hyperlink" => {
                let target = self.arg_raw();
                self.with_arg(move |f| f.link = Some(format!("#{target}")));
            }
            "hypertarget" => {
                let target = self.arg_raw();
                self.anchor(target);
                self.with_arg(|_| {});
            }
            "label" => {
                let l = self.arg_raw();
                self.anchor(l);
            }
            "ref" | "eqref" | "pageref" | "autoref" | "cref" | "Cref" | "nameref" => {
                let l = self.arg_raw();
                let mut f = self.fmt();
                f.link = Some(format!("#{l}"));
                self.para.push_text(&l, &f);
            }
            "cite" | "citep" | "citet" | "parencite" | "textcite" | "autocite" => {
                let _ = self.opt_arg();
                let _ = self.opt_arg();
                let keys = self.arg_raw();
                let keys: Vec<&str> = keys.split(',').map(str::trim).filter(|k| !k.is_empty()).collect();
                self.text(&format!("[{}]", keys.join(", ")));
            }
            "footnote" | "footnotetext" | "marginpar" => {
                let _ = self.opt_arg();
                self.text(" (");
                self.with_arg(|_| {});
                self.text(")");
            }
            "includegraphics" => {
                let _ = self.opt_arg();
                let file = self.arg_raw();
                let mut f = self.fmt();
                f.italic = true;
                self.para.push_text(&format!("[Picture: {}]", file.trim()), &f);
            }
            "caption" | "caption*" => {
                let _ = self.opt_arg();
                let toks = self.arg_tokens();
                self.flush();
                self.para.align = Some(Align::Center);
                self.inline_with(toks, |f| f.italic = true);
                self.flush();
            }
            "bibitem" => {
                let label = self.opt_arg();
                let key = self.arg_raw();
                self.flush();
                self.text(&format!("[{}] ", label.unwrap_or(key)));
            }
            "multicolumn" | "multirow" => {
                // Outside a tabular: keep the content.
                let _ = self.arg_raw();
                let _ = self.opt_arg();
                let _ = self.arg_raw();
                self.with_arg(|_| {});
            }
            "begin" | "end" => {}
            n if DROP_WITH_ARGS.contains(&n) => self.drop_args(n),
            _ => {
                // Unknown: drop the command, keep its arguments' text.
                let _ = self.opt_arg();
            }
        }
    }

    fn with_arg(&mut self, f: impl FnOnce(&mut Fmt)) {
        let toks = self.arg_tokens();
        self.inline_with(toks, f);
    }

    fn item(&mut self) {
        self.flush();
        let level = self.lists.len().saturating_sub(1).min(8) as u8;
        self.para = Para::default();
        if let Some(&ordered) = self.lists.last() {
            self.para.list = Some(ListInfo { ordered, level });
        }
        let label = self.opt_arg();
        if let Some(l) = label {
            let mut f = self.fmt();
            f.bold = true;
            self.para.push_text(&format!("{} ", l.trim()), &f);
        }
    }

    fn environment(&mut self, name: &str) {
        self.depth += 1;
        match name {
            "document" => {
                self.run(Some(name));
            }
            "itemize" | "enumerate" | "description" | "list" | "compactitem" | "compactenum" | "inparaenum" => {
                let _ = self.opt_arg();
                if name == "list" {
                    let _ = self.arg_raw();
                    let _ = self.arg_raw();
                }
                self.flush();
                self.lists.push(matches!(name, "enumerate" | "compactenum" | "inparaenum"));
                self.run(Some(name));
                self.flush();
                self.lists.pop();
            }
            "quote" | "quotation" | "verse" | "abstract" => {
                self.flush();
                if name == "abstract" {
                    let mut p = Para { align: Some(Align::Center), ..Default::default() };
                    p.push_text("Abstract", &Fmt { bold: true, ..Default::default() });
                    self.blocks.push(FBlock::Para(p));
                }
                let saved = self.ctx;
                self.ctx.kind = Kind::Quote;
                self.run(Some(name));
                self.flush();
                self.ctx = saved;
            }
            "center" | "flushleft" | "flushright" | "centering" => {
                self.flush();
                let align = match name {
                    "flushleft" => Align::Left,
                    "flushright" => Align::Right,
                    _ => Align::Center,
                };
                let saved = self.ctx;
                self.ctx.align = Some(align);
                self.run(Some(name));
                self.flush();
                self.ctx = saved;
            }
            "tabular" | "tabular*" | "tabularx" | "tabulary" | "longtable" | "longtable*" | "array" | "tblr" | "longtblr" => {
                let _ = self.opt_arg();
                if matches!(name, "tabular*" | "tabularx" | "tabulary") {
                    let _ = self.arg_raw();
                }
                // A real column spec is short; capping it bounds the work on hostile input.
                let spec: String = self.arg_source().chars().take(MAX_SPEC).collect();
                let toks = self.until_end(name);
                self.flush();
                let t = table(toks, &spec, self.depth);
                if !t.rows.is_empty() {
                    self.blocks.push(FBlock::Table(t));
                }
            }
            "thebibliography" => {
                let _ = self.arg_raw();
                self.flush();
                let mut p = Para::new(Kind::Heading(1 + self.top));
                p.push_text("References", &Fmt::default());
                self.blocks.push(FBlock::Para(p));
                self.run(Some(name));
                self.flush();
            }
            "minipage" | "figure" | "figure*" | "table" | "table*" | "wrapfigure" | "subfigure" | "titlepage" | "multicols" | "multicols*"
            | "small" | "footnotesize" | "large" | "theorem" | "lemma" | "proof" | "definition" | "example" | "remark" | "corollary"
            | "proposition" => {
                // Floats and boxes: their content, in order.
                let _ = self.opt_arg();
                if matches!(name, "minipage" | "subfigure" | "multicols" | "multicols*") {
                    let _ = self.arg_raw();
                }
                if name == "wrapfigure" {
                    let _ = self.arg_raw();
                    let _ = self.arg_raw();
                }
                self.flush();
                if matches!(name, "theorem" | "lemma" | "proof" | "definition" | "example" | "remark" | "corollary" | "proposition") {
                    let mut label: String = name.chars().take(1).flat_map(char::to_uppercase).chain(name.chars().skip(1)).collect();
                    label.push_str(". ");
                    self.para.push_text(&label, &Fmt { bold: name != "proof", italic: name == "proof", ..Default::default() });
                }
                let (saved_fmt, saved_ctx) = (self.fmt.clone(), self.ctx);
                if let Some(s) = size_of(name) {
                    self.fmt.size = (s > 0.0).then_some(s);
                }
                self.run(Some(name));
                self.flush();
                self.fmt = saved_fmt;
                self.ctx = saved_ctx;
            }
            _ => {
                // Unknown environment: its content.
                let _ = self.opt_arg();
                let (saved_fmt, saved_ctx) = (self.fmt.clone(), self.ctx);
                self.run(Some(name));
                self.fmt = saved_fmt;
                self.ctx = saved_ctx;
            }
        }
        self.depth -= 1;
    }

    /// The tokens up to the matching `\end{name}` (consumed).
    fn until_end(&mut self, name: &str) -> &'a [Tok] {
        let start = self.pos;
        let mut depth = 0usize;
        while let Some(tok) = self.t.get(self.pos) {
            match tok {
                Tok::Begin(n) if n == name => depth += 1,
                Tok::End(n) if n == name => {
                    if depth == 0 {
                        let toks = self.t.get(start..self.pos).unwrap_or_default();
                        self.pos += 1;
                        return toks;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.pos += 1;
        }
        self.t.get(start..).unwrap_or_default()
    }

    fn finish(mut self) -> (Vec<FBlock>, model::Meta) {
        self.flush_pending();
        self.flush();
        (self.blocks, self.meta)
    }
}

/// Plain text of tokens (arguments that are names, URLs, lengths).
fn raw_text(toks: &[Tok]) -> String {
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Text(x) => s.push_str(x),
            Tok::Tie => s.push(' '),
            Tok::Cmd(c) => match text_symbol(c) {
                Some(sym) => s.push_str(sym),
                None if c == "\\" => s.push(' '),
                None => {
                    s.push('\\');
                    s.push_str(c);
                }
            },
            Tok::Verb(v) | Tok::Math(v, _) => s.push_str(v),
            _ => {}
        }
    }
    s.trim().to_string()
}

/// Tokens back as LaTeX source (comments and layout whitespace normalised).
fn source_text(toks: &[Tok]) -> String {
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Text(x) => s.push_str(x),
            Tok::Open => s.push('{'),
            Tok::Close => s.push('}'),
            Tok::Tie => s.push('~'),
            Tok::Amp => s.push('&'),
            Tok::Par => s.push(' '),
            Tok::Cmd(c) => {
                s.push('\\');
                s.push_str(c);
                if c.chars().all(|c| c.is_ascii_alphabetic()) {
                    s.push(' ');
                }
            }
            Tok::Math(m, _) => {
                s.push('$');
                s.push_str(m);
                s.push('$');
            }
            Tok::Verb(v) | Tok::Verbatim(v) => s.push_str(v),
            Tok::Begin(n) => s.push_str(&format!("\\begin{{{n}}}")),
            Tok::End(n) => s.push_str(&format!("\\end{{{n}}}")),
        }
    }
    s.trim().to_string()
}

/// A `tabular` body as a table.
fn table(toks: &[Tok], spec: &str, depth: usize) -> FTable {
    let (spec_cols, widths) = spec_widths(spec);
    // Split rows at `\\` and cells at `&`, outside braces and nested environments.
    let mut rows: Vec<Vec<&[Tok]>> = Vec::new();
    let mut row: Vec<&[Tok]> = Vec::new();
    let mut start = 0usize;
    let mut brace = 0usize;
    let mut env = 0usize;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Open => brace += 1,
            Tok::Close => brace = brace.saturating_sub(1),
            Tok::Begin(_) => env += 1,
            Tok::End(_) => env = env.saturating_sub(1),
            Tok::Amp if brace == 0 && env == 0 => {
                row.push(toks.get(start..i).unwrap_or_default());
                start = i + 1;
            }
            Tok::Cmd(c) if brace == 0 && env == 0 && (c == "\\" || c == "tabularnewline") => {
                row.push(toks.get(start..i).unwrap_or_default());
                rows.push(std::mem::take(&mut row));
                start = i + 1;
            }
            _ => {}
        }
        if rows.len() >= wordcraft_doc::table::MAX_ROWS {
            break;
        }
    }
    if start < toks.len() {
        row.push(toks.get(start..).unwrap_or_default());
    }
    if !row.is_empty() {
        rows.push(row);
    }
    let mut out = FTable { rows: Vec::new(), widths: Vec::new(), borderless: false };
    // Rows still covered by a `\multirow` above, per grid column.
    let mut covering: Vec<u32> = Vec::new();
    for r in rows {
        let mut cells = Vec::new();
        let mut g = 0usize;
        for cell_toks in r {
            let mut cell = Cell::default();
            let mut content = trim_cell(cell_toks);
            // `\multicolumn{n}{spec}{content}`, `\multirow{n}{width}{content}`.
            let mut sub = Parser::new(content, depth + 1);
            loop {
                sub.skip_spaces();
                match sub.t.get(sub.pos) {
                    Some(Tok::Cmd(c)) if c == "multicolumn" => {
                        sub.pos += 1;
                        cell.colspan = sub.arg_raw().trim().parse::<u32>().unwrap_or(1).clamp(1, 63);
                        let _ = sub.arg_raw();
                        content = sub.arg_tokens();
                        sub = Parser::new(content, depth + 1);
                    }
                    Some(Tok::Cmd(c)) if c == "multirow" || c == "multirow*" => {
                        sub.pos += 1;
                        let _ = sub.opt_arg();
                        cell.rowspan = sub.arg_raw().trim().parse::<i64>().map(|n| n.unsigned_abs().clamp(1, 1000) as u32).unwrap_or(1);
                        let _ = sub.opt_arg();
                        let _ = sub.arg_raw();
                        let _ = sub.opt_arg();
                        content = sub.arg_tokens();
                        sub = Parser::new(content, depth + 1);
                    }
                    Some(Tok::Cmd(c)) if c == "cellcolor" => {
                        sub.pos += 1;
                        let model = sub.opt_arg();
                        let spec = sub.arg_raw();
                        cell.shading = parse_color(model.as_deref(), &spec);
                    }
                    _ => break,
                }
            }
            if covering.get(g).copied().unwrap_or(0) > 0 {
                cell.covered = true;
            }
            if !cell.covered && depth < MAX_NEST {
                sub.run(None);
                let (blocks, _) = sub.finish();
                cell.blocks = blocks;
            }
            let span = cell.colspan as usize;
            if covering.len() < g + span {
                covering.resize(g + span, 0);
            }
            for k in g..g + span {
                if let Some(c) = covering.get_mut(k)
                    && cell.rowspan > 1
                    && !cell.covered
                {
                    *c = cell.rowspan;
                }
            }
            g += span;
            cells.push(cell);
        }
        for c in covering.iter_mut() {
            *c = c.saturating_sub(1);
        }
        // `\hline` alone after the last `\\` is not a row.
        let empty = cells.iter().all(|c| !c.covered && c.blocks.is_empty());
        if empty && cells.len() <= 1 {
            continue;
        }
        out.rows.push(cells);
    }
    let cols = out.cols();
    if !widths.is_empty() && widths.len() == cols && spec_cols == cols {
        out.widths = widths;
    }
    out
}

/// A cell's tokens without the rules (`\hline`…) and spaces around them.
fn trim_cell(toks: &[Tok]) -> &[Tok] {
    let skip = |t: &Tok| match t {
        Tok::Cmd(c) => {
            matches!(c.as_str(), "hline" | "toprule" | "midrule" | "bottomrule" | "endhead" | "endfirsthead" | "endfoot" | "endlastfoot" | "noalign")
        }
        Tok::Text(s) => s.trim().is_empty(),
        _ => false,
    };
    let mut a = 0;
    while let Some(t) = toks.get(a) {
        if skip(t) {
            a += 1;
        } else if matches!(t, Tok::Cmd(c) if c == "cline" || c == "cmidrule" || c == "addlinespace") {
            // `\cline{2-3}`, `\cmidrule(lr){1-2}`: the command and its argument.
            a += 1;
            while let Some(Tok::Text(s)) = toks.get(a) {
                if s.trim().is_empty() || s.trim_start().starts_with('(') || s.trim_start().starts_with('[') {
                    a += 1;
                } else {
                    break;
                }
            }
            if toks.get(a) == Some(&Tok::Open) {
                while let Some(t) = toks.get(a) {
                    a += 1;
                    if *t == Tok::Close {
                        break;
                    }
                }
            }
        } else {
            break;
        }
    }
    let mut b = toks.len();
    while b > a && toks.get(b - 1).is_some_and(skip) {
        b -= 1;
    }
    toks.get(a..b).unwrap_or_default()
}

/// Parse LaTeX source into a flow.
pub fn parse(src: &str) -> Flow {
    let toks = tokenize(src);
    // The preamble only contributes metadata; without `\begin{document}` it is all body.
    let body_at = toks.iter().position(|t| matches!(t, Tok::Begin(n) if n == "document"));
    let mut meta = model::Meta::default();
    let (body, preamble): (&[Tok], &[Tok]) = match body_at {
        Some(i) => (toks.get(i + 1..).unwrap_or_default(), toks.get(..i).unwrap_or_default()),
        None => (&toks[..], &[]),
    };
    let mut pre = Parser::new(preamble, 0);
    while let Some(t) = pre.t.get(pre.pos) {
        pre.pos += 1;
        if let Tok::Cmd(c) = t {
            match c.as_str() {
                "title" => meta.title = pre.arg_text(),
                "author" => meta.author = pre.arg_text(),
                _ => {}
            }
        }
    }
    let mut p = Parser::new(body, 0);
    p.meta = meta;
    // `\part` and `\chapter` put `\section` one level down each.
    let uses = |names: [&str; 2]| body.iter().any(|t| matches!(t, Tok::Cmd(c) if names.contains(&c.as_str())));
    p.top = u8::from(uses(["part", "part*"])) + u8::from(uses(["chapter", "chapter*"]));
    p.run(Some("document"));
    let (blocks, meta) = p.finish();
    Flow { blocks, meta }
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FBlock;

    fn paras(src: &str) -> Vec<Para> {
        parse(src).blocks.into_iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
    }

    fn texts(src: &str) -> Vec<String> {
        paras(src).iter().map(|p| p.text()).collect()
    }

    #[test]
    fn unclosed_begin_keeps_its_brace() {
        // `env_name` used to consume the `{` before giving up on a missing `}`.
        let toks = tokenize("\\begin{ x");
        assert_eq!(toks.first(), Some(&Tok::Cmd("begin".into())), "{toks:?}");
        assert!(toks.contains(&Tok::Open), "{toks:?}");
        let toks = tokenize(&format!("\\end{{{}}}", "n".repeat(100)));
        assert!(toks.contains(&Tok::Open), "{toks:?}");
    }

    #[test]
    fn spec_widths_repeats_are_bounded() {
        assert_eq!(spec_widths("*{3}{p{1cm}}").0, 3);
        assert_eq!(spec_widths("*{2}{*{2}{l}}").0, 4);
        let deep = format!("{}l{}", "*{1}{".repeat(MAX_NEST + 5), "}".repeat(MAX_NEST + 5));
        let (cols, widths) = spec_widths(&deep);
        assert!(cols <= wordcraft_doc::table::MAX_COLS && widths.is_empty());
        let wide = format!("{}p{{1cm}}{}", "*{64}{".repeat(30), "}".repeat(30));
        let (cols, widths) = spec_widths(&wide);
        assert_eq!(cols, wordcraft_doc::table::MAX_COLS);
        assert!(widths.len() <= wordcraft_doc::table::MAX_COLS);
    }

    const ARTICLE: &str = r#"\documentclass[11pt]{article}
\usepackage[utf8]{inputenc}
\usepackage{amsmath} % maths
\newcommand{\R}{\mathbb{R}}
\title{On \emph{Sample} Documents}
\author{Ada Lovelace \and Charles Babbage}
\date{\today}
\begin{document}
\maketitle

\section{Introduction}\label{sec:intro}
This is \textbf{bold}, \textit{italic}, \underline{underlined} and \texttt{code}.
TeX quotes ``like this'' and dashes -- or --- with 50\% and \$5 \& more~text.

A second paragraph with a footnote\footnote{The note.} and a
\href{https://example.com}{link}. % a comment
See Section~\ref{sec:intro}.

\subsection*{Lists}
\begin{itemize}
  \item First
  \item Second
  \begin{enumerate}
    \item Nested one
    \item[b)] Nested two
  \end{enumerate}
\end{itemize}

\begin{quote}
Quoted text.
\end{quote}

\begin{center}
Centered.
\end{center}

\begin{verbatim}
fn main() {
    println!("{}", 1 % 2);
}
\end{verbatim}

\begin{table}[h]
\centering
\begin{tabular}{|p{2cm}|l|}
\hline
\textbf{Name} & Value \\
\hline
\multicolumn{2}{|c|}{Wide} \\
alpha & $x^2$ \\
\hline
\end{tabular}
\caption{A table.}
\end{table}

Energy: $E = mc^2$ and
\[ \frac{a+b}{2} \leq \sqrt{\alpha} \]
\begin{equation}
  \sum_{i=1}^{n} i = \frac{n(n+1)}{2}
\end{equation}

\newpage
Caf\'e na\"ive \c{c}a \v{s}\'{e} and {\bfseries grouped bold} after.
{\color{red} red text} \textcolor[HTML]{00FF00}{green}.
\end{document}
"#;

    #[test]
    fn article_structure() {
        let flow = parse(ARTICLE);
        assert_eq!(flow.meta.title, "On Sample Documents");
        assert_eq!(flow.meta.author, "Ada Lovelace Charles Babbage");
        let ps = paras(ARTICLE);
        let find = |t: &str| ps.iter().find(|p| p.text().trim() == t).unwrap_or_else(|| panic!("no {t:?} in {:?}", texts(ARTICLE)));
        assert_eq!(find("On Sample Documents").kind, Kind::Title);
        assert_eq!(find("Introduction").kind, Kind::Heading(1));
        assert_eq!(find("Lists").kind, Kind::Heading(2));
        assert_eq!(find("First").list, Some(ListInfo { ordered: false, level: 0 }));
        assert_eq!(find("Nested one").list, Some(ListInfo { ordered: true, level: 1 }));
        assert!(ps.iter().any(|p| p.text().trim() == "b) Nested two" && p.list.is_some()), "{:?}", texts(ARTICLE));
        assert_eq!(find("Quoted text.").kind, Kind::Quote);
        assert_eq!(find("Centered.").align, Some(Align::Center));
        assert_eq!(find("fn main() {").kind, Kind::Code);
        assert_eq!(find("println!(\"{}\", 1 % 2);").kind, Kind::Code, "verbatim keeps % and indentation");
        assert!(ps.iter().any(|p| p.text() == "    println!(\"{}\", 1 % 2);"));
        assert_eq!(find("A table.").align, Some(Align::Center));
        let body = ps.iter().find(|p| p.text().starts_with("This is")).unwrap();
        let has = |t: &str, f: &dyn Fn(&Fmt) -> bool| body.inlines.iter().any(|i| matches!(i, Inline::Text(x, fm) if x == t && f(fm)));
        assert!(has("bold", &|f| f.bold));
        assert!(has("italic", &|f| f.italic));
        assert!(has("underlined", &|f| f.underline));
        assert!(has("code", &|f| f.code));
        assert!(body.text().contains("“like this” and dashes – or — with 50% and $5 & more\u{A0}text."), "{}", body.text());
        let second = ps.iter().find(|p| p.text().starts_with("A second")).unwrap();
        assert!(second.text().contains("footnote (The note.) and a link."), "{}", second.text());
        assert!(second.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "link" && f.link.as_deref() == Some("https://example.com"))));
        assert!(!second.text().contains("comment"));
        let accents = ps.iter().find(|p| p.text().starts_with("Café")).unwrap();
        assert!(accents.page_break, "\\newpage");
        assert!(accents.text().starts_with("Café naïve ça šé and grouped bold after."), "{}", accents.text());
        assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "grouped bold" && f.bold)));
        assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t.starts_with(" after.") && !f.bold)), "{:?}", accents.inlines);
        assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t.contains("red text") && f.color == Some(Rgb(255, 0, 0)))));
        assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "green" && f.color == Some(Rgb(0, 255, 0)))));
        // Preamble definitions and layout commands leave no text.
        let all = texts(ARTICLE).join("\n");
        for gone in ["mathbb", "utf8", "11pt", "today", "sec:intro}", "amsmath", "maths"] {
            assert!(!all.contains(gone), "{gone:?} leaked: {all}");
        }
    }

    #[test]
    fn tables_and_math() {
        let flow = parse(ARTICLE);
        let t = flow.blocks.iter().find_map(|b| if let FBlock::Table(t) = b { Some(t) } else { None }).expect("table");
        assert_eq!(t.rows.len(), 3, "{t:?}");
        assert_eq!(t.rows[0].iter().map(Cell::text).collect::<Vec<_>>(), ["Name", "Value"]);
        assert_eq!(t.rows[1].len(), 1);
        assert_eq!(t.rows[1][0].colspan, 2);
        assert_eq!(t.rows[1][0].text(), "Wide");
        assert!(t.widths.is_empty(), "the l column has no width");
        let eq = |p: &Para| {
            p.inlines.iter().find_map(|i| if let Inline::Equation { linear, display } = i { Some((linear.clone(), *display)) } else { None })
        };
        let ps = paras(ARTICLE);
        let energy = ps.iter().find(|p| p.text().starts_with("Energy")).unwrap();
        assert_eq!(eq(energy), Some(("E = mc^2".to_string(), false)));
        let displays: Vec<_> = ps.iter().filter_map(eq).filter(|(_, d)| *d).map(|(l, _)| l).collect();
        assert_eq!(displays, ["(a+b)/2 ≤ √(α)", "∑_(i=1)^n i = (n(n+1))/2"]);
        let cell = &t.rows[2][1];
        let FBlock::Para(p) = &cell.blocks[0] else { panic!() };
        assert_eq!(eq(p), Some(("x^2".to_string(), false)));
    }

    #[test]
    fn multirow_cells_and_widths() {
        let src = r"\begin{tabular}{p{1in}p{2cm}}
\multirow{2}{*}{Tall} & a \\
 & b \\
\end{tabular}";
        let flow = parse(src);
        let FBlock::Table(t) = &flow.blocks[0] else { panic!("{:?}", flow.blocks) };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0][0].rowspan, 2);
        assert!(t.rows[1][0].covered);
        assert_eq!(t.rows[1][1].text(), "b");
        assert_eq!(t.widths.len(), 2);
        assert!((t.widths[0] - (72.27 + 12.0)).abs() < 0.1, "{:?}", t.widths);
    }

    #[test]
    fn fragments_without_a_document_environment() {
        assert_eq!(texts("Just \\textbf{text}.\n\nTwo."), ["Just text.", "Two."]);
        assert_eq!(texts(""), Vec::<String>::new());
        assert_eq!(texts("\\unknown{kept} \\unknownopt[dropped]{kept too}"), ["kept kept too"]);
        assert_eq!(texts("a\\\\b\\newline c"), ["a\nb\nc"]);
        assert_eq!(texts("\\verb|\\x{}| y"), ["\\x{} y"]);
        assert_eq!(texts("\\chapter{C}\\section{S}").len(), 2);
        let ps = paras("\\chapter{C}\\section{S}");
        assert_eq!((ps[0].kind, ps[1].kind), (Kind::Heading(1), Kind::Heading(2)));
    }

    #[test]
    fn math_conversions() {
        for (tex, linear) in [
            (r"\frac{1}{2}", "1/2"),
            (r"x_{i}^{2}", "x_i^2"),
            (r"x^{10}", "x^10"),
            (r"e^{i\pi}", "e^(iπ)"),
            (r"\sqrt[3]{x}", "√(3&x)"),
            (r"\left( a \right)", "( a )"),
            (r"\sin x + \text{if } y", "sin x + if y"),
            (r"a \cdot b \times c \neq d", "a ⋅ b × c ≠ d"),
            (r"\unknowncmd", r"\unknowncmd"),
        ] {
            assert_eq!(latex_to_linear(tex), linear, "{tex}");
        }
        for (linear, tex) in [
            ("x=(-b±√(b^2-4ac))/(2a)", r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}"),
            ("α ≤ β", r"\alpha \leq \beta"),
            ("αβ", r"\alpha\beta"),
            ("πr", r"\pi r"),
            ("a_(i+1)", r"a_{i+1}"),
            ("50% & #", r"50\% \& \#"),
            ("√(3&x)", r"\sqrt[3]{x}"),
        ] {
            assert_eq!(linear_to_latex(linear), tex, "{linear}");
        }
    }

    #[test]
    fn equations_survive_a_round_trip() {
        let mut d = Document::new();
        let mut p = wordcraft_doc::Paragraph::with_text("Area ", Default::default());
        let n = p.len();
        p.insert_object(
            n,
            wordcraft_doc::para::InlineObject::Equation { linear: "π r^2".into(), display: false, math: Default::default() },
            &Default::default(),
        )
        .unwrap();
        d.body = vec![wordcraft_doc::para_block(p)];
        let tex = export(&d);
        assert!(tex.contains("$\\pi r^2$") && tex.contains("\\usepackage{amsmath}"), "{tex}");
        let back = import(tex.as_bytes());
        let wordcraft_doc::Block::Para(bp) = &*back.body[0] else { panic!() };
        assert!(
            bp.objects.iter().any(|o| matches!(o, wordcraft_doc::para::InlineObject::Equation { linear, .. } if linear == "π r^2")),
            "{:?}",
            bp.objects
        );
        // Formats without equations keep the text.
        let md = crate::markdown::export(&d);
        assert!(md.contains("π r^2"), "{md}");
    }

    #[test]
    fn special_characters_round_trip() {
        let text = "50% of $5 & #1_a {b} ~c^ \\d <e> |f| -- ``g'' tab\there";
        let d = Document::from_text(text);
        let tex = export(&d);
        let back = import(tex.as_bytes());
        assert_eq!(back.plain_text(Default::default()).trim(), text.replace('\t', "\u{2003}"), "{tex}");
    }

    #[test]
    fn exported_tables_size_their_columns() {
        let mut t = FTable { rows: Vec::new(), widths: vec![100.0, 200.0], borderless: false };
        let cell = |s: &str| Cell {
            blocks: vec![FBlock::Para({
                let mut p = Para::default();
                p.push_text(s, &Fmt::default());
                p
            })],
            ..Default::default()
        };
        t.rows.push(vec![cell("a"), cell("b")]);
        t.rows.push(vec![Cell { colspan: 2, ..cell("wide") }]);
        let mut w = Writer::default();
        w.table(&t);
        assert!(w.out.contains("\\begin{tabular}{|p{88pt}|p{188pt}|}"), "{}", w.out);
        assert!(w.out.contains("\\multicolumn{2}{|p{288pt}|}{wide}"), "{}", w.out);
        let back = parse(&w.out);
        let FBlock::Table(bt) = &back.blocks[0] else { panic!() };
        assert_eq!(bt.widths, vec![100.0, 200.0]);
        assert_eq!(bt.rows[1][0].colspan, 2);
    }
}
