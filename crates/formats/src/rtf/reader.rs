use super::date::{dttm_to_iso, iso_date};
use super::parse::column_grid;
use super::state::{
    CellDef, CommentData, Dest, HeaderFooter, MARK_CLOSE, MARK_OPEN, MAX_GROUPS, MAX_NOTE_BLOCKS, MAX_NOTES, MAX_REV_AUTHORS, MAX_SECTIONS, NoteData,
    OldRow, Output, PageGeom, RowFmt, SavedRow, SecFmt, St, Sub, TableAcc,
};
use super::styles::{StyleDef, drop_style_formatting, style_fmt};
use super::{Arc, Cell, FBlock, FTable, Fmt, HashMap, HashSet, Inline, Kind, Meta, Para, Rgb, model};

#[derive(Default)]
pub(super) struct Reader {
    pub(super) st: St,
    stack: Vec<St>,
    overflow: usize,
    pub(super) fonts: HashMap<i32, Arc<str>>,
    pub(super) font_cur: i32,
    pub(super) font_name: String,
    /// Code pages of the fonts whose charset names one (`\fcharset204` → 1251).
    pub(super) font_cps: HashMap<i32, u16>,
    pub(super) font_cp_cur: Option<u16>,
    /// The document's code page (`\ansicpg`); 0 until it names one (Windows-1252 then).
    pub(super) ansicpg: u16,
    /// The default font (`\deff`), whose charset applies before any `\f`.
    pub(super) deff: i32,
    /// The document's page (`\paperw`, `\margl`…) and the current section's formatting.
    pub(super) page: PageGeom,
    pub(super) sec: SecFmt,
    /// The headers and footers of each section ended so far (`\sect`); `hf_parts` holds the
    /// current section's.
    pub(super) section_parts: Vec<Vec<(HeaderFooter, Vec<FBlock>)>>,
    /// The tracked change of each section ended so far: who made it, and the section's
    /// properties before it.
    pub(super) section_changes: Vec<Option<(model::RevMark, wordcraft_doc::SectionProps)>>,
    /// The section's formatting set aside while its `\oldsprops` group is read.
    sec_saved: Option<Box<SecFmt>>,
    /// The header or footer being read: the group depth it started at, which one, and the
    /// body's output, put back when the group closes.
    pub(super) hf: Option<(usize, HeaderFooter, Box<Output>)>,
    /// The current section's headers and footers, in order.
    pub(super) hf_parts: Vec<(HeaderFooter, Vec<FBlock>)>,
    /// The cell border side being described (`\clbrdrt`, `l`, `b`, `r`): its line style, width
    /// and colour follow.
    pub(super) cell_side: Option<char>,
    /// Each table row's cell right edges (`\cellx`, twips), for the table's column grid.
    table_rights: Vec<Vec<i32>>,
    pub(super) colors: Vec<Option<Rgb>>,
    pub(super) color_cur: (u8, u8, u8, bool),
    pub(super) styles: HashMap<i32, String>,
    /// Each style's own character formatting, as its `\stylesheet` entry writes it.
    pub(super) style_defs: HashMap<i32, StyleDef>,
    pub(super) style_cur: i32,
    pub(super) style_name: String,
    pub(super) style_def_cur: StyleDef,
    pub(super) meta: Meta,
    pub(super) info_text: String,
    // Lists: list id → ordered per level; override ls → list id.
    pub(super) lists: HashMap<i32, Vec<bool>>,
    pub(super) list_cur: Vec<bool>,
    pub(super) list_cur_id: i32,
    pub(super) overrides: HashMap<i32, i32>,
    pub(super) ov_list: i32,
    pub(super) ov_ls: i32,
    pub(super) fields: Vec<String>,
    // Picture being read.
    pub(super) pict_depth: Option<usize>,
    pub(super) pict_hex: Vec<u8>,
    pub(super) pict_half: Option<u8>,
    pub(super) pict_kind: u8,
    pub(super) pict_w: Option<f32>,
    pub(super) pict_h: Option<f32>,
    pub(super) pict_sx: f32,
    pub(super) pict_sy: f32,
    /// `\picw`/`\pich` as written, read for metafiles (0 when absent).
    pub(super) pict_nat: (i32, i32),
    /// The mapping mode `N` of `\wmetafileN`.
    pub(super) pict_mm: i32,
    pub(super) bkmk: Option<(usize, String)>,
    /// Authors of tracked changes (`\revtbl`), and the entry being read.
    rev_authors: Vec<String>,
    pub(super) rev_name: String,
    pub(super) skip_chars: usize,
    pub(super) pending_high: Option<u16>,
    /// A streaming decoder for the current multibyte code page, and the code page it was made for.
    /// A lead byte and its trail byte decode together even when a text word, a group boundary or the
    /// end of a chunk falls between them, so a run of any length never splits a character.
    pub(super) mb_decoder: Option<encoding_rs::Decoder>,
    pub(super) mb_cp: u16,
    // Output.
    pub(super) body: Vec<FBlock>,
    pub(super) para: Para,
    pub(super) para_has: bool,
    table: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    cell: Vec<FBlock>,
    pub(super) defs: Vec<CellDef>,
    pub(super) def_cur: CellDef,
    /// The row definition's table and row words, the row as it was before a tracked change
    /// (`\oldtprops`), and the row set aside while that group is read.
    pub(super) row_fmt: RowFmt,
    pub(super) old_row: Option<Box<OldRow>>,
    old_saved: Option<Box<SavedRow>>,
    /// The table being read: its formatting beyond the cells and its tracked changes.
    table_acc: TableAcc,
    pub(super) created: Option<(u32, u32, u32, u32, u32)>,
    pub(super) in_creatim: bool,
    pub(super) rule_pending: bool,
    pub(super) info_depth: Option<usize>,
    // Notes and comments (only when the caller wants them).
    pub(super) marks_on: bool,
    pub(super) sub: Option<Sub>,
    pub(super) notes: Vec<NoteData>,
    pub(super) comments: Vec<CommentData>,
    pub(super) atn_text: String,
    pub(super) atn_initials: String,
    pub(super) atn_author: String,
    pub(super) atn_date: String,
    pub(super) atn_parts: Option<(u32, u32, u32, u32, u32)>,
    pub(super) atn_ref: Option<i32>,
    pub(super) last_end: Option<i32>,
    pub(super) range_starts: HashSet<i32>,
    pub(super) range_ends: HashSet<i32>,
    pub(super) claimed: HashSet<i32>,
    pub(super) synth: i32,
}

impl Reader {
    pub(super) fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Set the output aside (a header or footer starts) or put it back (it ended), returning the
    /// other one.
    fn swap_output(&mut self, mut o: Output) -> Output {
        std::mem::swap(&mut o.body, &mut self.body);
        std::mem::swap(&mut o.para, &mut self.para);
        std::mem::swap(&mut o.para_has, &mut self.para_has);
        std::mem::swap(&mut o.table, &mut self.table);
        std::mem::swap(&mut o.row, &mut self.row);
        std::mem::swap(&mut o.cell, &mut self.cell);
        std::mem::swap(&mut o.defs, &mut self.defs);
        std::mem::swap(&mut o.def_cur, &mut self.def_cur);
        std::mem::swap(&mut o.row_fmt, &mut self.row_fmt);
        std::mem::swap(&mut o.old_row, &mut self.old_row);
        std::mem::swap(&mut o.rule_pending, &mut self.rule_pending);
        std::mem::swap(&mut o.table_rights, &mut self.table_rights);
        std::mem::swap(&mut o.table_acc, &mut self.table_acc);
        o
    }

    /// The border of the cell side being described, else of the paragraph side.
    pub(super) fn cell_border_slot(&mut self) -> Option<&mut Option<wordcraft_doc::props::Border>> {
        // A table's sides (`\trbrdr…`) are kept upper case.
        let (b, side) = match self.cell_side {
            Some(c) if c.is_ascii_uppercase() => (&mut self.row_fmt.borders, c.to_ascii_lowercase()),
            Some(c) => (&mut self.def_cur.borders, c),
            None => (&mut self.st.borders, self.st.border_side?),
        };
        match side {
            't' => Some(&mut b.top),
            'l' => Some(&mut b.left),
            'b' => Some(&mut b.bottom),
            'r' => Some(&mut b.right),
            'h' => Some(&mut b.between),
            'v' => Some(&mut b.inside_v),
            _ => None,
        }
    }

    /// Whether groups nest deeper than the reader follows (their ends restore no state).
    pub(super) fn overflowed(&self) -> bool {
        self.overflow > 0
    }

    /// An `\oldsprops` group starts: its words are read over a copy of the section's formatting.
    pub(super) fn start_old_section(&mut self) {
        let mut old = self.sec.clone();
        old.old = None;
        self.sec_saved = Some(Box::new(std::mem::replace(&mut self.sec, old)));
    }

    /// The `\oldsprops` group ended: what it read is the section's formatting before its tracked
    /// change.
    fn end_old_section(&mut self) {
        let Some(saved) = self.sec_saved.take() else { return };
        let old = std::mem::replace(&mut self.sec, *saved);
        // Who and when may be written in the group too (the spec's old properties syntax).
        if old.auth.is_some() {
            self.sec.auth = old.auth;
        }
        if old.date.is_some() {
            self.sec.date = old.date;
        }
        self.sec.old = Some(Box::new(old));
    }

    /// The tracked change of the current section: who made it, and its properties before.
    pub(super) fn section_change(&self) -> Option<(model::RevMark, wordcraft_doc::SectionProps)> {
        let old = self.sec.old.as_deref()?;
        Some((self.rev_mark(self.sec.auth.unwrap_or(0), self.sec.date.unwrap_or(0)), old.props(&self.page)))
    }

    /// An `\oldtprops` group starts: the row's table and row words are read over a copy of
    /// them, its cell definitions from none.
    pub(super) fn start_old_row(&mut self) {
        let saved = SavedRow {
            defs: std::mem::take(&mut self.defs),
            def_cur: std::mem::take(&mut self.def_cur),
            fmt: self.row_fmt.clone(),
            cell_side: self.cell_side.take(),
        };
        self.old_saved = Some(Box::new(saved));
    }

    /// The `\oldtprops` group ended: what it read is the row definition before its tracked
    /// change, and the row's own definition comes back.
    fn end_old_row(&mut self) {
        let Some(saved) = self.old_saved.take() else { return };
        let saved = *saved;
        let old = OldRow { fmt: std::mem::replace(&mut self.row_fmt, saved.fmt), defs: std::mem::replace(&mut self.defs, saved.defs) };
        self.def_cur = saved.def_cur;
        self.cell_side = saved.cell_side;
        if old.fmt.auth.is_some() {
            self.row_fmt.auth = old.fmt.auth;
        }
        if old.fmt.date.is_some() {
            self.row_fmt.date = old.fmt.date;
        }
        self.old_row = Some(Box::new(old));
    }

    /// A header or footer group starts: read it into an output of its own.
    pub(super) fn start_header_footer(&mut self, which: HeaderFooter) {
        let body = self.swap_output(Output::default());
        self.hf = Some((self.depth(), which, Box::new(body)));
        self.st.intbl = false;
    }

    /// The header or footer group closed: keep what it held and go back to the body.
    pub(super) fn end_header_footer(&mut self) {
        let Some((_, which, body)) = self.hf.take() else { return };
        if self.para_has || !self.para.inlines.is_empty() {
            self.end_para(false);
        }
        self.close_table();
        let part = self.swap_output(*body);
        // A section names each one once; a repeat replaces it.
        self.hf_parts.retain(|(w, _)| *w != which);
        if self.hf_parts.len() < 16 {
            self.hf_parts.push((which, part.body));
        }
    }

    /// A footnote, endnote or comment group starts: read it into an output of its own.
    pub(super) fn start_sub(&mut self, comment: bool) {
        if self.overflow > 0 || self.notes.len() >= MAX_NOTES || self.comments.len() >= MAX_NOTES {
            self.st.dest = Dest::Skip;
            return;
        }
        let saved = self.swap_output(Output::default());
        self.sub = Some(Sub { depth: self.depth(), comment, endnote: false, saved: Box::new(saved) });
        self.st.intbl = false;
    }

    /// A place marker as text in the current paragraph.
    pub(super) fn push_mark(&mut self, tag: char, n: i64, fmt: Fmt) {
        self.para.push_text(&format!("{MARK_OPEN}{tag}{n}{MARK_CLOSE}"), &fmt);
        self.para_has = true;
    }

    /// The footnote or comment group closed: keep its text and leave a marker in the body.
    pub(super) fn end_sub(&mut self) {
        let Some(sub) = self.sub.take() else { return };
        if self.para_has || !self.para.inlines.is_empty() {
            self.end_para(false);
        }
        self.close_table();
        let fmt = self.stack.last().map(|s| s.fmt.clone()).unwrap_or_default();
        let mut inner = self.swap_output(*sub.saved);
        inner.body.truncate(MAX_NOTE_BLOCKS);
        if !sub.comment {
            self.push_mark('N', self.notes.len() as i64, fmt);
            self.notes.push(NoteData { endnote: sub.endnote, blocks: inner.body });
            return;
        }
        let want = self.atn_ref.take().or(self.last_end.take());
        let id = match want {
            Some(i) if self.range_starts.contains(&i) && self.range_ends.contains(&i) && self.claimed.insert(i) => i,
            _ => {
                // No range of its own: the comment sits at this point.
                self.synth = self.synth.saturating_sub(1);
                let id = self.synth;
                self.push_mark('B', i64::from(id), fmt.clone());
                self.push_mark('E', i64::from(id), fmt);
                id
            }
        };
        let date =
            if !self.atn_date.is_empty() { std::mem::take(&mut self.atn_date) } else { self.atn_parts.take().map(iso_date).unwrap_or_default() };
        self.atn_parts = None;
        self.comments.push(CommentData {
            rtf_id: id,
            author: std::mem::take(&mut self.atn_author),
            initials: std::mem::take(&mut self.atn_initials),
            date,
            blocks: inner.body,
        });
    }

    /// A comment word's text group closed (`\atrfstart 3`, `\atnauthor Ann`…).
    pub(super) fn commit_atn(&mut self, kind: u8) {
        let text = std::mem::take(&mut self.atn_text);
        let text = text.trim();
        let id = text.parse::<i32>().ok();
        let fmt = self.stack.last().map(|s| s.fmt.clone()).unwrap_or_default();
        match kind {
            1 => {
                if let Some(id) = id {
                    self.range_starts.insert(id);
                    self.push_mark('B', i64::from(id), fmt);
                }
            }
            2 => {
                if let Some(id) = id {
                    self.range_ends.insert(id);
                    self.last_end = Some(id);
                    self.push_mark('E', i64::from(id), fmt);
                }
            }
            3 => self.atn_initials = text.to_string(),
            4 => self.atn_author = text.to_string(),
            5 => {
                // Either `\yr\mo\dy\hr\min` words (read as they came) or a packed date-time number.
                if let Some(parts) = self.atn_parts.take() {
                    self.atn_date = iso_date(parts);
                } else if let Some(n) = text.parse::<u32>().ok().filter(|n| *n != 0) {
                    self.atn_date = iso_date((1900 + ((n >> 20) & 0x1FF), (n >> 16) & 0xF, (n >> 11) & 0x1F, (n >> 6) & 0x1F, n & 0x3F));
                }
            }
            6 => self.atn_ref = id,
            _ => {}
        }
    }

    /// `\sect`: end the paragraph, and with it the section, which takes its formatting and the
    /// headers and footers read since the last break. Inside a header, footer or table, and past
    /// [`MAX_SECTIONS`], it only ends the paragraph.
    pub(super) fn end_section(&mut self) {
        self.end_para_mark();
        if self.hf.is_some() || self.sub.is_some() || self.st.intbl || self.section_parts.len() + 1 >= MAX_SECTIONS {
            return;
        }
        let props = self.sec.props(&self.page);
        let change = self.section_change();
        if let Some(FBlock::Para(p)) = self.body.last_mut() {
            p.section = Some(Box::new(props));
            self.section_parts.push(std::mem::take(&mut self.hf_parts));
            self.section_changes.push(change);
        }
    }

    /// A section formatting word: applied to the current section, unless it's inside a header
    /// or footer (whose `\sectd` isn't the body's). False when `w` isn't one.
    pub(super) fn section_word(&mut self, w: &str, p: Option<i32>) -> bool {
        use wordcraft_doc::section::{NumFormat, SectionStart};
        let on = p != Some(0);
        let pv = p.unwrap_or(0);
        // Inside a header or footer the word is read into a scratch copy and dropped.
        let mut scratch;
        let sec = if self.hf.is_some() || self.sub.is_some() {
            scratch = self.sec.clone();
            &mut scratch
        } else {
            &mut self.sec
        };
        match w {
            "sectd" => *sec = SecFmt::default(),
            "titlepg" => sec.title_page = on,
            "pgwsxn" => sec.page.w = Some(pv),
            "pghsxn" => sec.page.h = Some(pv),
            "marglsxn" => sec.page.left = Some(pv),
            "margrsxn" => sec.page.right = Some(pv),
            "margtsxn" => sec.page.top = Some(pv),
            "margbsxn" => sec.page.bottom = Some(pv),
            "headery" => sec.page.header = Some(pv),
            "footery" => sec.page.footer = Some(pv),
            "lndscpsxn" => sec.page.landscape = on,
            "sbknone" => sec.start = SectionStart::Continuous,
            "sbkcol" => sec.start = SectionStart::NextColumn,
            "sbkpage" => sec.start = SectionStart::NextPage,
            "sbkeven" => sec.start = SectionStart::EvenPage,
            "sbkodd" => sec.start = SectionStart::OddPage,
            "cols" => sec.cols = Some(pv),
            "colsx" => sec.colsx = Some(pv),
            "linebetcol" => sec.linebetcol = on,
            "pgnrestart" => sec.pgn_restart = true,
            "pgncont" => sec.pgn_restart = false,
            "pgnstarts" => sec.pgn_start = Some(pv),
            "pgndec" => sec.pgn_fmt = NumFormat::Decimal,
            "pgnucrm" => sec.pgn_fmt = NumFormat::UpperRoman,
            "pgnlcrm" => sec.pgn_fmt = NumFormat::LowerRoman,
            "pgnucltr" => sec.pgn_fmt = NumFormat::UpperLetter,
            "pgnlcltr" => sec.pgn_fmt = NumFormat::LowerLetter,
            "srauth" => sec.auth = Some(pv),
            // DTTMs are 32-bit patterns, written signed or unsigned.
            "srdate" => sec.date = Some(pv as u32),
            _ => return false,
        }
        true
    }

    pub(super) fn open(&mut self) {
        self.skip_chars = 0;
        if self.stack.len() >= MAX_GROUPS {
            self.overflow += 1;
            return;
        }
        self.stack.push(self.st.clone());
    }

    pub(super) fn close(&mut self) {
        // A group end decodes the multibyte bytes read in it, before the state is restored.
        self.finish_multibyte();
        self.skip_chars = 0;
        if self.overflow > 0 {
            self.overflow -= 1;
            return;
        }
        if self.sub.as_ref().is_some_and(|s| s.depth == self.depth()) {
            self.end_sub();
        }
        let closing = self.st.dest;
        // Destination-level commits.
        match closing {
            Dest::FontTbl if !self.font_name.is_empty() => self.commit_font(),
            Dest::StyleSheet if !self.style_name.is_empty() => self.commit_style(),
            Dest::RevTbl if !self.rev_name.trim().is_empty() => self.commit_author(),
            Dest::Info(k) => {
                let t = std::mem::take(&mut self.info_text).trim().to_string();
                match k {
                    1 => self.meta.title = t,
                    2 => self.meta.author = t,
                    3 => self.meta.subject = t,
                    4 => self.meta.keywords = t,
                    5 => self.meta.description = t,
                    _ => {}
                }
                self.in_creatim = false;
            }
            Dest::Atn(k) => self.commit_atn(k),
            _ => {}
        }
        let Some(mut prev) = self.stack.pop() else { return };
        // The end of a section's or a row's old formatting read in the body.
        if prev.dest == Dest::Body {
            match closing {
                Dest::OldSect => self.end_old_section(),
                Dest::OldRow => self.end_old_row(),
                _ => {}
            }
        }
        // The end of old formatting read in the body: the formatting it holds is the change's
        // old formatting (nested old-formatting groups only change their parent's copy).
        if let Dest::OldProps(para) = closing
            && prev.dest == Dest::Body
        {
            if para {
                prev.chg_p.old = Some(Box::new(self.para_props(&self.st)));
            } else {
                prev.chg_c.old = Some(Box::new(std::mem::take(&mut self.st.old_words)));
                if let Some(a) = self.st.old_rev.0 {
                    prev.chg_c.auth = a;
                }
                if let Some(d) = self.st.old_rev.1 {
                    prev.chg_c.date = d;
                }
            }
        }
        if self.pict_depth.is_some_and(|d| self.depth() < d) {
            self.pict_depth = None;
            self.finish_pict();
        }
        if let Some((d, _)) = &self.bkmk
            && self.depth() < *d
            && let Some((_, name)) = self.bkmk.take()
        {
            let name = name.trim().to_string();
            if !name.is_empty() {
                self.para.inlines.push(Inline::Anchor(name));
                self.para_has = true;
            }
        }
        self.st = prev;
        if self.hf.as_ref().is_some_and(|(d, _, _)| self.depth() < *d) {
            self.end_header_footer();
        }
    }

    pub(super) fn commit_author(&mut self) {
        let name = std::mem::take(&mut self.rev_name).trim().to_string();
        if self.rev_authors.len() < MAX_REV_AUTHORS {
            self.rev_authors.push(name);
        }
    }

    /// The tracked-change mark for `\revtbl` author index `i` and DTTM `dttm`; an index outside
    /// the table is "Unknown".
    pub(super) fn rev_mark(&self, i: i32, dttm: u32) -> model::RevMark {
        let author = usize::try_from(i).ok().and_then(|i| self.rev_authors.get(i)).cloned().unwrap_or_else(|| "Unknown".to_string());
        model::RevMark { author, date: dttm_to_iso(dttm).unwrap_or_default(), id: None }
    }

    /// `\par` (or `\sect`): end the paragraph, whose mark is tracked as inserted or deleted when the text
    /// at it is.
    pub(super) fn end_para_mark(&mut self) {
        let r = self.st.rev;
        self.para.mark_ins = r.ins.then(|| Box::new(self.rev_mark(r.auth, r.dttm)));
        self.para.mark_del = r.del.then(|| Box::new(self.rev_mark(r.authdel, r.dttmdel)));
        self.end_para(false);
    }

    /// End the current paragraph (`\par`, `\cell`).
    pub(super) fn end_para(&mut self, in_cell: bool) {
        let read = std::mem::take(&mut self.para);
        let props = self.para_props(&self.st);
        let mut p = Para { inlines: read.inlines, mark_ins: read.mark_ins, mark_del: read.mark_del, ..props };
        if p.kind == Kind::Normal && p.inlines.is_empty() && self.rule_pending {
            p.kind = Kind::Rule;
        }
        self.rule_pending = false;
        let c = &self.st.chg_p;
        if let Some(old) = &c.old {
            p.fmt_change = Some(Box::new(model::FmtChange { mark: self.rev_mark(c.auth, c.date), old: (**old).clone() }));
        }
        // Headings, code, titles and quotes keep their style formatting implicit, in the text's
        // formatting and in the formatting before a tracked change alike. The style is the
        // file's when its name gave the kind (not an outline level on another style).
        let base = if self.style_kind(&self.st) == Some(p.kind) { self.style_base(p.kind, self.st.style) } else { style_fmt(p.kind) };
        for i in &mut p.inlines {
            if let Inline::Text(_, f) = i {
                drop_style_formatting(p.kind, &base, f);
                if let Some(c) = f.fmt_change.as_deref_mut() {
                    drop_style_formatting(p.kind, &base, &mut c.old);
                }
            }
        }
        self.para_has = false;
        if self.st.intbl || in_cell {
            self.cell.push(FBlock::Para(p));
        } else {
            self.close_table();
            self.body.push(FBlock::Para(p));
        }
    }

    pub(super) fn end_cell(&mut self) {
        self.end_para(true);
        let blocks = std::mem::take(&mut self.cell);
        if self.row.len() < 63 {
            self.row.push(Cell { blocks, ..Default::default() });
        }
    }

    pub(super) fn end_row(&mut self) {
        if self.para_has {
            self.end_cell();
        }
        let mut cells = std::mem::take(&mut self.row);
        let defs = self.defs.clone();
        // The row's definition before a tracked change, and who made it when.
        let old = self.old_row.take();
        let mark = self.rev_mark(self.row_fmt.auth.unwrap_or(0), self.row_fmt.date.unwrap_or(0));
        let mut changed = false;
        let table_borders = self.row_fmt.visible_borders().is_some();
        let mut out: Vec<Cell> = Vec::new();
        let mut rights: Vec<i32> = Vec::new();
        for (i, mut c) in cells.drain(..).enumerate() {
            let d = defs.get(i).cloned().unwrap_or_default();
            // A merged-in cell widens the cell it joins.
            if d.hmerge == 2
                && let Some(last) = rights.last_mut()
            {
                *last = d.right;
            } else {
                rights.push(d.right);
            }
            c.shading = d.shading;
            c.borders = d.cell_borders(table_borders);
            c.header = self.row_fmt.header;
            c.valign = d.valign;
            c.fmt_change = old.as_ref().and_then(|o| cell_change(o.defs.get(i), &d, table_borders)).map(|old| {
                changed = true;
                Box::new(model::FmtChange { mark: mark.clone(), old })
            });
            match d.vmerge {
                1 => c.rowspan = 2,
                2 => c.covered = true,
                _ => {}
            }
            if d.hmerge == 2
                && let Some(prev) = out.last_mut()
            {
                prev.colspan = (prev.colspan + 1).min(63);
                continue;
            }
            out.push(c);
        }
        if !out.is_empty() && self.table.len() < wordcraft_doc::table::MAX_ROWS {
            rights.truncate(out.len());
            self.table.push(out);
            self.table_rights.push(rights);
            self.end_row_formatting(old.as_deref(), mark, changed);
        }
    }

    /// The table and row formatting of the row just read, and its tracked changes: the table's
    /// when its alignment, width or borders were others (the first row's counts), the row's when
    /// its height, keeping together or header flag were. `cells_changed`: one of its cells has a
    /// change of its own. A change that alters none of these is kept as the row's, so who made it
    /// and when stay.
    fn end_row_formatting(&mut self, old: Option<&OldRow>, mark: model::RevMark, cells_changed: bool) {
        let fmt = &self.row_fmt;
        let acc = &mut self.table_acc;
        acc.table.get_or_insert_with(|| fmt.clone());
        acc.heights.push(fmt.row_height());
        acc.keeps.push(fmt.keep);
        let mut row_change = None;
        if let Some(o) = old {
            let o = &o.fmt;
            let table_changed = (o.align, o.table_width(), o.visible_borders()) != (fmt.align, fmt.table_width(), fmt.visible_borders());
            if table_changed && acc.change.is_none() {
                acc.change = Some(Box::new(model::FmtChange { mark: mark.clone(), old: o.table_props() }));
            }
            let row_changed = (o.row_height(), o.keep, o.header) != (fmt.row_height(), fmt.keep, fmt.header);
            if row_changed || (!table_changed && !cells_changed) {
                let old = if row_changed { o.row_props() } else { fmt.row_props() };
                row_change = Some(Box::new(model::FmtChange { mark, old }));
            }
        }
        acc.row_changes.push(row_change);
    }

    pub(super) fn close_table(&mut self) {
        if !self.row.is_empty() || !self.cell.is_empty() {
            if !self.cell.is_empty() {
                let blocks = std::mem::take(&mut self.cell);
                self.row.push(Cell { blocks, ..Default::default() });
            }
            self.end_row();
        }
        let acc = std::mem::take(&mut self.table_acc);
        if !self.table.is_empty() {
            let mut rows = std::mem::take(&mut self.table);
            let rights = std::mem::take(&mut self.table_rights);
            let widths = column_grid(&mut rows, &rights);
            // Word draws only the borders the cells name (and the table's, `\trbrdr…`): no grid
            // style, each cell its own.
            self.body.push(FBlock::Table(FTable { rows, widths, borderless: true, extra: table_extra(acc), kept: None, wrap: None }));
        }
    }
}

/// The formatting before a tracked change of a cell defined as `now`, defined as `old` before:
/// its shading, borders, vertical alignment and preferred width, when one of them was other.
fn cell_change(old: Option<&CellDef>, now: &CellDef, table_borders: bool) -> Option<wordcraft_doc::props::CellProps> {
    let old = old?;
    let look = |d: &CellDef| (d.shading, d.visible_borders(), d.valign, d.width_pt());
    if look(old) == look(now) {
        return None;
    }
    // No width: the width it has now.
    let width = old.width_pt().filter(|w| Some(*w) != now.width_pt());
    Some(wordcraft_doc::props::CellProps {
        shading: old.shading,
        borders: old.cell_borders(table_borders),
        valign: old.valign,
        width,
        ..Default::default()
    })
}

/// What a table keeps beyond its cells, when it has any of it.
fn table_extra(acc: TableAcc) -> Option<Box<model::TableExtra>> {
    let t = acc.table.unwrap_or_default();
    let (width, width_pct) = t.table_width();
    let borders = t.visible_borders();
    let any = t.align.is_some()
        || width.is_some()
        || width_pct.is_some()
        || borders.is_some()
        || acc.heights.iter().any(Option::is_some)
        || acc.keeps.iter().any(|k| *k)
        || acc.change.is_some()
        || acc.row_changes.iter().any(Option::is_some);
    any.then(|| {
        Box::new(model::TableExtra {
            align: t.align,
            width,
            width_pct,
            heights: acc.heights,
            keep_rows: acc.keeps,
            borders,
            fmt_change: acc.change,
            row_changes: acc.row_changes,
            ..Default::default()
        })
    })
}
