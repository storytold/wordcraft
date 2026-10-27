use super::import::Pages;
use super::limits::{MAX_INFO_CHARS, MAX_OPEN_INSERTIONS, MAX_REGIONS, MAX_SECTIONS, MAX_XML_DEPTH};
use super::styles::Styles;
use super::xml::{Attrs, Ev, events, get, length, local};
use super::*;

/// An embedded ODF formula sub-document's `content.xml`, read for its `math:math` element
/// (ODF 1.3 §10.4.3.2 wraps it in `office:document-content`/`office:body`/`office:math`, but
/// some writers emit it bare): the first element named `math` in document order, with its
/// subtree, passed to [`crate::mathml::read`]. `None` when there's no such element, it holds
/// nothing, or the XML is malformed. Nesting and element count are bounded like other XML here.
fn read_formula(xml: &[u8]) -> Option<(String, bool)> {
    use crate::mathml::{El, Item, MAX_NODES, local};
    let mut stack: Vec<El> = Vec::new();
    let mut math: Option<El> = None;
    let mut over = 0usize;
    let mut count = 0usize;
    events(xml, |ev| {
        if math.is_some() {
            return;
        }
        count += 1;
        if count > MAX_NODES * 4 {
            math = stack.drain(..).next();
            return;
        }
        match ev {
            Ev::Start(q, a) => {
                if stack.is_empty() && local(&q) != "math" {
                    return;
                }
                if stack.len() < wordcraft_doc::math::MAX_DEPTH {
                    stack.push(El { name: q, attrs: a, children: Vec::new() });
                } else {
                    over += 1;
                }
            }
            Ev::End(_) => {
                if over > 0 {
                    over -= 1;
                } else if stack.len() > 1 {
                    if let Some(done) = stack.pop()
                        && let Some(parent) = stack.last_mut()
                    {
                        parent.children.push(Item::El(done));
                    }
                } else if stack.len() == 1 {
                    math = stack.pop();
                }
            }
            Ev::Text(t) => {
                if over == 0
                    && let Some(top) = stack.last_mut()
                {
                    top.children.push(Item::Text(t));
                }
            }
        }
    });
    crate::mathml::read(&math?)
}

struct TableB {
    rows: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    /// Covered cells still owed to the previous cell's column span.
    hcover: usize,
    /// Widths in points of the `table:table-column`s so far, 0 where the style gives none.
    widths: Vec<f32>,
}

impl TableB {
    fn finish(self) -> FTable {
        // A column without a width leaves the whole grid to the equal split.
        let widths = if self.widths.iter().all(|w| *w > 0.0) { self.widths } else { Vec::new() };
        FTable { rows: self.rows, widths, borderless: false }
    }
}

/// A `text:changed-region` read from `text:tracked-changes`.
#[derive(Default)]
struct Region {
    kind: Option<RevisionKind>,
    /// Sequence number: tells regions by the same author at the same time apart.
    id: u32,
    author: String,
    date: String,
    /// A deletion's deleted paragraphs.
    blocks: Vec<FBlock>,
}

impl Region {
    fn mark(&self) -> RevMark {
        RevMark { author: self.author.trim().to_string(), date: self.date.trim().to_string(), id: Some(self.id) }
    }
}

/// Reader state saved while a deletion's content is read into its own container.
struct DelState {
    containers: usize,
    tables: usize,
    cells: usize,
    lists_saved: usize,
    fmt: usize,
    lists: Vec<(Option<String>, bool)>,
    item_used: Vec<bool>,
    /// Insertions open inside the deleted content (text inserted, then deleted).
    ins: Vec<(String, RevMark)>,
}

pub(super) struct Body<'a> {
    styles: &'a Styles,
    zip: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>,
    containers: Vec<Vec<FBlock>>,
    fmt: Vec<Fmt>,
    para: Option<Para>,
    /// The text read last ended in a space collapsed from XML whitespace (not a `text:s`).
    xml_space_end: bool,
    para_cont: bool,
    /// Open lists: (style name, item open, item used).
    lists: Vec<(Option<String>, bool)>,
    item_used: Vec<bool>,
    /// Lists outside the table cells being read.
    lists_saved: Vec<(Vec<(Option<String>, bool)>, Vec<bool>)>,
    tables: Vec<TableB>,
    cells: Vec<Cell>,
    pub(super) in_text: bool,
    skip: usize,
    depth: usize,
    /// `(width, height, picture bytes, alt text, equation read from an embedded `draw:object`)`.
    frame: Option<(Option<f32>, Option<f32>, Option<Vec<u8>>, String, Option<(String, bool)>)>,
    in_desc: bool,
    /// The master pages' page setups.
    pages: &'a Pages,
    /// The current master page's setup (a column section's page, without its columns).
    page: SectionProps,
    /// The open section; where its blocks start in the body; how many sections have ended.
    pub(super) sec: SectionProps,
    pub(super) sec_from: usize,
    sections: usize,
    /// Element depth of the open columned `text:section`.
    cols_at: Option<usize>,
    /// Inside `text:tracked-changes`.
    in_tc: bool,
    /// `text:track-changes` of the document's tracked changes (default on), when it has them.
    pub(super) track: Option<bool>,
    /// Changed regions read, by id.
    regions: HashMap<String, Region>,
    /// Regions seen (capped at [`MAX_REGIONS`]).
    region_count: usize,
    /// The region being read.
    region: Option<(String, Region)>,
    /// Depth of the `office:change-info` being read, and the field its text goes to.
    info_at: Option<usize>,
    info_field: Option<bool>,
    /// Reading a deletion's content.
    del: Option<DelState>,
    /// Insertions open in the body: (region id, revision).
    active_ins: Vec<(String, RevMark)>,
}

impl<'a> Body<'a> {
    pub(super) fn new(styles: &'a Styles, pages: &'a Pages, zip: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Self {
        let page = pages.first.clone().unwrap_or_default();
        Body {
            styles,
            zip,
            containers: vec![Vec::new()],
            fmt: Vec::new(),
            para: None,
            xml_space_end: false,
            para_cont: false,
            lists: Vec::new(),
            lists_saved: Vec::new(),
            item_used: Vec::new(),
            tables: Vec::new(),
            cells: Vec::new(),
            in_text: false,
            skip: 0,
            depth: 0,
            frame: None,
            in_desc: false,
            pages,
            sec: page.clone(),
            page,
            sec_from: 0,
            sections: 0,
            cols_at: None,
            in_tc: false,
            track: None,
            regions: HashMap::new(),
            region_count: 0,
            region: None,
            info_at: None,
            info_field: None,
            del: None,
            active_ins: Vec::new(),
        }
    }

    pub(super) fn feed(&mut self, ev: Ev) {
        match ev {
            Ev::Start(q, a) => {
                if self.depth < MAX_XML_DEPTH {
                    self.start(&q, &a);
                } else {
                    self.depth += 1;
                }
            }
            Ev::End(q) => {
                if self.depth <= MAX_XML_DEPTH {
                    self.end(&q);
                } else {
                    self.depth -= 1;
                }
            }
            Ev::Text(t) => self.text(&t),
        }
    }

    /// The blocks read, with anything still open closed.
    pub(super) fn take_blocks(&mut self) -> Vec<FBlock> {
        self.end_para();
        while self.containers.len() > 1 {
            let c = self.containers.pop().unwrap_or_default();
            self.container().extend(c);
        }
        let mut blocks = self.containers.pop().unwrap_or_default();
        for t in self.tables.drain(..) {
            if !t.rows.is_empty() {
                blocks.push(FBlock::Table(t.finish()));
            }
        }
        blocks
    }
}

impl Body<'_> {
    fn container(&mut self) -> &mut Vec<FBlock> {
        if self.containers.is_empty() {
            self.containers.push(Vec::new());
        }
        let n = self.containers.len() - 1;
        &mut self.containers[n]
    }

    fn cur_fmt(&self) -> Fmt {
        self.fmt.last().cloned().unwrap_or_default()
    }

    /// Reading the body itself, not a table cell or a frame.
    fn in_body(&self) -> bool {
        self.containers.len() == 1 && self.tables.is_empty() && self.frame.is_none()
    }

    /// End the open section before what comes next, which starts section `next`. An open section
    /// with nothing in it yet takes `next`'s place instead. Only in the body, and at most
    /// [`MAX_SECTIONS`]. True when what comes next follows a section break.
    fn break_section(&mut self, next: SectionProps) -> bool {
        if !self.in_body() {
            return false;
        }
        let Some(body) = self.containers.first_mut() else { return false };
        if body.len() <= self.sec_from {
            let restart = self.sec.page_num_start;
            self.sec = next;
            self.sec.page_num_start = self.sec.page_num_start.or(restart);
            return self.sections > 0;
        }
        if self.sections + 1 >= MAX_SECTIONS {
            return false;
        }
        // A section ends on a paragraph: after a table, an empty one (as Word writes it).
        if !matches!(body.last(), Some(FBlock::Para(p)) if p.section.is_none()) {
            body.push(FBlock::Para(Para::default()));
        }
        if let Some(FBlock::Para(p)) = body.last_mut() {
            p.section = Some(Box::new(std::mem::replace(&mut self.sec, next)));
        }
        self.sec_from = body.len();
        self.sections += 1;
        true
    }

    /// A body paragraph or table of style `name` that names a master page starts a new page with
    /// that page's setup and, when the style says so, page numbering restarted (ODF 1.3 applies
    /// `style:page-number` only with a master page). The section that starts there, if any.
    ///
    /// Rare cases left simple: a master-page paragraph first inside a columned `text:section`
    /// replaces that (still empty) column section, so the columns are lost; a columned section
    /// nested in another is read as part of the outer one.
    fn page_section(&mut self, name: &str) -> Option<SectionProps> {
        if !self.in_body() {
            return None;
        }
        let (master, number) = self.styles.page_start(name);
        let restart = number.flatten();
        let page = master.and_then(|m| self.pages.masters.get(m))?.clone();
        self.page = page.clone();
        Some(SectionProps { start: SectionStart::NextPage, page_num_start: restart, ..page })
    }

    fn start_para(&mut self, style: Option<&str>, heading: Option<u8>) {
        self.end_para();
        let (mut kind, align, mut pb, rule, keep_next, base) =
            style.map(|s| self.styles.para(s)).unwrap_or((Kind::Normal, None, false, false, false, Fmt::default()));
        // The section break starts the new page; no page break besides.
        if let Some(next) = style.and_then(|s| self.page_section(s))
            && self.break_section(next)
        {
            pb = false;
        }
        if let Some(h) = heading {
            kind = Kind::Heading(h.clamp(1, 6));
        }
        let mut p = Para { kind, align, page_break: pb, keep_next, ..Default::default() };
        if rule && kind == Kind::Normal {
            p.kind = Kind::Rule;
        }
        if let Some(lv) = self.lists.len().checked_sub(1)
            && self.lists.last().is_some_and(|l| l.1)
        {
            let style = self.lists.iter().rev().find_map(|l| l.0.clone());
            let ordered = style.and_then(|s| self.styles.lists.get(&s)).and_then(|v| v.get(lv.min(9)).copied()).unwrap_or(false);
            p.list = Some(ListInfo { ordered, level: lv.min(8) as u8, start: 1 });
            self.para_cont = self.item_used.last().copied().unwrap_or(false);
            if let Some(u) = self.item_used.last_mut() {
                *u = true;
            }
        } else {
            self.para_cont = false;
        }
        self.fmt.push(base);
        self.para = Some(p);
    }

    fn end_para(&mut self) {
        let Some(mut p) = self.para.take() else { return };
        self.fmt.pop();
        // A paragraph ending inside an open insertion has an inserted mark. (Paragraphs that
        // `place_deletion` splits off keep only the marks read inside their region.)
        if p.mark_ins.is_none() {
            p.mark_ins = self.open_insertion().map(Box::new);
        }
        self.finish_para(p);
    }

    /// The innermost insertion open where text is being read (the body, or deleted content).
    fn open_insertion(&self) -> Option<RevMark> {
        let open = match &self.del {
            Some(d) => &d.ins,
            None => &self.active_ins,
        };
        open.last().map(|(_, m)| m.clone())
    }

    /// Add a finished paragraph to the current container.
    fn finish_para(&mut self, mut p: Para) {
        // Spaces next to tracked text are part of a change.
        let tracked = self.del.is_some()
            || p.mark_ins.is_some()
            || p.mark_del.is_some()
            || p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) if f.ins.is_some() || f.del.is_some()));
        // Leading XML whitespace is never read (see `text`); a trailing one is trimmed here.
        // Explicit `text:s` spaces and tabs stay.
        let xml_space_end = std::mem::take(&mut self.xml_space_end);
        if p.kind != Kind::Code
            && !tracked
            && xml_space_end
            && let Some(Inline::Text(t, _)) = p.inlines.last_mut()
            && t.ends_with(' ')
        {
            t.pop();
            if t.is_empty() {
                p.inlines.pop();
            }
        }
        if p.kind == Kind::Rule && !p.is_empty() {
            p.kind = Kind::Normal;
        }
        if matches!(p.kind, Kind::Code | Kind::Heading(_) | Kind::Title) {
            for i in &mut p.inlines {
                if let Inline::Text(_, f) = i {
                    if p.kind == Kind::Code {
                        f.code = false;
                        f.font = None;
                    }
                    f.size = None;
                }
            }
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

    fn push_text(&mut self, t: &str) {
        let mut f = self.cur_fmt();
        f.ins = self.open_insertion();
        if self.para.is_none() {
            if t.trim().is_empty() {
                return;
            }
            self.start_para(None, None);
        }
        self.xml_space_end = false;
        if let Some(p) = &mut self.para {
            p.push_text(t, &f);
        }
    }

    fn text(&mut self, t: &str) {
        if self.skip > 0 || !self.in_text {
            return;
        }
        if self.info_at.is_some() {
            if let (Some(field), Some((_, r))) = (self.info_field, &mut self.region) {
                let s = if field { &mut r.author } else { &mut r.date };
                let room = MAX_INFO_CHARS.saturating_sub(s.chars().count());
                s.extend(t.chars().take(room));
            }
            return;
        }
        if self.in_tc && self.del.is_none() {
            return;
        }
        if self.in_desc {
            if let Some(fr) = &mut self.frame {
                fr.3.push_str(t);
            }
            return;
        }
        let Some(p) = &self.para else {
            return;
        };
        // Collapse XML whitespace.
        let mut prev_space = match p.inlines.last() {
            Some(Inline::Text(x, _)) => x.ends_with([' ', '\n', '\t']),
            Some(_) => false,
            None => true,
        };
        let mut s = String::with_capacity(t.len());
        for c in t.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r') {
                if !prev_space {
                    s.push(' ');
                    prev_space = true;
                }
            } else {
                s.push(c);
                prev_space = false;
            }
        }
        if !s.is_empty() {
            self.push_text(&s);
            self.xml_space_end = s.ends_with(' ');
        }
    }

    fn start(&mut self, q: &str, a: &Attrs) {
        self.depth += 1;
        if q == "office:text" {
            self.in_text = true;
            return;
        }
        if self.skip > 0 {
            self.skip += 1;
            return;
        }
        if !self.in_text {
            return;
        }
        if self.info_at.is_some() {
            self.info_field = match local(q) {
                "creator" => Some(true),
                "date" => Some(false),
                _ => None,
            };
            return;
        }
        if self.in_tc {
            if self.del.is_none() {
                self.tracked_start(q, a);
                return;
            }
            // Inside deleted content: only insertions (text inserted, then deleted) nest.
            if matches!(local(q), "change-start" | "change-end") {
                let regions = &self.regions;
                if let Some(d) = &mut self.del {
                    change_mark(regions, &mut d.ins, local(q) == "change-start", a);
                }
                return;
            }
            if matches!(local(q), "tracked-changes" | "changed-region" | "insertion" | "deletion" | "format-change" | "change") {
                self.skip = 1;
                return;
            }
            if local(q) == "change-info" && self.region.is_some() {
                self.info_at = Some(self.depth);
                return;
            }
        }
        match local(q) {
            "tracked-changes" => {
                if self.para.is_none() && self.tables.is_empty() && self.cells.is_empty() && self.frame.is_none() {
                    self.in_tc = true;
                    self.track = Some(get(a, "text:track-changes") != Some("false"));
                } else {
                    self.skip = 1;
                }
            }
            "change" => {
                if let Some(id) = get(a, "text:change-id")
                    && self.regions.get(id).is_some_and(|r| r.kind == Some(RevisionKind::Delete))
                    && let Some(r) = self.regions.remove(id)
                {
                    let mark = r.mark();
                    self.place_deletion(r.blocks, mark);
                }
            }
            "change-start" | "change-end" => change_mark(&self.regions, &mut self.active_ins, local(q) == "change-start", a),
            "note"
            | "annotation"
            | "text-box"
            | "sequence-decls"
            | "forms"
            | "variable-decls"
            | "user-field-decls"
            | "index-title-template"
            | "table-of-content-source" => {
                self.skip = 1;
            }
            // Drawing shapes (ODF 1.3 §10.3), with their text, until shapes are read (#1654).
            // Groups are walked, so the pictures in them still are.
            "rect" | "line" | "polyline" | "polygon" | "regular-polygon" | "path" | "circle" | "ellipse" | "connector" | "caption" | "measure"
            | "custom-shape"
                if q.starts_with("draw:") =>
            {
                self.skip = 1;
            }
            "p" | "h" if !self.in_desc => {
                let h =
                    if local(q) == "h" { Some(get(a, "text:outline-level").and_then(|v| v.trim().parse::<u8>().ok()).unwrap_or(1)) } else { None };
                self.start_para(get(a, "text:style-name"), h);
            }
            "span" => {
                let base = self.cur_fmt();
                let f = match get(a, "text:style-name") {
                    Some(s) => self.styles.text_fmt(s, &base),
                    None => base,
                };
                self.fmt.push(f);
            }
            "a" => {
                let mut f = self.cur_fmt();
                if let Some(h) = get(a, "xlink:href").filter(|h| !h.is_empty()) {
                    f.link = model::kept_link(h);
                }
                if let Some(s) = get(a, "text:style-name") {
                    f = self.styles.text_fmt(s, &f);
                }
                self.fmt.push(f);
            }
            "s" => {
                let n = get(a, "text:c").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 1000);
                self.push_text(&" ".repeat(n));
            }
            "tab" => self.push_text("\t"),
            "line-break" => self.push_text("\n"),
            "bookmark" | "bookmark-start" => {
                if let Some(n) = get(a, "text:name") {
                    if self.para.is_none() {
                        self.start_para(None, None);
                    }
                    if let Some(p) = &mut self.para {
                        p.inlines.push(Inline::Anchor(n.to_string()));
                    }
                }
            }
            "list" => {
                self.end_para();
                if self.lists.len() < 64 {
                    self.lists.push((get(a, "text:style-name").map(String::from), false));
                    self.item_used.push(false);
                } else {
                    self.skip = 1;
                }
            }
            "list-item" | "list-header" => {
                if let Some(l) = self.lists.last_mut() {
                    l.1 = true;
                }
                if let Some(u) = self.item_used.last_mut() {
                    *u = false;
                }
            }
            "table" if q.starts_with("table:") => {
                self.end_para();
                // A body table whose style names a master page starts that page (ODF 1.3 §19.509).
                if let Some(next) = get(a, "table:style-name").and_then(|s| self.page_section(s)) {
                    self.break_section(next);
                }
                if self.tables.len() >= model::MAX_DEPTH {
                    self.skip = 1;
                    return;
                }
                self.tables.push(TableB { rows: Vec::new(), row: Vec::new(), hcover: 0, widths: Vec::new() });
            }
            "table-column" => {
                let w = get(a, "table:style-name").and_then(|n| self.styles.column_width(n)).unwrap_or(0.0);
                let n = get(a, "table:number-columns-repeated").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).max(1);
                if let Some(t) = self.tables.last_mut() {
                    let room = wordcraft_doc::table::MAX_COLS.saturating_sub(t.widths.len());
                    t.widths.extend(std::iter::repeat_n(w, n.min(room)));
                }
            }
            "table-row" => {
                if let Some(t) = self.tables.last_mut() {
                    t.row.clear();
                    t.hcover = 0;
                }
            }
            "table-cell" => {
                self.end_para();
                let span = get(a, "table:number-columns-spanned").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 63);
                let rows = get(a, "table:number-rows-spanned").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 1000);
                let shading = get(a, "table:style-name").and_then(|s| self.styles.cell_background(s));
                let valign = get(a, "table:style-name").and_then(|s| self.styles.cell_valign(s));
                if let Some(t) = self.tables.last_mut() {
                    t.hcover = span as usize - 1;
                }
                self.cells.push(Cell { colspan: span, rowspan: rows, shading, valign, ..Default::default() });
                self.containers.push(Vec::new());
                self.lists_saved.push((std::mem::take(&mut self.lists), std::mem::take(&mut self.item_used)));
            }
            "covered-table-cell" => {
                if let Some(t) = self.tables.last_mut() {
                    if t.hcover > 0 {
                        t.hcover -= 1;
                    } else if t.row.len() < 63 {
                        t.row.push(Cell { covered: true, ..Default::default() });
                    }
                }
            }
            "frame" => {
                let w = get(a, "svg:width").and_then(length);
                let h = get(a, "svg:height").and_then(length);
                self.frame = Some((w, h, None, String::new(), None));
            }
            "image" => {
                if let Some(href) = get(a, "xlink:href")
                    && let Some(fr) = &mut self.frame
                    && fr.2.is_none()
                    && fr.4.is_none()
                {
                    let path = href.trim_start_matches("./");
                    fr.2 = (self.zip)(path);
                }
            }
            // An embedded object (ODF 1.3 §10.4.3): a formula sub-document's `content.xml` holds
            // a `math:math` tree (#3099). Other object kinds have none, so the frame still falls
            // back to its `draw:image` replacement preview, read at `end` below.
            "object" => {
                if let Some(href) = get(a, "xlink:href")
                    && let Some(fr) = &mut self.frame
                    && fr.4.is_none()
                {
                    let path = format!("{}/content.xml", href.trim_start_matches("./").trim_end_matches('/'));
                    if let Some(xml) = (self.zip)(&path) {
                        fr.4 = read_formula(&xml);
                    }
                }
            }
            "desc" | "title" if self.frame.is_some() => self.in_desc = true,
            // A columned section (ODF 1.3 §5.4) is a continuous section with its columns.
            "section" if q.starts_with("text:") => {
                self.end_para();
                if self.cols_at.is_none()
                    && self.in_body()
                    && let Some(columns) = get(a, "text:style-name")
                        .and_then(|s| self.styles.columns(s))
                        // A section whose columns match the open section's (a plain one-column
                        // section in a one-column page) is no new section.
                        .filter(|c| c.count != self.sec.columns.count || (c.count > 1 && *c != self.sec.columns))
                {
                    let next = SectionProps { start: SectionStart::Continuous, page_num_start: None, columns, ..self.page.clone() };
                    self.break_section(next);
                    self.cols_at = Some(self.depth);
                }
            }
            _ => {}
        }
    }

    /// An element inside `text:tracked-changes`, outside deleted content.
    fn tracked_start(&mut self, q: &str, a: &Attrs) {
        match local(q) {
            "changed-region" if self.region.is_none() && self.region_count < MAX_REGIONS => {
                let id = get(a, "text:id").unwrap_or_default().to_string();
                let r = Region { id: self.region_count as u32, ..Default::default() };
                self.region_count += 1;
                self.region = Some((id, r));
            }
            "insertion" | "deletion" | "format-change" if self.region.as_ref().is_some_and(|(_, r)| r.kind.is_none()) => {
                let kind = match local(q) {
                    "insertion" => RevisionKind::Insert,
                    "deletion" => RevisionKind::Delete,
                    _ => RevisionKind::Format,
                };
                if let Some((_, r)) = &mut self.region {
                    r.kind = Some(kind);
                }
                if kind == RevisionKind::Delete {
                    self.containers.push(Vec::new());
                    self.del = Some(DelState {
                        containers: self.containers.len(),
                        tables: self.tables.len(),
                        cells: self.cells.len(),
                        lists_saved: self.lists_saved.len(),
                        fmt: self.fmt.len(),
                        lists: std::mem::take(&mut self.lists),
                        item_used: std::mem::take(&mut self.item_used),
                        ins: Vec::new(),
                    });
                }
            }
            "change-info" if self.region.is_some() => self.info_at = Some(self.depth),
            _ => self.skip = 1,
        }
    }

    /// End a deletion's content: its paragraphs go to the region being read.
    fn close_deletion(&mut self) {
        if self.del.is_none() {
            return;
        }
        self.end_para();
        let Some(d) = self.del.take() else { return };
        let base = d.containers.saturating_sub(1).min(self.containers.len());
        let blocks: Vec<FBlock> = self.containers.split_off(base).into_iter().flatten().filter(|b| matches!(b, FBlock::Para(_))).collect();
        if self.containers.is_empty() {
            self.containers.push(Vec::new());
        }
        self.tables.truncate(d.tables);
        self.cells.truncate(d.cells);
        self.lists_saved.truncate(d.lists_saved);
        self.fmt.truncate(d.fmt);
        self.lists = d.lists;
        self.item_used = d.item_used;
        self.frame = None;
        self.in_desc = false;
        self.para_cont = false;
        if let Some((_, r)) = &mut self.region {
            r.blocks = blocks;
        }
    }

    /// Put a deletion's paragraphs back at a `text:change` point, as deleted text. Inside a
    /// paragraph, each deleted paragraph mark splits it; between paragraphs, each deleted
    /// paragraph stands alone with its mark deleted.
    fn place_deletion(&mut self, blocks: Vec<FBlock>, mark: RevMark) {
        let mut paras: Vec<Para> = Vec::new();
        for b in blocks {
            let FBlock::Para(mut p) = b else { continue };
            // Pictures and other objects carry no revision mark: leave them out.
            p.inlines.retain(|i| matches!(i, Inline::Text(..) | Inline::Anchor(_)));
            for i in &mut p.inlines {
                if let Inline::Text(_, f) = i {
                    f.del = Some(mark.clone());
                }
            }
            paras.push(p);
        }
        let append = |dst: &mut Para, src: Vec<Inline>| {
            for i in src {
                match i {
                    Inline::Text(t, f) => dst.push_text(&t, &f),
                    other => dst.inlines.push(other),
                }
            }
        };
        let Some(mut cur) = self.para.take() else {
            self.para_cont = false;
            for mut p in paras {
                p.mark_del = Some(Box::new(mark.clone()));
                self.finish_para(p);
            }
            return;
        };
        let n = paras.len();
        let mut it = paras.into_iter();
        let mut first_mark_ins = None;
        if let Some(first) = it.next() {
            first_mark_ins = first.mark_ins;
            append(&mut cur, first.inlines);
        }
        if n < 2 {
            self.para = Some(cur);
            return;
        }
        let mut rest = Para { kind: cur.kind, list: cur.list, align: cur.align, ..Default::default() };
        // The host paragraph now ends at the first deleted paragraph's mark.
        cur.mark_del = Some(Box::new(mark.clone()));
        cur.mark_ins = first_mark_ins;
        self.finish_para(cur);
        for (k, mut p) in it.enumerate() {
            if k + 2 == n {
                // The last deleted paragraph runs on into the rest of the body paragraph.
                rest.kind = p.kind;
                rest.align = p.align.or(rest.align);
                append(&mut rest, p.inlines);
            } else {
                p.mark_del = Some(Box::new(mark.clone()));
                self.finish_para(p);
            }
        }
        self.para = Some(rest);
    }

    fn end(&mut self, q: &str) {
        self.depth = self.depth.saturating_sub(1);
        if q == "office:text" {
            if self.del.is_some() {
                // Unclosed deleted content (hostile): drop it.
                self.close_deletion();
            }
            self.in_tc = false;
            self.region = None;
            self.info_at = None;
            self.end_para();
            self.in_text = false;
            return;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        if !self.in_text {
            return;
        }
        if let Some(d) = self.info_at {
            if self.depth < d {
                self.info_at = None;
            }
            self.info_field = None;
            return;
        }
        if self.in_tc {
            match local(q) {
                "deletion" if self.del.is_some() => {
                    self.close_deletion();
                    return;
                }
                "changed-region" => {
                    // A deletion left open (hostile) ends with its region.
                    self.close_deletion();
                    if let Some((id, r)) = self.region.take()
                        && r.kind.is_some()
                    {
                        // The first region with an id wins.
                        self.regions.entry(id).or_insert(r);
                    }
                    return;
                }
                "tracked-changes" => {
                    self.close_deletion();
                    self.in_tc = false;
                    self.region = None;
                    return;
                }
                _ if self.del.is_some() => {}
                _ => return,
            }
        }
        match local(q) {
            "p" | "h" if !self.in_desc => self.end_para(),
            "span" | "a" if self.fmt.len() > usize::from(self.para.is_some()) => {
                self.fmt.pop();
            }
            "list" => {
                self.end_para();
                self.lists.pop();
                self.item_used.pop();
            }
            "list-item" | "list-header" => {
                self.end_para();
                if let Some(l) = self.lists.last_mut() {
                    l.1 = false;
                }
            }
            "table-cell" => {
                self.end_para();
                let blocks = if self.containers.len() > 1 { self.containers.pop().unwrap_or_default() } else { Vec::new() };
                (self.lists, self.item_used) = self.lists_saved.pop().unwrap_or_default();
                let mut cell = self.cells.pop().unwrap_or_default();
                cell.blocks = blocks;
                if let Some(t) = self.tables.last_mut()
                    && t.row.len() < 63
                {
                    t.row.push(cell);
                }
            }
            "table-row" => {
                if let Some(t) = self.tables.last_mut() {
                    let r = std::mem::take(&mut t.row);
                    if !r.is_empty() && t.rows.len() < wordcraft_doc::table::MAX_ROWS {
                        t.rows.push(r);
                    }
                }
            }
            "table-header-rows" => {
                if let Some(t) = self.tables.last_mut() {
                    for r in &mut t.rows {
                        for c in r {
                            c.header = true;
                        }
                    }
                }
            }
            "table" if q.starts_with("table:") => {
                self.end_para();
                if let Some(t) = self.tables.pop()
                    && !t.rows.is_empty()
                {
                    self.container().push(FBlock::Table(t.finish()));
                }
            }
            "desc" | "title" => self.in_desc = false,
            "section" if q.starts_with("text:") && self.cols_at == Some(self.depth + 1) => {
                self.end_para();
                self.cols_at = None;
                let next = SectionProps { start: SectionStart::Continuous, page_num_start: None, ..self.page.clone() };
                self.break_section(next);
            }
            "frame" => {
                if let Some((w, h, img, alt, eq)) = self.frame.take() {
                    let inline = match eq {
                        Some((linear, display)) => Some(Inline::Equation { linear, display }),
                        None => img.and_then(|data| make_img(data, w, h, alt.trim())).map(Inline::Image),
                    };
                    if let Some(inline) = inline {
                        if self.para.is_none() {
                            self.start_para(None, None);
                        }
                        if let Some(p) = &mut self.para {
                            p.inlines.push(inline);
                        }
                    }
                }
                self.frame = None;
            }
            _ => {}
        }
    }
}

/// A `text:change-start` (`start`) or `text:change-end` (attributes `a`) among the insertions
/// `open`: unknown ids, ends without a start and starts past [`MAX_OPEN_INSERTIONS`] are ignored.
fn change_mark(regions: &HashMap<String, Region>, open: &mut Vec<(String, RevMark)>, start: bool, a: &Attrs) {
    let Some(id) = get(a, "text:change-id") else { return };
    if !start {
        if let Some(k) = open.iter().position(|(o, _)| o == id) {
            open.remove(k);
        }
        return;
    }
    if let Some(r) = regions.get(id).filter(|r| r.kind == Some(RevisionKind::Insert))
        && open.len() < MAX_OPEN_INSERTIONS
        && !open.iter().any(|(o, _)| o == id)
    {
        open.push((id.to_string(), r.mark()));
    }
}
