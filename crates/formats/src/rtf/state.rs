use super::{Align, Cell, FBlock, Fmt, Para, Rgb, VAlign};

/// Deepest group nesting we track; deeper groups share the innermost state.
pub(super) const MAX_GROUPS: usize = 512;

/// Largest picture we decode (bytes).
pub(super) const MAX_PICT: usize = 64 << 20;
/// Most notes, and most comments, one file may bring in.
pub(super) const MAX_NOTES: usize = 5_000;
/// Most blocks kept of one note's or comment's text.
pub(super) const MAX_NOTE_BLOCKS: usize = 500;
/// Brackets of a place marker in text: a note reference or a comment range end travels through the
/// flow model as `OPEN tag number CLOSE` (private-use characters, which both the reader and the
/// writer keep out of real text) and becomes an inline object once the document exists.
pub(super) const MARK_OPEN: char = '\u{F8F0}';
pub(super) const MARK_CLOSE: char = '\u{F8F1}';

/// Most sections we read (`\sect`); later breaks end paragraphs only.
pub(super) const MAX_SECTIONS: usize = 10_000;

// ---------------------------------------------------------------------------------------------
// Import

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Dest {
    Body,
    Skip,
    FontTbl,
    ColorTbl,
    StyleSheet,
    Info(u8),
    ListTable,
    ListOverride,
    FldInst,
    Pict,
    Bookmark,
    /// A comment's text-valued word: 1 `\atrfstart`, 2 `\atrfend`, 3 `\atnid`, 4 `\atnauthor`,
    /// 5 `\atndate`/`\atntime`, 6 `\atnref`.
    Atn(u8),
    /// The author table of tracked changes (`\revtbl`).
    RevTbl,
    /// Formatting before a tracked change (`\oldpprops` when true, else `\oldcprops`): read
    /// into the group's own state, which the group's end keeps as the change's old formatting.
    OldProps(bool),
    /// A shape's description (`\shpinst`), and the name (`\sn`) and value (`\sv`) of one of
    /// its properties.
    ShpInst,
    ShpSn,
    ShpSv,
}

/// A `\shp` group's placement and wrapping, read from `\shp` and `\shpinst` for the picture in
/// its `\shprslt` (#6221).
#[derive(Clone, Debug, Default)]
pub(super) struct ShpPicSt {
    pub(super) shp_depth: usize,
    pub(super) rslt_depth: Option<usize>,
    /// `\shpleft`, `\shptop`, `\shpright`, `\shpbottom` (twips), and whether any was given.
    pub(super) rect: [i64; 4],
    pub(super) has_rect: bool,
    pub(super) bx: Option<wordcraft_doc::para::Anchor>,
    pub(super) by: Option<wordcraft_doc::para::Anchor>,
    pub(super) bx_ignore: bool,
    pub(super) by_ignore: bool,
    pub(super) wr: Option<i32>,
    pub(super) wrk: i32,
    pub(super) behind: bool,
    pub(super) sn: String,
    pub(super) sv: String,
    pub(super) shape_type: Option<i64>,
    pub(super) posrelh: Option<i64>,
    pub(super) posrelv: Option<i64>,
    pub(super) posh: Option<i64>,
    pub(super) posv: Option<i64>,
    pub(super) pseudo_inline: bool,
    pub(super) hidden: bool,
    pub(super) flip_h: bool,
    pub(super) flip_v: bool,
    pub(super) rotation: Option<i64>,
    pub(super) behind_prop: Option<bool>,
    /// `dxWrapDistLeft`, `dxWrapDistRight`, `dyWrapDistTop`, `dyWrapDistBottom` (EMUs).
    pub(super) dist: [Option<i64>; 4],
}

impl ShpPicSt {
    /// Record a `\shp` placement or wrapping control word (`\shpleft`…`\shpbottom`,
    /// `\shpbx…`/`\shpby…`, `\shpwr`, `\shpwrk`, `\shpfblwtxt`), written either directly in
    /// `\shp` or inside `\shpinst`. Returns whether `w` was one.
    pub(super) fn control_word(&mut self, w: &str, p: Option<i32>) -> bool {
        use wordcraft_doc::para::Anchor;
        let pv = p.unwrap_or(0);
        match w {
            "shpleft" => {
                self.rect[0] = i64::from(pv);
                self.has_rect = true;
            }
            "shptop" => {
                self.rect[1] = i64::from(pv);
                self.has_rect = true;
            }
            "shpright" => {
                self.rect[2] = i64::from(pv);
                self.has_rect = true;
            }
            "shpbottom" => {
                self.rect[3] = i64::from(pv);
                self.has_rect = true;
            }
            "shpbxpage" => self.bx = Some(Anchor::Page),
            "shpbxmargin" => self.bx = Some(Anchor::Margin),
            "shpbxcolumn" => self.bx = Some(Anchor::Column),
            "shpbxignore" => self.bx_ignore = true,
            "shpbypage" => self.by = Some(Anchor::Page),
            "shpbymargin" => self.by = Some(Anchor::Margin),
            "shpbypara" => self.by = Some(Anchor::Paragraph),
            "shpbyignore" => self.by_ignore = true,
            "shpwr" => self.wr = Some(pv),
            "shpwrk" => self.wrk = pv,
            "shpfblwtxt" => self.behind = p != Some(0),
            _ => return false,
        }
        true
    }

    /// Commit the `\sn`/`\sv` property pair just read in `\shpinst`.
    pub(super) fn commit_prop(&mut self) {
        let v = self.sv.trim().parse::<i64>().ok();
        let on = v.map(|v| v != 0);
        match self.sn.trim() {
            "shapeType" => self.shape_type = v,
            "posrelh" => self.posrelh = v,
            "posrelv" => self.posrelv = v,
            "posh" => self.posh = v,
            "posv" => self.posv = v,
            "fPseudoInline" => self.pseudo_inline = on == Some(true),
            "fHidden" => self.hidden = on == Some(true),
            "fFlipH" => self.flip_h = on == Some(true),
            "fFlipV" => self.flip_v = on == Some(true),
            "rotation" => self.rotation = v,
            "fBehindDocument" => self.behind_prop = on,
            "dxWrapDistLeft" => self.dist[0] = v,
            "dxWrapDistRight" => self.dist[1] = v,
            "dyWrapDistTop" => self.dist[2] = v,
            "dyWrapDistBottom" => self.dist[3] = v,
            _ => {}
        }
        self.sn.clear();
        self.sv.clear();
    }

    /// Floating placement for the picture in `\shprslt`, when the shape places it outside the
    /// line (`shapeType` 75 or omitted, with wrapping or flip/rotation).
    pub(super) fn float(&self) -> Option<Box<wordcraft_doc::para::Float>> {
        use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap, WrapText};
        if self.shape_type.is_some_and(|t| t != 75) {
            return None;
        }
        let align = |v: Option<i64>| match v? {
            1 => Some(FloatAlign::Start),
            2 => Some(FloatAlign::Center),
            3 => Some(FloatAlign::End),
            4 => Some(FloatAlign::Inside),
            5 => Some(FloatAlign::Outside),
            _ => None,
        };
        let points = |v: i64, per: f32| (v.clamp(-31_680 * 12_700, 31_680 * 12_700) as f32 / per).clamp(-1584.0, 1584.0);
        let rot = self.rotation.map_or(0.0, |r| (r.clamp(-360 * 65_536, 360 * 65_536) as f32 / 65_536.0).rem_euclid(360.0));
        let mut f = Float { flip_h: self.flip_h, flip_v: self.flip_v, rot, hidden: self.hidden, ..Float::default() };
        if !self.pseudo_inline
            && (self.wr.is_some()
                || self.behind
                || self.behind_prop == Some(true)
                || self.has_rect
                || self.bx.is_some()
                || self.by.is_some()
                || self.posrelh.is_some()
                || self.posrelv.is_some()
                || self.posh.is_some()
                || self.posv.is_some())
        {
            let behind = self.behind_prop.unwrap_or(self.behind);
            f.wrap = match self.wr {
                Some(1) => Wrap::TopAndBottom,
                Some(2) => Wrap::Square,
                Some(4) => Wrap::Tight,
                Some(5) => Wrap::Through,
                _ if behind => Wrap::BehindText,
                _ => Wrap::InFrontOfText,
            };
            f.wrap_text = match self.wrk {
                1 => WrapText::Left,
                2 => WrapText::Right,
                3 => WrapText::Largest,
                _ => WrapText::BothSides,
            };
            let rel_h = self.posrelh.and_then(|v| match v {
                0 => Some(Anchor::Margin),
                1 => Some(Anchor::Page),
                2 => Some(Anchor::Column),
                3 => Some(Anchor::Character),
                4 => Some(Anchor::LeftMargin),
                5 => Some(Anchor::RightMargin),
                6 => Some(Anchor::InsideMargin),
                7 => Some(Anchor::OutsideMargin),
                _ => None,
            });
            let rel_v = self.posrelv.and_then(|v| match v {
                0 => Some(Anchor::Margin),
                1 => Some(Anchor::Page),
                2 => Some(Anchor::Paragraph),
                3 => Some(Anchor::Line),
                4 => Some(Anchor::TopMargin),
                5 => Some(Anchor::BottomMargin),
                6 => Some(Anchor::InsideMargin),
                7 => Some(Anchor::OutsideMargin),
                _ => None,
            });
            f.h_rel = if self.bx_ignore { rel_h.or(self.bx) } else { self.bx.or(rel_h) }.unwrap_or(Anchor::Column);
            f.v_rel = if self.by_ignore { rel_v.or(self.by) } else { self.by.or(rel_v) }.unwrap_or(Anchor::Paragraph);
            f.x = points(self.rect[0], 20.0);
            f.y = points(self.rect[1], 20.0);
            f.h_align = align(self.posh);
            f.v_align = align(self.posv);
            let dist = |v: Option<i64>, d: f32| v.map_or(d, |v| points(v, 12_700.0).max(0.0));
            f.dist = dist(self.dist[0].or(self.dist[1]), 9.0);
            f.dist_top = dist(self.dist[2], 0.0);
            f.dist_bottom = dist(self.dist[3], 0.0);
        }
        (f != Float::default()).then(|| Box::new(f))
    }
}

/// A footnote, endnote or comment group being read into an output of its own.
pub(super) struct Sub {
    pub(super) depth: usize,
    pub(super) comment: bool,
    pub(super) endnote: bool,
    pub(super) saved: Box<Output>,
}

/// A note read: its kind and text.
pub(super) struct NoteData {
    pub(super) endnote: bool,
    pub(super) blocks: Vec<FBlock>,
}

/// A comment read: the id its range markers carry, who wrote it and when, and its text.
pub(super) struct CommentData {
    pub(super) rtf_id: i32,
    pub(super) author: String,
    pub(super) initials: String,
    pub(super) date: String,
    pub(super) blocks: Vec<FBlock>,
}

/// A tracked formatting change being read (`\crauth`/`\crdate` with `\oldcprops`,
/// `\prauth`/`\prdate` with `\oldpprops`): author (`\revtbl` index), date (DTTM) and the old
/// formatting, once its group has been read.
#[derive(Clone, Debug, Default)]
pub(super) struct FmtRevSt<T> {
    pub(super) auth: i32,
    pub(super) date: u32,
    pub(super) old: Option<Box<T>>,
}

/// Character formatting words of an `\oldcprops` group, with their parameters.
type OldWords = Vec<(String, Option<i32>)>;

/// Most words kept from one `\oldcprops` group (hostile input).
pub(super) const MAX_OLD_WORDS: usize = 64;

/// Most authors read from a `\revtbl` (hostile input).
pub(super) const MAX_REV_AUTHORS: usize = 10_000;

/// Tracked-change character properties: `\revised` / `\deleted`, their authors (`\revtbl`
/// indices) and dates (DTTM).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RevSt {
    pub(super) ins: bool,
    pub(super) del: bool,
    pub(super) auth: i32,
    pub(super) authdel: i32,
    pub(super) dttm: u32,
    pub(super) dttmdel: u32,
}

/// Which header or footer a `\header…`/`\footer…` group is: footer or header, and the pages
/// (`\headerf` first, `\headerl` left = even, `\header`/`\headerr` the others).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HeaderFooter {
    pub(super) footer: bool,
    pub(super) pages: HfPages,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum HfPages {
    Default,
    First,
    Even,
}

/// The reader's output so far, set aside while a header or footer is read.
#[derive(Default)]
pub(super) struct Output {
    pub(super) body: Vec<FBlock>,
    pub(super) para: Para,
    pub(super) para_has: bool,
    pub(super) table: Vec<Vec<Cell>>,
    pub(super) row: Vec<Cell>,
    pub(super) cell: Vec<FBlock>,
    pub(super) defs: Vec<CellDef>,
    pub(super) def_cur: CellDef,
    pub(super) row_header: bool,
    pub(super) rule_pending: bool,
    pub(super) table_rights: Vec<Vec<i32>>,
}

#[derive(Clone, Debug)]
pub(super) struct St {
    pub(super) dest: Dest,
    pub(super) fmt: Fmt,
    pub(super) rev: RevSt,
    /// Tracked change of the character formatting (reset by `\plain`). Its old formatting is
    /// the `\oldcprops` words, applied on top of the run's formatting when text is read: Word
    /// writes only the properties that changed there.
    pub(super) chg_c: FmtRevSt<OldWords>,
    /// While an `\oldcprops` group is read: its character formatting words, and the author and
    /// date of the change (`\crauth`/`\crdate`, which Word writes inside the group).
    pub(super) old_words: OldWords,
    pub(super) old_rev: (Option<i32>, Option<u32>),
    /// Tracked change of the paragraph formatting (reset by `\pard`).
    pub(super) chg_p: FmtRevSt<Para>,
    pub(super) uc: usize,
    pub(super) hidden: bool,
    // Paragraph properties.
    pub(super) align: Option<Align>,
    pub(super) style: i32,
    pub(super) intbl: bool,
    pub(super) ls: Option<i32>,
    pub(super) ilvl: u8,
    pub(super) outline: Option<u8>,
    pub(super) pagebb: bool,
    pub(super) field: Option<usize>,
    /// The code page of the current font's charset (`\fcharset`), when it names one.
    pub(super) cp: Option<u16>,
    /// Space before/after (`\sb`, `\sa`), line spacing (`\sl`, `\slmult`) and indents
    /// (`\li`, `\ri`, `\fi`), twips.
    /// The paragraph's borders (`\brdrt`, `\brdrb`, `\brdrl`, `\brdrr`) and the side being
    /// described.
    pub(super) borders: wordcraft_doc::props::Borders,
    pub(super) border_side: Option<char>,
    pub(super) sb: Option<i32>,
    pub(super) sa: Option<i32>,
    pub(super) sl: i32,
    pub(super) slmult: bool,
    pub(super) li: Option<i32>,
    pub(super) ri: Option<i32>,
    pub(super) fi: Option<i32>,
}

impl Default for St {
    fn default() -> Self {
        St {
            dest: Dest::Body,
            fmt: Fmt::default(),
            rev: RevSt::default(),
            chg_c: FmtRevSt::default(),
            chg_p: FmtRevSt::default(),
            old_words: Vec::new(),
            old_rev: (None, None),
            uc: 1,
            hidden: false,
            align: None,
            style: 0,
            intbl: false,
            ls: None,
            ilvl: 0,
            outline: None,
            pagebb: false,
            field: None,
            cp: None,
            borders: Default::default(),
            border_side: None,
            sb: None,
            sa: None,
            sl: 0,
            slmult: false,
            li: None,
            ri: None,
            fi: None,
        }
    }
}

#[derive(Default, Clone)]
pub(super) struct CellDef {
    pub(super) vmerge: u8,
    pub(super) hmerge: u8,
    pub(super) shading: Option<Rgb>,
    pub(super) right: i32,
    /// The borders the cell names (`\clbrdrt` … `\clbrdrr`, each with its line style, `\brdrw`
    /// width and `\brdrcf` colour).
    pub(super) borders: wordcraft_doc::props::Borders,
    pub(super) valign: Option<VAlign>,
}

/// Page size and margins in twips (`\paperw`, `\margl`… and their `…sxn` section forms).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PageGeom {
    pub(super) w: Option<i32>,
    pub(super) h: Option<i32>,
    pub(super) left: Option<i32>,
    pub(super) right: Option<i32>,
    pub(super) top: Option<i32>,
    pub(super) bottom: Option<i32>,
    pub(super) header: Option<i32>,
    pub(super) footer: Option<i32>,
    pub(super) landscape: bool,
}

impl PageGeom {
    /// The section properties these give, Word's defaults where the file names none.
    fn section(&self) -> wordcraft_doc::SectionProps {
        let mut s = wordcraft_doc::SectionProps::default();
        // Twips to points, ignoring nonsense (zero, negative, or beyond Word's 22").
        let pt = |v: Option<i32>, lo: f32, hi: f32| v.map(|t| t as f32 / 20.0).filter(|v| v.is_finite() && (lo..=hi).contains(v));
        if let Some(w) = pt(self.w, 72.0, 1584.0) {
            s.page_w = w;
        }
        if let Some(h) = pt(self.h, 72.0, 1584.0) {
            s.page_h = h;
        }
        if self.landscape && s.page_w < s.page_h {
            std::mem::swap(&mut s.page_w, &mut s.page_h);
        }
        s.landscape = s.page_w > s.page_h;
        for (v, f) in [
            (self.left, &mut s.margin_left),
            (self.right, &mut s.margin_right),
            (self.top, &mut s.margin_top),
            (self.bottom, &mut s.margin_bottom),
            (self.header, &mut s.header),
            (self.footer, &mut s.footer),
        ] {
            if let Some(v) = pt(v, 0.0, 720.0) {
                *f = v;
            }
        }
        // Margins that leave no room for text fall back to Word's.
        let d = wordcraft_doc::SectionProps::default();
        if s.page_w - s.margin_left - s.margin_right < 72.0 {
            (s.margin_left, s.margin_right) = (d.margin_left, d.margin_right);
        }
        if s.page_h - s.margin_top - s.margin_bottom < 72.0 {
            (s.margin_top, s.margin_bottom) = (d.margin_top, d.margin_bottom);
        }
        s
    }
}

/// Section formatting (`\sectd` resets it): the section's page words (`\pgwsxn`…, over the
/// document's `\paperw`…), its break, columns, page numbering and title page.
#[derive(Clone, Debug, Default)]
pub(super) struct SecFmt {
    pub(super) page: PageGeom,
    pub(super) start: wordcraft_doc::section::SectionStart,
    pub(super) cols: Option<i32>,
    pub(super) colsx: Option<i32>,
    pub(super) linebetcol: bool,
    pub(super) pgn_restart: bool,
    pub(super) pgn_start: Option<i32>,
    pub(super) pgn_fmt: wordcraft_doc::section::NumFormat,
    pub(super) title_page: bool,
}

impl SecFmt {
    /// The section's properties over the document's page (`doc`); headers and footers unset.
    pub(super) fn props(&self, doc: &PageGeom) -> wordcraft_doc::SectionProps {
        let p = &self.page;
        let page = PageGeom {
            w: p.w.or(doc.w),
            h: p.h.or(doc.h),
            left: p.left.or(doc.left),
            right: p.right.or(doc.right),
            top: p.top.or(doc.top),
            bottom: p.bottom.or(doc.bottom),
            header: p.header.or(doc.header),
            footer: p.footer.or(doc.footer),
            // A section that gives its own page size or orientation has its own orientation;
            // the document's `\landscape` only applies to sections that give neither.
            landscape: if p.w.is_some() || p.h.is_some() || p.landscape { p.landscape } else { doc.landscape },
        };
        let mut s = page.section();
        s.start = self.start;
        if let Some(n) = self.cols {
            s.columns.count = n.clamp(1, 45) as u32;
        }
        if let Some(x) = self.colsx {
            s.columns.space = (x as f32 / 20.0).clamp(0.0, 1584.0);
        }
        s.columns.separator = self.linebetcol;
        if self.pgn_restart {
            s.page_num_start = Some(self.pgn_start.unwrap_or(1).clamp(0, 32_767) as u32);
        }
        s.page_num_format = self.pgn_fmt;
        s.title_page = self.title_page;
        s
    }
}
