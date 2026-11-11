use super::import::Pages;
use super::limits::{
    MAX_COMMENTS, MAX_EXTRA_SPACES, MAX_FIELD_RESULT, MAX_INFO_CHARS, MAX_OPEN_INSERTIONS, MAX_REGIONS, MAX_SECTIONS, MAX_XML_DEPTH,
};
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
    /// A watermark found (in a header): see [`Self::start_shape`].
    pub(super) watermark: Option<wordcraft_doc::Watermark>,
    /// The shape being read is a watermark's, if its geometry follows its text: its size, turn,
    /// colour and see-through, and whether its geometry is fontwork.
    watermark_shape: Option<((f32, f32), f32, Option<wordcraft_doc::props::Rgb>, bool, bool)>,
    pub(super) manifest: Option<&'a HashMap<String, String>>,
    containers: Vec<Vec<FBlock>>,
    /// Drawings anchored in the text but outside any paragraph (LibreOffice's page-anchored
    /// frames before the first one): the next paragraph's, so they don't make a paragraph of
    /// their own (which would start the document on another page style).
    loose: Vec<Inline>,
    /// The markup of each drawing in `loose`, as read, while each one's is (`loose_whole`): the
    /// paragraph they join keeps it before its own (see `feed_at`).
    loose_raw: Vec<std::sync::Arc<str>>,
    loose_whole: bool,
    /// The drawing outside a paragraph being read: where it starts, its depth, and how many
    /// drawings were waiting before it.
    loose_open: Option<(usize, usize, usize)>,
    fmt: Vec<Fmt>,
    /// For each paragraph being read: the size its heading, title or code style gives its text.
    para_sizes: Vec<(Option<f32>, bool)>,
    /// That size for the paragraph being finished.
    para_size: Option<f32>,
    para: Option<Para>,
    /// The text read last ended in a space collapsed from XML whitespace (not a `text:s`).
    xml_space_end: bool,
    /// What is left of [`MAX_EXTRA_SPACES`].
    pub(super) spaces_left: usize,
    para_cont: bool,
    /// Open lists: (style name, item open, item used).
    lists: Vec<(Option<String>, bool)>,
    item_used: Vec<bool>,
    /// Lists outside the table cells being read, and the list number there.
    lists_saved: Vec<(Vec<(Option<String>, bool)>, Vec<bool>, u32)>,
    tables: Vec<TableB>,
    cells: Vec<Cell>,
    /// How many containers each open cell's content makes open (its top level, where sections
    /// are kept around the blocks as in the body's).
    cell_tops: Vec<usize>,
    /// Each open cell's formula, as a Word formula field code (§19.646 `table:formula`).
    formulas: Vec<Option<String>>,
    pub(super) in_text: bool,
    skip: usize,
    depth: usize,
    /// The markup being read (`content.xml`), for keeping paragraphs' own (empty: none kept).
    pub(super) raw: &'a [u8],
    /// The paragraph whose markup is being kept (see [`KeptOpen`]).
    kept_open: Option<KeptOpen>,
    /// The body table whose markup is being kept (its `done`: the container's length before it).
    kept_table: Option<KeptOpen>,
    /// The body index (a table of contents…) whose markup is being kept, likewise.
    kept_index: Option<KeptOpen>,
    /// Sections hidden (§19.818 `text:display="none"`) open: their depth, how many containers were
    /// open, and the length of the one holding them, at their start.
    hidden_sections: Vec<(usize, usize, usize)>,
    /// A hidden section just read: the containers open and where its blocks start, hidden
    /// before the next event (when what ends with it has ended).
    hide_pending: Option<(usize, usize)>,
    /// The top-level list whose markup is being kept, likewise (its items' paragraphs all kept).
    kept_list: Option<KeptOpen>,
    /// Sections whose wrapper is kept as read (in the body's or a text box's top level): start
    /// tag, depth, the length of what holds it before it and how many containers are open;
    /// `None` for one read as a columned section (kept as a section break) or anywhere else.
    kept_sections: Vec<Option<(std::sync::Arc<str>, usize, usize, usize)>>,
    /// Each section's master page, as the paragraph or table starting it named it (`None`: as
    /// the file's default, or a columned section), in order; and the one just named.
    pub(super) section_masters: Vec<Option<String>>,
    pending_master: Option<String>,
    /// Paragraphs finished so far (a kept paragraph must come out of its element whole).
    paras_done: usize,
    /// `(width, height, picture bytes, alt text, equation read from an embedded `draw:object`)`.
    frame: Option<Frame>,
    /// The first picture the open frame offers that WordCraft can't show: an object coming after it
    /// keeps it as its replacement.
    frame_unread: Option<Vec<u8>>,
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
    /// That geometry's markup as read (see `ShapeExtra::odf_geometry`): being read, then
    /// finished for the shape it ends in.
    geometry_xml: Option<String>,
    pub(in crate::odt) shape_geometry: Option<std::sync::Arc<String>>,
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
            watermark: None,
            watermark_shape: None,
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
            loose_raw: Vec::new(),
            loose_whole: true,
            loose_open: None,
            fmt: Vec::new(),
            para: None,
            xml_space_end: false,
            spaces_left: MAX_EXTRA_SPACES,
            para_cont: false,
            para_sizes: Vec::new(),
            para_size: None,
            lists: Vec::new(),
            lists_saved: Vec::new(),
            item_used: Vec::new(),
            tables: Vec::new(),
            cells: Vec::new(),
            cell_tops: Vec::new(),
            formulas: Vec::new(),
            in_text: false,
            skip: 0,
            depth: 0,
            raw: &[],
            kept_open: None,
            kept_table: None,
            kept_index: None,
            kept_list: None,
            hidden_sections: Vec::new(),
            hide_pending: None,
            kept_sections: Vec::new(),
            section_masters: vec![None],
            pending_master: None,
            paras_done: 0,
            frame: None,
            frame_unread: None,
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
            geometry_xml: None,
            shape_geometry: None,
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

    /// [`Body::feed_at`] for a section hidden (§19.818 `text:display="none"`): what it holds is
    /// hidden text, so it stays hidden written without the section (see [`Body::hide_read`]).
    fn hidden_section(&mut self, ev: &Ev) {
        match ev {
            Ev::Start(q, a) if q == "text:section" && get(a, "text:display").map(str::trim) == Some("none") && self.hidden_sections.len() < 64 => {
                self.end_para();
                let at = (self.depth + 1, self.containers.len(), self.container().len());
                self.hidden_sections.push(at);
            }
            Ev::End(q) if q == "text:section" && self.hidden_sections.last().is_some_and(|h| h.0 == self.depth) => {
                if let Some((_, open, from)) = self.hidden_sections.pop() {
                    self.hide_pending = Some((open, from));
                }
            }
            _ => {}
        }
    }

    /// The hidden section just read made hidden text. Its blocks are written from the model,
    /// hidden: markup kept as read doesn't say so itself (its section might not be written
    /// around it).
    pub(super) fn hide_read(&mut self) {
        if let Some((open, from)) = self.hide_pending.take()
            && self.containers.len() == open
        {
            hide(self.container().get_mut(from..).unwrap_or_default(), 0);
        }
    }

    /// [`Body::feed_at`] for a top-level list (§5.3.1): kept whole, as read, when each of its
    /// paragraphs is (a paragraph's own markup can't say how its list numbers and draws its
    /// labels: a label followed by a space, a picture bullet, an unnumbered header). Not one
    /// continuing another by its `xml:id`, which WordCraft may write anew (one without its own
    /// is given one, for a list continuing it to name). `None` when it took the event.
    fn whole_list(&mut self, ev: Ev, at: (usize, usize)) -> Option<Ev> {
        match &ev {
            Ev::Start(q, a)
                if q == "text:list"
                    && self.kept_list.is_none()
                    && self.lists.is_empty()
                    && self.para.is_none()
                    && get(a, "text:continue-list").is_none()
                    && !self.raw.is_empty()
                    && self.in_text
                    && self.skip == 0
                    && self.del.is_none()
                    && self.loose.is_empty()
                    && self.at_body_level() =>
            {
                self.end_para();
                let done = self.container().len();
                self.kept_list =
                    Some(KeptOpen { from: at.0, depth: self.depth + 1, clean: true, done, comments: KeptComments::default(), prefix: String::new() });
                Some(ev)
            }
            Ev::End(q) if q == "text:list" && self.kept_list.as_ref().is_some_and(|k| k.depth == self.depth) => {
                let k = self.kept_list.take();
                self.feed(ev);
                if let Some(k) = k
                    && self.containers.len() == 1
                    && let Some(xml) = self.raw.get(k.from..at.1).and_then(|b| std::str::from_utf8(b).ok())
                {
                    let blocks = self.container();
                    let n = blocks.len().saturating_sub(k.done);
                    let whole = |b: &FBlock| matches!(b, FBlock::Para(p) if p.kept.is_some() && p.index.is_none() && p.wrap.is_none());
                    if n > 0
                        && blocks.get(k.done..).is_some_and(|r| r.iter().all(whole))
                        && let Some(FBlock::Para(p)) = blocks.get_mut(k.done)
                    {
                        p.index = Some(Box::new((std::sync::Arc::from(xml), n)));
                    }
                }
                None
            }
            _ => Some(ev),
        }
    }

    /// [`Body::feed_at`] for a drawing outside a paragraph (a page-anchored frame in the text):
    /// its markup kept, for the paragraph it joins. `None` when it took the event.
    fn loose_drawing(&mut self, ev: Ev, at: (usize, usize)) -> Option<Ev> {
        match &ev {
            Ev::Start(q, _)
                if self.loose_open.is_none()
                    && q.starts_with("draw:")
                    && is_drawing(q)
                    && self.para.is_none()
                    && self.kept_open.is_none()
                    && !self.raw.is_empty()
                    && self.in_text
                    && self.skip == 0
                    && self.at_body_level() =>
            {
                self.loose_open = Some((at.0, self.depth + 1, self.loose.len()));
            }
            Ev::End(_) if self.loose_open.is_some_and(|(_, d, _)| d == self.depth) => {
                self.feed(ev);
                if let Some((from, _, before)) = self.loose_open.take() {
                    match self.raw.get(from..at.1).and_then(|b| std::str::from_utf8(b).ok()) {
                        Some(xml) if self.loose.len() == before + 1 => self.loose_raw.push(std::sync::Arc::from(xml)),
                        _ if self.loose.len() == before => {}
                        _ => self.loose_whole = false,
                    }
                }
                return None;
            }
            _ => {}
        }
        Some(ev)
    }

    /// [`Body::feed`], `at` the event's markup in [`Body::raw`]: a body paragraph that comes out
    /// whole and refers to nothing outside itself keeps its markup (`Para::kept`), so a save
    /// that doesn't change it writes it as it was (see `wordcraft_doc::kept`).
    pub(super) fn feed_at(&mut self, ev: Ev, at: (usize, usize)) {
        self.hide_read();
        self.hidden_section(&ev);
        // A body index, kept whole: its settings and templates, which WordCraft writes its own.
        match &ev {
            Ev::Start(q, _) => match &mut self.kept_index {
                Some(k) => k.clean &= !(q.starts_with("draw:") || q.starts_with("office:annotation") || q.starts_with("text:change")),
                None if q.starts_with("text:")
                    && is_index(local(q))
                    && !self.raw.is_empty()
                    && self.in_text
                    && self.skip == 0
                    && self.containers.len() == 1
                    && self.tables.is_empty()
                    && self.frame.is_none()
                    && self.note.is_none()
                    && self.del.is_none() =>
                {
                    self.end_para();
                    self.kept_index = Some(KeptOpen {
                        from: at.0,
                        depth: self.depth + 1,
                        clean: true,
                        done: self.container().len(),
                        comments: KeptComments::default(),
                        prefix: String::new(),
                    });
                }
                None => {}
            },
            Ev::End(q) if q.starts_with("text:") && is_index(local(q)) && self.kept_index.as_ref().is_some_and(|k| k.depth == self.depth) => {
                let k = self.kept_index.take();
                self.feed(ev);
                if let Some(k) = k
                    && k.clean
                    && self.containers.len() == 1
                    && let Some(xml) = self.raw.get(k.from..at.1).and_then(|b| std::str::from_utf8(b).ok())
                {
                    let blocks = self.container();
                    let n = blocks.len().saturating_sub(k.done);
                    if n > 0
                        && blocks.get(k.done..).is_some_and(|r| r.iter().all(|b| matches!(b, FBlock::Para(_))))
                        && let Some(FBlock::Para(p)) = blocks.get_mut(k.done)
                    {
                        p.index = Some(Box::new((std::sync::Arc::from(xml), n)));
                    }
                }
                return;
            }
            _ => {}
        }
        // A body section: its start tag kept, put around its paragraphs at its end.
        match &ev {
            Ev::Start(q, _) if q == "text:section" && self.kept_sections.len() < 64 => {
                let body = !self.raw.is_empty()
                    && self.in_text
                    && self.skip == 0
                    && self.frame.is_none()
                    && self.del.is_none()
                    && self.lists.is_empty()
                    && self.at_story_top();
                let tag = self.raw.get(at.0..at.1).and_then(|b| std::str::from_utf8(b).ok()).filter(|t| !t.ends_with("/>")).map(std::sync::Arc::from);
                let depth = self.depth + 1;
                let cols = self.cols_at;
                self.feed(ev);
                // Read as a columned section (a section break): no wrapper besides.
                let columned = self.cols_at != cols;
                let len = self.container().len();
                let open = self.containers.len();
                self.kept_sections.push(tag.filter(|_| body && !columned).map(|t| (t, depth, len, open)));
                return;
            }
            Ev::End(q) if q == "text:section" && !self.kept_sections.is_empty() => {
                let depth = self.depth;
                self.feed(ev);
                if let Some(Some((tag, _, from, open))) = self.kept_sections.pop().filter(|s| s.as_ref().is_none_or(|s| s.1 == depth))
                    && self.containers.len() == open
                    && self.at_story_top()
                {
                    // Around top-level paragraphs and tables: the first opens it, the last closes it.
                    let blocks = self.container();
                    // (A list item too: the list its items are written as opens and closes them,
                    // a section boundary ending it.)
                    if blocks.len() > from {
                        if let Some(b) = blocks.get_mut(from) {
                            block_wrap(b).0.insert(0, tag);
                        }
                        if let Some(b) = blocks.last_mut() {
                            let w = block_wrap(b);
                            w.1 = w.1.saturating_add(1);
                        }
                    }
                }
                return;
            }
            _ => {}
        }
        // A body table, kept whole as the paragraphs are.
        match &ev {
            Ev::Start(q, _) => match &mut self.kept_table {
                Some(k) => k.clean &= keepable(q),
                None if q == "table:table"
                    && self.tables.is_empty()
                    && !self.raw.is_empty()
                    && self.in_text
                    && self.skip == 0
                    && self.note.is_none()
                    && self.frame.is_none()
                    && self.del.is_none()
                    && self.field.is_none()
                    && self.loose.is_empty() =>
                {
                    // Before it starts: what ends the paragraph open before it.
                    self.end_para();
                    self.kept_table = Some(KeptOpen {
                        from: at.0,
                        depth: self.depth + 1,
                        clean: true,
                        done: self.container().len(),
                        comments: KeptComments::default(),
                        prefix: String::new(),
                    });
                }
                None => {}
            },
            Ev::End(q) if q == "table:table" && self.kept_table.as_ref().is_some_and(|k| k.depth == self.depth) => {
                let k = self.kept_table.take();
                self.feed(ev);
                if let Some(k) = k
                    && k.clean
                    && let Some(xml) = self.raw.get(k.from..at.1).and_then(|b| std::str::from_utf8(b).ok())
                    && self.container().len() == k.done + 1
                    && let Some(FBlock::Table(t)) = self.container().last_mut()
                {
                    t.kept = Some(std::sync::Arc::from(xml));
                }
                return;
            }
            _ => {}
        }
        let Some(ev) = self.loose_drawing(ev, at) else { return };
        let Some(ev) = self.whole_list(ev, at) else { return };
        match &ev {
            Ev::Start(q, a) => match &mut self.kept_open {
                Some(k) => {
                    // Comments and tracked changes paired in it.
                    let paired = q.starts_with("office:annotation")
                        || matches!(q.as_str(), "text:change" | "text:change-start" | "text:change-end")
                        || mark_pair(q).is_some();
                    k.clean &= (paired || keepable(q)) && k.comments.start(q, a, self.depth + 1, self.paras_done);
                }
                None if (q == "text:p" || q == "text:h")
                    && !self.raw.is_empty()
                    && self.in_text
                    && self.skip == 0
                    && self.para.is_none()
                    && self.note.is_none()
                    && self.frame.is_none()
                    && self.del.is_none()
                    && self.field.is_none()
                    && !self.in_desc =>
                {
                    // Drawings read before it that join it: their markup before its own, when
                    // each one's was kept.
                    let clean = self.loose.is_empty() || (self.loose_whole && self.loose_raw.len() == self.loose.len());
                    let prefix = if self.loose.is_empty() { String::new() } else { self.loose_raw.concat() };
                    self.kept_open =
                        Some(KeptOpen { from: at.0, depth: self.depth + 1, clean, done: self.paras_done, comments: KeptComments::default(), prefix });
                }
                None => {}
            },
            // A comment in it ending: its paragraphs counted (once read, its last included).
            Ev::End(_) if self.kept_open.as_ref().is_some_and(|k| k.comments.ends_at(self.depth)) => {
                self.feed(ev);
                let done = self.paras_done;
                if let Some(k) = &mut self.kept_open {
                    k.comments.end(done);
                }
                return;
            }
            Ev::End(q) if (q == "text:p" || q == "text:h") && self.kept_open.as_ref().is_some_and(|k| k.depth == self.depth) => {
                if let Some(k) = self.kept_open.take()
                    && k.clean
                    && k.comments.closed()
                    && k.done + k.comments.paras == self.paras_done
                    && let Some(p) = &mut self.para
                    && let Some(xml) = self.raw.get(k.from..at.1).and_then(|b| std::str::from_utf8(b).ok())
                {
                    p.kept = Some(std::sync::Arc::from(format!("{}{xml}", k.prefix)));
                    p.group = k.comments.group();
                }
            }
            _ => {}
        }
        self.feed(ev);
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
        self.hide_read();
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
    /// Whether what is read goes in the body's top level, a text box's or a table cell's (not a
    /// note's or a comment's): where sections are kept around the blocks.
    fn at_story_top(&self) -> bool {
        // A table cell's (the innermost one's).
        if let Some(&top) = self.cell_tops.last()
            && self.containers.len() == top
        {
            return true;
        }
        match &self.note {
            None => self.containers.len() == 1 && self.tables.is_empty(),
            Some(n) => n.text_box.is_some() && self.containers.len() == n.containers + 1 && self.tables.len() == n.tables,
        }
    }

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

    /// The size of the text being read, points (its own, else the document's).
    pub(in crate::odt) fn text_size(&self) -> f32 {
        self.cur_fmt().size.or_else(|| self.styles.doc_size()).unwrap_or(12.0)
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
            // The open section, empty, takes its place: its master page too.
            if let Some(m) = self.pending_master.take()
                && let Some(last) = self.section_masters.last_mut()
            {
                *last = Some(m);
            }
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
        self.section_masters.push(self.pending_master.take());
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
        self.pending_master = master.map(str::to_string);
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
        // A heading looks as its style says (a Heading 2 style without a level attribute, which
        // makes it level 1, still looks as Heading 2); its outline level is the attribute's.
        let mut level = None;
        if let Some(h) = heading {
            match kind {
                Kind::Heading(k) if k != h.clamp(1, 6) => level = Some(h.clamp(1, 10) - 1),
                _ => kind = Kind::Heading(h.clamp(1, 6)),
            }
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
        if let Some(l) = level {
            p.extra.get_or_insert_with(Default::default).outline_level = Some(l);
        }
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
        let kind_size = matches!(kind, Kind::Code | Kind::Heading(_) | Kind::Title).then(|| style.and_then(|s| self.styles.named_size(s))).flatten();
        self.para_sizes.push((kind_size, style.is_some_and(|s| self.styles.states_size(s))));
        self.fmt.push(base);
        // Drawings waiting outside a paragraph come first in it, when it is the text's own (not
        // a text box's, a note's or a cell's).
        if !self.loose.is_empty() && self.at_body_level() {
            let mut loose = std::mem::take(&mut self.loose);
            loose.append(&mut p.inlines);
            p.inlines = loose;
            self.loose_raw.clear();
            self.loose_whole = true;
        }
        self.para = Some(p);
    }

    fn end_para(&mut self) {
        // An index entry is marked within one paragraph.
        self.index_marks.clear();
        let Some(mut p) = self.para.take() else { return };
        self.glue_connectors(&mut p);
        self.para_size = self.para_sizes.pop().and_then(|(size, stated)| size.filter(|_| stated));
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
            // The style says the size of its text, stated again or not: text of another size keeps
            // its own.
            let style_size = self.para_size;
            for i in &mut p.inlines {
                if let Inline::Text(_, f) = i {
                    if p.kind == Kind::Code {
                        f.code = false;
                        f.font = None;
                    }
                    if f.size == style_size {
                        f.size = None;
                    }
                }
            }
        }
        self.paras_done += 1;
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
            // Two elements made one paragraph: neither's markup is it.
            prev.kept = None;
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
            if self.skip == 1
                && (q == "draw:equation" || q == "draw:handle")
                && let Some(xml) = &mut self.geometry_xml
            {
                xml.push_str(&element_xml(q, a, true));
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
            if let Some(wm) = &mut self.watermark_shape {
                wm.4 = get(a, "draw:text-path").is_some_and(|v| v.trim() == "true") || t.starts_with("fontwork");
            }
            self.shape_kind = Some(super::shapes::kind_of(t));
            self.geometry = super::geometry::Pending::of(t, a);
            // Kept as read, its mirroring aside (written from the shape's own).
            let kept: Attrs = a.iter().filter(|(k, _)| !k.starts_with("draw:mirror-")).cloned().collect();
            self.geometry_xml = (self.geometry.is_some() || super::shapes::unknown_preset(t)).then(|| element_xml(q, &kept, false));
            self.shape_geometry = None;
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
                        page: get(a, "text:anchor-page-number").map(String::from),
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
                let share = self.styles.column_share(get(a, "table:style-name")).unwrap_or(0.0);
                let cells = get(a, "table:default-cell-style-name").and_then(|s| self.styles.cell_background(s));
                let n = get(a, "table:number-columns-repeated").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).max(1);
                if let Some(t) = self.tables.last_mut() {
                    let room = wordcraft_doc::table::MAX_COLS.saturating_sub(t.widths.len());
                    t.widths.extend(std::iter::repeat_n(w, n.min(room)));
                    t.shares.extend(std::iter::repeat_n(share, n.min(room)));
                    t.column_backgrounds.extend(std::iter::repeat_n(cells, n.min(room)));
                }
            }
            "table-row" => self.start_row(a),
            "table-cell" => {
                self.end_para();
                let span = get(a, "table:number-columns-spanned").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 63);
                let rows = get(a, "table:number-rows-spanned").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 1000);
                // WordCraft's tables have their backgrounds on their cells: a row's or table's on
                // those with none of their own, as it shows behind them. A cell with no style has
                // its column's default cell style; LibreOffice doesn't apply a row's.
                let behind = self.tables.last().and_then(|t| t.row_background.or(t.background));
                let in_column = self.tables.last().and_then(|t| {
                    let at: usize = t.row.iter().map(|c| c.colspan.max(1) as usize).sum();
                    t.column_backgrounds.get(at).copied().flatten()
                });
                let own = get(a, "table:style-name").map_or(in_column, |s| self.styles.cell_background(s));
                let shading = own.or(behind);
                let valign = get(a, "table:style-name").and_then(|s| self.styles.cell_valign(s));
                let borders = get(a, "table:style-name").and_then(|s| self.styles.cell_borders(s));
                let margins = get(a, "table:style-name").and_then(|s| self.styles.cell_padding(s));
                let direction = get(a, "table:style-name").map(|s| self.styles.cell_direction(s)).unwrap_or_default();
                let picture = get(a, "table:style-name").and_then(|s| self.styles.cell_picture(s)).and_then(|p| self.cell_picture(p));
                if let Some(t) = self.tables.last_mut() {
                    t.hcover = span as usize - 1;
                }
                self.cells.push(Cell { colspan: span, rowspan: rows, shading, valign, borders, margins, direction, picture, ..Default::default() });
                self.formulas.push(get(a, "table:formula").and_then(super::cell_formula::word_of));
                self.containers.push(Vec::new());
                self.cell_tops.push(self.containers.len());
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
                // What a covered cell holds isn't shown (§9.1.5): skipped (it went before the table).
                self.skip = 1;
            }
            "frame" => {
                let w = get(a, "svg:width").and_then(length);
                let h = get(a, "svg:height").and_then(length);
                self.frame = Some((w, h, None, String::new(), None, None, None));
                self.frame_unread = None;
                let g = get(a, "draw:style-name").map(|s| self.styles.graphic(s)).unwrap_or_default();
                self.frame_float = Some(super::frames::place(a, w.unwrap_or(0.0), h.unwrap_or(0.0), &g, self.text_size()));
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
                    let balance = get(a, "text:style-name").and_then(|s| self.styles.section_balance(s));
                    let next = SectionProps {
                        start: SectionStart::Continuous,
                        page_num_start: None,
                        columns,
                        balance_columns: balance,
                        ..self.page.clone()
                    };
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
        // What the shares divide: an aligned table's stated width, however wide, as LibreOffice
        // keeps it; else its part of the text width.
        let room = width.filter(|_| align.is_some() && rel.is_none()).or_else(|| width_pct.map(|p| self.sec.text_width() * p / 100.0));
        let contained = !self.tables.is_empty() || self.containers.len() > 1 || self.sec.columns.count > 1;
        let extra =
            model::TableExtra { align, width: None, width_pct, heights: Vec::new(), keep_rows: Vec::new(), fixed: true, space, header_rows: 0 };
        let background = get(a, "table:style-name").and_then(|s| self.styles.table_background(s));
        self.tables.push(TableB {
            rows: Vec::new(),
            row: Vec::new(),
            hcover: 0,
            widths: Vec::new(),
            shares: Vec::new(),
            room,
            contained,
            column_backgrounds: Vec::new(),
            extra,
            background,
            row_background: None,
        });
    }

    /// A table row (`table:table-row`) begins: its height, kept as rows are (see the row's end).
    fn start_row(&mut self, a: &Attrs) {
        let height = get(a, "table:style-name").and_then(|s| self.styles.row_height(s));
        let background = get(a, "table:style-name").and_then(|s| self.styles.table_background(s));
        let keep = self.styles.row_keep(get(a, "table:style-name"));
        if let Some(t) = self.tables.last_mut() {
            t.row_background = background;
            t.row.clear();
            t.hcover = 0;
            if t.extra.heights.len() < wordcraft_doc::table::MAX_ROWS {
                t.extra.heights.truncate(t.rows.len());
                t.extra.heights.push(height);
                t.extra.keep_rows.truncate(t.rows.len());
                t.extra.keep_rows.push(keep);
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
                // Where it stands on the line (§20.387), as an in-line frame does.
                let g = self.styles.shape_graphic(get(a, "draw:style-name"));
                let raise = super::frames::place(a, w, h, &g, self.text_size()).y;
                control.form_raise = (raise.is_finite() && raise != 0.0).then_some(raise);
            }
            let fmt = self.cur_fmt();
            self.push_inline(Inline::Control { control: Box::new(control), text, fmt });
        }
    }

    /// Part of a frame: its picture, embedded object, description or contour.
    fn frame_part(&mut self, q: &str, a: &Attrs) {
        match local(q) {
            "image" => {
                // A chart's replacement picture (what readers that can't draw it show): kept with
                // the chart as read, written back with it while it is unchanged.
                if let Some(href) = get(a, "xlink:href")
                    && let Some(fr) = &mut self.frame
                    && fr.5.is_some()
                    && let Some(o) = fr.6.as_mut().filter(|o| o.replacement.is_none())
                {
                    o.replacement = (self.zip)(href.trim_start_matches("./")).map(std::sync::Arc::new);
                    return;
                }
                if let Some(href) = get(a, "xlink:href")
                    && let Some(fr) = &mut self.frame
                    && fr.2.is_none()
                    && fr.4.is_none()
                    && fr.5.is_none()
                {
                    let path = href.trim_start_matches("./");
                    // A frame may offer one picture in several formats, in order of preference
                    // (LibreOffice writes its own `.svm` metafile before a PNG): the first
                    // WordCraft can read. An object's replacement is the first, kept whatever it
                    // is, also when the object is still to come.
                    if let Some(d) = (self.zip)(path) {
                        let readable = drawing::readable_picture(&d);
                        match &mut fr.6 {
                            Some(o) if o.replacement.is_none() => o.replacement = Some(std::sync::Arc::new(d.clone())),
                            Some(_) => {}
                            None if !readable => {
                                self.frame_unread.get_or_insert_with(|| d.clone());
                            }
                            None => {}
                        }
                        if readable {
                            fr.2 = Some(d);
                        }
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
                // A replacement picture read before the object: the first the frame offered.
                if let Some(o) = &mut fr.6 {
                    o.replacement = self.frame_unread.take().or_else(|| fr.2.clone()).map(std::sync::Arc::new);
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
            self.push_inline(Inline::Field { instr: set, result: String::new(), fmt, locked: false });
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
                // A size in percent is of the size it is in: its heading's, where nothing nearer
                // states one.
                let sized = Fmt { size: base.size.or(self.para_sizes.last().and_then(|(size, _)| *size)), ..base.clone() };
                let fmt = get(a, "text:style-name").map_or_else(
                    || base.clone(),
                    |s| {
                        let mut fmt = self.styles.span_fmt(s, &sized);
                        // That size is only for working out a percent: saying nothing of size, it has none.
                        if base.size.is_none() && self.styles.span_fmt(s, &base).size.is_none() {
                            fmt.size = None;
                        }
                        fmt
                    },
                );
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
            // Outside a paragraph `push_text` drops spaces, so they cost no budget.
            "s" if self.para.is_some() => {
                let count = get(a, "text:c").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 1000);
                let want = count.saturating_sub(1);
                let extra = want.min(self.spaces_left);
                if extra < want {
                    model::dropped(model::Dropped::Spaces);
                }
                self.spaces_left -= extra;
                self.push_text(&" ".repeat(1 + extra));
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
                            let field = Inline::Field { instr, result: String::new(), fmt: Fmt::default(), locked: false };
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

    /// A cell's background picture from its package path, `style:repeat` (§20.349: no-repeat,
    /// repeat, stretch; unstated, repeat) and `style:position` (§20.335: two of left, center,
    /// right, top, bottom; unstated, center).
    fn cell_picture(&mut self, (href, repeat, position): (String, String, String)) -> Option<Box<model::CellPic>> {
        let data = (self.zip)(href.trim_start_matches("./"))?;
        let img = model::make_img(data, None, None, "")?;
        let repeat = match repeat.as_str() {
            "no-repeat" => wordcraft_doc::props::PictureRepeat::None,
            "stretch" => wordcraft_doc::props::PictureRepeat::Stretch,
            _ => wordcraft_doc::props::PictureRepeat::Tile,
        };
        let mut align = [0.5, 0.5];
        for word in position.split_whitespace() {
            match word {
                "left" => align[0] = 0.0,
                "right" => align[0] = 1.0,
                "top" => align[1] = 0.0,
                "bottom" => align[1] = 1.0,
                _ => {}
            }
        }
        Some(Box::new(model::CellPic { img, repeat, align }))
    }

    /// A skipped element ends; the end of a custom shape's geometry works out its path and
    /// keeps the geometry as read.
    fn end_skipped(&mut self) {
        self.skip -= 1;
        if self.skip > 0 {
            return;
        }
        // Not a custom shape's geometry (a covered cell, say): nothing more.
        if self.geometry.is_none() && self.geometry_xml.is_none() {
            return;
        }
        let xml = self.geometry_xml.take();
        let keep = |x: Option<String>| x.filter(|x| x.len() < MAX_GEOMETRY_XML).map(|x| std::sync::Arc::new(format!("{x}</draw:enhanced-geometry>")));
        if let Some(g) = self.geometry.take()
            && let Some(f) = g.freeform()
        {
            self.shape_kind = Some(wordcraft_doc::para::ShapeKind::Freeform);
            self.shape_path = Some(std::sync::Arc::new(f));
            self.shape_geometry = keep(xml);
        } else if self.shape_kind == Some(wordcraft_doc::para::ShapeKind::Rectangle) {
            // A preset WordCraft draws as a rectangle: its geometry kept as read.
            self.shape_geometry = keep(xml);
        }
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
            self.end_skipped();
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
                self.cell_tops.pop();
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

/// The most markup a custom shape's kept geometry may take (see `ShapeExtra::odf_geometry`).
const MAX_GEOMETRY_XML: usize = 256 * 1024;

/// Element `q` with attributes `a` as markup: empty (`<q …/>`) or an opening tag.
fn element_xml(q: &str, a: &Attrs, empty: bool) -> String {
    let attrs: String = a.iter().map(|(k, v)| format!(" {k}=\"{}\"", super::writer::x(v))).collect();
    format!("<{q}{attrs}{}>", if empty { "/" } else { "" })
}

/// A body paragraph whose markup is being kept: where its element starts in the markup, its
/// depth, whether everything in it so far can be kept, and how many paragraphs were finished
/// before it.
struct KeptOpen {
    from: usize,
    depth: usize,
    clean: bool,
    done: usize,
    /// The comments in a paragraph being kept (see [`KeptComments`]).
    comments: KeptComments,
    /// The markup of drawings read before it that join it, kept before its own.
    prefix: String,
}

/// The comments (§14.1 `office:annotation`), tracked changes (§5.5.7), index and reference
/// marks over text in a paragraph being kept, and the text boxes it shows. Begun and ended in
/// it, they are its own; one reaching into another paragraph (a comment ending there, a
/// change, a box continuing in another's) ties it to that one, as a link of its [`Group`]
/// (`model::Group`): the paragraphs are kept as read together, or not at all.
#[derive(Default)]
struct KeptComments {
    /// The keys of those begun, and of those ended.
    opened: Vec<String>,
    ended: Vec<String>,
    /// What ties the paragraph to others: `(key, needed)` (see `model::Group::links`).
    links: Vec<(String, bool)>,
    /// The comment or drawing being read: its depth and how many paragraphs were finished
    /// before it.
    open: Option<(usize, usize)>,
    /// The comments' and drawings' own paragraphs, which aren't the paragraph's.
    paras: usize,
}

/// Most pairs and links a paragraph kept as read has (hostile input).
const MAX_PAIRS: usize = 64;

impl KeptComments {
    /// Element `q` (`a` its attributes, at `depth`) starting in the paragraph, `done`
    /// paragraphs finished so far: false when the paragraph can't be kept for it.
    fn start(&mut self, q: &str, a: &Attrs, depth: usize, done: usize) -> bool {
        if self.opened.len() + self.links.len() >= MAX_PAIRS {
            return false;
        }
        let comment = |n: &str| format!("\u{3}{n}");
        match q {
            // A comment in a drawing's text (its own story): not kept.
            "office:annotation" if self.open.is_some() => false,
            "office:annotation" => {
                // A reply to one in another paragraph needs it.
                if let Some(p) = get(a, "loext:parent-name").map(comment)
                    && !self.opened.contains(&p)
                {
                    self.links.push((p, true));
                }
                if let Some(n) = get(a, "office:name").map(comment) {
                    self.opened.push(n);
                }
                self.open = Some((depth, done));
                true
            }
            "office:annotation-end" => match get(a, "office:name").map(comment) {
                Some(n) => {
                    self.end_or_need(n);
                    true
                }
                None => false,
            },
            "text:change-start" => match get(a, "text:change-id") {
                Some(i) => {
                    self.opened.push(format!("\u{0}{i}"));
                    true
                }
                None => false,
            },
            "text:change-end" => match get(a, "text:change-id") {
                Some(i) => {
                    self.end_or_need(format!("\u{0}{i}"));
                    true
                }
                None => false,
            },
            "text:change" => get(a, "text:change-id").is_some(),
            // An index or reference mark over text (§6.6.2, §8.1.10).
            q if mark_pair(q).is_some() => {
                let Some((kind, attr, start)) = mark_pair(q) else { return false };
                let Some(key) = get(a, attr).map(|k| format!("\u{2}{kind}\u{2}{k}")) else { return false };
                if start {
                    self.opened.push(key);
                } else {
                    self.end_or_need(key);
                }
                true
            }
            // A text box continuing in another (§19.112 `draw:chain-next-name`) needs it.
            "draw:text-box" => {
                if let Some(next) = get(a, "draw:chain-next-name").map(str::trim).filter(|n| !n.is_empty()) {
                    self.links.push((format!("\u{4}{next}"), true));
                }
                true
            }
            // A drawing or a note: the paragraphs of its text (a text box's, a shape's, the
            // note's) are its story's. A named frame is there for a box continuing in it.
            q if self.open.is_none() && (q == "text:note" || q.starts_with("draw:") && is_drawing(q)) => {
                if q == "draw:frame"
                    && let Some(n) = get(a, "draw:name").map(str::trim).filter(|n| !n.is_empty())
                {
                    self.links.push((format!("\u{4}{n}"), false));
                }
                self.open = Some((depth, done));
                true
            }
            _ => true,
        }
    }

    /// `key` ended: one begun in the paragraph, or one begun in another, needed.
    fn end_or_need(&mut self, key: String) {
        if self.opened.contains(&key) {
            self.ended.push(key);
        } else {
            self.links.push((key, true));
        }
    }

    /// Whether the comment or drawing being read is the element ending at `depth`.
    fn ends_at(&self, depth: usize) -> bool {
        self.open.is_some_and(|(d, _)| d == depth)
    }

    /// The comment being read ended, `done` paragraphs finished so far.
    fn end(&mut self, done: usize) {
        if let Some((_, before)) = self.open.take() {
            self.paras += done.saturating_sub(before);
        }
    }

    /// Whether nothing is left open in it (a comment being read); and its links: what it begun
    /// and didn't end needs the paragraph ending it, and what it begun is there for another
    /// to need (a reply, an end).
    fn closed(&self) -> bool {
        self.open.is_none()
    }

    /// The paragraph's [`Group`](model::Group) links, when it has any.
    fn group(&self) -> Option<Box<model::Group>> {
        let mut links = self.links.clone();
        for k in &self.opened {
            links.push((k.clone(), !self.ended.contains(k)));
        }
        (!links.is_empty()).then(|| Box::new(model::Group { links, id: None }))
    }
}

/// Whether markup holding element `q` can be kept as read. Pictures, shapes and text boxes can
/// (their text is a story of its own, kept while it is as read: see `KeptComments`), and
/// embedded objects (a formula's text and a chart's data are the paragraph's, an object's files
/// are kept with it), plugins and floating frames; form controls (named afresh in the forms)
/// can't, nor
/// marks that pair up with others elsewhere by ids the writer gives (comments and tracked
/// changes are paired apart). Bookmarks can: their names are kept, so either end may be
/// written as read and the other from the model.
fn keepable(q: &str) -> bool {
    !(q == "draw:control"
        || q.starts_with("office:annotation")
        || q.starts_with("text:change")
        || q.starts_with("form:")
        || (q.ends_with("-start") || q.ends_with("-end")) && !q.starts_with("text:bookmark-"))
}

/// `blocks` hidden: their text and paragraph marks, and those in their tables' cells.
fn hide(blocks: &mut [FBlock], depth: usize) {
    let hidden = |f: &mut Fmt| f.extra.get_or_insert_with(Default::default).hidden = Some(true);
    for b in blocks {
        match b {
            FBlock::Para(p) => {
                for i in &mut p.inlines {
                    match i {
                        Inline::Text(_, f)
                        | Inline::Control { fmt: f, .. }
                        | Inline::Ruby { fmt: f, .. }
                        | Inline::Field { fmt: f, .. }
                        | Inline::Note { fmt: f, .. } => {
                            hidden(f);
                        }
                        _ => {}
                    }
                }
                p.mark.get_or_insert_with(Default::default).hidden = Some(true);
                (p.kept, p.index) = (None, None);
            }
            FBlock::Table(t) if depth < model::MAX_DEPTH => {
                t.kept = None;
                for c in t.rows.iter_mut().flatten() {
                    hide(&mut c.blocks, depth + 1);
                }
            }
            FBlock::Table(_) => {}
        }
    }
}

/// For an index or reference mark's start or end (`q`): its kind, the attribute naming it, and
/// whether it starts.
fn mark_pair(q: &str) -> Option<(&str, &'static str, bool)> {
    let (kind, start) = if let Some(k) = q.strip_suffix("-start") { (k, true) } else { (q.strip_suffix("-end")?, false) };
    match kind {
        "text:toc-mark" | "text:user-index-mark" | "text:alphabetical-index-mark" => Some((kind, "text:id", start)),
        "text:reference-mark" => Some((kind, "text:name", start)),
        _ => None,
    }
}

/// Whether `q` is a drawing that can hold text: a frame, a group or a shape.
fn is_drawing(q: &str) -> bool {
    matches!(
        local(q),
        "frame"
            | "g"
            | "rect"
            | "ellipse"
            | "circle"
            | "line"
            | "custom-shape"
            | "polygon"
            | "polyline"
            | "path"
            | "regular-polygon"
            | "connector"
            | "caption"
            | "measure"
    )
}

/// The section wrappers kept around top-level block `b` (a paragraph or a table), made empty
/// when it has none yet.
fn block_wrap(b: &mut FBlock) -> &mut (Vec<std::sync::Arc<str>>, u32) {
    match b {
        FBlock::Para(p) => p.wrap.get_or_insert_with(Default::default),
        FBlock::Table(t) => t.wrap.get_or_insert_with(Default::default),
    }
}
