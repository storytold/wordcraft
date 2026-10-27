use super::css::{apply_style, css_len_pt};
use super::import::collapse_ws;
use super::*;

/// Open elements kept at most (deeper start tags are ignored).
const MAX_STACK: usize = 256;

// ---------------------------------------------------------------------------------------------
// Builder

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum ElKind {
    Inline,
    Block,
    Heading(u8),
    Quote,
    Pre,
    List(bool),
    Li,
    Table,
    Tr,
    Cell,
    Head,
}

#[derive(Clone, Debug)]
struct El {
    name: String,
    kind: ElKind,
    fmt: Fmt,
    align: Option<Align>,
    preserve: bool,
    page_break: bool,
    page_break_after: bool,
    /// Li: a paragraph of this item was already emitted.
    used: bool,
}

struct TableB {
    rows: Vec<Vec<Cell>>,
    row: Option<Vec<Cell>>,
    /// The table or one of its cells asks for borders.
    bordered: bool,
}

/// Whether an element's `border` attribute or `style` declares a visible border.
fn declares_border(attrs: &[(String, String)], is_table: bool) -> bool {
    if is_table && attr(attrs, "border").is_some_and(|b| b.trim().parse::<u32>().map(|n| n > 0).unwrap_or(b.trim().is_empty())) {
        return true;
    }
    let Some(style) = attr(attrs, "style") else { return false };
    style.split(';').any(|decl| {
        let Some((name, value)) = decl.split_once(':') else { return false };
        let name = name.trim().to_ascii_lowercase();
        if !matches!(name.as_str(), "border" | "border-top" | "border-right" | "border-bottom" | "border-left") {
            return false;
        }
        let value = value.trim().to_ascii_lowercase();
        !value.is_empty() && !value.split_whitespace().any(|tok| matches!(tok, "none" | "hidden" | "0" | "0px" | "0pt"))
    })
}

/// Loads the bytes of an image an `img` names by a source other than a `data:` URI (a path
/// relative to the HTML file). `None` when it can't be found. A picture used more than once can
/// share one buffer.
pub type ImageLoader<'a> = &'a dyn Fn(&str) -> Option<Arc<Vec<u8>>>;

pub(super) struct Builder<'r> {
    images: ImageLoader<'r>,
    containers: Vec<Vec<FBlock>>,
    cells: Vec<Cell>,
    tables: Vec<TableB>,
    stack: Vec<El>,
    para: Option<Para>,
    /// The current paragraph continues a list item (joins the previous paragraph).
    para_cont: bool,
    /// Pending page break from preceding elements with page-break-after or unconsumed break.
    pending_page_break: bool,
    /// Whether the current paragraph's first / last text was read with preserved white space
    /// (CSS `white-space: pre…`): those edge spaces are kept when the paragraph ends.
    pre_lead: Option<bool>,
    pre_tail: bool,
    meta: Meta,
    title: Option<String>,
}

fn is_void(n: &str) -> bool {
    matches!(
        n,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
            | "basefont"
            | "frame"
    )
}

pub(super) fn block_kind(n: &str) -> Option<ElKind> {
    Some(match n {
        "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav" | "aside" | "figure" | "figcaption" | "address" | "dl" | "dt"
        | "dd" | "center" | "form" | "fieldset" | "body" | "html" | "caption" | "thead" | "tbody" | "tfoot" | "details" | "summary" | "legend"
        | "hgroup" => ElKind::Block,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => ElKind::Heading(n.get(1..2).and_then(|d| d.parse().ok()).unwrap_or(1)),
        "blockquote" => ElKind::Quote,
        "pre" | "listing" | "xmp" | "plaintext" => ElKind::Pre,
        "ul" | "menu" | "dir" => ElKind::List(false),
        "ol" => ElKind::List(true),
        "li" => ElKind::Li,
        "table" => ElKind::Table,
        "tr" => ElKind::Tr,
        "td" | "th" => ElKind::Cell,
        "head" => ElKind::Head,
        _ => return None,
    })
}

/// Closes an open `p` (HTML's "close a p element" on block starts).
fn closes_p(n: &str) -> bool {
    matches!(
        n,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "div"
            | "dl"
            | "fieldset"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
            | "figure"
            | "center"
            | "dd"
            | "dt"
            | "li"
    )
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

impl<'r> Builder<'r> {
    pub(super) fn new(images: ImageLoader<'r>) -> Builder<'r> {
        Builder {
            images,
            containers: vec![Vec::new()],
            cells: Vec::new(),
            tables: Vec::new(),
            stack: Vec::new(),
            para: None,
            para_cont: false,
            pending_page_break: false,
            pre_lead: None,
            pre_tail: false,
            meta: Meta::default(),
            title: None,
        }
    }

    fn fmt(&self) -> Fmt {
        self.stack.last().map(|e| e.fmt.clone()).unwrap_or_default()
    }

    fn preserve(&self) -> bool {
        self.stack.last().is_some_and(|e| e.preserve)
    }

    pub(super) fn in_head(&self) -> bool {
        self.stack.iter().any(|e| e.kind == ElKind::Head)
    }

    /// Paragraph context from the open elements (innermost first, up to the enclosing cell).
    pub(super) fn start_para(&mut self) -> &mut Para {
        if self.para.is_none() {
            let mut p = Para::default();
            let mut list_level: Option<(bool, usize)> = None;
            let mut li_at: Option<usize> = None;
            let mut lists = 0usize;
            for (i, e) in self.stack.iter().enumerate().rev() {
                match e.kind {
                    ElKind::Cell => {
                        // CSS inherits `text-align` from the cell, its row, row group and table
                        // (#1381); the paragraph's own (found above) wins.
                        if p.align.is_none() {
                            p.align = self.stack.get(..=i).and_then(|outer| {
                                outer
                                    .iter()
                                    .rev()
                                    .take_while(|o| o.kind != ElKind::Table)
                                    .chain(outer.iter().rev().find(|o| o.kind == ElKind::Table))
                                    .find_map(|o| o.align)
                            });
                        }
                        break;
                    }
                    ElKind::Table => break,
                    ElKind::Heading(n) if p.kind == Kind::Normal => p.kind = Kind::Heading(n.clamp(1, 6)),
                    ElKind::Pre if p.kind == Kind::Normal => p.kind = Kind::Code,
                    ElKind::Quote if p.kind == Kind::Normal => p.kind = Kind::Quote,
                    ElKind::Li if li_at.is_none() => li_at = Some(i),
                    ElKind::List(o) => {
                        if list_level.is_none() {
                            list_level = Some((o, 0));
                        }
                        lists += 1;
                    }
                    _ => {}
                }
                if p.align.is_none() && e.align.is_some() && e.kind != ElKind::Inline {
                    p.align = e.align;
                }
            }
            if let (Some((o, _)), Some(li)) = (list_level, li_at) {
                p.list = Some(ListInfo { ordered: o, level: lists.saturating_sub(1).min(8) as u8, start: 1 });
                self.para_cont = self.stack.get(li).is_some_and(|e| e.used);
                if let Some(e) = self.stack.get_mut(li) {
                    e.used = true;
                }
            } else {
                self.para_cont = false;
            }
            let pb_before = self.stack.iter_mut().rev().take_while(|e| e.kind != ElKind::Cell).any(|e| std::mem::take(&mut e.page_break));
            p.page_break = pb_before || std::mem::take(&mut self.pending_page_break);
            self.para = Some(p);
        }
        self.para.get_or_insert_with(Para::default)
    }

    fn container(&mut self) -> &mut Vec<FBlock> {
        if self.containers.is_empty() {
            self.containers.push(Vec::new());
        }
        let n = self.containers.len() - 1;
        &mut self.containers[n]
    }

    fn flush(&mut self) {
        let Some(mut p) = self.para.take() else { return };
        let (pre_lead, pre_tail) = (self.pre_lead.take().unwrap_or(false), std::mem::take(&mut self.pre_tail));
        if p.kind != Kind::Code {
            // Preserved white space keeps its spaces and tabs at the edges; line breaks there
            // (source formatting) still go.
            const ALL: &[char] = &[' ', '\t', '\r', '\n'];
            const BREAKS: &[char] = &['\r', '\n'];
            p.trim_with(if pre_lead { BREAKS } else { ALL }, if pre_tail { BREAKS } else { ALL });
            if p.text().trim().is_empty() {
                // Only white space (indentation between blocks): nothing to keep.
                p.trim();
            }
        }
        if p.inlines.len() == 1 && p.text() == "\u{00A0}" {
            // `<p>&nbsp;</p>`: an empty paragraph on purpose.
            p.inlines.clear();
            self.para_cont = false;
            self.container().push(FBlock::Para(p));
            return;
        }
        if p.is_empty() && p.kind != Kind::Code && p.list.is_none() {
            // Whitespace-only text between blocks.
            return;
        }
        let cont = std::mem::take(&mut self.para_cont);
        let level = p.list.map(|l| l.level);
        let c = self.container();
        if cont
            && let Some(FBlock::Para(prev)) = c.last_mut()
            && prev.list.is_some()
            && prev.list.map(|l| l.level) == level
        {
            prev.push_text("\n", &Fmt::default());
            prev.inlines.extend(p.inlines);
            return;
        }
        c.push(FBlock::Para(p));
    }

    pub(super) fn text(&mut self, t: &str) {
        if self.in_head() {
            if let Some(title) = &mut self.title {
                title.push_str(t);
            }
            return;
        }
        if t.is_empty() {
            return;
        }
        let inside_table_only = matches!(
            self.stack.iter().rev().find(|e| matches!(e.kind, ElKind::Table | ElKind::Tr | ElKind::Cell)).map(|e| e.kind),
            Some(ElKind::Table | ElKind::Tr)
        );
        if inside_table_only && t.trim().is_empty() {
            return;
        }
        let code = self.stack.iter().any(|e| e.kind == ElKind::Pre);
        let f = self.fmt();
        if code {
            let mut first = true;
            for line in t.split('\n') {
                if !first {
                    // A newline right after <pre> is ignored.
                    let empty_start = self.para.as_ref().is_none_or(|p| p.inlines.is_empty()) && self.pre_fresh();
                    if !empty_start {
                        self.start_para();
                        self.flush();
                    }
                }
                first = false;
                let line = line.trim_end_matches('\r');
                if !line.is_empty() {
                    self.start_para().push_text(line, &Fmt { code: false, ..f.clone() });
                    self.mark_pre_used();
                }
            }
            return;
        }
        if self.preserve() {
            self.note_edge(true);
            self.start_para().push_text(t, &f);
            return;
        }
        // Collapse whitespace.
        let mut s = String::with_capacity(t.len());
        let mut prev_space = self.para.as_ref().is_none_or(|p| match p.inlines.last() {
            Some(Inline::Text(x, _)) => x.ends_with([' ', '\n']),
            Some(_) => false,
            None => true,
        });
        for c in t.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000C}') {
                if !prev_space {
                    s.push(' ');
                    prev_space = true;
                }
            } else {
                s.push(c);
                prev_space = false;
            }
        }
        if s.is_empty() {
            return;
        }
        self.note_edge(false);
        self.start_para().push_text(&s, &f);
    }

    /// Record whether text about to be added keeps its white space (see `pre_lead`).
    fn note_edge(&mut self, preserved: bool) {
        if self.para.as_ref().is_none_or(|p| p.inlines.is_empty()) {
            self.pre_lead = Some(preserved);
        }
        self.pre_tail = preserved;
    }

    /// Add a `<ruby>` element's base text and reading as one inline (#1681). Half-marked ruby (no
    /// base, or no reading) is plain text instead: a guide that annotates nothing is no guide, and
    /// the text it held is never lost.
    pub(super) fn push_ruby(&mut self, base: &str, ruby: &str) {
        let preserve = self.preserve();
        let text = |s: &str| if preserve { s.to_string() } else { collapse_ws(s) };
        let (base, ruby) = (text(base), text(ruby));
        let f = self.fmt();
        if base.is_empty() || ruby.is_empty() {
            self.start_para().push_text(if base.is_empty() { &ruby } else { &base }, &f);
            return;
        }
        self.start_para().inlines.push(Inline::Ruby { base, ruby, fmt: f });
    }

    fn pre_fresh(&self) -> bool {
        self.stack.iter().rev().find(|e| e.kind == ElKind::Pre).is_some_and(|e| !e.used)
    }

    fn mark_pre_used(&mut self) {
        if let Some(e) = self.stack.iter_mut().rev().find(|e| e.kind == ElKind::Pre) {
            e.used = true;
        }
    }

    /// Pop elements down to and including index `k`.
    fn pop_to(&mut self, k: usize) {
        while self.stack.len() > k {
            let Some(e) = self.stack.pop() else { break };
            self.close(&e);
        }
    }

    fn close(&mut self, e: &El) {
        if e.page_break_after || e.page_break {
            self.pending_page_break = true;
        }
        if e.kind != ElKind::Inline {
            self.flush();
        }
        match e.kind {
            ElKind::Cell => {
                self.flush();
                let blocks = if self.containers.len() > 1 { self.containers.pop().unwrap_or_default() } else { Vec::new() };
                let mut cell = self.cells.pop().unwrap_or_default();
                cell.blocks = blocks;
                if let Some(t) = self.tables.last_mut() {
                    t.row.get_or_insert_with(Vec::new).push(cell);
                }
            }
            ElKind::Tr => {
                if let Some(t) = self.tables.last_mut()
                    && let Some(r) = t.row.take()
                {
                    t.rows.push(r);
                }
            }
            ElKind::Table => {
                if let Some(mut t) = self.tables.pop() {
                    if let Some(r) = t.row.take() {
                        t.rows.push(r);
                    }
                    let borderless = !t.bordered;
                    // Keep at most MAX_ROWS rows (the model drops the rest anyway): bounds the
                    // O(rows × rowspan) work of `insert_covered` on a table of millions of `<tr>`.
                    let rows: Vec<_> = t.rows.into_iter().filter(|r| !r.is_empty()).take(wordcraft_doc::table::MAX_ROWS).collect();
                    let mut ft = FTable { rows, widths: Vec::new(), borderless };
                    if !ft.rows.is_empty() {
                        ft.insert_covered();
                        self.container().push(FBlock::Table(ft));
                    }
                }
            }
            ElKind::Head => {}
            _ => {}
        }
    }

    /// Index of the innermost open element named `name`, not looking past a table boundary
    /// unless `cross_tables`.
    fn find_open(&self, pred: impl Fn(&El) -> bool, stop: impl Fn(&El) -> bool) -> Option<usize> {
        for (i, e) in self.stack.iter().enumerate().rev() {
            if pred(e) {
                return Some(i);
            }
            if stop(e) {
                return None;
            }
        }
        None
    }

    pub(super) fn close_head_for_start(&mut self, name: &str) {
        // A body-content start tag implicitly ends an omitted </head>.
        if !matches!(
            name,
            "html" | "head" | "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "noscript" | "script" | "style" | "template" | "title"
        ) && let Some(k) = self.find_open(|e| e.kind == ElKind::Head, |_| false)
        {
            self.pop_to(k);
        }
    }

    pub(super) fn start(&mut self, name: &str, attrs: &[(String, String)], self_close: bool) {
        // Head metadata.
        match name {
            "title" => {
                self.title = Some(String::new());
            }
            "meta" => {
                let n = attr(attrs, "name").unwrap_or("").to_ascii_lowercase();
                let c = attr(attrs, "content").unwrap_or("").to_string();
                match n.as_str() {
                    "author" => self.meta.author = c,
                    "description" => self.meta.description = c,
                    "keywords" => self.meta.keywords = c,
                    "subject" => self.meta.subject = c,
                    _ => {}
                }
                return;
            }
            _ => {}
        }
        let is_table_part = matches!(name, "tr" | "td" | "th");
        let in_table = self.tables.len() < model::MAX_DEPTH;
        // Implicit closes.
        if closes_p(name)
            && let Some(k) = self.find_open(|e| e.name == "p", |e| matches!(e.kind, ElKind::Cell | ElKind::Table | ElKind::Li | ElKind::Quote))
        {
            self.pop_to(k);
        }
        if matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") && self.stack.last().is_some_and(|e| matches!(e.kind, ElKind::Heading(_))) {
            let k = self.stack.len() - 1;
            self.pop_to(k);
        }
        if name == "li"
            && let Some(k) = self.find_open(|e| e.kind == ElKind::Li, |e| matches!(e.kind, ElKind::List(_) | ElKind::Table | ElKind::Cell))
        {
            self.pop_to(k);
        }
        if matches!(name, "dt" | "dd")
            && let Some(k) =
                self.find_open(|e| e.name == "dt" || e.name == "dd", |e| matches!(e.kind, ElKind::Table | ElKind::Cell) || e.name == "dl")
        {
            self.pop_to(k);
        }
        if is_table_part && !self.tables.is_empty() {
            if matches!(name, "td" | "th")
                && let Some(k) = self.find_open(|e| e.kind == ElKind::Cell, |e| matches!(e.kind, ElKind::Table | ElKind::Tr))
            {
                self.pop_to(k);
            }
            if name == "tr"
                && let Some(k) = self.find_open(|e| matches!(e.kind, ElKind::Tr | ElKind::Cell), |e| e.kind == ElKind::Table)
            {
                // Pop to the outermost of cell/row inside the current table.
                let mut kk = k;
                while kk > 0 && self.stack.get(kk - 1).is_some_and(|e| matches!(e.kind, ElKind::Tr | ElKind::Cell)) {
                    kk -= 1;
                }
                self.pop_to(kk);
            }
        }
        if name == "a"
            && !self.in_head()
            && let Some(id) = attr(attrs, "id").or_else(|| attr(attrs, "name")).filter(|s| !s.is_empty())
        {
            self.start_para().inlines.push(Inline::Anchor(id.to_string()));
        }
        // Element-specific work.
        match name {
            "br" => {
                if self.stack.iter().any(|e| e.kind == ElKind::Pre) {
                    self.start_para();
                    self.flush();
                } else {
                    let f = self.fmt();
                    self.start_para().push_text("\n", &f);
                }
                return;
            }
            "hr" => {
                self.flush();
                self.container().push(FBlock::Para(Para::new(Kind::Rule)));
                return;
            }
            "wbr" => {
                let f = self.fmt();
                self.start_para().push_text("\u{200B}", &f);
                return;
            }
            "img" => {
                let alt = attr(attrs, "alt").unwrap_or("").to_string();
                let dim = |k: &str| attr(attrs, k).and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok()).map(|p| p * 0.75);
                let mut w = dim("width");
                let mut h = dim("height");
                if let Some(st) = attr(attrs, "style") {
                    for decl in st.split(';') {
                        if let Some((k, v)) = decl.split_once(':') {
                            match k.trim().to_ascii_lowercase().as_str() {
                                "width" => w = css_len_pt(v).or(w),
                                "height" => h = css_len_pt(v).or(h),
                                _ => {}
                            }
                        }
                    }
                }
                let img = attr(attrs, "src")
                    .and_then(|src| if src.trim_start().starts_with("data:") { data_uri(src).map(Arc::new) } else { (self.images)(src.trim()) })
                    .and_then(|d| make_img(d, w, h, &alt));
                let f = self.fmt();
                match img {
                    Some(im) => self.start_para().inlines.push(Inline::Image(im)),
                    None if !alt.is_empty() => self.start_para().push_text(&alt, &f),
                    None => {}
                }
                return;
            }
            _ => {}
        }
        if is_void(name) || self_close && block_kind(name).is_none() && !matches!(name, "a" | "span") {
            return;
        }
        if self.stack.len() >= MAX_STACK {
            return;
        }
        let mut kind = block_kind(name).unwrap_or(ElKind::Inline);
        if matches!(kind, ElKind::Table | ElKind::Tr | ElKind::Cell) && !in_table {
            kind = ElKind::Block;
        }
        if matches!(kind, ElKind::Tr | ElKind::Cell) && self.tables.is_empty() {
            kind = ElKind::Block;
        }
        if kind != ElKind::Inline {
            self.flush();
        }
        let mut fmt = self.fmt();
        let mut align = None;
        let mut preserve = self.preserve();
        let mut page_break = false;
        let mut page_break_after = false;
        match name {
            // A header cell is bold, as in browsers and Word, unless its own CSS says otherwise.
            "b" | "strong" | "th" => fmt.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" | "address" => fmt.italic = true,
            "u" | "ins" => fmt.underline = true,
            "s" | "strike" | "del" => fmt.strike = true,
            "code" | "kbd" | "tt" | "samp" => fmt.code = true,
            "sup" => fmt.sup = true,
            "sub" => fmt.sub = true,
            "small" => fmt.size = Some(fmt.size.map_or(10.0, |s| (s * 0.833).max(1.0))),
            "big" => fmt.size = Some(fmt.size.map_or(14.0, |s| (s * 1.2).min(1638.0))),
            "mark" => fmt.background = Some(wordcraft_doc::Rgb(255, 255, 0)),
            "font" => {
                if let Some(c) = attr(attrs, "color").and_then(parse_color) {
                    fmt.color = Some(c);
                }
                if let Some(face) = attr(attrs, "face") {
                    let fam = face.split(',').next().unwrap_or("").trim().to_string();
                    if model::is_mono(&fam) {
                        fmt.code = true;
                    } else if !fam.is_empty() {
                        fmt.font = model::kept_font(&fam).or(fmt.font.take());
                    }
                }
                if let Some(sz) = attr(attrs, "size") {
                    let s = sz.trim();
                    let val = if let Some(rest) = s.strip_prefix('+') {
                        rest.parse::<i32>().ok().map(|d| 3 + d)
                    } else if let Some(rest) = s.strip_prefix('-') {
                        rest.parse::<i32>().ok().map(|d| 3 - d)
                    } else {
                        s.parse::<i32>().ok()
                    };
                    if let Some(sz) = val {
                        fmt.size = Some([7.5, 10.0, 12.0, 13.5, 18.0, 24.0, 36.0].get((sz.clamp(1, 7) - 1) as usize).copied().unwrap_or(12.0));
                    }
                }
            }
            "a" => {
                if let Some(h) = attr(attrs, "href").filter(|h| wordcraft_doc::link::is_safe_href(h)) {
                    fmt.link = model::kept_link(h.trim());
                }
            }
            _ => {}
        }
        // `align` on a table is about where the table sits (not imported); it doesn't align the
        // text in its cells.
        if let Some(a) = attr(attrs, "align").filter(|_| kind != ElKind::Table) {
            align = match a.to_ascii_lowercase().as_str() {
                "center" | "middle" => Some(Align::Center),
                "right" => Some(Align::Right),
                "justify" => Some(Align::Justify),
                "left" => Some(Align::Left),
                _ => None,
            };
        }
        if name == "center" {
            align = Some(Align::Center);
        }
        if let Some(st) = attr(attrs, "style") {
            apply_style(st, &mut fmt, &mut align, &mut preserve, &mut page_break, &mut page_break_after);
        }
        if kind == ElKind::Pre {
            preserve = true;
        }
        match kind {
            ElKind::Table => self.tables.push(TableB { rows: Vec::new(), row: None, bordered: declares_border(attrs, true) }),
            ElKind::Tr => {
                if let Some(t) = self.tables.last_mut()
                    && let Some(r) = t.row.take()
                {
                    t.rows.push(r);
                }
            }
            ElKind::Cell => {
                let tb = self.find_open(|e| e.kind == ElKind::Tr, |e| e.kind == ElKind::Table).is_none();
                if let Some(t) = self.tables.last_mut()
                    && declares_border(attrs, false)
                {
                    t.bordered = true;
                }
                if tb && let Some(t) = self.tables.last_mut() {
                    // A cell without a row: start one.
                    if let Some(r) = t.row.take() {
                        t.rows.push(r);
                    }
                    t.row = Some(Vec::new());
                }
                let num = |k: &str| attr(attrs, k).and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.clamp(1, 63));
                let shading = attr(attrs, "bgcolor").and_then(parse_color).or(fmt.background);
                let mut valign = attr(attrs, "valign").and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                    "top" => Some(VAlign::Top),
                    "middle" | "center" => Some(VAlign::Center),
                    "bottom" => Some(VAlign::Bottom),
                    _ => None,
                });
                if let Some(st) = attr(attrs, "style") {
                    for decl in st.split(';') {
                        if let Some((k, v)) = decl.split_once(':')
                            && k.trim().eq_ignore_ascii_case("vertical-align")
                        {
                            match v.trim().to_ascii_lowercase().as_str() {
                                "top" => valign = Some(VAlign::Top),
                                "middle" | "center" => valign = Some(VAlign::Center),
                                "bottom" => valign = Some(VAlign::Bottom),
                                _ => {}
                            }
                        }
                    }
                }
                self.cells.push(Cell {
                    blocks: Vec::new(),
                    colspan: num("colspan").unwrap_or(1),
                    rowspan: attr(attrs, "rowspan").and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.clamp(1, 1000)).unwrap_or(1),
                    covered: false,
                    header: name == "th",
                    shading,
                    borders: None,
                    valign,
                });
                fmt.background = None;
                self.containers.push(Vec::new());
            }
            _ => {}
        }
        self.stack.push(El { name: name.to_string(), kind, fmt, align, preserve, page_break, page_break_after, used: false });
    }

    pub(super) fn end(&mut self, name: &str) {
        if name == "title" {
            if let Some(t) = self.title.take() {
                self.meta.title = t.split_whitespace().collect::<Vec<_>>().join(" ");
            }
            return;
        }
        let crosses = matches!(name, "table");
        let target = if matches!(name, "td" | "th") {
            self.find_open(|e| e.kind == ElKind::Cell, |e| e.kind == ElKind::Table)
        } else if name == "tr" {
            self.find_open(|e| e.kind == ElKind::Tr, |e| e.kind == ElKind::Table)
        } else {
            self.find_open(|e| e.name == name, |e| !crosses && matches!(e.kind, ElKind::Table | ElKind::Cell))
        };
        if let Some(k) = target {
            self.pop_to(k);
        }
    }

    pub(super) fn finish(mut self) -> Flow {
        self.pop_to(0);
        self.flush();
        // Unclosed cells: fold remaining containers into the body.
        while self.containers.len() > 1 {
            let c = self.containers.pop().unwrap_or_default();
            self.container().extend(c);
        }
        let blocks = self.containers.pop().unwrap_or_default();
        if let Some(t) = self.title.take() {
            self.meta.title = t;
        }
        Flow { blocks, meta: self.meta }
    }
}
