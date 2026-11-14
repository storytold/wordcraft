use super::{Align, FBlock, FTable, Fmt, HashMap, Inline, Kind, Para, RevMark, Rgb, VAlign, mime_of, model};
use wordcraft_doc::control::ControlKind;
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing};

mod charts;
mod drawing;
mod indexes;
mod notes;
mod props;
mod tables;
mod tracked;

use drawing::Place;
pub(super) use drawing::WATERMARK_NAME;
use indexes::index_at;
pub(super) use indexes::{IndexKind, index_instr};
use notes::annotation_name;
pub(super) use props::{
    align_attr, border_attrs, generated_style, generating_style, is_script, lang_attrs, odf_border, odf_name, props_text, script_attrs,
    spacing_props, tab_stops,
};
use props::{parent_style, text_props};
use tracked::{Item, join_deleted_marks};

// ---------------------------------------------------------------------------------------------
// Export

/// A list style level as written: numbered or not, its label and its start value.
#[derive(Clone, Copy, Default, PartialEq)]
struct ListOut {
    ordered: bool,
    label: Option<model::Label>,
    start: u32,
}

/// WordCraft number format → ODF's `style:num-format` (ODF 1.3 §19.500); formats ODF lacks are
/// written as decimal.
pub(super) fn odf_num_format(f: wordcraft_doc::section::NumFormat) -> &'static str {
    use wordcraft_doc::section::NumFormat as F;
    match f {
        F::LowerRoman => "i",
        F::UpperRoman => "I",
        F::LowerLetter => "a",
        F::UpperLetter => "A",
        F::None => "",
        _ => "1",
    }
}

pub(super) fn x(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\u{FFFE}' | '\u{FFFF}' => {}
            c if c.is_control() && !matches!(c, '\t' | '\n') => {}
            _ => o.push(c),
        }
    }
    o
}

pub(super) const NS: &str = concat!(
    "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" ",
    "xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" ",
    "xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" ",
    "xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" ",
    "xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" ",
    "xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" ",
    "xmlns:xlink=\"http://www.w3.org/1999/xlink\" ",
    "xmlns:dc=\"http://purl.org/dc/elements/1.1/\" ",
    "xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" ",
    "xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" ",
    "xmlns:number=\"urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0\" ",
    "xmlns:form=\"urn:oasis:names:tc:opendocument:xmlns:form:1.0\" ",
    "xmlns:ooo=\"http://openoffice.org/2004/office\" ",
    "xmlns:ooow=\"http://openoffice.org/2004/writer\" ",
    "xmlns:oooc=\"http://openoffice.org/2004/calc\" ",
    "xmlns:script=\"urn:oasis:names:tc:opendocument:xmlns:script:1.0\" ",
    "xmlns:dom=\"http://www.w3.org/2001/xml-events\" ",
    "xmlns:loext=\"urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0\" ",
    "office:version=\"1.3\""
);

/// Marks a graphic style (in `Writer::graphic_styles`) as a text box frame's: derived from the
/// Frame style (see `Writer::automatic_styles`).
pub(super) const FRAME_PARENT: &str = "\u{0}frame";
/// Marks a graphic style as a picture frame's: derived from the Graphics style, without which
/// LibreOffice doesn't show a floating picture inside a text box.
pub(super) const GRAPHICS_PARENT: &str = "\u{0}graphics";

/// The run of list items written as one list from `start` in `blocks`: where it ends, and each
/// level's kind as its first item there says. An item of another list, or (lists not told apart)
/// a top-level item of the other kind, starts a new list; with section wrappers (`wraps`), so
/// does an item opening a section, and one closing a section ends it.
fn list_run(blocks: &[FBlock], start: usize, wraps: bool) -> (usize, [ListOut; 9]) {
    let p = match blocks.get(start) {
        Some(FBlock::Para(p)) => p,
        _ => return (start.saturating_add(1), [ListOut { ordered: false, label: None, start: 1 }; 9]),
    };
    let id = p.list.map_or(0, |l| l.list);
    let first = ListOut { ordered: p.list.is_some_and(|l| l.ordered), label: None, start: 1 };
    let mut kinds = [first; 9];
    let mut seen = [false; 9];
    let mut i = start;
    while let Some(FBlock::Para(q)) = blocks.get(i) {
        let Some(li) = q.list else { break };
        let l = li.level.min(8) as usize;
        if li.list != id || (id == 0 && l == 0 && seen[0] && kinds[0].ordered != li.ordered) {
            break;
        }
        let wrap = q.wrap.as_deref().filter(|_| wraps);
        if i > start && wrap.is_some_and(|w| !w.0.is_empty()) {
            break;
        }
        if let (Some(s), Some(k)) = (seen.get_mut(l), kinds.get_mut(l))
            && !*s
        {
            *s = true;
            *k = ListOut { ordered: li.ordered, label: li.label, start: li.start };
        }
        i += 1;
        if wrap.is_some_and(|w| w.1 > 0) {
            break;
        }
    }
    (i.max(start + 1), kinds)
}

/// The `xml:id` of the list markup `xml` (kept as read) begins with, if it does.
fn list_xml_id(xml: &str) -> Option<String> {
    let head = xml.strip_prefix("<text:list")?.split('>').next()?;
    let at = head.find("xml:id=\"")? + "xml:id=\"".len();
    let len = head.get(at..)?.find('"')?;
    head.get(at..at + len).map(str::to_string)
}

/// Paragraph `p` begins with a column break (Word's), which ODF states on the paragraph.
fn starts_column(p: &Para) -> bool {
    matches!(p.inlines.first(), Some(Inline::Text(t, _)) if t.starts_with(wordcraft_doc::para::COLUMN_BREAK))
}

pub(super) fn pt(v: f32) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    // A value rounding to nothing is 0, not "-0".
    format!("{}pt", if s == "-0" { "0" } else { s })
}

/// A section that starts a page: the master page it uses and where page numbering restarts.
type SecStart = (String, Option<u32>);

#[derive(Default)]
pub(super) struct Writer {
    /// Bookmarks that cover text (written as a start and an end), and those started and ended.
    pub(super) ranged: std::collections::HashSet<String>,
    bookmarks: std::collections::HashSet<String>,
    bookmark_ends: std::collections::HashSet<String>,
    /// The date and time styles fields are shown in: (Word picture, style name, definition).
    data_styles: Vec<(String, String, String)>,
    /// The sequences (`SEQ`) fields count, declared at the start of the text.
    pub(super) sequences: std::collections::BTreeSet<String>,
    /// Indexes written so far (their names).
    pub(super) indexes: usize,
    /// An index's entries are being written: their links are in LibreOffice's Index Link style.
    in_index: bool,
    /// The named dashes, gradients and arrowheads its graphic styles use.
    pub(super) draw_defs: super::strokes::DrawDefs,
    /// The custom paragraph styles paragraphs were written in (document style ids).
    pub(super) named: std::collections::BTreeSet<String>,
    /// The custom character styles text was written in.
    pub(super) named_text: std::collections::BTreeSet<String>,
    /// Text boxes written so far (their frames' names).
    boxes: usize,
    /// Notes written so far (their ids), and footnotes and endnotes among them (their numbers).
    notes: usize,
    footnotes: u32,
    endnotes: u32,
    /// The footnotes' and endnotes' number format and first number (see `writer::notes`); unset,
    /// LibreOffice's defaults.
    pub(super) note_numbers: Option<[(wordcraft_doc::section::NumFormat, u32); 2]>,
    /// Paragraph style that plain paragraphs get: the note style inside a note.
    note_parent: Option<&'static str>,
    /// Prefix of every name it makes (styles, lists, tables, pictures): headers and footers are
    /// written by a second writer whose styles go in `styles.xml`, apart from the body's.
    pub(super) prefix: &'static str,
    /// Automatic text styles: properties XML → name.
    text_styles: Vec<(String, String)>,
    /// Automatic paragraph styles: (parent, properties XML, master page) → name.
    para_styles: Vec<((String, String, String), String)>,
    /// List styles: each level's kind, label and start.
    list_styles: Vec<[ListOut; 9]>,
    /// The `xml:id` of the first list written for each flow list number, so a list that shows
    /// up again continues it.
    list_xml: HashMap<u32, String>,
    /// Table styles: (name, column widths).
    tables: Vec<Vec<f32>>,
    /// Each table's alignment and relative width (percent), by table.
    table_looks: Vec<(Option<Align>, Option<f32>)>,
    /// Each table's whether it starts a page (its first paragraph does).
    table_breaks: Vec<bool>,
    /// Each table's own space above and below, points.
    table_spaces: Vec<[Option<f32>; 2]>,
    /// Row styles: their `style:table-row-properties` attributes.
    row_styles: Vec<String>,
    /// The master page (and page-number restart) each table in `tables` starts.
    table_starts: Vec<Option<SecStart>>,
    /// Automatic styles of the columned `text:section`s written.
    pub(super) section_styles: Vec<String>,
    /// The page a section starts, for the next top-level block.
    pub(super) pending_start: Option<SecStart>,
    /// The next top-level list continues the one that ended the previous section.
    pub(super) continue_list: bool,
    /// `xml:id` of the last top-level list written, for `text:continue-list`.
    last_list: Option<String>,
    /// The page the block being written starts.
    master: Option<SecStart>,
    /// The definitions kept paragraphs may refer to (see `super::kept`), and those the kept
    /// paragraphs written so far need: automatic, common, font faces.
    pub(super) kept_defs: std::sync::Arc<std::collections::BTreeMap<String, std::sync::Arc<str>>>,
    /// The package files the markup written as read links to (its pictures): written as read.
    pub(super) kept_files_used: std::collections::BTreeSet<String>,
    /// The file's tracked changes markup written as read marks, written as read among the
    /// regions (each once).
    kept_changes: std::collections::BTreeSet<String>,
    /// The groups of paragraphs written as read together that are whole (see `model::Group`).
    pub(super) groups_whole: std::collections::HashSet<u32>,
    /// The sections kept as read opened so far (their markup's address: see `open_wrap`).
    wraps_opened: std::collections::HashSet<usize>,
    /// Lists kept as read given an `xml:id` (see `kept_run`).
    kept_lists: usize,
    /// The reference marks (§6.6.2) begun over text in the paragraph being written, to end.
    refs_open: std::collections::BTreeSet<String>,
    /// The file's own package paths, kept as read: names WordCraft's own pictures don't take.
    pub(super) reserved: std::sync::Arc<std::collections::BTreeSet<String>>,
    pub(super) kept_used: super::kept::Needed,
    /// The file's own master pages are written (its page setup is as read): kept markup may
    /// start one.
    pub(super) kept_pages: bool,
    /// Sections kept as read that are open (see `Para::wrap`).
    pub(super) sections_open: usize,
    /// The depth of the blocks being written whose section wrappers are written too, besides
    /// the body's top (0): a text box's story's own top.
    story_top: Option<usize>,
    /// Cell styles by (shading, vertical-alignment).
    cell_styles: Vec<String>,
    /// Graphic styles of floating pictures: their `style:graphic-properties` attributes.
    graphic_styles: Vec<String>,
    pub(super) pictures: Vec<(String, std::sync::Arc<Vec<u8>>, String)>,
    /// Object sub-documents (formulas, charts): their directory, `content.xml` and media type.
    pub(super) objects: Vec<(String, String, &'static str)>,
    /// Objects named so far (formulas, charts and kept objects share `Object n` names), and the
    /// kept objects' folders with their media types, for the manifest.
    pub(super) object_names: usize,
    pub(super) object_dirs: Vec<(String, String)>,
    /// The document's bibliography sources by tag: `CITATION` fields of one are written as
    /// bibliography marks carrying it.
    pub(super) sources: HashMap<String, wordcraft_doc::Source>,
    /// The document's custom properties' names: `DOCPROPERTY` fields naming one are written as
    /// `text:user-defined`.
    pub(super) custom_props: std::collections::HashSet<String>,
    /// The custom properties declared as user fields (`text:user-field-get`).
    pub(super) user_fields: std::collections::HashSet<String>,
    /// The names SET fields give values (the document's variables, §7.4.2): REF to one is a
    /// `text:variable-get`.
    pub(super) variables: std::collections::BTreeSet<String>,
    /// The kinds of the shapes with drawing ids, which connectors' glue points depend on.
    pub(super) conn_kinds: HashMap<u32, wordcraft_doc::para::ShapeKind>,
    /// Ruby was written: its automatic ruby style is.
    ruby_used: bool,
    /// Form controls written (`form:*` elements for `office:forms`), and whether this writer's
    /// text may hold them (the body's; not headers and footers).
    pub(super) forms: Vec<String>,
    pub(super) forms_allowed: bool,
    /// The theme's accent colours, which charts' series are drawn in.
    pub(super) accents: Vec<wordcraft_doc::Rgb>,
    /// `text:changed-region` elements written so far.
    pub(super) regions: Vec<String>,
    /// The insertion open in the body, and its region id.
    open_ins: Option<(RevMark, String)>,
    /// Style of the paragraph being written.
    cur_style: String,
    /// Styles of the paragraphs joined into the one being written (deleted paragraph marks),
    /// and which of them the text being written belongs to.
    chain: Vec<String>,
    part: usize,
}

impl Writer {
    /// A writer whose names all start with `prefix`.
    pub(super) fn with_prefix(prefix: &'static str) -> Self {
        Writer { prefix, ..Default::default() }
    }

    pub(in crate::odt) fn text_style(&mut self, f: &Fmt) -> Option<String> {
        let mut props = text_props(f);
        // A custom character style: the automatic style's parent (after a separator in the key).
        if let Some(id) = f.extra.as_ref().and_then(|x| x.style.as_deref()) {
            self.named_text.insert(id.to_string());
            props.push('\u{1}');
            props.push_str(&odf_name(id));
        }
        if props.is_empty() {
            return None;
        }
        if let Some((_, n)) = self.text_styles.iter().find(|(p, _)| *p == props) {
            return Some(n.clone());
        }
        let n = format!("{}T{}", self.prefix, self.text_styles.len() + 1);
        self.text_styles.push((props, n.clone()));
        Some(n)
    }

    fn para_style(&mut self, p: &Para) -> String {
        self.para_style_at(p, None)
    }

    /// The paragraph style of `p`; `start` makes it begin a page of that master page (ODF 1.3
    /// §19.471 `style:master-page-name`), restarting page numbering where asked (§20.322).
    fn para_style_at(&mut self, p: &Para, start: Option<&SecStart>) -> String {
        let named = p.extra.as_ref().and_then(|x| x.style.as_deref());
        let parent = match (p.kind, self.note_parent, named) {
            // A custom style: written as a common style of the same name (see `custom_styles`).
            (_, _, Some(id)) => {
                self.named.insert(id.to_string());
                odf_name(id)
            }
            (Kind::Normal, Some(n), None) => n.to_string(),
            _ => parent_style(p.kind).to_string(),
        };
        let mut props = String::new();
        match p.align {
            Some(Align::Center) => props.push_str(" fo:text-align=\"center\""),
            Some(Align::Right) => props.push_str(" fo:text-align=\"end\""),
            Some(Align::Justify) | Some(Align::Distribute) => props.push_str(" fo:text-align=\"justify\""),
            Some(Align::Left) => props.push_str(" fo:text-align=\"start\""),
            None => {}
        }
        // The master page starts the page itself: no page break besides.
        if p.page_break && start.is_none() {
            props.push_str(" fo:break-before=\"page\"");
        } else if !p.page_break && starts_column(p) {
            // A column break first (dropped from the text): the paragraph starts a column.
            props.push_str(" fo:break-before=\"column\"");
        }
        if let Some(n) = start.and_then(|(_, n)| *n) {
            props.push_str(&format!(" style:page-number=\"{n}\""));
        }

        if p.keep_next {
            props.push_str(" fo:keep-with-next=\"always\"");
        }
        props.push_str(&spacing_props(&p.spacing));
        // Child elements of the paragraph properties: tab stops (§17.8), a drop cap (§17.9).
        let mut children = String::new();
        if let Some(tabs) = p.extra.as_ref().and_then(|x| x.tabs.as_ref()) {
            children.push_str(&tab_stops(tabs));
        }
        if let Some((lines, distance)) = p.extra.as_ref().and_then(|x| x.drop_cap.filter(|l| *l > 1).map(|l| (l.min(10), x.drop_cap_distance))) {
            let d = distance.filter(|d| d.is_finite()).map_or(0.0, |d| d.clamp(0.0, 1584.0));
            children.push_str(&format!("<style:drop-cap style:lines=\"{lines}\" style:length=\"1\" style:distance=\"{}\"/>", pt(d)));
        }
        if p.kind == Kind::Rule {
            props.push_str(" fo:border-bottom=\"0.75pt solid #a0a0a0\" fo:padding-bottom=\"1pt\"");
        } else {
            props.push_str(&border_attrs(p.borders));
        }
        if let Some(bg) = p.extra.as_ref().and_then(|x| x.shading) {
            props.push_str(&format!(" fo:background-color=\"#{}\"", bg.hex().to_ascii_lowercase()));
        }
        if let Some(c) = p.extra.as_ref().and_then(|x| x.contextual_spacing) {
            props.push_str(&format!(" style:contextual-spacing=\"{c}\""));
        }
        if p.extra.as_ref().and_then(|x| x.bidi) == Some(true) {
            props.push_str(" style:writing-mode=\"rl-tb\"");
        }
        // How its lines break across pages (§20.247, §20.251, §20.250).
        if p.extra.as_ref().and_then(|x| x.keep_lines) == Some(true) {
            props.push_str(" fo:keep-together=\"always\"");
        }
        if p.extra.as_ref().and_then(|x| x.widow_control) == Some(false) {
            props.push_str(" fo:widows=\"0\" fo:orphans=\"0\"");
        }
        // The children go after a separator in the key; text properties (hyphenation, a text
        // property in ODF, §20.217) after another.
        if !children.is_empty() {
            props.push('\u{1}');
            props.push_str(&children);
        }
        // A paragraph without text: its mark formatting, its size giving its height.
        let mark = p.mark.as_deref().filter(|_| p.textless()).map(super::writer::props::props_text).unwrap_or_default();
        let hyphenate = p.extra.as_ref().and_then(|x| x.suppress_hyphens) == Some(false);
        if hyphenate || !mark.is_empty() {
            props.push('\u{2}');
            props.push_str(&mark);
        }
        if hyphenate {
            props.push_str(" fo:hyphenate=\"true\"");
        }
        let master = start.map(|(m, _)| m.clone()).unwrap_or_default();
        if props.is_empty() && master.is_empty() {
            return parent;
        }
        let key = (parent, props, master);
        if let Some((_, n)) = self.para_styles.iter().find(|(k, _)| *k == key) {
            return n.clone();
        }
        let n = format!("{}P{}", self.prefix, self.para_styles.len() + 1);
        self.para_styles.push((key, n.clone()));
        n
    }

    fn plain_inlines(&mut self, inl: &[Inline], out: &mut String) {
        let mut prev_space = true;
        let last = inl.len().saturating_sub(1);
        for (idx, i) in inl.iter().enumerate() {
            match i {
                Inline::Text(t, f) => {
                    let mut body = String::new();
                    let mut spaces = 0usize;
                    let flush = |body: &mut String, spaces: &mut usize, prev_space: bool| {
                        if *spaces == 0 {
                            return;
                        }
                        let (lit, extra) = if prev_space { (0, *spaces) } else { (1, *spaces - 1) };
                        if lit == 1 {
                            body.push(' ');
                        }
                        if extra > 0 {
                            body.push_str(&format!("<text:s text:c=\"{extra}\"/>"));
                        }
                        *spaces = 0;
                    };
                    for c in t.chars() {
                        if c == ' ' {
                            spaces += 1;
                            continue;
                        }
                        flush(&mut body, &mut spaces, prev_space);
                        prev_space = false;
                        match c {
                            '\t' => {
                                // The reader collapses a literal space after a tab, so spaces
                                // that follow are written as `<text:s/>`.
                                body.push_str("<text:tab/>");
                                prev_space = true;
                            }
                            '\n' => {
                                body.push_str("<text:line-break/>");
                                prev_space = true;
                            }
                            '\u{000C}' | '\u{000E}' => {}
                            c => body.push_str(&x(&c.to_string())),
                        }
                    }
                    if spaces > 0 {
                        // The reader drops a literal space at the end of a paragraph.
                        flush(&mut body, &mut spaces, prev_space || idx == last);
                        prev_space = true;
                    }
                    // A script, `file:`, UNC or other non-web target is written as plain text, as
                    // the HTML and Markdown exporters do (LibreOffice follows `file:`/UNC hrefs).
                    let link = f.link.as_deref().filter(|l| wordcraft_doc::link::is_safe_href(l));
                    // A tracked formatting change (§5.5.7): the text between its marks.
                    let change = f.fmt_change.as_ref().map(|c| self.region("format-change", &c.mark, ""));
                    if let Some(ct) = &change {
                        out.push_str(&format!("<text:change-start text:change-id=\"{ct}\"/>"));
                    }
                    if let Some(l) = link {
                        // An index entry's link looks like the entry, not like a web link.
                        let look = if self.in_index { " text:style-name=\"Index_20_Link\" text:visited-style-name=\"Index_20_Link\"" } else { "" };
                        out.push_str(&format!("<text:a xlink:type=\"simple\" xlink:href=\"{}\"{look}>", x(l)));
                    }
                    let fs = Fmt { link: None, ..f.clone() };
                    match self.text_style(&fs) {
                        Some(n) => out.push_str(&format!("<text:span text:style-name=\"{n}\">{body}</text:span>")),
                        None => out.push_str(&body),
                    }
                    if link.is_some() {
                        out.push_str("</text:a>");
                    }
                    if let Some(ct) = change {
                        out.push_str(&format!("<text:change-end text:change-id=\"{ct}\"/>"));
                    }
                }
                Inline::Image(img) => {
                    self.image(img, Place::Text(img.float.as_deref()), out);
                    prev_space = false;
                }
                Inline::Group { w, h, float, members } => {
                    self.group((*w, *h), float.as_deref(), members, out);
                    prev_space = false;
                }
                // A bookmark (§6.2.1): over the text up to its end, or a point; each name once.
                Inline::Anchor(a) if self.bookmarks.insert(a.clone()) => {
                    // One ending right where it starts is a point.
                    let empty = matches!(inl.get(idx + 1), Some(Inline::AnchorEnd(e)) if e == a);
                    if empty {
                        self.bookmark_ends.insert(a.clone());
                    }
                    let el = if self.ranged.contains(a) && !empty { "bookmark-start" } else { "bookmark" };
                    out.push_str(&format!("<text:{el} text:name=\"{}\"/>", x(a)));
                    self.reference_mark(a, inl.get(idx + 1..).unwrap_or(&[]), empty, out);
                }
                Inline::Anchor(_) => {}
                Inline::AnchorEnd(a) => self.anchor_end(a, out),
                Inline::Note { endnote, custom, blocks, fmt } => {
                    self.styled_note(*endnote, custom, blocks, fmt, out);
                    prev_space = false;
                }
                Inline::Comment { id, comment } => {
                    self.comment(*id, comment, out);
                    prev_space = false;
                }
                Inline::Shape { kind, w, h, float, blocks, look, path } => {
                    self.drawing_shape(*kind, (*w, *h), Place::Text(float.as_deref()), (look, path.as_deref()), blocks, out);
                    prev_space = false;
                }
                // A field around content: an index's is its element (see `index_at`).
                Inline::FieldStart(_) | Inline::FieldEnd => {}
                Inline::CommentEnd(id) => out.push_str(&format!("<office:annotation-end office:name=\"{}\"/>", annotation_name(self.prefix, *id))),
                Inline::Field { instr, result, fmt, locked } => {
                    match self
                        .field_element(instr, result)
                        .and_then(|el| if *locked { super::fields::locked(&el, instr, result) } else { Some(super::fields::unfixed(&el)) })
                    {
                        Some(el) => {
                            match self.text_style(&Fmt { link: None, ..fmt.clone() }) {
                                Some(n) => out.push_str(&format!("<text:span text:style-name=\"{n}\">{el}</text:span>")),
                                None => out.push_str(&el),
                            }
                            prev_space = false;
                        }
                        // No ODF field for it: what it shows, as text.
                        None => {
                            self.plain_inlines(&[Inline::Text(result.clone(), fmt.clone())], out);
                            prev_space = result.ends_with(' ');
                        }
                    }
                }
                // Inside a tracked change: its base text, which the text path marks as changed.
                Inline::Ruby { base, fmt, .. } if fmt.ins.is_some() || fmt.del.is_some() => {
                    self.plain_inlines(&[Inline::Text(base.clone(), fmt.clone())], out);
                    prev_space = false;
                }
                // Ruby (§6.4): the base text in its formatting, the ruby text over it.
                Inline::Ruby { base, ruby, fmt } => {
                    self.ruby_used = true;
                    let base_xml = match self.text_style(&Fmt { link: None, ..fmt.clone() }) {
                        Some(n) => format!("<text:span text:style-name=\"{n}\">{}</text:span>", x(base)),
                        None => x(base),
                    };
                    out.push_str(&format!(
                        "<text:ruby text:style-name=\"{}Ru1\"><text:ruby-base>{base_xml}</text:ruby-base><text:ruby-text>{}</text:ruby-text></text:ruby>",
                        self.prefix,
                        x(ruby)
                    ));
                    prev_space = false;
                }
                Inline::Equation { linear, display } => {
                    self.formula(linear, *display, out);
                    prev_space = false;
                }
                Inline::Figure(_) => {}
                Inline::Chart(c) => {
                    self.chart(c, Place::Text(c.float.as_deref()), out);
                    prev_space = false;
                }
                // A form control (in the body: forms belong to the text, ODF 1.3 §13.2), placed
                // here; elsewhere, or of a kind forms don't have, its text.
                // A control read from a placeholder (§7.7.2), in the body or anywhere else: a
                // placeholder while it shows it, else what was typed in its place.
                Inline::Control { control, text, fmt } if super::forms::is_placeholder(control) => {
                    if control.showing_placeholder && control.kind == (ControlKind::Text { multi_line: false }) {
                        // Its description: the control's title.
                        let start = match control.form_xml.strip_suffix('>') {
                            Some(head) if !control.title.is_empty() => format!("{head} text:description=\"{}\">", x(&control.title)),
                            _ => control.form_xml.clone(),
                        };
                        let el = format!("{start}{}</text:placeholder>", x(control.placeholder_shown()));
                        match self.text_style(&Fmt { link: None, ..fmt.clone() }) {
                            Some(n) => out.push_str(&format!("<text:span text:style-name=\"{n}\">{el}</text:span>")),
                            None => out.push_str(&el),
                        }
                        prev_space = false;
                    } else {
                        self.plain_inlines(&[Inline::Text(text.clone(), fmt.clone())], out);
                        prev_space = text.ends_with(' ');
                    }
                }
                Inline::Control { control, text, fmt } => {
                    let id = format!("control{}", self.forms.len() + 1);
                    match super::forms::form_xml(control, text, &id, self.forms.len() + 1).filter(|_| self.forms_allowed) {
                        Some((xml, w, h)) => {
                            // Text boxes with a flat border (`fo:border="solid"`, LibreOffice's for
                            // its own text fields), not its sunken 3D one.
                            let flat = if xml.starts_with("<form:text ") || xml.starts_with("<form:textarea") { " fo:border=\"solid\"" } else { "" };
                            self.forms.push(xml);
                            // Placed by a style, standing on the baseline or raised as it was read
                            // (LibreOffice puts a control with no placement at the line's top).
                            let raise = control.form_raise.filter(|r| r.is_finite()).map(|r| r.clamp(-1584.0, 1584.0)).unwrap_or(0.0);
                            let float = wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::Inline, y: raise, ..Default::default() };
                            let (_, props, xy) = drawing::placement(drawing::Place::Text(Some(&float)), w, h);
                            let style = self.graphic_style(format!("{props}{flat}"));
                            out.push_str(&format!(
                                "<draw:control draw:style-name=\"{style}\" text:anchor-type=\"as-char\"{xy} draw:z-index=\"0\" svg:width=\"{}\" svg:height=\"{}\" draw:control=\"{id}\"/>",
                                pt(w),
                                pt(h)
                            ));
                            prev_space = false;
                        }
                        None => {
                            self.plain_inlines(&[Inline::Text(text.clone(), fmt.clone())], out);
                            prev_space = text.ends_with(' ');
                        }
                    }
                }
            }
        }
    }

    /// Paragraph `p`, or with `chain` the paragraphs joined at deleted paragraph marks (`p`
    /// is the first).
    /// Close the sections kept as read that are still open (at the end of what holds them).
    pub(super) fn close_sections(&mut self, out: &mut String) {
        if self.sections_open > 0 {
            self.close_ins(out);
        }
        while self.sections_open > 0 {
            out.push_str("</text:section>");
            self.sections_open -= 1;
        }
    }

    /// Paragraph `p` as it was read, when it still is and what it refers to was kept too (see
    /// `super::kept`); false when it is written from the model.
    fn kept_para(&mut self, p: &Para, out: &mut String) -> bool {
        let Some(xml) = &p.kept else { return false };
        if !self.group_whole(p) {
            return false;
        }
        self.kept_markup(xml, out)
    }

    /// Whether paragraph `p`'s markup kept as read can be written so: it belongs to no group, or
    /// to one whose every paragraph is as read.
    fn group_whole(&self, p: &Para) -> bool {
        p.group.as_ref().and_then(|g| g.id).is_none_or(|(id, _)| self.groups_whole.contains(&id))
    }

    /// Markup kept as read (a paragraph's or a table's), with the definitions it names; false
    /// when one of them can't be written as read (see `super::kept::needed`).
    pub(in crate::odt) fn kept_markup(&mut self, xml: &str, out: &mut String) -> bool {
        if !self.kept_defs_of(xml) {
            return false;
        }
        self.close_ins(out);
        out.push_str(xml);
        // With the pictures it shows. Its bookmarks' names are taken: the model's don't repeat them.
        super::kept::hrefs(xml, &mut self.kept_files_used);
        let (starts, ends) = super::kept::bookmarks(xml);
        self.bookmarks.extend(starts);
        self.bookmark_ends.extend(ends);
        true
    }

    /// The definitions markup kept as read names, added to those written; false when one of
    /// them can't be written as read.
    fn kept_defs_of(&mut self, xml: &str) -> bool {
        // The tracked changes it marks, as read, and those their deleted content marks (text
        // inserted, then deleted); with the styles they name. Not one without its date, which
        // ODF requires (§4.3.2.10 in `office:change-info`): it is written from the model, dated.
        let mut todo: Vec<String> = super::kept::changes(xml).into_iter().collect();
        let mut regions: Vec<(String, std::sync::Arc<str>)> = Vec::new();
        while let Some(id) = todo.pop() {
            if self.kept_changes.contains(&id) || regions.iter().any(|(r, _)| *r == id) {
                continue;
            }
            match self.kept_defs.get(&format!("change:{id}")).filter(|r| r.contains("<dc:date>")) {
                Some(r) if regions.len() < 1000 => {
                    todo.extend(super::kept::changes(r));
                    regions.push((id, r.clone()));
                }
                _ => return false,
            }
        }
        let mut names = super::kept::names(xml);
        for (_, r) in &regions {
            names.extend(super::kept::names(r));
        }
        let Some((auto, common, fonts)) = super::kept::needed(&self.kept_defs, names, self.kept_pages) else { return false };
        // Those their content marks first, as a reader meets them (and as the file had them).
        for (id, r) in regions.into_iter().rev() {
            self.kept_changes.insert(id);
            self.regions.push(r.to_string());
        }
        self.kept_used.0.extend(auto);
        self.kept_used.1.extend(common);
        self.kept_used.2.extend(fonts);
        self.sequences.extend(super::kept::sequences(xml));
        true
    }

    fn para_in(&mut self, p: &Para, chain: Option<&Vec<Para>>, out: &mut String) {
        if chain.is_none() && (self.master.is_none() || self.kept_pages) && self.kept_para(p, out) {
            // Its own page start, if any, is in its markup.
            self.master = None;
            return;
        }
        let paras: &[Para] = chain.map(Vec::as_slice).unwrap_or(std::slice::from_ref(p));
        let start = self.master.take();
        self.chain = chain.map(|c| c.iter().map(|q| self.para_style(q)).collect()).unwrap_or_default();
        self.part = 0;
        let style = self.para_style_at(p, start.as_ref());
        self.cur_style = style.clone();
        let tag = match p.kind {
            Kind::Heading(n) => {
                // Its own outline level when it sets one apart from its style's.
                let level = p.extra.as_ref().and_then(|x| x.outline_level).map_or(n.into(), |l| u32::from(l) + 1);
                out.push_str(&format!("<text:h text:style-name=\"{style}\" text:outline-level=\"{}\">", level.clamp(1, 10)));
                "</text:h>"
            }
            _ => {
                out.push_str(&format!("<text:p text:style-name=\"{style}\">"));
                "</text:p>"
            }
        };
        let tracked = paras.len() > 1
            || p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) | Inline::Ruby { fmt: f, .. } | Inline::Field { fmt: f, .. } if f.ins.is_some() || f.del.is_some()));
        if tracked {
            let mut items = Vec::new();
            for (k, q) in paras.iter().enumerate() {
                items.extend(q.inlines.iter().map(Item::Inl));
                if k + 1 < paras.len() {
                    items.push(Item::Mark(q));
                }
            }
            self.items(&items, out);
        } else {
            self.close_ins(out);
            self.plain_inlines(&p.inlines, out);
        }
        // An inserted paragraph mark: the insertion runs on past the paragraph's end.
        match paras.last().and_then(|q| q.mark_ins.as_deref()) {
            Some(m) => self.open_ins(m, out),
            None => self.close_ins(out),
        }
        out.push_str(tag);
        self.chain.clear();
    }

    /// Paragraph `p` (`chain`: the paragraphs a tracked deletion joins it to) with the sections
    /// kept as read that open before it and close after it, at the top only.
    fn wrapped_para(&mut self, p: &Para, chain: Option<&Vec<Para>>, out: &mut String, depth: usize) {
        let wrap = p.wrap.as_deref().filter(|_| self.wraps_at(depth));
        let skipped = self.open_wrap(wrap, out);
        self.para_in(p, chain, out);
        self.close_wrap(wrap, skipped, out);
    }

    /// Whether blocks at `depth` are written with the section wrappers kept around them: the
    /// body's top-level ones and a text box's story's.
    fn wraps_at(&self, depth: usize) -> bool {
        depth == 0 || self.story_top == Some(depth)
    }

    /// `blocks` (a text box's story, at `depth`) with the sections kept around them, all closed
    /// inside it: the sections open outside it stay open.
    pub(in crate::odt) fn story_blocks(&mut self, blocks: &[FBlock], out: &mut String, depth: usize) {
        let saved = (self.story_top.replace(depth), std::mem::take(&mut self.sections_open));
        self.blocks(blocks, out, depth);
        self.close_sections(out);
        (self.story_top, self.sections_open) = saved;
    }

    /// The run of top-level paragraphs `p` begins kept as read whole (an index's, a list's),
    /// written as it was, when it can be: how many blocks it took.
    fn kept_run(&mut self, blocks: &[FBlock], i: usize, out: &mut String) -> Option<usize> {
        let Some(FBlock::Para(p)) = blocks.get(i) else { return None };
        let ix = p.index.as_deref()?;
        // Its paragraphs' groups whole (one tied to a paragraph since edited writes it anew).
        let run = blocks.get(i..i + ix.1.max(1)).unwrap_or(&[]);
        if !run.iter().all(|b| !matches!(b, FBlock::Para(q) if !self.group_whole(q))) {
            return None;
        }
        if !(self.master.is_none() || self.kept_pages) {
            return None;
        }
        // The sections kept as read its first paragraph opens and its last closes, around it.
        let mut markup = String::new();
        if !self.kept_markup(&ix.0, &mut markup) {
            return None;
        }
        // A list without its own `xml:id` (older files' lists) is given one, for a list going on
        // with it to name.
        if p.list.is_some()
            && list_xml_id(&ix.0).is_none()
            && let Some(at) = markup.find("<text:list").map(|i| i + "<text:list".len())
        {
            self.kept_lists += 1;
            markup.insert_str(at, &format!(" xml:id=\"{}lstk{}\"", self.prefix, self.kept_lists));
        }
        let last = match blocks.get(i + ix.1.max(1) - 1) {
            Some(FBlock::Para(q)) => q.wrap.as_deref().map(|w| (Vec::new(), w.1)),
            _ => None,
        };
        let _ = self.open_wrap(p.wrap.as_deref().map(|w| (w.0.clone(), 0)).as_ref(), out);
        out.push_str(&markup);
        self.close_wrap(last.as_ref(), 0, out);
        self.master = None;
        // A list kept so: a list continuing it, written from the model, names it.
        if let Some(l) = p.list
            && let Some(id) = markup.find("<text:list").and_then(|i| markup.get(i..)).and_then(list_xml_id)
        {
            if l.list != 0 {
                self.list_xml.entry(l.list).or_insert_with(|| id.clone());
            }
            self.last_list = Some(id);
        }
        out.push('\n');
        Some(ix.1.max(1))
    }

    /// The end of bookmark `a` (and of the reference mark by its name, when one is open).
    fn anchor_end(&mut self, a: &str, out: &mut String) {
        if self.ranged.contains(a) && self.bookmarks.contains(a) && self.bookmark_ends.insert(a.to_string()) {
            out.push_str(&format!("<text:bookmark-end text:name=\"{}\"/>", x(a)));
        }
        if self.refs_open.remove(a) {
            out.push_str(&format!("<text:reference-mark-end text:name=\"{}\"/>", x(a)));
        }
    }

    /// Bookmark `a` was the file's reference mark (§6.6.2, read as a bookmark): one by its name
    /// too, which `text:reference-ref` names (the bookmark is for `text:bookmark-ref`). Over the
    /// text up to its end when that is in the paragraph (`rest`, after it), else at a point.
    fn reference_mark(&mut self, a: &str, rest: &[Inline], empty: bool, out: &mut String) {
        if !self.kept_defs.contains_key(&format!("refmark:{a}")) {
            return;
        }
        let ends_here = !empty && rest.iter().any(|i| matches!(i, Inline::AnchorEnd(e) if e == a));
        if ends_here && self.refs_open.len() < 1000 {
            self.refs_open.insert(a.to_string());
            out.push_str(&format!("<text:reference-mark-start text:name=\"{}\"/>", x(a)));
        } else {
            out.push_str(&format!("<text:reference-mark text:name=\"{}\"/>", x(a)));
        }
    }

    /// The sections kept as read that open before a top-level block (`wrap`, see `Para::wrap`),
    /// each once (a copy of the paragraph opening one shares its markup): how many were
    /// skipped so.
    fn open_wrap(&mut self, wrap: Option<&(Vec<std::sync::Arc<str>>, u32)>, out: &mut String) -> u32 {
        let mut skipped = 0u32;
        for tag in wrap.map(|w| w.0.as_slice()).unwrap_or(&[]) {
            if !self.wraps_opened.insert(std::sync::Arc::as_ptr(tag).cast::<u8>() as usize) {
                skipped = skipped.saturating_add(1);
                continue;
            }
            self.close_ins(out);
            // With the section style it names.
            self.kept_defs_of(tag);
            out.push_str(tag);
            self.sections_open += 1;
        }
        skipped
    }

    /// The sections kept as read that close after a top-level block: only those open (one
    /// whose start was edited away isn't), so what is written stays balanced.
    fn close_wrap(&mut self, wrap: Option<&(Vec<std::sync::Arc<str>>, u32)>, skipped: u32, out: &mut String) {
        for _ in 0..wrap.map_or(0, |w| w.1.saturating_sub(skipped)) {
            if self.sections_open > 0 {
                self.close_ins(out);
                out.push_str("</text:section>");
                self.sections_open -= 1;
            }
        }
    }

    pub(super) fn blocks(&mut self, blocks: &[FBlock], out: &mut String, depth: usize) {
        let joined = join_deleted_marks(blocks);
        let (blocks, chains) = match &joined {
            Some((b, c)) => (b.as_slice(), Some(c)),
            None => (blocks, None),
        };
        let chain = |i: usize| chains.and_then(|c| c.get(&i));
        let mut i = 0;
        while let Some(b) = blocks.get(i) {
            // Only the first top-level block of a section starts its page.
            if depth == 0 {
                self.master = self.pending_start.take();
                // An index or a list kept as read (its paragraphs all as read): as it was.
                if chains.is_none()
                    && let Some(n) = self.kept_run(blocks, i, out)
                {
                    i += n;
                    continue;
                }
                // A table of contents or another index, with its entries.
                if let Some((n, kind, titled)) = index_at(blocks, i) {
                    let joined = chains.is_some_and(|c| (i..i + n).any(|k| c.contains_key(&k)));
                    if !joined {
                        self.index(blocks.get(i..i + n).unwrap_or(&[]), &kind, titled, out);
                        i += n;
                        continue;
                    }
                }
            }
            match b {
                FBlock::Table(t) => {
                    if depth < model::MAX_DEPTH {
                        let wrap = t.wrap.as_deref().filter(|_| self.wraps_at(depth));
                        let skipped = self.open_wrap(wrap, out);
                        self.table(t, out, depth);
                        self.close_wrap(wrap, skipped, out);
                    }
                    i += 1;
                }
                FBlock::Para(p) if p.list.is_some() => {
                    let start = i;
                    let id = p.list.map_or(0, |l| l.list);
                    let (end, kinds) = list_run(blocks, start, self.wraps_at(depth));
                    i = end;
                    // The sections kept as read its first item opens and its last closes, around it.
                    let wrap_of = |k: usize| match blocks.get(k) {
                        Some(FBlock::Para(q)) => q.wrap.as_deref().filter(|_| self.wraps_at(depth)),
                        _ => None,
                    };
                    let (opens, closes) = (wrap_of(start).map(|w| (w.0.clone(), 0)), wrap_of(end.saturating_sub(1)).map(|w| (Vec::new(), w.1)));
                    let _ = self.open_wrap(opens.as_ref(), out);
                    self.list_styles.push(kinds);
                    let lname = format!("{}L{}", self.prefix, self.list_styles.len());
                    let id_of_list = id;
                    // Open lists: per level, whether a list-item is open.
                    let mut stack: Vec<bool> = Vec::new();
                    for (qi, q) in blocks.get(start..i).unwrap_or(&[]).iter().enumerate() {
                        let FBlock::Para(q) = q else { continue };
                        let lv = q.list.map(|l| l.level.min(8) as usize).unwrap_or(0);
                        while stack.len() > lv + 1 {
                            if stack.pop() == Some(true) {
                                out.push_str("</text:list-item>");
                            }
                            out.push_str("</text:list>");
                        }
                        while stack.len() < lv + 1 {
                            if let Some(top) = stack.last_mut()
                                && !*top
                            {
                                out.push_str("<text:list-item>");
                                *top = true;
                            }
                            if stack.is_empty() {
                                let id = format!("{}lst{}", self.prefix, self.list_styles.len());
                                // A list cut by a section boundary goes on numbering: continue-list names the
                                // preceding list by its xml:id, whatever its style name (ODF 1.3 §19.743).
                                let prev = self.last_list.clone();
                                let cont = match prev {
                                    Some(prev) if depth == 0 && start == 0 && std::mem::take(&mut self.continue_list) => {
                                        format!(" text:continue-list=\"{prev}\"")
                                    }
                                    // A list that showed up before goes on with it.
                                    _ => match self.list_xml.get(&id_of_list) {
                                        Some(x) if id_of_list != 0 => format!(" text:continue-list=\"{x}\""),
                                        _ => String::new(),
                                    },
                                };
                                if id_of_list != 0 {
                                    self.list_xml.entry(id_of_list).or_insert_with(|| id.clone());
                                }
                                if depth == 0 {
                                    self.last_list = Some(id.clone());
                                }
                                out.push_str(&format!("<text:list xml:id=\"{id}\" text:style-name=\"{lname}\"{cont}>"));
                            } else {
                                out.push_str("<text:list>");
                            }
                            stack.push(false);
                        }
                        if let Some(top) = stack.last_mut() {
                            if *top {
                                out.push_str("</text:list-item>");
                            }
                            *top = true;
                        }
                        out.push_str("<text:list-item>");
                        self.para_in(q, chain(start + qi), out);
                    }
                    while let Some(open) = stack.pop() {
                        if open {
                            out.push_str("</text:list-item>");
                        }
                        out.push_str("</text:list>");
                    }
                    self.close_wrap(closes.as_ref(), 0, out);
                    out.push('\n');
                }
                FBlock::Para(p) => {
                    self.wrapped_para(p, chain(i), out, depth);
                    out.push('\n');
                    i += 1;
                }
            }
        }
    }

    /// The ODF element for field `instr` showing `result`, with the date style and sequence
    /// declaration it needs; `None` when ODF has none.
    fn field_element(&mut self, instr: &str, result: &str) -> Option<String> {
        // A citation of a source the document has (§8.1.24).
        if let Some(s) = instr.trim_start().strip_prefix("CITATION").and_then(|r| r.split_whitespace().next()).and_then(|t| self.sources.get(t)) {
            return Some(super::fields::bib_mark(s, &x(result)));
        }
        // A custom document property's value (§7.5.6).
        // A variable given a value, hidden (§7.4.5): a REF after it shows the value.
        if let Some((name, value)) = super::fields::set_of(instr).filter(|(n, _)| self.variables.contains(*n)) {
            return Some(format!(
                "<text:variable-set text:name=\"{}\" text:display=\"none\" office:value-type=\"string\" office:string-value=\"{}\"/>",
                x(name),
                x(&value)
            ));
        }
        // A variable's value (§7.4.6).
        if let Some(name) = instr.trim_start().strip_prefix("REF").and_then(|r| r.split_whitespace().next()).filter(|n| self.variables.contains(*n)) {
            return Some(format!("<text:variable-get text:name=\"{}\">{}</text:variable-get>", x(name), x(result)));
        }
        // A user field's value (§7.4.9), or a document property's (§7.5.6).
        if let Some(n) = super::fields::docproperty_name(instr).filter(|n| self.user_fields.contains(*n)) {
            return Some(format!("<text:user-field-get text:name=\"{}\">{}</text:user-field-get>", x(n), x(result)));
        }
        if let Some(n) = super::fields::docproperty_name(instr).filter(|n| self.custom_props.contains(*n)) {
            return Some(format!("<text:user-defined text:name=\"{}\">{}</text:user-defined>", x(n), x(result)));
        }
        let ds = self.data_style(instr);
        if let Some(seq) = super::fields::sequence_name(instr) {
            self.sequences.insert(seq.to_string());
        }
        super::fields::element(instr, &x(result), ds.as_deref())
    }

    /// The date or time style for field code `instr`'s `\@` picture (one per picture).
    fn data_style(&mut self, instr: &str) -> Option<String> {
        let pic = super::datastyle::picture_of(instr)?;
        if let Some((_, name, _)) = self.data_styles.iter().find(|(p, ..)| *p == pic) {
            return Some(name.clone());
        }
        let name = format!("{}N{}", self.prefix, self.data_styles.len() + 1);
        let xml = super::datastyle::style_xml(&name, &pic)?;
        self.data_styles.push((pic, name.clone(), xml));
        Some(name)
    }

    pub(super) fn automatic_styles(&self) -> String {
        format!("<office:automatic-styles>{}</office:automatic-styles>", self.style_defs())
    }

    /// The cells' automatic styles: each one's attributes, and what goes inside (a background
    /// picture).
    fn cell_style_defs(&self) -> String {
        let mut s = String::new();
        for (i, key) in self.cell_styles.iter().enumerate() {
            let (props, child) = key.split_once(tables::CELL_CHILD).unwrap_or((key.as_str(), ""));
            let inner = if child.is_empty() { "/>".to_string() } else { format!(">{child}</style:table-cell-properties>") };
            s.push_str(&format!(
                "<style:style style:name=\"{}Cell{}\" style:family=\"table-cell\"><style:table-cell-properties{props}{inner}</style:style>",
                self.prefix,
                i + 1
            ));
        }
        s
    }

    /// The automatic styles made so far, without their `office:automatic-styles` element.
    pub(super) fn style_defs(&self) -> String {
        let mut s: String = self.data_styles.iter().map(|(.., xml)| xml.as_str()).collect();
        for (props, n) in &self.text_styles {
            let (props, parent) = match props.split_once('\u{1}') {
                Some((p, parent)) => (p, format!(" style:parent-style-name=\"{parent}\"")),
                None => (props.as_str(), String::new()),
            };
            s.push_str(&format!("<style:style style:name=\"{n}\" style:family=\"text\"{parent}><style:text-properties{props}/></style:style>"));
        }
        for ((parent, props, master), n) in &self.para_styles {
            let master = if master.is_empty() { String::new() } else { format!(" style:master-page-name=\"{}\"", x(master)) };
            let (props, text) =
                props.split_once('\u{2}').map_or((props.as_str(), String::new()), |(p, t)| (p, format!("<style:text-properties{t}/>")));
            let (attrs, children) = props.split_once('\u{1}').unwrap_or((props, ""));
            s.push_str(&format!(
                "<style:style style:name=\"{n}\" style:family=\"paragraph\" style:parent-style-name=\"{parent}\"{master}><style:paragraph-properties{attrs}>{children}</style:paragraph-properties>{text}</style:style>"
            ));
        }
        for (i, kinds) in self.list_styles.iter().enumerate() {
            s.push_str(&format!("<text:list-style style:name=\"{}L{}\">", self.prefix, i + 1));
            for (l, out) in kinds.iter().enumerate() {
                let ordered = &out.ordered;
                let lv = l + 1;
                // The level's own place, else WordCraft's default: 36 pt a level, hanging 18 pt.
                let (indent, hanging) = out.label.and_then(|lb| lb.position()).unwrap_or((36.0 * lv as f32, 18.0));
                let align = match out.label.and_then(|lb| lb.align) {
                    Some(Align::Right) => " fo:text-align=\"end\"",
                    Some(Align::Center) => " fo:text-align=\"center\"",
                    _ => "",
                };
                let props = format!(
                    "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"{align}><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{}\" fo:text-indent=\"{}\" fo:margin-left=\"{}\"/></style:list-level-properties>",
                    pt(indent),
                    pt(-hanging),
                    pt(indent)
                );
                if *ordered {
                    let fmt = ["1", "a", "i"].get(l % 3).copied().unwrap_or("1");
                    // The level's own label, else WordCraft's default for the level.
                    let attrs = match out.label {
                        Some(lb) if lb.format != wordcraft_doc::section::NumFormat::Bullet => {
                            let mut a = format!(" style:num-format=\"{}\"", odf_num_format(lb.format));
                            for (k, v) in [("style:num-prefix", lb.prefix.as_str()), ("style:num-suffix", lb.suffix.as_str())] {
                                if !v.is_empty() {
                                    a.push_str(&format!(" {k}=\"{}\"", x(v)));
                                }
                            }
                            if lb.shown > 1 {
                                a.push_str(&format!(" text:display-levels=\"{}\"", lb.shown.min(lv as u8)));
                            }
                            a
                        }
                        _ => format!(" style:num-suffix=\".\" style:num-format=\"{fmt}\""),
                    };
                    let start = if out.start != 1 { format!(" text:start-value=\"{}\"", out.start) } else { String::new() };
                    s.push_str(&format!("<text:list-level-style-number text:level=\"{lv}\"{attrs}{start}>{props}</text:list-level-style-number>"));
                } else {
                    let own = out.label.and_then(|lb| lb.bullet()).map(|c| c.to_string());
                    let ch = own.as_deref().unwrap_or_else(|| ["•", "◦", "▪"].get(l % 3).copied().unwrap_or("•"));
                    let ch = x(ch);
                    s.push_str(&format!(
                        "<text:list-level-style-bullet text:level=\"{lv}\" text:bullet-char=\"{ch}\">{props}</text:list-level-style-bullet>"
                    ));
                }
            }
            s.push_str("</text:list-style>");
        }
        for (i, widths) in self.tables.iter().enumerate() {
            let tn = format!("{}{}", self.prefix, i + 1);
            let total: f32 = widths.iter().sum();
            let (master, number) = match self.table_starts.get(i).and_then(Option::as_ref) {
                Some((m, n)) => {
                    (format!(" style:master-page-name=\"{}\"", x(m)), n.map(|n| format!(" style:page-number=\"{n}\"")).unwrap_or_default())
                }
                None => (String::new(), String::new()),
            };
            // Aligned, or narrower than the text: as wide as its columns, with its relative width.
            let (align, pct) = self.table_looks.get(i).copied().unwrap_or_default();
            let brk = if self.table_breaks.get(i).copied().unwrap_or(false) { " fo:break-before=\"page\"" } else { "" };
            let align = match align {
                Some(Align::Left) => "left",
                Some(Align::Center) => "center",
                Some(Align::Right) => "right",
                _ if pct.is_some() => "left",
                _ => "margins",
            };
            let rel = pct.map(|p| format!(" style:rel-width=\"{}%\"", (p * 100.0).round() / 100.0)).unwrap_or_default();
            let space: String = ["fo:margin-top", "fo:margin-bottom"]
                .iter()
                .zip(self.table_spaces.get(i).copied().unwrap_or_default())
                .filter_map(|(k, v)| v.filter(|v| v.is_finite() && *v > 0.0).map(|v| format!(" {k}=\"{}\"", pt(v.min(1584.0)))))
                .collect();
            s.push_str(&format!(
                "<style:style style:name=\"Table{tn}\" style:family=\"table\"{master}><style:table-properties style:width=\"{}\"{rel} table:align=\"{align}\"{space}{number}{brk}/></style:style>",
                pt(total)
            ));
            for (c, w) in widths.iter().enumerate() {
                // A table filling the margins gives its columns' shares too (§20.334
                // `style:rel-column-width`), so on a narrower page they keep their proportions.
                // (LibreOffice lays out an aligned table with shares otherwise: only these.)
                let share = if align == "margins" {
                    let rel = if w.is_finite() { (w.clamp(0.0, 1584.0) * 100.0).round().max(1.0) as u32 } else { 1 };
                    format!(" style:rel-column-width=\"{rel}*\"")
                } else {
                    String::new()
                };
                s.push_str(&format!(
                    "<style:style style:name=\"Table{tn}.C{}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{}\"{share}/></style:style>",
                    c + 1,
                    pt(*w)
                ));
            }
        }
        if self.ruby_used {
            s.push_str(&format!(
                "<style:style style:name=\"{}Ru1\" style:family=\"ruby\"><style:ruby-properties style:ruby-position=\"above\" style:ruby-align=\"center\"/></style:style>",
                self.prefix
            ));
        }
        for (i, props) in self.row_styles.iter().enumerate() {
            s.push_str(&format!(
                "<style:style style:name=\"{}Row{}\" style:family=\"table-row\"><style:table-row-properties{props}/></style:style>",
                self.prefix,
                i + 1
            ));
        }
        for (i, props) in self.graphic_styles.iter().enumerate() {
            // A text box's frame derives from LibreOffice's Frame style, without which
            // LibreOffice ignores its border (`fo:border`) and outlines it as a drawing shape.
            let (parent, props) = if let Some(rest) = props.strip_prefix(FRAME_PARENT) {
                (" style:parent-style-name=\"Frame\"", rest)
            } else if let Some(rest) = props.strip_prefix(GRAPHICS_PARENT) {
                (" style:parent-style-name=\"Graphics\"", rest)
            } else {
                ("", props.as_str())
            };
            // Laid out where WordCraft places it, not held in the text's area: said, as the
            // standard gives no default (§20.267) and a file's default style may say otherwise.
            let flow = if props.contains("style:flow-with-text") { "" } else { " style:flow-with-text=\"false\"" };
            s.push_str(&format!(
                "<style:style style:name=\"{}fr{}\" style:family=\"graphic\"{parent}><style:graphic-properties{props}{flow}/></style:style>",
                self.prefix,
                i + 1
            ));
        }
        s.push_str(&self.cell_style_defs());
        for st in &self.section_styles {
            s.push_str(st);
        }
        s
    }
}
