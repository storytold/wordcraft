use super::math::linear_to_latex;
use super::{Align, Cell, Document, FBlock, FTable, Fmt, Inline, Kind, Para, Rgb, model};

/// Text width of the exported article in points (`\linewidth` when reading widths).
pub(super) const LINE_WIDTH: f32 = 345.0;

/// LaTeX's `\tabcolsep` (both sides of a cell), points.
pub(super) const TABCOLSEP: f32 = 6.0;

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
pub(super) struct Writer {
    pub(super) out: String,
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
            // The environment ends at the first `\end{verbatim}`, so keep it from appearing. The
            // control characters go first: removing one afterwards could join a delimiter back up.
            let l: String = l.chars().filter(|c| *c == '\t' || !c.is_control()).collect();
            let l = l.replace("\\end{verbatim}", "\\end {verbatim}");
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
                    push_apart(&mut o, &s);
                }
                Inline::Image(img) => {
                    let alt = if img.alt.trim().is_empty() { "Picture".to_string() } else { format!("Picture: {}", img.alt.trim()) };
                    o.push_str(&format!("\\textit{{[{}]}}", escape_text(&alt)));
                }
                Inline::Anchor(name) => o.push_str(&format!("\\hypertarget{{{}}}{{}}", escape_url(name))),
                Inline::Figure(_) => {}
                // LaTeX keeps no ruby (no package): its base text, in the run's formatting.
                Inline::Ruby { base, fmt, .. } => {
                    let s = self.span(base, fmt);
                    push_apart(&mut o, &s);
                }
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
                // An internal bookmark reference, not a URL.
                Some(target) => format!("\\hyperlink{{{}}}{{{s}}}", escape_url(target)),
                // A script, `file:`, UNC or other non-web target becomes plain text: a `\href`
                // compiles to a PDF `/URI` action the reader would otherwise follow on click.
                None if wordcraft_doc::link::is_safe_href(l) => format!("\\href{{{}}}{{{s}}}", escape_url(l)),
                None => s,
            };
        }
        s
    }

    pub(super) fn table(&mut self, t: &FTable) {
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
        let spec: String = if t.borderless {
            widths.iter().map(|w| col(*w)).collect::<Vec<_>>().join(" ")
        } else {
            let s: String = widths.iter().map(|w| format!("{}|", col(*w))).collect();
            format!("|{s}")
        };
        let mut o = if t.borderless {
            format!("\\noindent\\begin{{tabular}}{{{spec}}}\n")
        } else {
            format!("\\noindent\\begin{{tabular}}{{{spec}}}\n\\hline\n")
        };
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
                    let col_spec = if t.borderless {
                        col(w)
                    } else {
                        let left = if g == 0 { "|" } else { "" };
                        format!("{left}{}|", col(w))
                    };
                    body = format!("\\multicolumn{{{span}}}{{{col_spec}}}{{{body}}}");
                }
                cells.push(body);
                g += span;
            }
            if cells.is_empty() {
                continue;
            }
            o.push_str(&cells.join(" & "));
            if t.borderless {
                o.push_str(" \\\\\n");
            } else {
                o.push_str(" \\\\\n\\hline\n");
            }
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

/// Appends a run, writing `{}` between a `-` or `'` ending `o` and the same character starting
/// `s`, so TeX does not join them across runs into a dash or quote.
fn push_apart(o: &mut String, s: &str) {
    if let Some(c) = s.chars().next()
        && (c == '-' || c == '\'')
        && o.ends_with(c)
    {
        o.push_str("{}");
    }
    o.push_str(s);
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
            // An opening bracket here would become an optional item or line-break argument.
            '[' if !line_has_text => o.push_str("{[}"),
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
