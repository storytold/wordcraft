use super::date::dttm_to_iso;
use super::parse::column_grid;
use super::state::{CellDef, Dest, HeaderFooter, MAX_GROUPS, MAX_REV_AUTHORS, MAX_SECTIONS, Output, PageGeom, SecFmt, St};
use super::styles::{StyleDef, drop_style_formatting, style_fmt};
use super::*;

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
    pub(super) row_header: bool,
    pub(super) created: Option<(u32, u32, u32, u32, u32)>,
    pub(super) in_creatim: bool,
    pub(super) rule_pending: bool,
    pub(super) info_depth: Option<usize>,
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
        std::mem::swap(&mut o.row_header, &mut self.row_header);
        std::mem::swap(&mut o.rule_pending, &mut self.rule_pending);
        std::mem::swap(&mut o.table_rights, &mut self.table_rights);
        o
    }

    /// The border of the cell side being described, else of the paragraph side.
    pub(super) fn cell_border_slot(&mut self) -> Option<&mut Option<wordcraft_doc::props::Border>> {
        let (b, side) = match self.cell_side {
            Some(c) => (&mut self.def_cur.borders, c),
            None => (&mut self.st.borders, self.st.border_side?),
        };
        match side {
            't' => Some(&mut b.top),
            'l' => Some(&mut b.left),
            'b' => Some(&mut b.bottom),
            'r' => Some(&mut b.right),
            _ => None,
        }
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

    /// `\sect`: end the paragraph, and with it the section, which takes its formatting and the
    /// headers and footers read since the last break. Inside a header, footer or table, and past
    /// [`MAX_SECTIONS`], it only ends the paragraph.
    pub(super) fn end_section(&mut self) {
        self.end_para_mark();
        if self.hf.is_some() || self.st.intbl || self.section_parts.len() + 1 >= MAX_SECTIONS {
            return;
        }
        let props = self.sec.props(&self.page);
        if let Some(FBlock::Para(p)) = self.body.last_mut() {
            p.section = Some(Box::new(props));
            self.section_parts.push(std::mem::take(&mut self.hf_parts));
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
        let sec = if self.hf.is_some() {
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
            _ => {}
        }
        let Some(mut prev) = self.stack.pop() else { return };
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
            c.borders = d.borders.any_visible().then_some(d.borders);
            c.header = self.row_header;
            c.valign = d.valign;
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
        }
    }

    pub(super) fn close_table(&mut self) {
        if !self.row.is_empty() || !self.cell.is_empty() {
            if !self.cell.is_empty() {
                let blocks = std::mem::take(&mut self.cell);
                self.row.push(Cell { blocks, ..Default::default() });
            }
            self.end_row();
        }
        if !self.table.is_empty() {
            let mut rows = std::mem::take(&mut self.table);
            let rights = std::mem::take(&mut self.table_rights);
            let widths = column_grid(&mut rows, &rights);
            // Word draws only the borders the cells name: no grid style, each cell its own.
            self.body.push(FBlock::Table(FTable { rows, widths, borderless: true }));
        }
    }
}
