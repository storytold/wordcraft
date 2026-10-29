use super::lex::{Tok, accent_mark, compose, ligatures, text_symbol, tokenize};
use super::limits::{MAX_ANCHOR_RUN, MAX_NEST, MAX_SOURCE, MAX_SPEC};
use super::math::latex_to_linear;
use super::table::{raw_text, source_text, table};
use super::units::{length_pt, parse_color, size_of};
use super::{Align, Document, FBlock, Flow, Fmt, Inline, Kind, ListInfo, Para, model};

// =============================================================================================
// Import

/// Import a LaTeX source file. A source longer than [`MAX_SOURCE`] is an error: importing only
/// its start would lose the rest of the writing without a word.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    import_limited(bytes, MAX_SOURCE)
}

/// [`import`] with `max` bytes as the limit.
pub(super) fn import_limited(bytes: &[u8], max: usize) -> Result<Document, String> {
    if bytes.len() > max {
        let limit = if max >= 1 << 20 && max.is_multiple_of(1 << 20) { format!("{} MiB", max >> 20) } else { format!("{max} bytes") };
        return Err(format!("this LaTeX file is larger than {limit}, the most WordCraft imports; split it into smaller files"));
    }
    model::begin_import();
    let mut d = model::to_doc(&parse(&crate::txt::decode(bytes)));
    d.ensure_nonempty();
    Ok(d)
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

/// Paragraph-level context from the enclosing environments.
#[derive(Clone, Copy, Default)]
struct Ctx {
    kind: Kind,
    align: Option<Align>,
}

pub(super) struct Parser<'a> {
    pub(super) t: &'a [Tok],
    pub(super) pos: usize,
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
    pub(super) fn new(t: &'a [Tok], depth: usize) -> Parser<'a> {
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

    pub(super) fn skip_spaces(&mut self) {
        while let Some(Tok::Text(s)) = self.t.get(self.pos) {
            if !s.trim().is_empty() {
                break;
            }
            self.pos += 1;
        }
    }

    /// The tokens of an optional `[…]` argument, if one follows.
    pub(super) fn opt_arg(&mut self) -> Option<String> {
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
    pub(super) fn arg_tokens(&mut self) -> &'a [Tok] {
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
    pub(super) fn arg_raw(&mut self) -> String {
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

    pub(super) fn run(&mut self, env: Option<&str>) {
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
                    // A group scopes declarations (`{\bfseries …}`, `{\centering …\par}`):
                    // character formatting and paragraph alignment come back at its end.
                    let toks = {
                        self.pos -= 1;
                        self.arg_tokens()
                    };
                    let saved_ctx = self.ctx;
                    self.inline_with(toks, |_| {});
                    self.ctx = saved_ctx;
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
                // An accent over nothing prints the accent alone: the spacing form of the mark.
                None => match name {
                    "`" | "^" | "~" => name.to_string(),
                    "'" => "\u{B4}".to_string(),
                    "\"" => "\u{A8}".to_string(),
                    _ => mark.to_string(),
                },
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
                let url = model::kept_link(&self.arg_raw());
                self.with_arg(move |f| f.link = url);
            }
            "url" | "nolinkurl" => {
                let url = self.arg_raw();
                let mut f = self.fmt();
                if name == "url" {
                    f.link = model::kept_link(&url);
                }
                self.para.push_text(&url, &f);
            }
            "hyperlink" => {
                let target = self.arg_raw();
                let link = model::kept_link(&format!("#{target}"));
                self.with_arg(move |f| f.link = link);
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
                f.link = model::kept_link(&format!("#{l}"));
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
            self.para.list = Some(ListInfo { ordered, level, start: 1 });
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

    pub(super) fn finish(mut self) -> (Vec<FBlock>, model::Meta) {
        self.flush_pending();
        self.flush();
        (self.blocks, self.meta)
    }
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
