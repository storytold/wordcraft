//! The text of every story, as Word's character stream: paragraphs and table cells end with
//! their marks, fields become begin/separator/end characters, pictures and note references
//! special characters, each character carrying its CHPX and each paragraph its PAPX
//! ([MS-DOC] §2.4.1 Retrieving Text, §2.4.3 Table Text).

use std::collections::HashMap;
use std::sync::Arc;

use wordcraft_doc::para::{InlineObject, NoteKind, Paragraph};
use wordcraft_doc::props::{CharProps, ParaProps};
use wordcraft_doc::section::SectionProps;
use wordcraft_doc::table::Table;
use wordcraft_doc::{Block, Document};

use super::grpprl::{self, CharCtx, Grpprl, ParaCtx};
use super::picture::Pictures;
use super::tables;

/// Paragraph mark, cell or row end, section mark and the other characters Word gives meaning.
pub(crate) const PARA: u16 = 0x0D;
pub(crate) const CELL: u16 = 0x07;
pub(crate) const SECTION: u16 = 0x0C;
const FIELD_BEGIN: u16 = 0x13;
const FIELD_SEP: u16 = 0x14;
const FIELD_END: u16 = 0x15;
const NOTE_REF: u16 = 0x02;
const PICTURE: u16 = 0x01;

/// Deepest field nesting written; deeper fields keep their result text only.
const MAX_FIELD_DEPTH: usize = 20;

/// One paragraph's mark: the CP after it, the paragraph's style and PAPX grpprl.
pub(crate) struct ParaEnd {
    pub(crate) end_cp: u32,
    pub(crate) istd: u16,
    pub(crate) grpprl: Arc<Vec<u8>>,
}

/// One field character ([MS-DOC] §2.9.84 Fld): its CP (absolute) and its two bytes.
pub(crate) struct FieldChar {
    pub(crate) cp: u32,
    pub(crate) fld: [u8; 2],
}

/// A note reference in the main text: its CP, kind, story (part id) and whether it is numbered
/// automatically.
pub(crate) struct NoteRefAt {
    pub(crate) cp: u32,
    pub(crate) kind: NoteKind,
    pub(crate) part: u32,
    pub(crate) auto: bool,
}

/// What the writer needs to look up while building the text.
pub(crate) struct Lookup<'d> {
    pub(crate) doc: &'d Document,
    /// Style id → istd.
    pub(crate) styles: HashMap<String, u16>,
    /// The istd of `Normal`.
    pub(crate) normal: u16,
    /// Model num id → 1-based LFO index.
    pub(crate) lfos: HashMap<u32, u16>,
    /// Font names in table order and their indexes.
    pub(crate) fonts: Vec<String>,
    font_index: HashMap<String, u16>,
    /// Revision authors (`SttbfRMark`, index 0 = "Unknown") and their indexes.
    pub(crate) authors: Vec<String>,
}

impl<'d> Lookup<'d> {
    pub(crate) fn new(doc: &'d Document, styles: HashMap<String, u16>, normal: u16, lfos: HashMap<u32, u16>) -> Self {
        Lookup { doc, styles, normal, lfos, fonts: Vec::new(), font_index: HashMap::new(), authors: vec!["Unknown".to_string()] }
    }
}

impl CharCtx for Lookup<'_> {
    fn font(&mut self, name: &str) -> u16 {
        let name = name.trim();
        if let Some(&i) = self.font_index.get(name) {
            return i;
        }
        // The font table keeps names Word can store (it counts them in 16-bit units).
        let i = u16::try_from(self.fonts.len()).unwrap_or(0).min(0x7FFF);
        if self.fonts.len() < 0x7FFF {
            self.fonts.push(name.chars().take(31).collect());
            self.font_index.insert(name.to_string(), i);
        }
        i
    }

    fn char_style(&self, id: &str) -> Option<u16> {
        let st = self.doc.styles.get(id)?;
        (st.kind == wordcraft_doc::styles::StyleKind::Character).then(|| self.styles.get(id).copied()).flatten()
    }

    fn revision(&mut self, rev: u32) -> Option<(u16, u32)> {
        let r = self.doc.revisions.get(rev as usize)?;
        let author = if r.author.trim().is_empty() { "Unknown" } else { r.author.trim() };
        let ibst = match self.authors.iter().position(|a| a == author) {
            Some(i) => i,
            None => {
                self.authors.push(author.to_string());
                self.authors.len() - 1
            }
        };
        Some((u16::try_from(ibst).unwrap_or(0), super::dttm(&r.date)))
    }
}

impl ParaCtx for Lookup<'_> {
    fn lfo(&self, num: u32) -> u16 {
        self.lfos.get(&num).copied().unwrap_or(0)
    }
}

/// Grpprls are shared: a run or paragraph names one by its index.
#[derive(Default)]
struct Interned {
    all: Vec<Arc<Vec<u8>>>,
    index: HashMap<Arc<Vec<u8>>, usize>,
}

impl Interned {
    fn get(&mut self, g: Vec<u8>) -> usize {
        if let Some(&i) = self.index.get(&g) {
            return i;
        }
        let g = Arc::new(g);
        self.all.push(g.clone());
        self.index.insert(g, self.all.len() - 1);
        self.all.len() - 1
    }
}

/// Counts of content the binary format (as written here) leaves out, logged once per save.
#[derive(Default, Debug)]
pub(crate) struct Dropped {
    pub(crate) shapes: usize,
    pub(crate) comments: usize,
    pub(crate) nested_tables: usize,
    pub(crate) floating_pictures: usize,
    pub(crate) pictures: usize,
    pub(crate) equations: usize,
}

/// The character stream of every story, with what each character and paragraph carries.
pub(crate) struct Text<'d> {
    pub(crate) look: Lookup<'d>,
    pub(crate) units: Vec<u16>,
    /// Run starts: (CP, interned CHPX), each run up to the next start.
    runs: Vec<(u32, usize)>,
    chpx: Interned,
    /// CHPX → the same with `sprmCFSpec`, for field characters.
    spec: HashMap<usize, usize>,
    pub(crate) paras: Vec<ParaEnd>,
    /// Field characters of the subdocument being written, then of each finished one.
    pub(crate) fields: Vec<FieldChar>,
    /// Bookmarks: name, start CP, end CP (absolute), in the order they start.
    pub(crate) bookmarks: Vec<(String, u32, Option<u32>)>,
    pub(crate) notes: Vec<NoteRefAt>,
    /// Section ends in the main text: the CP after each section mark, and its properties.
    pub(crate) sections: Vec<(u32, SectionProps)>,
    pub(crate) pictures: Pictures,
    pub(crate) dropped: Dropped,
    /// Fields begun with a `FieldStart` and not yet ended (their nesting depth counts here).
    open_fields: usize,
    /// The note story being written: its own reference mark is its leading 0x02.
    note: Option<(NoteKind, u32)>,
    /// Inside a table cell: section marks can't end a cell's paragraph.
    in_cell: bool,
}

impl<'d> Text<'d> {
    pub(crate) fn new(look: Lookup<'d>) -> Self {
        Text {
            look,
            units: Vec::new(),
            runs: Vec::new(),
            chpx: Interned::default(),
            spec: HashMap::new(),
            paras: Vec::new(),
            fields: Vec::new(),
            bookmarks: Vec::new(),
            notes: Vec::new(),
            sections: Vec::new(),
            pictures: Pictures::default(),
            dropped: Dropped::default(),
            open_fields: 0,
            note: None,
            in_cell: false,
        }
    }

    /// The CP the next character gets.
    pub(crate) fn cp(&self) -> u32 {
        u32::try_from(self.units.len()).unwrap_or(u32::MAX)
    }

    /// The character runs: (start CP, end CP, CHPX grpprl).
    pub(crate) fn char_runs(&self) -> Vec<(u32, u32, &[u8])> {
        let end = self.cp();
        self.runs
            .iter()
            .enumerate()
            .map(|(i, &(cp, g))| {
                let next = self.runs.get(i + 1).map_or(end, |r| r.0);
                (cp, next, self.chpx.all.get(g).map_or(&[][..], |v| v.as_slice()))
            })
            .collect()
    }

    fn emit(&mut self, unit: u16, chpx: usize) {
        let cp = self.cp();
        if self.runs.last().is_none_or(|&(_, g)| g != chpx) {
            self.runs.push((cp, chpx));
        }
        self.units.push(unit);
    }

    /// The interned CHPX of `props`, plus `extra` sprms.
    fn chpx_of(&mut self, props: &CharProps, extra: Option<&Grpprl>) -> usize {
        let mut g = grpprl::chpx(props, &mut self.look);
        if let Some(e) = extra {
            g.extend(e);
        }
        g.truncate_to(255);
        self.chpx.get(g.0)
    }

    fn text(&mut self, s: &str, chpx: usize) {
        for c in s.chars() {
            self.char(c, chpx);
        }
    }

    /// One character of text, mapped to what Word stores for it.
    fn char(&mut self, c: char, chpx: usize) {
        let unit = match c {
            '\n' => 0x0B,
            '\t' => 0x09,
            '\u{C}' => 0x0C,
            '\u{E}' => 0x0E,
            '\u{2011}' => 0x1E,
            '\u{AD}' => 0x1F,
            '\r' | '\u{FFFC}' => return,
            c if (c as u32) < 0x20 => return,
            c => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf).iter() {
                    self.emit(*u, chpx);
                }
                return;
            }
        };
        self.emit(unit, chpx);
    }

    /// A field character: the text's formatting with `sprmCFSpec` on, as every field character
    /// MUST have ([MS-DOC] §2.8.25).
    fn field_char(&mut self, ch: u16, second: u8, chpx: usize) {
        let spec = match self.spec.get(&chpx) {
            Some(&s) => s,
            None => {
                let mut g = Grpprl(self.chpx.all.get(chpx).map_or_else(Vec::new, |g| g.as_ref().clone()));
                g.truncate_to(252);
                g.u8(grpprl::C_SPEC, 1);
                let s = self.chpx.get(g.0);
                self.spec.insert(chpx, s);
                s
            }
        };
        self.fields.push(FieldChar { cp: self.cp(), fld: [ch as u8, second] });
        self.emit(ch, spec);
    }

    /// A whole field: begin, its code, separator and result, end.
    fn field(&mut self, instr: &str, result: Option<&str>, locked: bool, dirty: bool, chpx: usize) {
        if self.open_fields >= MAX_FIELD_DEPTH {
            if let Some(r) = result {
                self.text(r, chpx);
            }
            return;
        }
        self.field_char(FIELD_BEGIN, flt(instr), chpx);
        self.text(&format!(" {} ", instr.trim()), chpx);
        if let Some(r) = result {
            self.field_char(FIELD_SEP, 0, chpx);
            self.text(r, chpx);
        }
        self.field_char(FIELD_END, end_flags(result.is_some(), locked, dirty, self.open_fields > 0), chpx);
    }

    /// Write `blocks` as a story's paragraphs and tables.
    pub(crate) fn blocks(&mut self, blocks: &[Arc<Block>], top: bool) {
        for b in blocks {
            match &**b {
                Block::Para(p) => self.paragraph(p, top, None),
                Block::Table(t) => self.table(t),
            }
        }
    }

    /// Write a note's story, its reference mark first.
    pub(crate) fn note_story(&mut self, kind: NoteKind, part: u32, blocks: &[Arc<Block>]) {
        let own_mark = blocks
            .first()
            .and_then(|b| b.as_para())
            .is_some_and(|p| p.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { kind: k, id, .. } if *k == kind && *id == part)));
        self.note = Some((kind, part));
        if !own_mark {
            let style = self.note_ref_style(kind);
            let chpx = self.special(&style);
            self.emit(NOTE_REF, chpx);
        }
        if blocks.is_empty() {
            self.mark(&Paragraph::new(), PARA, &Grpprl::default());
        }
        self.blocks(blocks, false);
        self.note = None;
    }

    /// A character style for note references ("footnote reference"), when the document has it.
    fn note_ref_style(&self, kind: NoteKind) -> CharProps {
        let id = match kind {
            NoteKind::Footnote => "FootnoteReference",
            NoteKind::Endnote => "EndnoteReference",
        };
        let style = self.look.styles.contains_key(id).then(|| id.to_string());
        CharProps { style, ..Default::default() }
    }

    /// The CHPX of a special character (a note reference, a picture): `sprmCFSpec` on.
    fn special(&mut self, props: &CharProps) -> usize {
        let mut extra = Grpprl::default();
        extra.u8(grpprl::C_SPEC, 1);
        self.chpx_of(props, Some(&extra))
    }

    /// One paragraph: its text, then its mark (`mark` overrides the paragraph mark, for cells;
    /// a paragraph ending a main-text section takes a section mark).
    pub(crate) fn paragraph(&mut self, p: &Paragraph, top: bool, cell: Option<(u16, &Grpprl)>) {
        let mut objects = p.objects.iter();
        let mut link: Option<(String, usize)> = None;
        let mut at = 0usize;
        for run in &p.runs {
            let end = at.saturating_add(run.len).min(p.text.len());
            let Some(s) = p.text.get(at..end) else { break };
            at = end;
            if s.is_empty() {
                continue;
            }
            // A run's link becomes a HYPERLINK field around the runs that share it.
            let mut props = run.props.clone();
            let target = props.link.take();
            if link.as_ref().map(|l| &l.0) != target.as_ref() {
                if let Some((_, chpx)) = link.take() {
                    self.field_char(FIELD_END, end_flags(true, false, false, self.open_fields > 0), chpx);
                }
                if let Some(t) = &target {
                    let chpx = self.chpx_of(&props, None);
                    self.field_char(FIELD_BEGIN, FLT_HYPERLINK, chpx);
                    self.text(&format!(" {} ", hyperlink_instr(t)), chpx);
                    self.field_char(FIELD_SEP, 0, chpx);
                    link = Some((t.clone(), chpx));
                }
            }
            let chpx = self.chpx_of(&props, None);
            for c in s.chars() {
                if c == '\u{FFFC}' {
                    if let Some(o) = objects.next() {
                        self.object(o, &props, chpx);
                    }
                } else {
                    self.char(c, chpx);
                }
            }
        }
        if let Some((_, chpx)) = link.take() {
            self.field_char(FIELD_END, end_flags(true, false, false, self.open_fields > 0), chpx);
        }
        match cell {
            Some((mark, extra)) => self.mark(p, mark, extra),
            None => {
                let section = top && !self.in_cell && p.section.is_some();
                self.mark(p, if section { SECTION } else { PARA }, &Grpprl::default());
                if section && let Some(s) = &p.section {
                    self.sections.push((self.cp(), (**s).clone()));
                }
            }
        }
    }

    /// The paragraph mark: its character formatting is the paragraph's mark formatting; the
    /// PAPX carries the style and paragraph properties (plus `extra`, for table cells).
    pub(crate) fn mark(&mut self, p: &Paragraph, mark: u16, extra: &Grpprl) {
        let chpx = self.chpx_of(&p.mark, None);
        self.emit(mark, chpx);
        let (istd, g) = self.papx_of(&p.props, extra);
        self.paras.push(ParaEnd { end_cp: self.cp(), istd, grpprl: g });
    }

    pub(crate) fn papx_of(&mut self, props: &ParaProps, extra: &Grpprl) -> (u16, Arc<Vec<u8>>) {
        let istd = props.style.as_deref().and_then(|s| self.look.styles.get(s).copied()).unwrap_or(self.look.normal);
        let mut g = grpprl::papx(props, &self.look);
        g.extend(extra);
        g.truncate_to(super::fkp::MAX_PAPX_GRPPRL);
        (istd, Arc::new(g.0))
    }

    fn object(&mut self, o: &InlineObject, props: &CharProps, chpx: usize) {
        match o {
            InlineObject::Field { instr, result, locked, dirty, .. } => self.field(instr, Some(result), *locked, *dirty, chpx),
            InlineObject::FieldStart { instr, locked, dirty } => {
                if self.open_fields >= MAX_FIELD_DEPTH {
                    self.open_fields += 1;
                    return;
                }
                self.field_char(FIELD_BEGIN, flt(instr), chpx);
                self.text(&format!(" {} ", instr.trim()), chpx);
                self.field_char(FIELD_SEP, 0, chpx);
                // The end's flags are decided now: they don't depend on the result.
                self.open_fields += 1;
                let _ = (locked, dirty);
            }
            InlineObject::FieldEnd => {
                if self.open_fields == 0 {
                    return;
                }
                self.open_fields -= 1;
                if self.open_fields < MAX_FIELD_DEPTH {
                    self.field_char(FIELD_END, end_flags(true, false, false, self.open_fields > 0), chpx);
                }
            }
            InlineObject::NoteRef { kind, id, custom } => {
                if self.note == Some((*kind, *id)) {
                    let c = self.special(props);
                    self.emit(NOTE_REF, c);
                } else if self.note.is_none() {
                    let cp = self.cp();
                    let c = self.special(props);
                    self.emit(NOTE_REF, c);
                    self.notes.push(NoteRefAt { cp, kind: *kind, part: *id, auto: custom.is_empty() });
                }
            }
            InlineObject::BookmarkStart { name } => {
                if !name.is_empty() && self.bookmarks.iter().all(|(n, _, _)| n != name) {
                    self.bookmarks.push((name.clone(), self.cp(), None));
                }
            }
            InlineObject::BookmarkEnd { name } => {
                let cp = self.cp();
                if let Some(b) = self.bookmarks.iter_mut().find(|(n, _, e)| n == name && e.is_none()) {
                    b.2 = Some(cp);
                }
            }
            InlineObject::Image { media, w, h, crop, float, alt, .. } => {
                if float.wrap != wordcraft_doc::para::Wrap::Inline {
                    self.dropped.floating_pictures += 1;
                }
                let Some(bytes) = self.look.doc.media.get(media).cloned() else {
                    self.dropped.pictures += 1;
                    return;
                };
                match self.pictures.add(&bytes, *w, *h, *crop, alt) {
                    Some(fc) => {
                        let mut extra = Grpprl::default();
                        extra.u8(grpprl::C_SPEC, 1);
                        extra.u32(grpprl::C_PIC_LOCATION, fc);
                        let c = self.chpx_of(props, Some(&extra));
                        self.emit(PICTURE, c);
                    }
                    None => self.dropped.pictures += 1,
                }
            }
            InlineObject::Equation { linear, .. } => {
                self.dropped.equations += 1;
                self.text(linear, chpx);
            }
            InlineObject::Ruby { base, .. } => self.text(base, chpx),
            InlineObject::PositionalTab { .. } => self.emit(0x09, chpx),
            InlineObject::Opaque { text, .. } => self.text(text, chpx),
            InlineObject::CommentStart { .. } => self.dropped.comments += 1,
            InlineObject::Shape { .. } | InlineObject::Group { .. } | InlineObject::Graphic { .. } => self.dropped.shapes += 1,
            InlineObject::CommentEnd { .. }
            | InlineObject::ControlStart { .. }
            | InlineObject::ControlEnd
            | InlineObject::PermStart { .. }
            | InlineObject::PermEnd { .. } => {}
        }
    }

    /// End fields left open at the end of a story, so every begin has its end; the ends get a
    /// paragraph of their own, as the story must end with a mark.
    pub(crate) fn close_fields(&mut self) {
        let mut wrote = false;
        while self.open_fields > 0 {
            self.open_fields -= 1;
            if self.open_fields < MAX_FIELD_DEPTH {
                let chpx = self.chpx.get(Vec::new());
                self.field_char(FIELD_END, end_flags(true, false, false, self.open_fields > 0), chpx);
                wrote = true;
            }
        }
        if wrote {
            self.mark(&Paragraph::new(), PARA, &Grpprl::default());
        }
    }

    /// A table: each cell's paragraphs end with a cell mark, each row with a row-end mark that
    /// carries the row's table properties ([MS-DOC] §2.4.3).
    fn table(&mut self, t: &Table) {
        let was = self.in_cell;
        self.in_cell = true;
        let rows = t.rows.len();
        let style_borders = t.props.style.as_deref().and_then(|s| self.look.doc.styles.get(s)).and_then(|s| s.table.as_ref()).and_then(|p| p.borders);
        for (ri, row) in t.rows.iter().enumerate() {
            // A row has at most 63 cells: the text of any past them goes in the last one.
            let last = row.cells.len().min(tables::MAX_CELLS).saturating_sub(1);
            let mut in_table = Grpprl::default();
            grpprl::in_table(&mut in_table, false);
            for (ci, cell) in row.cells.iter().enumerate() {
                let mut paras: Vec<&Paragraph> = Vec::new();
                let mut owned: Vec<Paragraph> = Vec::new();
                flatten(&cell.blocks, &mut paras, &mut owned, &mut self.dropped.nested_tables);
                let all: Vec<&Paragraph> = if owned.is_empty() { paras } else { owned.iter().collect() };
                // Whether this cell's last paragraph ends the (possibly merged) cell.
                let ends_cell = ci < last || ci + 1 == row.cells.len();
                if all.is_empty() {
                    let mark = if ends_cell { CELL } else { PARA };
                    self.paragraph(&Paragraph::new(), false, Some((mark, &in_table)));
                    continue;
                }
                for (i, p) in all.iter().enumerate() {
                    let mark = if ends_cell && i + 1 == all.len() { CELL } else { PARA };
                    self.paragraph(p, false, Some((mark, &in_table)));
                }
            }
            let tapx = tables::row_end(t, ri, rows, style_borders);
            let mut extra = Grpprl::default();
            grpprl::in_table(&mut extra, true);
            extra.extend(&tapx);
            self.mark(&Paragraph::new(), CELL, &extra);
        }
        self.in_cell = was;
    }
}

/// A cell's paragraphs, nested tables flattened into their own paragraphs (the writer keeps one
/// level of tables). When there is nothing to flatten `paras` borrows; else `owned` holds all.
fn flatten<'a>(blocks: &'a [Arc<Block>], paras: &mut Vec<&'a Paragraph>, owned: &mut Vec<Paragraph>, nested: &mut usize) {
    if blocks.iter().all(|b| b.as_para().is_some()) {
        paras.extend(blocks.iter().filter_map(|b| b.as_para()));
        return;
    }
    *nested += 1;
    fn walk(blocks: &[Arc<Block>], out: &mut Vec<Paragraph>, depth: usize) {
        for b in blocks {
            match &**b {
                Block::Para(p) => out.push(p.clone()),
                Block::Table(t) if depth < 16 => {
                    for row in &t.rows {
                        for c in &row.cells {
                            walk(&c.blocks, out, depth + 1);
                        }
                    }
                }
                Block::Table(_) => {}
            }
        }
    }
    walk(blocks, owned, 0);
}

/// `flt` of a HYPERLINK field.
const FLT_HYPERLINK: u8 = 88;

/// The field type (`flt`, [MS-DOC] §2.9.87) of a field code, from its first word; 0x01 (an
/// unknown keyword) for anything not in the table, which Word reparses from the code anyway.
fn flt(instr: &str) -> u8 {
    let word = instr.split_whitespace().next().unwrap_or("").trim_start_matches('=').to_ascii_uppercase();
    if instr.trim_start().starts_with('=') {
        return 34;
    }
    match word.as_str() {
        "REF" => 3,
        "FTNREF" => 5,
        "SET" => 6,
        "IF" => 7,
        "INDEX" => 8,
        "TC" => 9,
        "STYLEREF" => 10,
        "SEQ" => 12,
        "TOC" => 13,
        "INFO" => 14,
        "TITLE" => 15,
        "SUBJECT" => 16,
        "AUTHOR" => 17,
        "KEYWORDS" => 18,
        "COMMENTS" => 19,
        "LASTSAVEDBY" => 20,
        "CREATEDATE" => 21,
        "SAVEDATE" => 22,
        "PRINTDATE" => 23,
        "REVNUM" => 24,
        "EDITTIME" => 25,
        "NUMPAGES" => 26,
        "NUMWORDS" => 27,
        "NUMCHARS" => 28,
        "FILENAME" => 29,
        "TEMPLATE" => 30,
        "DATE" => 31,
        "TIME" => 32,
        "PAGE" => 33,
        "QUOTE" => 35,
        "INCLUDE" => 36,
        "PAGEREF" => 37,
        "ASK" => 38,
        "FILLIN" => 39,
        "DATA" => 40,
        "NEXT" => 41,
        "NEXTIF" => 42,
        "SKIPIF" => 43,
        "MERGEREC" => 44,
        "DDE" => 45,
        "DDEAUTO" => 46,
        "GLOSSARY" => 47,
        "PRINT" => 48,
        "EQ" => 49,
        "GOTOBUTTON" => 50,
        "MACROBUTTON" => 51,
        "AUTONUMOUT" => 52,
        "AUTONUMLGL" => 53,
        "AUTONUM" => 54,
        "IMPORT" => 55,
        "LINK" => 56,
        "SYMBOL" => 57,
        "EMBED" => 58,
        "MERGEFIELD" => 59,
        "USERNAME" => 60,
        "USERINITIALS" => 61,
        "USERADDRESS" => 62,
        "BARCODE" => 63,
        "DOCVARIABLE" => 64,
        "SECTION" => 65,
        "SECTIONPAGES" => 66,
        "INCLUDEPICTURE" => 67,
        "INCLUDETEXT" => 68,
        "FILESIZE" => 69,
        "FORMTEXT" => 70,
        "FORMCHECKBOX" => 71,
        "NOTEREF" => 72,
        "TOA" => 73,
        "TA" => 74,
        "MERGESEQ" => 75,
        "AUTOTEXT" => 79,
        "COMPARE" => 80,
        "ADDIN" => 81,
        "FORMDROPDOWN" => 83,
        "ADVANCE" => 84,
        "DOCPROPERTY" => 85,
        "CONTROL" => 87,
        "HYPERLINK" => 88,
        "AUTOTEXTLIST" => 89,
        "LISTNUM" => 90,
        "HTMLCONTROL" => 91,
        "BIDIOUTLINE" => 92,
        "ADDRESSBLOCK" => 93,
        "GREETINGLINE" => 94,
        "SHAPE" => 95,
        "CITATION" => 96,
        "BIBLIOGRAPHY" => 97,
        _ => 1,
    }
}

/// The second byte of a field end ([MS-DOC] §2.9.84 grffldEnd): `fHasSep` when there is a
/// result, `fLocked`, `fResultsDirty` and `fNested`.
fn end_flags(has_sep: bool, locked: bool, dirty: bool, nested: bool) -> u8 {
    let mut f = 0u8;
    if dirty {
        f |= 0x04;
    }
    if locked {
        f |= 0x10;
    }
    if nested {
        f |= 0x40;
    }
    if has_sep {
        f |= 0x80;
    }
    f
}

/// The field code of a link: `HYPERLINK "url"`, `HYPERLINK \l "bookmark"` for an internal
/// one, or both for a link into another document's bookmark. A quote or backslash in an
/// argument is escaped with a backslash, as field codes do.
fn hyperlink_instr(target: &str) -> String {
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    match target.split_once('#') {
        Some(("", frag)) => format!("HYPERLINK \\l \"{}\"", q(frag)),
        Some((url, frag)) if !frag.is_empty() && !url.contains("://") => format!("HYPERLINK \"{}\" \\l \"{}\"", q(url), q(frag)),
        _ => format!("HYPERLINK \"{}\"", q(target)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_types() {
        assert_eq!(flt("PAGE"), 33);
        assert_eq!(flt(" hyperlink \"x\""), 88);
        assert_eq!(flt("= 2 + 2"), 34);
        assert_eq!(flt("NOSUCHFIELD"), 1);
    }

    #[test]
    fn hyperlink_codes() {
        assert_eq!(hyperlink_instr("https://example.org/a#b"), "HYPERLINK \"https://example.org/a#b\"");
        assert_eq!(hyperlink_instr("#Intro"), "HYPERLINK \\l \"Intro\"");
        assert_eq!(hyperlink_instr("other.doc#Part"), "HYPERLINK \"other.doc\" \\l \"Part\"");
        assert_eq!(crate::hyperlink_target(&hyperlink_instr("#Intro")).as_deref(), Some("#Intro"));
        assert_eq!(crate::hyperlink_target(&hyperlink_instr("https://example.org/?q=\"x\"")).as_deref(), Some("https://example.org/?q=\"x\""));
        assert_eq!(crate::hyperlink_target(&hyperlink_instr(r"file:///C:\docs\report.doc")).as_deref(), Some(r"file:///C:\docs\report.doc"));
        assert_eq!(crate::hyperlink_target(&hyperlink_instr("#a\"b")).as_deref(), Some("#a\"b"));
    }
}
