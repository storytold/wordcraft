use super::import::Pages;
use super::limits::{MAX_COMMENTS, MAX_FIELD_RESULT, MAX_INFO_CHARS, MAX_OPEN_INSERTIONS, MAX_REGIONS, MAX_SECTIONS, MAX_XML_DEPTH};
use super::styles::Styles;
use super::xml::{Attrs, Ev, events, get, length, local};
use super::{
    Borders, Cell, FBlock, FComment, FTable, Fmt, HashMap, Inline, Kind, ListInfo, Para, RevMark, RevisionKind, SectionProps, SectionStart, make_img,
    model,
};

/// Most drawings waiting outside a paragraph.
const MAX_LOOSE: usize = 10_000;

pub(in crate::odt) mod drawing;
mod indexes;
mod notes;
mod tables;
mod tracked;

use drawing::{OpenGroup, read_formula};
use indexes::{OpenIndex, is_index};
use notes::{OpenField, OpenNote, balance_comments};
use tables::TableB;
use tracked::{DelState, Region, change_mark};

/// Most bibliography sources read.
const MAX_SOURCES: usize = 10_000;

/// Most index entries over text open at once in a paragraph.
const MAX_OPEN_MARKS: usize = 64;

/// An open frame: its size, picture, alt text, equation, chart and other embedded object.
type Frame =
    (Option<f32>, Option<f32>, Option<Vec<u8>>, String, Option<(String, bool)>, Option<wordcraft_doc::chart::ChartSpec>, Option<model::OdfObject>);

pub(super) struct Body<'a> {
    styles: &'a Styles,
    zip: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>,
    /// The package's entry names and its manifest's media types, for embedded objects.
    pub(super) entries: &'a [String],
    /// The bibliography sources the citations carry.
    pub(super) sources: Vec<wordcraft_doc::Source>,
    /// The paragraph before had a page break after it: the next one starts a page.
    break_next: bool,
    /// Shapes' ids (`xml:id`, `draw:id`) as WordCraft drawing ids, and where the shapes with one
    /// sit (kind, frame relative to its anchor, turn, placement, the paragraph they are in), for
    /// connectors glued to them.
    pub(super) conn_ids: HashMap<String, u32>,
    pub(super) shape_places: HashMap<u32, (wordcraft_doc::para::ShapeKind, [f32; 4], wordcraft_doc::para::Float, usize)>,
    /// Paragraphs started so far: which paragraph a shape is in.
    pub(super) para_seq: usize,
    /// Index entries over text whose end hasn't come (in the paragraph being read): their id,
    /// keys and where they start: an inline, and a byte offset in it when it is text.
    index_marks: Vec<(String, (Option<String>, Option<String>), (usize, usize))>,
    /// A ruby being read: its base text, ruby text, which of them the text goes to (`true`:
    /// the ruby), and the base's formatting.
    ruby: Option<(String, String, Option<bool>, Fmt)>,
    /// A placeholder being read (§7.7.2 `text:placeholder`): its text, its run's formatting.
    placeholder: Option<(String, Fmt, &'static str, String)>,
    /// The form controls of `office:forms`, placed where a `draw:control` names them.
    forms: super::forms::FormsReader,
    pub(super) manifest: Option<&'a HashMap<String, String>>,
    containers: Vec<Vec<FBlock>>,
    /// Drawings anchored in the text but outside any paragraph (LibreOffice's page-anchored
    /// frames before the first one): the next paragraph's, so they don't make a paragraph of
    /// their own (which would start the document on another page style).
    loose: Vec<Inline>,
    fmt: Vec<Fmt>,
    para: Option<Para>,
    /// The text read last ended in a space collapsed from XML whitespace (not a `text:s`).
    xml_space_end: bool,
    para_cont: bool,
    /// Open lists: (style name, item open, item used).
    lists: Vec<(Option<String>, bool)>,
    item_used: Vec<bool>,
    /// Lists outside the table cells being read, and the list number there.
    lists_saved: Vec<(Vec<(Option<String>, bool)>, Vec<bool>, u32)>,
    tables: Vec<TableB>,
    cells: Vec<Cell>,
    /// Each open cell's formula, as a Word formula field code (§19.646 `table:formula`).
    formulas: Vec<Option<String>>,
    pub(super) in_text: bool,
    skip: usize,
    depth: usize,
    /// `(width, height, picture bytes, alt text, equation read from an embedded `draw:object`)`.
    frame: Option<Frame>,
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
    /// Format changes open where text is being read (ODF records no old formatting).
    active_fmt: Vec<(String, RevMark)>,
    /// The text field being read.
    field: Option<OpenField>,
    /// Where the frame being read floats.
    frame_float: Option<wordcraft_doc::para::Float>,
    /// The content of the text box in the frame being read.
    frame_box: Option<Vec<FBlock>>,
    /// The look of the frame being read, should it hold a text box.
    frame_look: model::BoxLook,
    /// The open frame's picture crop (`fo:clip`: top, right, bottom, left, points of its original
    /// size).
    frame_clip: Option<[f32; 4]>,
    /// The open frame's name (`draw:name`), which a linked text box names.
    pub(super) frame_name: Option<String>,
    /// The kind of the drawing shape being read (its text is read as a text box's).
    shape_kind: Option<wordcraft_doc::para::ShapeKind>,
    /// A custom shape's mirroring (left to right, top to bottom), from its geometry.
    shape_flip: (bool, bool),
    /// A freeform shape's paths (a polygon's, polyline's, path's or regular polygon's).
    shape_path: Option<std::sync::Arc<wordcraft_doc::freeform::Freeform>>,
    /// A custom shape's geometry being read, when its type has no WordCraft kind: drawn from its
    /// own path and equations.
    geometry: Option<Box<super::geometry::Pending>>,
    /// The turn of a group's member being read, from its `draw:transform`.
    member_rot: Option<f32>,
    /// The depth of the `draw:g` group being read: its shapes are placed by it, not anchored.
    group_at: Option<usize>,
    /// The table of contents or other index being read.
    index: Option<OpenIndex>,
    /// The group being read (its anchor, style and members so far), and where its member being
    /// read sits (unturned, from where the group is anchored).
    group: Option<OpenGroup>,
    member_at: Option<(f32, f32)>,
    /// The note or comment being read.
    note: Option<OpenNote>,
    /// The list being read (its flow number, [`ListInfo::list`]), the last one, the next number,
    /// lists by `xml:id`, and a start value an item set for the items from it on.
    list_id: u32,
    last_list: Option<u32>,
    next_list: u32,
    list_xml_ids: HashMap<String, u32>,
    item_start: Option<u32>,
    /// Comments by `office:name`: their flow ids; and the next flow id.
    comment_names: HashMap<String, u32>,
    next_comment: u32,
}

impl<'a> Body<'a> {
    pub(super) fn new(styles: &'a Styles, pages: &'a Pages, zip: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Self {
        let page = pages.first.clone().unwrap_or_default();
        Body {
            styles,
            zip,
            entries: &[],
            forms: Default::default(),
            break_next: false,
            sources: Vec::new(),
            ruby: None,
            placeholder: None,
            index_marks: Vec::new(),
            conn_ids: HashMap::new(),
            shape_places: HashMap::new(),
            para_seq: 0,
            manifest: None,
            containers: vec![Vec::new()],
            loose: Vec::new(),
            fmt: Vec::new(),
            para: None,
            xml_space_end: false,
            para_cont: false,
            lists: Vec::new(),
            lists_saved: Vec::new(),
            item_used: Vec::new(),
            tables: Vec::new(),
            cells: Vec::new(),
            formulas: Vec::new(),
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
            active_fmt: Vec::new(),
            field: None,
            frame_float: None,
            frame_box: None,
            frame_look: model::BoxLook::default(),
            frame_clip: None,
            frame_name: None,
            shape_kind: None,
            shape_flip: (false, false),
            shape_path: None,
            geometry: None,
            member_rot: None,
            group_at: None,
            index: None,
            group: None,
            member_at: None,
            note: None,
            comment_names: HashMap::new(),
            next_comment: 0,
            list_id: 0,
            last_list: None,
            next_list: 1,
            list_xml_ids: HashMap::new(),
            item_start: None,
        }
    }

    /// A new list number.
    fn new_list_id(&mut self) -> u32 {
        let id = self.next_list;
        self.next_list = self.next_list.saturating_add(1);
        id
    }

    /// A top-level `text:list` starting (ODF 1.3 §5.3.1): a list of its own, unless it continues
    /// the one before it (`text:continue-numbering`) or the one it names (`text:continue-list`).
    fn start_list_id(&mut self, a: &Attrs) {
        let named = get(a, "text:continue-list").and_then(|n| self.list_xml_ids.get(n)).copied();
        let before = (get(a, "text:continue-numbering").is_some_and(|v| v.trim() == "true")).then_some(self.last_list).flatten();
        let id = match named.or(before) {
            Some(id) => id,
            None => self.new_list_id(),
        };
        if let Some(x) = get(a, "xml:id").filter(|x| x.len() <= MAX_INFO_CHARS)
            && self.list_xml_ids.len() < MAX_SECTIONS
        {
            self.list_xml_ids.entry(x.to_string()).or_insert(id);
        }
        self.list_id = id;
        self.last_list = Some(id);
        self.item_start = None;
    }

    pub(super) fn feed(&mut self, ev: Ev) {
        match ev {
            // `office:forms` is read on its own; a `draw:control` (always empty) is its control.
            Ev::Start(q, a) if self.forms.start(&q, &a) => {}
            Ev::End(q) if self.forms.end(&q) => {}
            Ev::Start(q, a) if q == "draw:control" => self.place_control(&a),
            Ev::End(q) if q == "draw:control" => {}
            // Ruby (ODF 1.3 §6.4): its base and ruby text, read on their own.
            Ev::Start(q, _) if q == "text:ruby" && self.para.is_some() && self.field.is_none() => {
                self.ruby = Some((String::new(), String::new(), None, self.cur_fmt()));
            }
            Ev::Start(q, _) if self.ruby.is_some() && matches!(q.as_str(), "text:ruby-base" | "text:ruby-text") => {
                if let Some(r) = &mut self.ruby {
                    r.2 = Some(q == "text:ruby-text");
                }
            }
            Ev::End(q) if self.ruby.is_some() && matches!(q.as_str(), "text:ruby-base" | "text:ruby-text") => {
                if let Some(r) = &mut self.ruby {
                    r.2 = None;
                }
            }
            Ev::End(q) if q == "text:ruby" && self.ruby.is_some() => {
                if let Some((base, ruby, _, fmt)) = self.ruby.take() {
                    let (base, ruby) = (base.trim().to_string(), ruby.trim().to_string());
                    match (base.is_empty(), ruby.is_empty()) {
                        (false, false) => self.push_inline(Inline::Ruby { base, ruby, fmt }),
                        // A guide over nothing, or reading nothing: its text.
                        (false, true) | (true, false) => self.push_inline(Inline::Text(if base.is_empty() { ruby } else { base }, fmt)),
                        (true, true) => {}
                    }
                }
            }
            // A placeholder (§7.7.2): a plain text content control showing its text until
            // something is typed in its place.
            Ev::Start(q, a) if q == "text:placeholder" && self.para.is_some() && self.field.is_none() && self.placeholder.is_none() => {
                // Its description (§19.800 `text:description`): the control's title.
                let about = get(&a, "text:description").map(|d| d.chars().take(MAX_FIELD_RESULT).collect()).unwrap_or_default();
                self.placeholder = Some((String::new(), self.cur_fmt(), super::forms::placeholder_type(get(&a, "text:placeholder-type")), about));
            }
            Ev::Text(t) if self.placeholder.is_some() => {
                if let Some((s, ..)) = &mut self.placeholder {
                    let room = MAX_FIELD_RESULT.saturating_sub(s.len());
                    s.extend(t.chars().take(room));
                }
            }
            Ev::End(q) if q == "text:placeholder" && self.placeholder.is_some() => {
                if let Some((text, fmt, kind, about)) = self.placeholder.take() {
                    let mut control = wordcraft_doc::control::ContentControl::new(wordcraft_doc::control::ControlKind::Text { multi_line: false });
                    control.placeholder_text.clone_from(&text);
                    control.showing_placeholder = true;
                    // Its element, so it is written back a placeholder of its type.
                    control.form_xml = super::forms::placeholder_start(kind);
                    control.title = about;
                    let shown = control.placeholder_shown().to_string();
                    self.push_inline(Inline::Control { control: Box::new(control), text: shown, fmt });
                }
            }
            Ev::Text(t) if self.ruby.is_some() => {
                if let Some((base, ruby, Some(in_ruby), _)) = &mut self.ruby {
                    let s = if *in_ruby { ruby } else { base };
                    let room = MAX_FIELD_RESULT.saturating_sub(s.len());
                    s.extend(t.chars().take(room));
                }
            }
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
    /// The hidden form controls read (see [`super::forms::FormsReader::hidden`]).
    pub(super) fn take_hidden_controls(&mut self) -> Vec<String> {
        std::mem::take(&mut self.forms.hidden)
    }

    /// Whether the reader is in the text itself: not in a table, note, comment or frame.
    pub(in crate::odt) fn at_body_level(&self) -> bool {
        self.tables.is_empty() && self.cells.is_empty() && self.note.is_none() && self.frame.is_none() && self.containers.len() == 1
    }

    pub(super) fn take_blocks(&mut self) -> Vec<FBlock> {
        // What truncated XML left open: a field, a note or comment (its paragraph goes on).
        self.end_field();
        if self.note.is_some() {
            self.end_note();
        }
        self.end_para();
        // Drawings after the last paragraph: a paragraph of their own.
        if !self.loose.is_empty() {
            self.start_para(None, None);
            self.end_para();
        }
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
        balance_comments(&mut blocks);
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
        // Paragraphs of the text (not of a shape's, a note's or a comment's) say where shapes are.
        if self.note.is_none() {
            self.para_seq = self.para_seq.wrapping_add(1);
        }
        let look = style.map(|s| self.styles.para(s)).unwrap_or_default();
        let (mut kind, align, mut pb, rule, keep_next, base) = (look.kind, look.align, look.page_break, look.rule, look.keep_next, look.fmt);
        // The section break starts the new page; no page break besides.
        let mut new_page = false;
        if let Some(next) = style.and_then(|s| self.page_section(s))
            && self.break_section(next)
        {
            (pb, new_page) = (false, true);
        }
        if let Some(h) = heading {
            kind = Kind::Heading(h.clamp(1, 6));
        }
        // The paragraph before ended with a page break after it (`fo:break-after`): this one
        // starts the page.
        if std::mem::take(&mut self.break_next) && !new_page {
            pb = true;
        }
        let mut p = Para { kind, align, page_break: pb, keep_next, spacing: look.spacing, ..Default::default() };
        // A paragraph starting a column (`fo:break-before="column"`): Word's column break first.
        if look.column_break && !pb {
            p.inlines.push(Inline::Text(wordcraft_doc::para::COLUMN_BREAK.to_string(), model::Fmt::default()));
        }
        p.borders = look.borders;
        if let Some(n) = look.named {
            // The ODF name for now; import makes it the document style's id.
            p.extra.get_or_insert_with(Default::default).style = Some(n);
        }
        if let Some(bg) = look.shading {
            p.extra.get_or_insert_with(Default::default).shading = Some(bg);
        }
        if let Some(c) = look.contextual {
            p.extra.get_or_insert_with(Default::default).contextual_spacing = Some(c);
        }
        if look.rtl == Some(true) {
            p.extra.get_or_insert_with(Default::default).bidi = Some(true);
        }
        if let Some((lines, distance)) = look.drop_cap {
            let x = p.extra.get_or_insert_with(Default::default);
            (x.drop_cap, x.drop_cap_distance) = (Some(lines), (distance > 0.0).then_some(distance));
        }
        if look.keep_lines == Some(true) {
            p.extra.get_or_insert_with(Default::default).keep_lines = Some(true);
        }
        if look.widow_control == Some(false) {
            p.extra.get_or_insert_with(Default::default).widow_control = Some(false);
        }
        // Hyphenated: `Some(false)` (not suppressed); the document then hyphenates (see
        // `import::hyphenation`).
        if look.hyphenate == Some(true) {
            p.extra.get_or_insert_with(Default::default).suppress_hyphens = Some(false);
        }
        if look.break_after {
            self.break_next = true;
        }
        if let Some(mut tabs) = look.tabs {
            // LibreOffice measures tabs from the indent (its `TabsRelativeToIndent`); Word from
            // the margin.
            if self.styles.tabs_from_indent {
                let indent = look.tab_origin;
                for t in &mut tabs {
                    t.pos = (t.pos + indent).clamp(-1584.0, 1584.0);
                }
            }
            p.extra.get_or_insert_with(Default::default).tabs = Some(tabs);
        }
        if rule && kind == Kind::Normal {
            p.kind = Kind::Rule;
        }
        if let Some(lv) = self.lists.len().checked_sub(1)
            && self.lists.last().is_some_and(|l| l.1)
        {
            let style = self.lists.iter().rev().find_map(|l| l.0.clone());
            let level = style.and_then(|s| self.styles.lists.get(&s)).and_then(|v| v.get(lv.min(9)).copied()).unwrap_or_default();
            let start = self.item_start.take().unwrap_or(level.start);
            p.list = Some(ListInfo { ordered: level.ordered, level: lv.min(8) as u8, start, list: self.list_id, label: level.label });
            self.para_cont = self.item_used.last().copied().unwrap_or(false);
            if let Some(u) = self.item_used.last_mut() {
                *u = true;
            }
        } else {
            self.para_cont = false;
        }
        self.fmt.push(base);
        // Drawings waiting outside a paragraph come first in it, when it is the text's own (not
        // a text box's, a note's or a cell's).
        if !self.loose.is_empty() && self.at_body_level() {
            let mut loose = std::mem::take(&mut self.loose);
            loose.append(&mut p.inlines);
            p.inlines = loose;
        }
        self.para = Some(p);
    }

    fn end_para(&mut self) {
        // An index entry is marked within one paragraph.
        self.index_marks.clear();
        let Some(mut p) = self.para.take() else { return };
        self.glue_connectors(&mut p);
        // A paragraph without text has its own text formatting as its mark's: its size is its
        // height, and its drawings' runs have it.
        if let Some(base) = self.fmt.pop()
            && p.textless()
            && base != model::Fmt::default()
        {
            p.mark = Some(Box::new(crate::model::char_props(&base)));
        }
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
        // In a format change: a formatting revision whose old formatting is the text's own.
        if let Some((_, mark)) = self.active_fmt.last().filter(|_| self.del.is_none()) {
            let old = Fmt { ins: None, del: None, fmt_change: None, ..f.clone() };
            f.fmt_change = Some(Box::new(model::FmtChange { mark: mark.clone(), old }));
        }
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
        if let Some(c) = self.note.as_mut().and_then(|n| n.comment.as_mut())
            && let Some(k) = c.field
        {
            let s = match k {
                0 => &mut c.author,
                1 => &mut c.date,
                _ => &mut c.initials,
            };
            let room = MAX_INFO_CHARS.saturating_sub(s.chars().count());
            s.extend(t.chars().take(room));
            return;
        }
        if let Some(OpenField { result, .. }) = &mut self.field {
            // A field's shown text, its XML whitespace collapsed; bounded like any one text run.
            for c in t.chars() {
                if result.len() >= MAX_FIELD_RESULT {
                    break;
                }
                if matches!(c, ' ' | '\t' | '\n' | '\r') {
                    if !result.ends_with(' ') {
                        result.push(' ');
                    }
                } else {
                    result.push(c);
                }
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
            if self.skip == 1
                && q == "draw:equation"
                && let Some(g) = &mut self.geometry
            {
                g.equation(a);
            }
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
        if self.tracked_context(q, a) {
            return;
        }
        // A custom shape's geometry: its kind, or, for a type WordCraft has no kind for, its
        // path, worked out from its equations when the element ends. Its handles aren't read.
        if q == "draw:enhanced-geometry" && self.shape_kind.is_some() {
            let t = get(a, "draw:type").unwrap_or_default();
            self.shape_kind = Some(super::shapes::kind_of(t));
            self.geometry = super::geometry::Pending::of(t, a);
            // Mirrored (§20.112 `draw:mirror-horizontal`, §20.113 `draw:mirror-vertical`).
            let on = |k: &str| get(a, k).is_some_and(|v| v.trim() == "true");
            self.shape_flip = (on("draw:mirror-horizontal"), on("draw:mirror-vertical"));
            self.skip = 1;
            return;
        }
        if self.start_inline(q, a) || self.start_note_part(q, a) || self.start_list_part(q, a) || self.start_index_part(q, a) {
            return;
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
            "change-start" | "change-end" => {
                let start = local(q) == "change-start";
                change_mark(&self.regions, &mut self.active_ins, RevisionKind::Insert, start, a);
                change_mark(&self.regions, &mut self.active_fmt, RevisionKind::Format, start, a);
            }
            "note" | "annotation" | "text-box" | "sequence-decls" | "forms" | "variable-decls" | "user-field-decls" | "index-title-template" => {
                self.skip = 1;
            }
            // Drawing shapes (ODF 1.3 §10.3) anchored in the text, with their text.
            "rect" | "ellipse" | "circle" | "line" | "custom-shape" | "polygon" | "polyline" | "path" | "regular-polygon" | "connector"
                if q.starts_with("draw:") && self.frame.is_none() && self.note.is_none() && self.del.is_none() && !self.in_tc =>
            {
                self.start_shape(q, a);
            }
            // A group of shapes and pictures (§10.3.15): read as one object. Groups in it are walked,
            // their members its own.
            "g" if q == "draw:g" && self.group_at.is_none() => {
                self.group_at = Some(self.depth);
                if self.frame.is_none() && self.note.is_none() && self.del.is_none() && !self.in_tc {
                    self.group = Some(OpenGroup {
                        anchor: get(a, "text:anchor-type").map(String::from),
                        style: get(a, "draw:style-name").map(String::from),
                        members: Vec::new(),
                    });
                }
            }
            // Connectors, captions and measures, and shapes where none is read (in a note, a
            // comment, a shape's text or deleted text), are left out with their text.
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
            "table" if q.starts_with("table:") => {
                // Drawings waiting for a paragraph get their own before a table.
                if !self.loose.is_empty() && self.tables.is_empty() {
                    self.start_para(None, None);
                }
                self.end_para();
                // A body table whose style names a master page starts that page (ODF 1.3 §19.509).
                if let Some(next) = get(a, "table:style-name").and_then(|s| self.page_section(s)) {
                    self.break_section(next);
                }
                if self.tables.len() >= model::MAX_DEPTH {
                    self.skip = 1;
                    return;
                }
                // A table starting a page: its first paragraph starts it, as Word moves a table
                // whose first row's paragraph has a page break before it.
                if get(a, "table:style-name").is_some_and(|s| self.styles.table_breaks(s)) {
                    self.break_next = true;
                }
                self.start_table(a);
            }
            "table-column" => {
                let w = get(a, "table:style-name").and_then(|n| self.styles.column_width(n)).unwrap_or(0.0);
                let n = get(a, "table:number-columns-repeated").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).max(1);
                if let Some(t) = self.tables.last_mut() {
                    let room = wordcraft_doc::table::MAX_COLS.saturating_sub(t.widths.len());
                    t.widths.extend(std::iter::repeat_n(w, n.min(room)));
                }
            }
            "table-row" => self.start_row(a),
            "table-cell" => {
                self.end_para();
                let span = get(a, "table:number-columns-spanned").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 63);
                let rows = get(a, "table:number-rows-spanned").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 1000);
                let shading = get(a, "table:style-name").and_then(|s| self.styles.cell_background(s));
                let valign = get(a, "table:style-name").and_then(|s| self.styles.cell_valign(s));
                let borders = get(a, "table:style-name").and_then(|s| self.styles.cell_borders(s));
                let margins = get(a, "table:style-name").and_then(|s| self.styles.cell_padding(s));
                let direction = get(a, "table:style-name").map(|s| self.styles.cell_direction(s)).unwrap_or_default();
                if let Some(t) = self.tables.last_mut() {
                    t.hcover = span as usize - 1;
                }
                self.cells.push(Cell { colspan: span, rowspan: rows, shading, valign, borders, margins, direction, ..Default::default() });
                self.formulas.push(get(a, "table:formula").and_then(super::cell_formula::word_of));
                self.containers.push(Vec::new());
                self.lists_saved.push((std::mem::take(&mut self.lists), std::mem::take(&mut self.item_used), self.list_id));
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
                self.frame = Some((w, h, None, String::new(), None, None, None));
                let g = get(a, "draw:style-name").map(|s| self.styles.graphic(s)).unwrap_or_default();
                self.frame_float = Some(super::frames::place(a, w.unwrap_or(0.0), h.unwrap_or(0.0), &g));
                self.member_start(a, w.unwrap_or(0.0), h.unwrap_or(0.0), None);
                // Without a height, a text box in it grows to its text (`fo:min-height` below).
                self.frame_look = g.look(h.is_none(), &self.styles.defs);
                self.frame_name = get(a, "draw:name").map(|n| n.chars().take(256).collect());
                self.frame_clip = g.clip;
            }
            "image" | "object" | "object-ole" | "desc" | "title" | "contour-polygon" => self.frame_part(q, a),
            // A columned section (ODF 1.3 §5.4) is a continuous section with its columns.
            "section" if q.starts_with("text:") => {
                self.end_para();
                if self.cols_at.is_none()
                    && self.in_body()
                    && let Some(columns) = get(a, "text:style-name")
                        .and_then(|s| self.styles.columns(s, self.page.text_width()))
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

    /// A table (`table:table`) begins: its alignment and width, a relative width or a fixed
    /// one when aligned (a table filling the margins, LibreOffice's default, takes the text width).
    fn start_table(&mut self, a: &Attrs) {
        let (align, width, rel) = get(a, "table:style-name").map(|s| self.styles.table_look(s)).unwrap_or_default();
        // Filling the margins (§20.300 `margins`, the default), its columns are fitted to the
        // text width whatever widths they state, as LibreOffice fits them.
        let width_pct = match align {
            None => Some(rel.unwrap_or(100.0)),
            Some(_) => rel.or_else(|| width.map(|w| (w / self.sec.text_width().max(1.0) * 100.0).min(100.0))),
        };
        // LibreOffice's tables keep their columns' widths, whatever their content (and however
        // far past the margins): Word's fixed layout.
        let space = get(a, "table:style-name").map(|s| self.styles.table_space(s)).unwrap_or_default();
        let extra = model::TableExtra { align, width: None, width_pct, heights: Vec::new(), fixed: true, space, header_rows: 0 };
        self.tables.push(TableB { rows: Vec::new(), row: Vec::new(), hcover: 0, widths: Vec::new(), extra });
    }

    /// A table row (`table:table-row`) begins: its height, kept as rows are (see the row's end).
    fn start_row(&mut self, a: &Attrs) {
        let height = get(a, "table:style-name").and_then(|s| self.styles.row_height(s));
        if let Some(t) = self.tables.last_mut() {
            t.row.clear();
            t.hcover = 0;
            if t.extra.heights.len() < wordcraft_doc::table::MAX_ROWS {
                t.extra.heights.truncate(t.rows.len());
                t.extra.heights.push(height);
            }
        }
    }

    /// A form control (`draw:control`, ODF 1.3 §10.3.13) in the paragraph: the control it names,
    /// with its text, as a content control. Inside a field or a frame's text it is nothing.
    fn place_control(&mut self, a: &Attrs) {
        if self.field.is_some() {
            return;
        }
        if let Some((mut control, text)) = get(a, "draw:control").and_then(|id| self.forms.controls.get(id)).cloned() {
            // Its size (§19.539, §19.571).
            let len = |k: &str| get(a, k).and_then(length).filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(1584.0));
            if let (Some(w), Some(h)) = (len("svg:width"), len("svg:height")) {
                control.form_size = Some([w, h]);
            }
            let fmt = self.cur_fmt();
            self.push_inline(Inline::Control { control: Box::new(control), text, fmt });
        }
    }

    /// Part of a frame: its picture, embedded object, description or contour.
    fn frame_part(&mut self, q: &str, a: &Attrs) {
        match local(q) {
            "image" => {
                if let Some(href) = get(a, "xlink:href")
                    && let Some(fr) = &mut self.frame
                    && fr.2.is_none()
                    && fr.4.is_none()
                    && fr.5.is_none()
                    && fr.6.as_ref().is_none_or(|o| o.replacement.is_none())
                {
                    let path = href.trim_start_matches("./");
                    fr.2 = (self.zip)(path);
                    if let Some(o) = &mut fr.6 {
                        o.replacement = fr.2.clone().map(std::sync::Arc::new);
                    }
                }
            }
            // An embedded object (ODF 1.3 §10.4.3): a formula sub-document's `content.xml` holds
            // a `math:math` tree (#3099), a chart's an `office:chart`. Other object kinds (and
            // charts WordCraft can't edit) fall back to the frame's `draw:image` replacement
            // preview, read at `end` below.
            "object" | "object-ole" => {
                if let Some(href) = get(a, "xlink:href") {
                    self.read_object(href, local(q) == "object-ole");
                }
            }
            "desc" | "title" if self.frame.is_some() => self.in_desc = true,
            // The outline a frame's tight or through wrapping follows.
            "contour-polygon" if self.frame.is_some() => {
                if let Some(f) = &mut self.frame_float {
                    f.wrap_polygon = super::frames::contour_polygon(a);
                }
            }
            _ => {}
        }
    }

    /// The open frame's object at `href`: its formula, or else its chart, or else the object
    /// itself, kept whole (an OLE object's file, when `ole`).
    fn read_object(&mut self, href: &str, ole: bool) {
        if let Some(fr) = &mut self.frame
            && fr.4.is_none()
            && fr.5.is_none()
            && fr.6.is_none()
        {
            if !ole {
                let path = format!("{}/content.xml", href.trim_start_matches("./").trim_end_matches('/'));
                if let Some(xml) = (self.zip)(&path) {
                    fr.4 = read_formula(&xml);
                    if fr.4.is_none() {
                        fr.5 = super::chart::read_chart(&xml);
                    }
                }
            }
            // A chart is kept whole too, to be written back as it was while it is unchanged.
            if fr.4.is_none() {
                fr.6 = super::objects::gather(href, ole, self.entries, self.manifest, &mut *self.zip);
                // A replacement picture read before the object.
                if let Some(o) = &mut fr.6 {
                    o.replacement = fr.2.clone().map(std::sync::Arc::new);
                }
            }
        }
    }

    fn start_inline(&mut self, q: &str, a: &Attrs) -> bool {
        // Inside a field everything is its shown text.
        if self.field.is_some() {
            return true;
        }
        // A citation carries its source: the document's (the first by its identifier).
        if q == "text:bibliography-mark"
            && let Some(s) = super::fields::bib_source(a)
            && self.sources.len() < MAX_SOURCES
            && !self.sources.iter().any(|o| o.tag == s.tag)
        {
            self.sources.push(s);
        }
        // A variable given a value (§7.4.5 `text:variable-set`): a SET field, then, unless it is
        // hidden, a REF showing the value.
        if q == "text:variable-set"
            && self.frame.is_none()
            && let Some(set) = super::fields::set_instr(a)
        {
            let fmt = self.cur_fmt();
            self.push_inline(Inline::Field { instr: set, result: String::new(), fmt });
            if get(a, "text:display").map(str::trim) == Some("none") {
                self.skip = 1;
                return true;
            }
        }
        if q.starts_with("text:")
            && self.frame.is_none()
            && let Some(instr) = super::fields::instr_of(local(q), a, |n| self.styles.date_picture(n).map(String::from))
        {
            let anchor = (local(q) == "sequence")
                .then(|| get(a, "text:ref-name").map(str::trim).filter(|n| !n.is_empty() && n.len() <= 1024).map(String::from))
                .flatten();
            let fixed = get(a, "text:fixed").is_some_and(|v| v.trim() == "true");
            self.field = Some(OpenField { instr, result: String::new(), name: q.to_string(), depth: self.depth, fmt: self.cur_fmt(), anchor, fixed });
            return true;
        }
        match local(q) {
            "span" => {
                let base = self.cur_fmt();
                let fmt = get(a, "text:style-name").map_or_else(|| base.clone(), |s| self.styles.span_fmt(s, &base));
                self.fmt.push(fmt);
            }
            "a" => {
                let mut fmt = self.cur_fmt();
                if let Some(href) = get(a, "xlink:href").filter(|href| !href.is_empty()) {
                    fmt.link = model::kept_link(href);
                }
                if let Some(style) = get(a, "text:style-name") {
                    fmt = self.styles.text_fmt(style, &fmt);
                }
                self.fmt.push(fmt);
            }
            "s" => {
                let count = get(a, "text:c").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 1000);
                self.push_text(&" ".repeat(count));
            }
            "tab" => self.push_text("\t"),
            "line-break" => self.push_text("\n"),
            // A bookmark (§6.2.1): a point, or the start or end of the text it covers.
            // A reference mark (§7.6) is a bookmark too: references to it are Word's to bookmarks.
            // An index entry over text (§8.1.11–8.1.12): an `XE` field at its start, the entry the
            // marked text (in one paragraph).
            "alphabetical-index-mark-start" => {
                if let (Some(id), Some(p)) = (get(a, "text:id"), &self.para)
                    && self.index_marks.len() < MAX_OPEN_MARKS
                {
                    let keys = (get(a, "text:key1").map(String::from), get(a, "text:key2").map(String::from));
                    // Where it starts: after the last inline, or inside the text run the marked
                    // text will join.
                    let at = match p.inlines.last() {
                        Some(Inline::Text(t, _)) => (p.inlines.len() - 1, t.len()),
                        _ => (p.inlines.len(), 0),
                    };
                    self.index_marks.push((id.to_string(), keys, at));
                }
            }
            "alphabetical-index-mark-end" => {
                if let Some(at) = get(a, "text:id").and_then(|id| self.index_marks.iter().position(|(i, ..)| i == id)) {
                    let (_, (k1, k2), (from, off)) = self.index_marks.remove(at);
                    if let Some(p) = &mut self.para {
                        // The marked text: from the offset in the first run on.
                        let text: String = p
                            .inlines
                            .iter()
                            .enumerate()
                            .skip(from)
                            .filter_map(|(k, i)| match i {
                                Inline::Text(t, _) if k == from => t.get(off..),
                                Inline::Text(t, _) => Some(t.as_str()),
                                _ => None,
                            })
                            .collect();
                        if let Some(instr) = super::fields::xe_instr(k1.as_deref(), k2.as_deref(), &text) {
                            let field = Inline::Field { instr, result: String::new(), fmt: Fmt::default() };
                            // Inside a run: split it there.
                            let split = match p.inlines.get_mut(from) {
                                Some(Inline::Text(t, f)) if off > 0 && t.is_char_boundary(off) && off < t.len() => {
                                    let rest = t.split_off(off);
                                    Some(Inline::Text(rest, f.clone()))
                                }
                                _ => None,
                            };
                            match split {
                                Some(rest) => {
                                    p.inlines.insert(from + 1, rest);
                                    p.inlines.insert(from + 1, field);
                                }
                                None if off > 0 => p.inlines.insert((from + 1).min(p.inlines.len()), field),
                                None => p.inlines.insert(from.min(p.inlines.len()), field),
                            }
                        }
                    }
                }
            }
            "bookmark" | "bookmark-start" | "bookmark-end" | "reference-mark" | "reference-mark-start" | "reference-mark-end" => {
                if let Some(name) = get(a, "text:name").filter(|n| !n.is_empty()) {
                    let name = name.to_string();
                    self.push_inline(if local(q).ends_with("-end") { Inline::AnchorEnd(name) } else { Inline::Anchor(name) });
                }
            }
            _ => return false,
        }
        true
    }

    /// A list, list item or list header starting: true when handled.
    fn start_list_part(&mut self, q: &str, a: &Attrs) -> bool {
        if !q.starts_with("text:") {
            return false;
        }
        match local(q) {
            "list" => {
                self.end_para();
                if self.lists.is_empty() {
                    self.start_list_id(a);
                }
                if self.lists.len() < 64 {
                    self.lists.push((get(a, "text:style-name").map(String::from), false));
                    self.item_used.push(false);
                } else {
                    self.skip = 1;
                }
            }
            "list-item" | "list-header" => {
                // An item restarting the numbering begins a list of its own from there.
                if let Some(v) = get(a, "text:start-value").and_then(|v| v.trim().parse::<u32>().ok()) {
                    self.list_id = self.new_list_id();
                    self.last_list = Some(self.list_id);
                    self.item_start = Some(v.min(32_767));
                }
                if let Some(l) = self.lists.last_mut() {
                    l.1 = true;
                }
                if let Some(u) = self.item_used.last_mut() {
                    *u = false;
                }
            }
            _ => return false,
        }
        true
    }

    fn end(&mut self, q: &str) {
        self.depth = self.depth.saturating_sub(1);
        if self.end_note_or_field(q) {
            return;
        }
        if q == "office:text" {
            self.end_field();
            if self.note.is_some() {
                self.end_note();
            }
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
            if self.skip == 0
                && let Some(g) = self.geometry.take()
                && let Some(f) = g.freeform()
            {
                self.shape_kind = Some(wordcraft_doc::para::ShapeKind::Freeform);
                self.shape_path = Some(std::sync::Arc::new(f));
            }
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
                let mut blocks = if self.containers.len() > 1 { self.containers.pop().unwrap_or_default() } else { Vec::new() };
                if let Some(f) = self.formulas.pop().flatten() {
                    super::cell_formula::into_field(&mut blocks, f);
                }
                (self.lists, self.item_used, self.list_id) = self.lists_saved.pop().unwrap_or_default();
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
            "frame" => self.end_frame(),
            name if q.starts_with("text:") && is_index(name) && self.index.as_ref().is_some_and(|ix| ix.depth == self.depth + 1) => self.end_index(),
            "index-title" if q.starts_with("text:") => self.index_title(true),
            "g" if self.group_at == Some(self.depth + 1) => {
                self.group_at = None;
                self.end_group();
            }
            _ => {}
        }
    }
}
