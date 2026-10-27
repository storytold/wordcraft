use super::state::{CellDef, Dest, FmtRevSt, HeaderFooter, HfPages, MAX_OLD_WORDS, RevSt, St};
use super::styles::{StyleDef, is_char_format_word, is_format_word};
use super::*;

impl Reader {
    pub(super) fn word(&mut self, w: &str, p: Option<i32>, starred: bool, first_in_group: bool) {
        // Every control word ends a multibyte run: the code page it is read in may change here
        // (`\f`, `\fcharset`, `\cpg`, `\ansicpg`, `\pard`), and `\par` must see the text before it.
        self.finish_multibyte();
        let on = p != Some(0);
        let pv = p.unwrap_or(0);
        if self.st.dest == Dest::Skip && !(self.in_info() && matches!(w, "title" | "author" | "subject" | "keywords" | "doccomm" | "creatim")) {
            if self.in_creatim {
                let v = pv.clamp(0, 9999) as u32;
                let c = self.created.get_or_insert((0, 1, 1, 0, 0));
                match w {
                    "yr" => c.0 = v,
                    "mo" => c.1 = v,
                    "dy" => c.2 = v,
                    "hr" => c.3 = v,
                    "min" => c.4 = v,
                    _ => {}
                }
            }
            return;
        }
        // Destinations.
        let dest = match w {
            "fonttbl" => Some(Dest::FontTbl),
            "colortbl" => Some(Dest::ColorTbl),
            "stylesheet" => Some(Dest::StyleSheet),
            "info" => Some(Dest::Skip),
            "title" if self.st.dest == Dest::Skip && self.in_info() => Some(Dest::Info(1)),
            "author" if self.in_info() => Some(Dest::Info(2)),
            "subject" if self.in_info() => Some(Dest::Info(3)),
            "keywords" if self.in_info() => Some(Dest::Info(4)),
            "doccomm" if self.in_info() => Some(Dest::Info(5)),
            "listtable" => Some(Dest::ListTable),
            "revtbl" => Some(Dest::RevTbl),
            "oldcprops" => Some(Dest::OldProps(false)),
            "oldpprops" => Some(Dest::OldProps(true)),
            "listoverridetable" => Some(Dest::ListOverride),
            "fldinst" => Some(Dest::FldInst),
            // Transparent wrapper: read the nested pict without changing the destination.
            "shppict" => return,
            "pict" => Some(Dest::Pict),
            "bkmkstart" => Some(Dest::Bookmark),
            "header" | "footer" | "headerl" | "headerr" | "headerf" | "footerl" | "footerr" | "footerf"
                if self.hf.is_none() && self.st.dest == Dest::Body && !self.st.intbl =>
            {
                let footer = w.starts_with("footer");
                let pages = match w.strip_prefix(if footer { "footer" } else { "header" }) {
                    Some("f") => HfPages::First,
                    Some("l") => HfPages::Even,
                    _ => HfPages::Default,
                };
                self.start_header_footer(HeaderFooter { footer, pages });
                return;
            }
            "header" | "footer" | "headerl" | "headerr" | "headerf" | "footerl" | "footerr" | "footerf" | "footnote" | "annotation" | "pntext"
            | "listtext" | "object" | "nonshppict" | "xe" | "tc" | "txe" | "private" | "bkmkend" | "atnid" | "atnauthor" | "comment" | "shpinst"
            | "do" | "operator" | "company" | "manager" | "category" | "hlinkbase" | "revtime" | "printim" | "buptim" | "version" | "nofpages"
            | "nofwords" | "nofchars" | "edmins" | "vern" | "themedata" | "colorschememapping" | "latentstyles" | "datastore" | "xmlnstbl"
            | "rsidtbl" | "generator" | "mmathPr" | "pgdsctbl" | "filetbl" | "listpicture" | "userprops" | "docvar" | "objdata" | "fontemb"
            | "fontfile" | "panose" | "falt" | "mhtmltag" | "htmltag" | "pnseclvl" | "background" | "sp" | "pn" | "pgptbl" => Some(Dest::Skip),
            "creatim" if self.in_info() => {
                self.in_creatim = true;
                Some(Dest::Skip)
            }
            _ => None,
        };
        if let Some(d) = dest {
            match d {
                Dest::Bookmark => {
                    self.bkmk = Some((self.depth(), String::new()));
                }
                Dest::Pict => {
                    self.pict_depth = Some(self.depth());
                    self.pict_hex.clear();
                    self.pict_half = None;
                    self.pict_kind = 0;
                    self.pict_w = None;
                    self.pict_h = None;
                    self.pict_sx = 0.0;
                    self.pict_sy = 0.0;
                }
                Dest::FldInst if self.st.field.is_none() => self.start_field(),
                // Old character formatting holds the properties that changed (Word) or, after a
                // `\plain`, all of them; old paragraph formatting is complete from `\pard`.
                Dest::OldProps(false) => {
                    self.st.old_words.clear();
                    self.st.old_rev = (None, None);
                }
                Dest::OldProps(true) => self.word("pard", None, false, false),
                _ => {}
            }
            self.st.dest = d;
            return;
        }
        if starred && first_in_group {
            // Unknown ignorable destination.
            if !matches!(w, "fldinst" | "bkmkstart") {
                self.st.dest = Dest::Skip;
                return;
            }
        }
        match self.st.dest {
            Dest::Skip => {
                if self.in_creatim {
                    let v = pv.clamp(0, 9999) as u32;
                    let c = self.created.get_or_insert((0, 1, 1, 0, 0));
                    match w {
                        "yr" => c.0 = v,
                        "mo" => c.1 = v,
                        "dy" => c.2 = v,
                        "hr" => c.3 = v,
                        "min" => c.4 = v,
                        _ => {}
                    }
                }
                return;
            }
            // Font names take Unicode escapes (with fallback skipping) like the body.
            Dest::FontTbl if !matches!(w, "u" | "uc") => {
                match w {
                    "f" => {
                        self.font_cur = pv;
                        self.font_name.clear();
                        self.font_cp_cur = None;
                    }
                    "fcharset" => self.font_cp_cur = wordcraft_doc::encoding::charset_code_page(pv),
                    // A font's explicit code page (`\cpg1251`, `\cpg932`) wins over its charset.
                    "cpg" => {
                        if let Some(cp) = u16::try_from(pv).ok().filter(|cp| wordcraft_doc::encoding::is_text_code_page(*cp)) {
                            self.font_cp_cur = Some(cp);
                        }
                    }
                    _ => {}
                }
                return;
            }
            Dest::FontTbl => {}
            Dest::ColorTbl => {
                let v = pv.clamp(0, 255) as u8;
                match w {
                    "red" => {
                        self.color_cur.0 = v;
                        self.color_cur.3 = true;
                    }
                    "green" => {
                        self.color_cur.1 = v;
                        self.color_cur.3 = true;
                    }
                    "blue" => {
                        self.color_cur.2 = v;
                        self.color_cur.3 = true;
                    }
                    _ => {}
                }
                return;
            }
            Dest::StyleSheet => {
                let d = &mut self.style_def_cur;
                match w {
                    "s" | "cs" | "ds" | "ts" => {
                        self.style_cur = if w == "s" { pv } else { -1 - pv.saturating_abs() };
                        self.style_name.clear();
                        *d = StyleDef::default();
                    }
                    "sbasedon" => d.based_on = Some(pv),
                    "b" => d.bold = Some(on),
                    "i" => d.italic = Some(on),
                    "fs" => d.size = (pv > 0).then(|| (pv as f32 / 2.0).clamp(1.0, 1638.0)),
                    "f" => d.font = Some(pv),
                    _ => {}
                }
                return;
            }
            Dest::ListTable => {
                match w {
                    "list" => {
                        self.list_cur.clear();
                        self.list_cur_id = 0;
                    }
                    "listlevel" if self.list_cur.len() < 9 => {
                        self.list_cur.push(true);
                    }
                    "levelnfc" | "levelnfcn" => {
                        if let Some(l) = self.list_cur.last_mut() {
                            *l = !matches!(pv, 23 | 255);
                        }
                    }
                    "listid" => {
                        self.list_cur_id = pv;
                        if self.lists.len() < 4096 {
                            self.lists.insert(pv, self.list_cur.clone());
                        }
                    }
                    _ => {}
                }
                return;
            }
            Dest::ListOverride => {
                match w {
                    "listid" => self.ov_list = pv,
                    "ls" => {
                        self.ov_ls = pv;
                        if self.overrides.len() < 4096 {
                            self.overrides.insert(pv, self.ov_list);
                        }
                    }
                    _ => {}
                }
                return;
            }
            // Author names take Unicode escapes like the other text destinations.
            Dest::RevTbl if !matches!(w, "u" | "uc") => return,
            Dest::RevTbl => {}
            Dest::Pict => {
                match w {
                    "pngblip" => self.pict_kind = 1,
                    "jpegblip" => self.pict_kind = 2,
                    "picwgoal" => self.pict_w = Some(pv.clamp(1, 31_680) as f32 / 20.0),
                    "pichgoal" => self.pict_h = Some(pv.clamp(1, 31_680) as f32 / 20.0),
                    "picscalex" => self.pict_sx = pv.clamp(1, 1000) as f32,
                    "picscaley" => self.pict_sy = pv.clamp(1, 1000) as f32,
                    _ => {}
                }
                return;
            }
            // Text destinations take Unicode escapes (with fallback skipping) like the body.
            Dest::Info(_) | Dest::FldInst | Dest::Bookmark if !matches!(w, "u" | "uc") => return,
            Dest::Info(_) | Dest::FldInst | Dest::Bookmark => {}
            // Who changed the formatting and when, inside the old formatting (as Word writes it).
            Dest::OldProps(false) if matches!(w, "crauth" | "crdate") => {
                if w == "crauth" {
                    self.st.old_rev.0 = Some(pv);
                } else {
                    self.st.old_rev.1 = Some(pv as u32);
                }
                return;
            }
            // Old formatting takes formatting words only: no text, structure or document state,
            // and no paragraph reset among the character formatting.
            Dest::OldProps(_) if !is_format_word(w) => return,
            Dest::OldProps(false) if w == "pard" => return,
            // Border words there would edit the cell definition being read, not the group's copy.
            Dest::OldProps(_) if self.cell_side.is_some() && w.starts_with("brdr") => return,
            Dest::OldProps(false) => {
                if is_char_format_word(w) && self.st.old_words.len() < MAX_OLD_WORDS {
                    self.st.old_words.push((w.to_string(), p));
                }
            }
            Dest::OldProps(true) | Dest::Body => {}
        }
        match w {
            "par" => self.end_para_mark(),
            "sect" => self.end_section(),
            "cell" => self.end_cell(),
            "nestcell" => self.end_para(true),
            "row" => self.end_row(),
            "line" | "softline" => self.ch('\n'),
            "tab" => self.ch('\t'),
            "page" | "softpage" => self.ch('\u{000C}'),
            "column" => self.ch('\u{000E}'),
            "emdash" => self.ch('—'),
            "endash" => self.ch('–'),
            "bullet" => self.ch('•'),
            "lquote" => self.ch('‘'),
            "rquote" => self.ch('’'),
            "ldblquote" => self.ch('“'),
            "rdblquote" => self.ch('”'),
            "emspace" => self.ch('\u{2003}'),
            "enspace" => self.ch('\u{2002}'),
            "qmspace" => self.ch('\u{2005}'),
            "zwj" => self.ch('\u{200D}'),
            "zwnj" => self.ch('\u{200C}'),
            "zwbo" => self.ch('\u{200B}'),
            "zwnbo" => self.ch('\u{2060}'),
            "u" => {
                let u = (pv as i64).rem_euclid(65_536) as u16;
                if (0xD800..0xDC00).contains(&u) {
                    self.pending_high = Some(u);
                } else {
                    let c = match self.pending_high.take() {
                        Some(h) if (0xDC00..0xE000).contains(&u) => char::decode_utf16([h, u]).next().and_then(Result::ok).unwrap_or('\u{FFFD}'),
                        _ => char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}'),
                    };
                    self.ch(c);
                }
                self.skip_chars = self.st.uc;
            }
            "uc" => self.st.uc = pv.clamp(0, 8) as usize,
            "plain" => {
                self.st.fmt = Fmt { link: self.st.fmt.link.clone(), ..Default::default() };
                self.st.rev = RevSt::default();
                self.st.chg_c = FmtRevSt::default();
                self.st.hidden = false;
            }
            "pard" => {
                let field = self.st.field;
                // Revision marks are character formatting, which `\pard` leaves alone.
                let (dest, fmt, rev, uc, hidden) = (self.st.dest, self.st.fmt.clone(), self.st.rev, self.st.uc, self.st.hidden);
                let chg_c = std::mem::take(&mut self.st.chg_c);
                self.st = St { dest, fmt, rev, chg_c, uc, hidden, field, ..Default::default() };
            }
            "revised" => self.st.rev.ins = on,
            "deleted" => self.st.rev.del = on,
            "revauth" => self.st.rev.auth = pv,
            "revauthdel" => self.st.rev.authdel = pv,
            // DTTMs are 32-bit patterns, written signed or unsigned.
            "revdttm" => self.st.rev.dttm = pv as u32,
            "revdttmdel" => self.st.rev.dttmdel = pv as u32,
            "crauth" => self.st.chg_c.auth = pv,
            "crdate" => self.st.chg_c.date = pv as u32,
            "prauth" => self.st.chg_p.auth = pv,
            "prdate" => self.st.chg_p.date = pv as u32,
            "b" => self.st.fmt.bold = on,
            "i" => self.st.fmt.italic = on,
            "ul" | "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulth" | "ulw" | "ulwave" | "ulhwave" | "ulldash" | "ulthd" => {
                self.st.fmt.underline = on
            }
            "ulnone" => self.st.fmt.underline = false,
            "strike" | "striked" => self.st.fmt.strike = on,
            "super" | "up" => {
                self.st.fmt.sup = on;
                self.st.fmt.sub = false;
            }
            "sub" | "dn" => {
                self.st.fmt.sub = on;
                self.st.fmt.sup = false;
            }
            "nosupersub" => {
                self.st.fmt.sup = false;
                self.st.fmt.sub = false;
            }
            "fs" => self.st.fmt.size = if pv > 0 { Some((pv as f32 / 2.0).clamp(1.0, 1638.0)) } else { None },
            "ansicpg" => {
                if let Some(cp) = u16::try_from(pv).ok().filter(|cp| wordcraft_doc::encoding::is_text_code_page(*cp)) {
                    self.ansicpg = cp;
                }
            }
            // The Macintosh and IBM PC character sets: 8-bit text is Mac OS Roman, or DOS code
            // page 437 (`\pc`) or 850 (`\pca`) (#1653).
            "mac" if self.ansicpg == 0 => self.ansicpg = wordcraft_doc::encoding::MAC_ROMAN_CODE_PAGE,
            "pc" if self.ansicpg == 0 => self.ansicpg = 437,
            "pca" if self.ansicpg == 0 => self.ansicpg = 850,
            "deff" => self.deff = pv,
            "f" => {
                self.st.cp = self.font_cps.get(&pv).copied();
                let name = self.fonts.get(&pv).cloned();
                match name {
                    Some(n) if model::is_mono(&n) => {
                        self.st.fmt.code = true;
                        self.st.fmt.font = None;
                    }
                    Some(n) => {
                        self.st.fmt.code = false;
                        self.st.fmt.font = Some(n);
                    }
                    None => {}
                }
            }
            "cf" => self.st.fmt.color = self.colors.get(pv.max(0) as usize).copied().flatten(),
            "highlight" | "chcbpat" | "cb" => self.st.fmt.background = self.colors.get(pv.max(0) as usize).copied().flatten(),
            "v" => self.st.hidden = on,
            "ql" => self.st.align = Some(Align::Left),
            "qc" => self.st.align = Some(Align::Center),
            "qr" => self.st.align = Some(Align::Right),
            "qj" => self.st.align = Some(Align::Justify),
            "qd" => self.st.align = Some(Align::Distribute),
            "s" => self.st.style = pv,
            "outlinelevel" => self.st.outline = Some(pv.clamp(0, 9) as u8),
            "pagebb" => self.st.pagebb = on,
            "intbl" => self.st.intbl = true,
            "ls" => self.st.ls = Some(pv),
            "ilvl" => self.st.ilvl = pv.clamp(0, 8) as u8,
            "brdrb" | "brdrt" | "brdrl" | "brdrr" if self.cell_side.is_none() => {
                if w == "brdrb" && self.st.dest == Dest::Body {
                    self.rule_pending = true;
                }
                self.st.border_side = w.chars().last();
            }
            "trowd" => {
                self.defs.clear();
                self.def_cur = CellDef::default();
                self.row_header = false;
            }
            "trhdr" => self.row_header = true,
            "clvmgf" => self.def_cur.vmerge = 1,
            "clvmrg" => self.def_cur.vmerge = 2,
            "clmgf" => self.def_cur.hmerge = 1,
            "clmrg" => self.def_cur.hmerge = 2,
            "clvertalt" => self.def_cur.valign = Some(VAlign::Top),
            "clvertalc" => self.def_cur.valign = Some(VAlign::Center),
            "clvertalb" => self.def_cur.valign = Some(VAlign::Bottom),
            "clcbpat" => self.def_cur.shading = self.colors.get(pv.max(0) as usize).copied().flatten(),
            "sb" => self.st.sb = Some(pv),
            "sa" => self.st.sa = Some(pv),
            "sl" => self.st.sl = pv,
            "slmult" => self.st.slmult = on,
            "li" | "lin" => self.st.li = Some(pv),
            "ri" | "rin" => self.st.ri = Some(pv),
            "fi" => self.st.fi = Some(pv),
            "clbrdrt" => self.cell_side = Some('t'),
            "clbrdrl" => self.cell_side = Some('l'),
            "clbrdrb" => self.cell_side = Some('b'),
            "clbrdrr" => self.cell_side = Some('r'),
            "brdrs" | "brdrth" | "brdrsh" | "brdrdb" | "brdrdot" | "brdrdash" | "brdrdashsm" | "brdrdashd" | "brdrdashdd" | "brdrhair"
            | "brdrtriple" | "brdrthtnsg" | "brdrtnthsg" | "brdrthtnmg" | "brdrtnthmg" | "brdrwavy" | "brdrwavydb" | "brdrnone" | "brdrnil"
            | "brdrtbl"
                if self.cell_side.is_some() || self.st.border_side.is_some() =>
            {
                use wordcraft_doc::props::{Border, BorderStyle};
                let style = match w {
                    "brdrnone" | "brdrnil" | "brdrtbl" => BorderStyle::None,
                    "brdrth" => BorderStyle::Thick,
                    "brdrdb" | "brdrthtnsg" | "brdrtnthsg" | "brdrthtnmg" | "brdrtnthmg" | "brdrwavydb" => BorderStyle::Double,
                    "brdrdot" => BorderStyle::Dotted,
                    "brdrdash" | "brdrdashsm" => BorderStyle::Dashed,
                    "brdrdashd" | "brdrdashdd" => BorderStyle::DotDash,
                    "brdrtriple" => BorderStyle::Triple,
                    "brdrwavy" => BorderStyle::Wave,
                    _ => BorderStyle::Single,
                };
                let b = (style != BorderStyle::None).then_some(Border { style, ..Border::single(0.5) });
                if let Some(slot) = self.cell_border_slot() {
                    *slot = b;
                }
            }
            "brdrw" if self.cell_side.is_some() || self.st.border_side.is_some() => {
                let width = (pv as f32 / 20.0).clamp(0.25, 6.0);
                if let Some(Some(b)) = self.cell_border_slot() {
                    b.width = width;
                }
            }
            "brdrcf" if self.cell_side.is_some() || self.st.border_side.is_some() => {
                let color = self.colors.get(pv.max(0) as usize).copied().flatten();
                if let Some(Some(b)) = self.cell_border_slot() {
                    b.color = color;
                }
            }
            "paperw" => self.page.w = Some(pv),
            "paperh" => self.page.h = Some(pv),
            "margl" => self.page.left = Some(pv),
            "margr" => self.page.right = Some(pv),
            "margt" => self.page.top = Some(pv),
            "margb" => self.page.bottom = Some(pv),
            "landscape" => self.page.landscape = on,
            _ if self.section_word(w, p) => {}
            "cellx" => {
                self.cell_side = None;
                let mut d = std::mem::take(&mut self.def_cur);
                d.right = pv;
                if self.defs.len() < 63 {
                    self.defs.push(d);
                }
            }
            "field" => self.start_field(),
            "fldrslt" => {
                let link = self.st.field.and_then(|i| self.fields.get(i)).and_then(|s| hyperlink(s));
                if link.is_some() {
                    self.st.fmt.link = link.as_deref().and_then(model::kept_link);
                }
            }
            _ => {}
        }
    }

    fn start_field(&mut self) {
        if self.fields.len() < 100_000 {
            self.fields.push(String::new());
            self.st.field = Some(self.fields.len() - 1);
        }
    }

    fn in_info(&self) -> bool {
        self.info_depth.is_some_and(|d| self.depth() > d)
    }
}

/// The target of a `HYPERLINK "url"` (or `HYPERLINK "url" \l "bookmark"`, `HYPERLINK \l "bookmark"`) field instruction.
pub(super) fn hyperlink(instr: &str) -> Option<String> {
    let t = instr.trim();
    let rest = if t.get(..9).is_some_and(|h| h.eq_ignore_ascii_case("hyperlink")) {
        t[9..].trim()
    } else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }

    let mut url: Option<String> = None;
    let mut local: Option<String> = None;
    let mut switch: Option<&str> = None;

    let mut i = 0;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'\\' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'*') {
                i += 1;
            }
            switch = Some(&rest[start..i]);
            continue;
        }
        let arg = if bytes[i] == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            let val = &rest[start..i];
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            val.to_string()
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            rest[start..i].to_string()
        };
        let clean_arg = arg.replace("%22", "\"");
        if switch.take() == Some("\\l") {
            local = Some(clean_arg);
        } else if url.is_none() {
            url = Some(clean_arg);
        }
    }

    match (url, local) {
        (Some(u), Some(l)) if !u.is_empty() && !l.is_empty() => {
            let frag = l.trim_start_matches('#');
            if u.contains('#') { Some(u) } else { Some(format!("{u}#{frag}")) }
        }
        (Some(u), _) if !u.is_empty() => Some(u),
        (None, Some(l)) if !l.is_empty() => {
            let frag = l.trim_start_matches('#');
            Some(format!("#{frag}"))
        }
        _ => None,
    }
}
