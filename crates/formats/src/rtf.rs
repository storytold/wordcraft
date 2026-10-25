//! Rich Text Format.
//!
//! Export: font and colour tables, a small stylesheet (headings with outline levels, Title,
//! Quote, Code), list tables (`\listtable` / `\listoverridetable` with `\listtext` labels for
//! older readers), document info, page setup, paragraph formatting (`\pard`, alignment, page
//! break before), character formatting (`\b \i \ul \strike \super \sub \f \fs \cf \chcbpat`),
//! hyperlinks as `HYPERLINK` fields, bookmarks, pictures (`\pngblip` / `\jpegblip`) and tables
//! (`\trowd … \cellx … \cell … \row`, header rows, vertical merges, shading).
//!
//! Import: a tolerant reader for groups and destinations (unknown `\*` destinations, headers,
//! footers, notes and objects are skipped), `\par \line \tab \page`, `\uN` with `\ucN`
//! fallback skipping, `\'hh` and 8-bit text in the document's code page (`\ansicpg`, `\mac`, `\pc`, `\pca`) or
//! the current font's (`\fcharset`, `\cpg`: Cyrillic, Central European, Greek, Mac OS Roman, the
//! East Asian multibyte pages 932/936/949/950…; Windows-1252 when none is named). A multibyte
//! code page's bytes stream through one decoder, so a lead byte and its trail byte decode
//! together and a run that looks like a UTF-8 BOM is not mistaken for one; the formatting words
//! above, stylesheet-based
//! headings and `\outlinelevel`, lists through `\ls`/`\ilvl`, fields, bookmarks, pictures and
//! tables. Group nesting and picture sizes are capped.

use std::collections::HashMap;

use wordcraft_doc::props::VAlign;
use wordcraft_doc::{Align, Document, Rgb};

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, RevMark, make_img};

/// Deepest group nesting we track; deeper groups share the innermost state.
const MAX_GROUPS: usize = 512;
/// Largest picture we decode (bytes).
const MAX_PICT: usize = 64 << 20;
/// Most sections we read (`\sect`); later breaks end paragraphs only.
const MAX_SECTIONS: usize = 10_000;
/// The encoding of an East Asian multibyte code page (932 Shift-JIS, 936 GBK, 949 UHC, 950 Big5),
/// which [`Reader::finish_multibyte`] decodes a run with; `None` for the single-byte code pages.
fn multibyte_encoding(code_page: u16) -> Option<&'static encoding_rs::Encoding> {
    let label = wordcraft_doc::encoding::multibyte_encoding_name(code_page)?;
    encoding_rs::Encoding::for_label(label.as_bytes())
}

// ---------------------------------------------------------------------------------------------
// Export

/// RTF-escape text (non-ASCII as `\uN?`).
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            '\t' => o.push_str("\\tab "),
            '\n' => o.push_str("\\line "),
            '\u{000C}' => o.push_str("\\page "),
            '\u{000E}' => o.push_str("\\column "),
            '\u{00A0}' => o.push_str("\\~"),
            '\u{00AD}' => o.push_str("\\-"),
            '\u{2011}' => o.push_str("\\_"),
            c if c.is_ascii_control() => {}
            c if c.is_ascii() => o.push(c),
            c => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    o.push_str(&format!("\\u{}?", *u as i16));
                }
            }
        }
    }
    o
}

struct Tables {
    fonts: Vec<String>,
    colors: Vec<Rgb>,
}

impl Tables {
    fn font(&mut self, name: &str) -> usize {
        match self.fonts.iter().position(|f| f.eq_ignore_ascii_case(name)) {
            Some(i) => i,
            None => {
                self.fonts.push(name.to_string());
                self.fonts.len() - 1
            }
        }
    }
    /// Colour table index (1-based; 0 is "auto").
    fn color(&mut self, c: Rgb) -> usize {
        match self.colors.iter().position(|x| *x == c) {
            Some(i) => i + 1,
            None => {
                self.colors.push(c);
                self.colors.len()
            }
        }
    }
}

struct Writer {
    t: Tables,
    /// List definitions: per list, the ordered flag per level.
    lists: Vec<[bool; 9]>,
    /// Authors of tracked changes (`\revtbl` entries after "Unknown"), when the text has any.
    authors: Option<Vec<String>>,
    /// The character formatting the current paragraph's style gives its runs (heading bold and
    /// size, code font…): what a run's formatting without a direct value resolves to.
    base: Fmt,
    /// The body's sections in order (the last is the document's); empty when it has just one.
    sections: Vec<wordcraft_doc::SectionProps>,
    /// The section being written.
    sect_i: usize,
}

/// `\sectd` and a section's formatting, in the words the reader reads back.
fn sect_words(s: &wordcraft_doc::SectionProps) -> String {
    use wordcraft_doc::section::{NumFormat, SectionStart};
    let tw = |v: f32| if v.is_finite() { ((v * 20.0).round() as i64).max(0) } else { 0 };
    let mut o = String::from("\\sectd");
    o.push_str(match s.start {
        SectionStart::Continuous => "\\sbknone",
        SectionStart::NextColumn => "\\sbkcol",
        SectionStart::NextPage => "\\sbkpage",
        SectionStart::EvenPage => "\\sbkeven",
        SectionStart::OddPage => "\\sbkodd",
    });
    o.push_str(&format!("\\pgwsxn{}\\pghsxn{}", tw(s.page_w), tw(s.page_h)));
    if s.landscape {
        o.push_str("\\lndscpsxn");
    }
    o.push_str(&format!(
        "\\marglsxn{}\\margrsxn{}\\margtsxn{}\\margbsxn{}\\headery{}\\footery{}",
        tw(s.margin_left),
        tw(s.margin_right),
        tw(s.margin_top),
        tw(s.margin_bottom),
        tw(s.header),
        tw(s.footer)
    ));
    if s.columns.count > 1 {
        o.push_str(&format!("\\cols{}\\colsx{}", s.columns.count, tw(s.columns.space)));
        if s.columns.separator {
            o.push_str("\\linebetcol");
        }
    }
    if let Some(n) = s.page_num_start {
        o.push_str(&format!("\\pgnrestart\\pgnstarts{n}"));
    }
    match s.page_num_format {
        NumFormat::UpperRoman => o.push_str("\\pgnucrm"),
        NumFormat::LowerRoman => o.push_str("\\pgnlcrm"),
        NumFormat::UpperLetter => o.push_str("\\pgnucltr"),
        NumFormat::LowerLetter => o.push_str("\\pgnlcltr"),
        _ => {}
    }
    if s.title_page {
        o.push_str("\\titlepg");
    }
    o.push('\n');
    o
}

const HEAD_SIZES: [f32; 6] = [20.0, 16.0, 14.0, 12.0, 12.0, 11.0];

fn style_num(k: Kind) -> Option<u32> {
    match k {
        Kind::Heading(n) => Some(u32::from(n.clamp(1, 6))),
        Kind::Title => Some(7),
        Kind::Quote => Some(8),
        Kind::Code => Some(9),
        _ => None,
    }
}

fn rtf_border_style(s: wordcraft_doc::props::BorderStyle) -> &'static str {
    use wordcraft_doc::props::BorderStyle;
    match s {
        BorderStyle::None => "\\brdrnone",
        BorderStyle::Single => "\\brdrs",
        BorderStyle::Double => "\\brdrdb",
        BorderStyle::Dotted => "\\brdrdot",
        BorderStyle::Dashed => "\\brdrdash",
        BorderStyle::Thick => "\\brdrth",
        BorderStyle::Triple => "\\brdrtriple",
        BorderStyle::DotDash => "\\brdrdashd",
        BorderStyle::Wave => "\\brdrwavy",
    }
}

impl Writer {
    fn write_border(&mut self, prefix: &str, side: char, b: wordcraft_doc::props::Border, out: &mut String) {
        out.push_str(&format!("\\{prefix}{side}"));
        out.push_str(rtf_border_style(b.style));
        let w = ((b.width * 20.0).round() as i64).clamp(1, 150);
        out.push_str(&format!("\\brdrw{w}"));
        if prefix == "brdr" {
            let sp = (b.space * 20.0).round() as i64;
            if sp > 0 {
                out.push_str(&format!("\\brsp{sp}"));
            }
        }
        if let Some(c) = b.color {
            let i = self.t.color(c);
            out.push_str(&format!("\\brdrcf{i}"));
        }
    }

    fn cell_borders(&mut self, borders: Option<&wordcraft_doc::props::Borders>, borderless: bool, out: &mut String) {
        if let Some(b) = borders {
            for (side, opt) in [('t', &b.top), ('l', &b.left), ('b', &b.bottom), ('r', &b.right)] {
                if let Some(border) = opt.filter(|x| x.is_visible()) {
                    self.write_border("clbrdr", side, border, out);
                }
            }
        } else if !borderless {
            out.push_str("\\clbrdrt\\brdrs\\brdrw10\\clbrdrl\\brdrs\\brdrw10\\clbrdrb\\brdrs\\brdrw10\\clbrdrr\\brdrs\\brdrw10");
        }
    }

    /// `\revtbl` index of a tracked change's author (0 = "Unknown").
    fn author(&mut self, a: &str) -> usize {
        let authors = self.authors.get_or_insert_with(Vec::new);
        let a = a.trim();
        if a.is_empty() {
            return 0;
        }
        match authors.iter().position(|x| x == a) {
            Some(i) => i + 1,
            None => {
                authors.push(a.to_string());
                authors.len()
            }
        }
    }

    /// Control words of a tracked insertion and deletion (`\revised`, `\deleted`).
    fn rev_ctl(&mut self, ins: Option<&RevMark>, del: Option<&RevMark>) -> String {
        let mut ctl = String::new();
        if let Some(m) = ins {
            ctl.push_str(&self.rev_words("\\revised\\revauth", "\\revdttm", m));
        }
        if let Some(m) = del {
            ctl.push_str(&self.rev_words("\\deleted\\revauthdel", "\\revdttmdel", m));
        }
        ctl
    }

    /// `auth` with the `\revtbl` index of `m`'s author, then `date` with its DTTM when it has one.
    fn rev_words(&mut self, auth: &str, date: &str, m: &RevMark) -> String {
        let mut s = format!("{auth}{}", self.author(&m.author));
        if let Some(d) = iso_to_dttm(&m.date) {
            s.push_str(&format!("{date}{}", d as i32));
        }
        s
    }

    fn run(&mut self, t: &str, f: &Fmt, out: &mut String) {
        let mut ctl = String::new();
        if let Some(c) = &f.fmt_change {
            // RTF 1.9.1, as Word writes it: the properties the change altered, as they were,
            // then who changed it and when, all in the group. The group goes first so a control
            // word, not the group, ends the run's properties.
            let old = self.char_diff(&c.old, f);
            let words = self.rev_words("\\crauth", "\\crdate", &c.mark);
            ctl.push_str(&format!("{{\\*\\oldcprops {old}{words}}}"));
        }
        ctl.push_str(&self.rev_ctl(f.ins.as_ref(), f.del.as_ref()));
        ctl.push_str(&self.char_ctl(f));
        let body = if ctl.is_empty() { esc(t) } else { format!("{{{ctl} {}}}", esc(t)) };
        // A script, `file:`, UNC or other non-web target is written as plain text, as the HTML and
        // Markdown exporters do: Word follows a HYPERLINK field's target on click, and a UNC one
        // leaks the reader's NTLM credentials.
        match f.link.as_deref().filter(|l| wordcraft_doc::link::is_safe_href(l)) {
            Some(l) => {
                out.push_str(&format!("{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}", esc(&l.replace('"', "%22")), body));
            }
            None => out.push_str(&body),
        }
    }

    /// The character formatting control words that turn `cur` back into `old`: only the
    /// properties that differ, with explicit "off" values (`\b0`, `\ulnone`, `\cf0`…).
    fn char_diff(&mut self, old: &Fmt, cur: &Fmt) -> String {
        // A link's underline and colour are the link's, before the change and after; what a run
        // doesn't set directly is its paragraph style's, as Word writes resolved values.
        let old = &styled(&self.base, &Fmt { link: cur.link.clone(), ..old.clone() });
        let cur = &styled(&self.base, cur);
        let mut ctl = String::new();
        let underline = |f: &Fmt| f.underline || f.link.is_some();
        let flags = [
            (old.bold, cur.bold, "\\b", "\\b0"),
            (old.italic, cur.italic, "\\i", "\\i0"),
            (underline(old), underline(cur), "\\ul", "\\ulnone"),
            (old.strike, cur.strike, "\\strike", "\\strike0"),
        ];
        for (o, c, on, off) in flags {
            if o != c {
                ctl.push_str(if o { on } else { off });
            }
        }
        if (old.sup, old.sub) != (cur.sup, cur.sub) {
            ctl.push_str(if old.sup {
                "\\super"
            } else if old.sub {
                "\\sub"
            } else {
                "\\nosupersub"
            });
        }
        let font = |f: &Fmt| if f.code { Some(model::MONO_FONT.to_string()) } else { f.font.clone() };
        let old_font = font(old);
        if old_font != font(cur) {
            // No font is the document's default font, font 0 (Normal's).
            let i = old_font.map_or(0, |n| self.t.font(&n));
            ctl.push_str(&format!("\\f{i}"));
        }
        let half_points = |s: Option<f32>| s.map(|s| (s * 2.0).round().clamp(2.0, 3276.0) as i32);
        let old_size = half_points(old.size);
        if old_size != half_points(cur.size) {
            // No size is Normal's, RTF's default 12 pt.
            ctl.push_str(&format!("\\fs{}", old_size.unwrap_or(24)));
        }
        let color = |f: &Fmt| f.color.or(if f.link.is_some() { Some(Rgb(0x05, 0x63, 0xC1)) } else { None });
        let old_color = color(old);
        if old_color != color(cur) {
            let i = old_color.map_or(0, |c| self.t.color(c));
            ctl.push_str(&format!("\\cf{i}"));
        }
        if old.background != cur.background {
            let i = old.background.map_or(0, |c| self.t.color(c));
            ctl.push_str(&format!("\\chcbpat{i}"));
        }
        ctl
    }

    /// Character formatting control words of `f` (no revision marks).
    fn char_ctl(&mut self, f: &Fmt) -> String {
        let mut ctl = String::new();
        if f.bold {
            ctl.push_str("\\b");
        }
        if f.italic {
            ctl.push_str("\\i");
        }
        if f.underline || f.link.is_some() {
            ctl.push_str("\\ul");
        }
        if f.strike {
            ctl.push_str("\\strike");
        }
        if f.sup {
            ctl.push_str("\\super");
        } else if f.sub {
            ctl.push_str("\\sub");
        }
        if f.code {
            let i = self.t.font(model::MONO_FONT);
            ctl.push_str(&format!("\\f{i}"));
        } else if let Some(fam) = &f.font {
            let i = self.t.font(fam);
            ctl.push_str(&format!("\\f{i}"));
        }
        if let Some(s) = f.size {
            ctl.push_str(&format!("\\fs{}", (s * 2.0).round().clamp(2.0, 3276.0) as i32));
        }
        let color = f.color.or(if f.link.is_some() { Some(Rgb(0x05, 0x63, 0xC1)) } else { None });
        if let Some(c) = color {
            let i = self.t.color(c);
            ctl.push_str(&format!("\\cf{i}"));
        }
        if let Some(c) = f.background {
            let i = self.t.color(c);
            ctl.push_str(&format!("\\chcbpat{i}"));
        }
        ctl
    }

    fn picture(&mut self, img: &model::Img, out: &mut String) {
        let (data, blip) = match img.ext.as_str() {
            "png" => (img.data.to_vec(), "\\pngblip"),
            "jpeg" | "jpg" => (img.data.to_vec(), "\\jpegblip"),
            _ => {
                // Re-encode other formats as PNG, bounding the decode so a tiny file declaring
                // huge dimensions can't exhaust memory on export.
                let Some(dec) = model::decode_bounded(&img.data) else { return };
                let mut buf = Vec::new();
                if dec.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).is_err() {
                    return;
                }
                (buf, "\\pngblip")
            }
        };
        let (pw, ph) = model::image_px(&data).unwrap_or((1, 1));
        out.push_str(&format!(
            "{{\\pict{blip}\\picw{pw}\\pich{ph}\\picwgoal{}\\pichgoal{}\n",
            (img.w * 20.0).round() as i64,
            (img.h * 20.0).round() as i64
        ));
        for chunk in data.chunks(64) {
            for b in chunk {
                out.push_str(&format!("{b:02x}"));
            }
            out.push('\n');
        }
        out.push('}');
    }

    fn inlines(&mut self, inl: &[Inline], out: &mut String) {
        let inl = model::equations_as_text(inl);
        for i in inl.iter() {
            match i {
                Inline::Text(t, f) => self.run(t, f, out),
                Inline::Image(img) => self.picture(img, out),
                Inline::Anchor(a) => {
                    let a = esc(a);
                    out.push_str(&format!("{{\\*\\bkmkstart {a}}}{{\\*\\bkmkend {a}}}"));
                }
                // RTF keeps no ruby yet: its base text, in the run's formatting.
                Inline::Ruby { base, fmt, .. } => self.run(base, fmt, out),
                Inline::Figure(_) | Inline::Equation { .. } => {}
            }
        }
    }

    fn para(&mut self, p: &Para, intbl: bool, ls: Option<(usize, String)>, end: &str, out: &mut String) {
        let ctl = self.para_ctl(p, intbl);
        out.push_str(&ctl);
        if let Some(c) = &p.fmt_change {
            // RTF 1.9.1: the paragraph formatting before the change, then who changed it and when.
            let old = self.para_ctl(&c.old, intbl);
            // The old list is the same list instance (only its level may differ).
            let list = match (c.old.list, &ls) {
                (Some(li), Some((n, _))) => format!("\\ls{n}\\ilvl{}", li.level.min(8)),
                _ => String::new(),
            };
            out.push_str(&format!("{{\\*\\oldpprops {old}{list}}}"));
            let words = self.rev_words("\\prauth", "\\prdate", &c.mark);
            out.push_str(&words);
        }
        if p.kind != Kind::Rule
            && let Some(b) = &p.borders
        {
            for (side, opt) in [('t', &b.top), ('l', &b.left), ('b', &b.bottom), ('r', &b.right)] {
                if let Some(border) = opt.filter(|x| x.is_visible()) {
                    self.write_border("brdr", side, border, out);
                }
            }
        }
        if let (Some(li), Some((n, label))) = (p.list, &ls) {
            let lv = li.level.min(8);
            out.push_str(&format!("\\ls{n}\\ilvl{lv}\\fi-360\\li{}", 720 * (u32::from(lv) + 1)));
            // The closing brace ends the label; a space after it would be text.
            out.push_str(&format!(" {{\\listtext\\pard\\plain {}\\tab}}", esc(label)));
        } else {
            out.push(' ');
        }
        self.base = style_fmt(p.kind);
        self.inlines(&p.inlines, out);
        self.base = Fmt::default();
        // A tracked paragraph mark is the `\par` inside the revision's group.
        let marks = self.rev_ctl(p.mark_ins.as_deref(), p.mark_del.as_deref());
        if end == "\\par" && !marks.is_empty() {
            out.push_str(&format!("{{{marks} {end}}}"));
        } else {
            out.push_str(end);
        }
        out.push('\n');
    }

    /// The paragraph formatting control words of `p`, from `\pard\plain` (its list left out).
    fn para_ctl(&mut self, p: &Para, intbl: bool) -> String {
        let mut out = String::from("\\pard\\plain");
        if intbl {
            out.push_str("\\intbl");
        }
        if let Some(s) = style_num(p.kind) {
            out.push_str(&format!("\\s{s}"));
        }
        if let Kind::Heading(n) = p.kind {
            out.push_str(&format!("\\outlinelevel{}", n.clamp(1, 6) - 1));
        }
        match p.align {
            Some(Align::Center) => out.push_str("\\qc"),
            Some(Align::Right) => out.push_str("\\qr"),
            Some(Align::Justify) => out.push_str("\\qj"),
            Some(Align::Distribute) => out.push_str("\\qd"),
            _ => out.push_str("\\ql"),
        }
        if p.page_break {
            out.push_str("\\pagebb");
        }
        match p.kind {
            Kind::Quote => out.push_str("\\li720\\ri720\\i"),
            Kind::Code => {
                let f = self.t.font(model::MONO_FONT);
                out.push_str(&format!("\\sa0\\f{f}\\fs20"));
            }
            Kind::Rule => out.push_str("\\brdrb\\brdrs\\brdrw15\\brsp20"),
            Kind::Heading(n) => {
                let sz = HEAD_SIZES.get(usize::from(n.clamp(1, 6)) - 1).copied().unwrap_or(12.0);
                out.push_str(&format!("\\sb240\\sa80\\keepn\\b\\fs{}", (sz * 2.0) as i32));
            }
            Kind::Title => out.push_str("\\sa80\\fs56"),
            Kind::Normal => {}
        }
        out
    }

    fn blocks(&mut self, blocks: &[FBlock], intbl: bool, last_end: &str, out: &mut String, depth: usize) {
        let mut counter = model::ListCounter::default();
        let mut cur_list: Option<usize> = None;
        let mut top_ordered = false;
        let n = blocks.len();
        for (i, b) in blocks.iter().enumerate() {
            let mut end = if i + 1 == n { last_end.to_string() } else { "\\par".to_string() };
            match b {
                FBlock::Para(p) => {
                    // A paragraph that ends a section ends it with `\sect`; the next one begins.
                    if p.section.is_some() && !intbl && depth == 0 && self.sect_i + 1 < self.sections.len() {
                        self.sect_i += 1;
                        end = format!("\\sect\n{}", self.sections.get(self.sect_i).map(sect_words).unwrap_or_default());
                        // A body that ends on the break keeps its last section: an empty paragraph holds it.
                        if i + 1 == n {
                            end.push_str("\\pard\\plain \\par");
                        }
                    }
                    let ls = match p.list {
                        Some(li) => {
                            // A top-level item of the other kind starts a new list.
                            if li.level == 0 && cur_list.is_some() && top_ordered != li.ordered {
                                cur_list = None;
                            }
                            if li.level == 0 {
                                top_ordered = li.ordered;
                            }
                            let id = match cur_list {
                                Some(id) => id,
                                None => {
                                    // Level kinds from the whole run of list paragraphs.
                                    let mut kinds = [li.ordered; 9];
                                    let mut seen = [false; 9];
                                    for q in blocks.iter().skip(i) {
                                        let FBlock::Para(q) = q else { break };
                                        let Some(ql) = q.list else { break };
                                        let l = ql.level.min(8) as usize;
                                        if l == 0 && seen[0] && kinds[0] != ql.ordered {
                                            break;
                                        }
                                        if let (Some(s), Some(k)) = (seen.get_mut(l), kinds.get_mut(l))
                                            && !*s
                                        {
                                            *s = true;
                                            *k = ql.ordered;
                                        }
                                    }
                                    self.lists.push(kinds);
                                    counter.reset();
                                    self.lists.len()
                                }
                            };
                            cur_list = Some(id);
                            Some((id, counter.label(li)))
                        }
                        None => {
                            cur_list = None;
                            None
                        }
                    };
                    self.para(p, intbl, ls, &end, out);
                }
                FBlock::Table(t) => {
                    cur_list = None;
                    if intbl || depth > 0 {
                        // Nested table: its cells' paragraphs flow into this cell.
                        let inner: Vec<FBlock> = t.rows.iter().flatten().flat_map(|c| c.blocks.clone()).collect();
                        if depth < model::MAX_DEPTH {
                            self.blocks(&inner, intbl, &end, out, depth + 1);
                        }
                    } else {
                        self.table(t, out, depth);
                    }
                }
            }
        }
        if blocks.is_empty() && !last_end.is_empty() {
            out.push_str(&format!("\\pard\\plain{} {last_end}\n", if intbl { "\\intbl" } else { "" }));
        }
    }

    fn table(&mut self, t: &FTable, out: &mut String, depth: usize) {
        let cols = t.cols().max(1);
        let total = 9360.0f32; // 6.5" in twips
        let widths: Vec<f32> = if t.widths.len() == cols && t.widths.iter().all(|w| w.is_finite() && *w > 0.0) {
            t.widths.iter().map(|w| w * 20.0).collect()
        } else {
            vec![total / cols as f32; cols]
        };
        for row in &t.rows {
            out.push_str("\\trowd\\trgaph108\\trleft0");
            if !row.is_empty() && row.iter().all(|c| c.header) {
                out.push_str("\\trhdr");
            }
            let mut g = 0usize;
            let mut x = 0.0f32;
            for c in row {
                let span = c.colspan.clamp(1, 63) as usize;
                for k in g..(g + span).min(widths.len()) {
                    x += widths.get(k).copied().unwrap_or(0.0);
                }
                g += span;
                if c.covered {
                    out.push_str("\\clvmrg");
                } else if c.rowspan > 1 {
                    out.push_str("\\clvmgf");
                }
                match c.valign {
                    Some(VAlign::Top) => out.push_str("\\clvertalt"),
                    Some(VAlign::Center) => out.push_str("\\clvertalc"),
                    Some(VAlign::Bottom) => out.push_str("\\clvertalb"),
                    None => {}
                }
                self.cell_borders(c.borders.as_ref(), t.borderless, out);
                if let Some(s) = c.shading {
                    let i = self.t.color(s);
                    out.push_str(&format!("\\clcbpat{i}"));
                }
                out.push_str(&format!("\\cellx{}", x.round() as i64));
            }
            out.push('\n');
            for c in row {
                if c.covered {
                    out.push_str("\\pard\\plain\\intbl \\cell\n");
                } else {
                    let blocks: Vec<FBlock> = c
                        .blocks
                        .iter()
                        .cloned()
                        .map(|b| match b {
                            FBlock::Para(mut p) if c.header => {
                                for i in &mut p.inlines {
                                    if let Inline::Text(_, f) = i {
                                        f.bold = true;
                                    }
                                }
                                FBlock::Para(p)
                            }
                            other => other,
                        })
                        .collect();
                    self.blocks(&blocks, true, "\\cell", out, depth);
                }
            }
            out.push_str("\\row\n");
        }
    }
}

fn level_def(ordered: bool, l: usize) -> String {
    let indent = 720 * (l + 1);
    if ordered {
        let nfc = [0, 4, 2].get(l % 3).copied().unwrap_or(0);
        format!(
            "{{\\listlevel\\levelnfc{nfc}\\levelnfcn{nfc}\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1\\levelspace0\\levelindent0{{\\leveltext\\'02\\'{l:02x}.;}}{{\\levelnumbers\\'01;}}\\fi-360\\li{indent}\\lin{indent}}}"
        )
    } else {
        let ch = ['\u{2022}', '\u{25E6}', '\u{25AA}'].get(l % 3).copied().unwrap_or('\u{2022}');
        format!(
            "{{\\listlevel\\levelnfc23\\levelnfcn23\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1\\levelspace0\\levelindent0{{\\leveltext\\'01\\u{}?;}}{{\\levelnumbers;}}\\fi-360\\li{indent}\\lin{indent}}}",
            ch as u32 as i16
        )
    }
}

fn rtf_date(s: &str) -> Option<String> {
    let num = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<u32>().ok());
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi) = (num(11, 13).unwrap_or(0), num(14, 16).unwrap_or(0));
    Some(format!("\\yr{y}\\mo{mo}\\dy{d}\\hr{h}\\min{mi}"))
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Day of the week, 0 = Sunday (Sakamoto's method).
fn weekday(y: u32, m: u32, d: u32) -> u32 {
    const T: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y.saturating_sub(1) } else { y };
    (y + y / 4 - y / 100 + y / 400 + T.get(m.saturating_sub(1) as usize).copied().unwrap_or(0) + d) % 7
}

/// An RTF revision date (DTTM, RTF 1.9.1 "Revision Marks": minute in bits 0–5, hour 6–10, day
/// 11–15, month 16–19, year − 1900 20–28, weekday 29–31) as ISO 8601 (`2024-03-15T10:30:00Z`).
/// Zero or an impossible date is no date. DTTM has no time zone or seconds: times are read as
/// UTC (`Z`).
pub fn dttm_to_iso(v: u32) -> Option<String> {
    let (mi, h, d, mo, y) = (v & 0x3F, (v >> 6) & 0x1F, (v >> 11) & 0x1F, (v >> 16) & 0xF, 1900 + ((v >> 20) & 0x1FF));
    if v == 0 || mi > 59 || h > 23 || d == 0 || d > days_in_month(y, mo) {
        return None;
    }
    Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:00Z"))
}

/// An ISO 8601 date (`YYYY-MM-DD`, optionally `THH:MM…`; seconds and zone ignored) as an RTF
/// DTTM; `None` when it isn't one or falls outside DTTM's years (1900–2411). DTTM has no time
/// zone or seconds: the clock time is written as given (WordCraft keeps UTC, `Z`), and seconds
/// and any offset are dropped.
pub fn iso_to_dttm(s: &str) -> Option<u32> {
    let num = |a: usize, b: usize| s.get(a..b).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse::<u32>().ok());
    let sep = |i: usize, c: &[&str]| s.get(i..i + 1).is_some_and(|t| c.contains(&t));
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if !sep(4, &["-"]) || !sep(7, &["-"]) {
        return None;
    }
    let (h, mi) = if s.len() > 10 {
        if !sep(10, &["T", " "]) || !sep(13, &[":"]) {
            return None;
        }
        (num(11, 13)?, num(14, 16)?)
    } else {
        (0, 0)
    };
    if !(1900..=2411).contains(&y) || mi > 59 || h > 23 || d == 0 || d > days_in_month(y, mo) {
        return None;
    }
    Some(mi | (h << 6) | (d << 11) | (mo << 16) | ((y - 1900) << 20) | (weekday(y, mo, d) << 29))
}

/// RTF text of a document.
pub fn export(doc: &Document) -> String {
    let flow = model::from_doc_tracked(doc);
    let base_font = doc.styles.default_chr.font.clone().unwrap_or_else(|| wordcraft_doc::styles::BODY_FONT.to_string());
    let mut w = Writer {
        t: Tables { fonts: vec![base_font], colors: Vec::new() },
        lists: Vec::new(),
        authors: None,
        base: Fmt::default(),
        sections: Vec::new(),
        sect_i: 0,
    };
    let mut body = String::new();
    // Every section starts with its own `\sectd`; `\sect` breaks only where there are several.
    let secs: Vec<wordcraft_doc::SectionProps> = doc.sections().into_iter().map(|(_, s)| s.clone()).collect();
    if let Some(first) = secs.first() {
        body.push_str(&sect_words(first));
    }
    if secs.len() > 1 {
        w.sections = secs;
    }
    w.blocks(&flow.blocks, false, "\\par", &mut body, 0);
    export_with(&flow.meta, &w, &body, &doc.last_section)
}

fn export_with(m: &Meta, w: &Writer, body: &str, sec: &wordcraft_doc::SectionProps) -> String {
    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\deflang1033\\uc1\n{\\fonttbl");
    for (i, f) in w.t.fonts.iter().enumerate() {
        let fam = if model::is_mono(f) { "\\fmodern" } else { "\\fswiss" };
        out.push_str(&format!("{{\\f{i}{fam}\\fcharset0 {};}}", esc(f)));
    }
    out.push_str("}\n{\\colortbl;");
    for c in &w.t.colors {
        out.push_str(&format!("\\red{}\\green{}\\blue{};", c.0, c.1, c.2));
    }
    out.push_str("}\n{\\*\\generator WordCraft;}\n{\\stylesheet{\\s0\\snext0 Normal;}");
    for n in 1..=6u32 {
        let sz = HEAD_SIZES.get(n as usize - 1).copied().unwrap_or(12.0);
        out.push_str(&format!("{{\\s{n}\\sbasedon0\\snext0\\outlinelevel{}\\keepn\\b\\fs{} heading {n};}}", n - 1, (sz * 2.0) as i32));
    }
    out.push_str(
        "{\\s7\\sbasedon0\\snext0\\fs56 Title;}{\\s8\\sbasedon0\\snext0\\li720\\ri720\\i Quote;}{\\s9\\sbasedon0\\snext9\\sa0\\fs20 Code;}}\n",
    );
    if !w.lists.is_empty() {
        out.push_str("{\\*\\listtable");
        for (i, kinds) in w.lists.iter().enumerate() {
            let id = i + 1;
            out.push_str(&format!("\n{{\\list\\listtemplateid{id}\\listhybrid"));
            for (l, k) in kinds.iter().enumerate() {
                out.push_str(&level_def(*k, l));
            }
            out.push_str(&format!("{{\\listname ;}}\\listid{id}}}"));
        }
        out.push_str("}\n{\\*\\listoverridetable");
        for i in 1..=w.lists.len() {
            out.push_str(&format!("{{\\listoverride\\listid{i}\\listoverridecount0\\ls{i}}}"));
        }
        out.push_str("}\n");
    }
    if let Some(authors) = &w.authors {
        out.push_str("{\\*\\revtbl {Unknown;}");
        for a in authors {
            out.push_str(&format!("{{{};}}", esc(&a.replace(';', ","))));
        }
        out.push_str("}\n");
    }
    out.push_str("{\\info");
    for (k, v) in [("title", &m.title), ("author", &m.author), ("subject", &m.subject), ("keywords", &m.keywords), ("doccomm", &m.description)] {
        if !v.is_empty() {
            out.push_str(&format!("{{\\{k} {}}}", esc(v)));
        }
    }
    if let Some(d) = rtf_date(&m.created) {
        out.push_str(&format!("{{\\creatim{d}}}"));
    }
    out.push_str("}\n");
    let tw = |v: f32| (v * 20.0).round() as i64;
    let landscape = if sec.landscape { "\\landscape" } else { "" };
    out.push_str(&format!(
        "\\paperw{}\\paperh{}\\margl{}\\margr{}\\margt{}\\margb{}{landscape}\\viewkind4\n",
        tw(sec.page_w),
        tw(sec.page_h),
        tw(sec.margin_left),
        tw(sec.margin_right),
        tw(sec.margin_top),
        tw(sec.margin_bottom)
    ));
    out.push_str(body);
    out.push_str("}\n");
    out
}

// ---------------------------------------------------------------------------------------------
// Import

#[derive(Clone, Copy, Debug, PartialEq)]
enum Dest {
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
    /// The author table of tracked changes (`\revtbl`).
    RevTbl,
    /// Formatting before a tracked change (`\oldpprops` when true, else `\oldcprops`): read
    /// into the group's own state, which the group's end keeps as the change's old formatting.
    OldProps(bool),
}

/// A tracked formatting change being read (`\crauth`/`\crdate` with `\oldcprops`,
/// `\prauth`/`\prdate` with `\oldpprops`): author (`\revtbl` index), date (DTTM) and the old
/// formatting, once its group has been read.
#[derive(Clone, Debug, Default)]
struct FmtRevSt<T> {
    auth: i32,
    date: u32,
    old: Option<Box<T>>,
}

/// Character formatting words of an `\oldcprops` group, with their parameters.
type OldWords = Vec<(String, Option<i32>)>;

/// Most words kept from one `\oldcprops` group (hostile input).
const MAX_OLD_WORDS: usize = 64;

/// Most authors read from a `\revtbl` (hostile input).
const MAX_REV_AUTHORS: usize = 10_000;

/// Tracked-change character properties: `\revised` / `\deleted`, their authors (`\revtbl`
/// indices) and dates (DTTM).
#[derive(Clone, Copy, Debug, Default)]
struct RevSt {
    ins: bool,
    del: bool,
    auth: i32,
    authdel: i32,
    dttm: u32,
    dttmdel: u32,
}

/// Which header or footer a `\header…`/`\footer…` group is: footer or header, and the pages
/// (`\headerf` first, `\headerl` left = even, `\header`/`\headerr` the others).
#[derive(Clone, Copy, Debug, PartialEq)]
struct HeaderFooter {
    footer: bool,
    pages: HfPages,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum HfPages {
    Default,
    First,
    Even,
}

/// The reader's output so far, set aside while a header or footer is read.
#[derive(Default)]
struct Output {
    body: Vec<FBlock>,
    para: Para,
    para_has: bool,
    table: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    cell: Vec<FBlock>,
    defs: Vec<CellDef>,
    def_cur: CellDef,
    row_header: bool,
    rule_pending: bool,
    table_rights: Vec<Vec<i32>>,
}

#[derive(Clone, Debug)]
struct St {
    dest: Dest,
    fmt: Fmt,
    rev: RevSt,
    /// Tracked change of the character formatting (reset by `\plain`). Its old formatting is
    /// the `\oldcprops` words, applied on top of the run's formatting when text is read: Word
    /// writes only the properties that changed there.
    chg_c: FmtRevSt<OldWords>,
    /// While an `\oldcprops` group is read: its character formatting words, and the author and
    /// date of the change (`\crauth`/`\crdate`, which Word writes inside the group).
    old_words: OldWords,
    old_rev: (Option<i32>, Option<u32>),
    /// Tracked change of the paragraph formatting (reset by `\pard`).
    chg_p: FmtRevSt<Para>,
    uc: usize,
    hidden: bool,
    // Paragraph properties.
    align: Option<Align>,
    style: i32,
    intbl: bool,
    ls: Option<i32>,
    ilvl: u8,
    outline: Option<u8>,
    pagebb: bool,
    field: Option<usize>,
    /// The code page of the current font's charset (`\fcharset`), when it names one.
    cp: Option<u16>,
    /// Space before/after (`\sb`, `\sa`), line spacing (`\sl`, `\slmult`) and indents
    /// (`\li`, `\ri`, `\fi`), twips.
    /// The paragraph's borders (`\brdrt`, `\brdrb`, `\brdrl`, `\brdrr`) and the side being
    /// described.
    borders: wordcraft_doc::props::Borders,
    border_side: Option<char>,
    sb: Option<i32>,
    sa: Option<i32>,
    sl: i32,
    slmult: bool,
    li: Option<i32>,
    ri: Option<i32>,
    fi: Option<i32>,
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
struct CellDef {
    vmerge: u8,
    hmerge: u8,
    shading: Option<Rgb>,
    right: i32,
    /// The borders the cell names (`\clbrdrt` … `\clbrdrr`, each with its line style, `\brdrw`
    /// width and `\brdrcf` colour).
    borders: wordcraft_doc::props::Borders,
    valign: Option<VAlign>,
}

/// Page size and margins in twips (`\paperw`, `\margl`… and their `…sxn` section forms).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PageGeom {
    w: Option<i32>,
    h: Option<i32>,
    left: Option<i32>,
    right: Option<i32>,
    top: Option<i32>,
    bottom: Option<i32>,
    header: Option<i32>,
    footer: Option<i32>,
    landscape: bool,
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
struct SecFmt {
    page: PageGeom,
    start: wordcraft_doc::section::SectionStart,
    cols: Option<i32>,
    colsx: Option<i32>,
    linebetcol: bool,
    pgn_restart: bool,
    pgn_start: Option<i32>,
    pgn_fmt: wordcraft_doc::section::NumFormat,
    title_page: bool,
}

impl SecFmt {
    /// The section's properties over the document's page (`doc`); headers and footers unset.
    fn props(&self, doc: &PageGeom) -> wordcraft_doc::SectionProps {
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

#[derive(Default)]
struct Reader {
    st: St,
    stack: Vec<St>,
    overflow: usize,
    fonts: HashMap<i32, String>,
    font_cur: i32,
    font_name: String,
    /// Code pages of the fonts whose charset names one (`\fcharset204` → 1251).
    font_cps: HashMap<i32, u16>,
    font_cp_cur: Option<u16>,
    /// The document's code page (`\ansicpg`); 0 until it names one (Windows-1252 then).
    ansicpg: u16,
    /// The default font (`\deff`), whose charset applies before any `\f`.
    deff: i32,
    /// The document's page (`\paperw`, `\margl`…) and the current section's formatting.
    page: PageGeom,
    sec: SecFmt,
    /// The headers and footers of each section ended so far (`\sect`); `hf_parts` holds the
    /// current section's.
    section_parts: Vec<Vec<(HeaderFooter, Vec<FBlock>)>>,
    /// The header or footer being read: the group depth it started at, which one, and the
    /// body's output, put back when the group closes.
    hf: Option<(usize, HeaderFooter, Box<Output>)>,
    /// The current section's headers and footers, in order.
    hf_parts: Vec<(HeaderFooter, Vec<FBlock>)>,
    /// The cell border side being described (`\clbrdrt`, `l`, `b`, `r`): its line style, width
    /// and colour follow.
    cell_side: Option<char>,
    /// Each table row's cell right edges (`\cellx`, twips), for the table's column grid.
    table_rights: Vec<Vec<i32>>,
    colors: Vec<Option<Rgb>>,
    color_cur: (u8, u8, u8, bool),
    styles: HashMap<i32, String>,
    /// Each style's own character formatting, as its `\stylesheet` entry writes it.
    style_defs: HashMap<i32, StyleDef>,
    style_cur: i32,
    style_name: String,
    style_def_cur: StyleDef,
    meta: Meta,
    info_text: String,
    // Lists: list id → ordered per level; override ls → list id.
    lists: HashMap<i32, Vec<bool>>,
    list_cur: Vec<bool>,
    list_cur_id: i32,
    overrides: HashMap<i32, i32>,
    ov_list: i32,
    ov_ls: i32,
    fields: Vec<String>,
    // Picture being read.
    pict_depth: Option<usize>,
    pict_hex: Vec<u8>,
    pict_half: Option<u8>,
    pict_kind: u8,
    pict_w: Option<f32>,
    pict_h: Option<f32>,
    pict_sx: f32,
    pict_sy: f32,
    bkmk: Option<(usize, String)>,
    /// Authors of tracked changes (`\revtbl`), and the entry being read.
    rev_authors: Vec<String>,
    rev_name: String,
    skip_chars: usize,
    pending_high: Option<u16>,
    /// A streaming decoder for the current multibyte code page, and the code page it was made for.
    /// A lead byte and its trail byte decode together even when a text word, a group boundary or the
    /// end of a chunk falls between them, so a run of any length never splits a character.
    mb_decoder: Option<encoding_rs::Decoder>,
    mb_cp: u16,
    // Output.
    body: Vec<FBlock>,
    para: Para,
    para_has: bool,
    table: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    cell: Vec<FBlock>,
    defs: Vec<CellDef>,
    def_cur: CellDef,
    row_header: bool,
    created: Option<(u32, u32, u32, u32, u32)>,
    in_creatim: bool,
    rule_pending: bool,
    info_depth: Option<usize>,
}

impl Reader {
    fn depth(&self) -> usize {
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
    fn cell_border_slot(&mut self) -> Option<&mut Option<wordcraft_doc::props::Border>> {
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
    fn start_header_footer(&mut self, which: HeaderFooter) {
        let body = self.swap_output(Output::default());
        self.hf = Some((self.depth(), which, Box::new(body)));
        self.st.intbl = false;
    }

    /// The header or footer group closed: keep what it held and go back to the body.
    fn end_header_footer(&mut self) {
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
    fn end_section(&mut self) {
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
    fn section_word(&mut self, w: &str, p: Option<i32>) -> bool {
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

    fn open(&mut self) {
        self.skip_chars = 0;
        if self.stack.len() >= MAX_GROUPS {
            self.overflow += 1;
            return;
        }
        self.stack.push(self.st.clone());
    }

    fn close(&mut self) {
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

    fn commit_author(&mut self) {
        let name = std::mem::take(&mut self.rev_name).trim().to_string();
        if self.rev_authors.len() < MAX_REV_AUTHORS {
            self.rev_authors.push(name);
        }
    }

    /// The tracked-change mark for `\revtbl` author index `i` and DTTM `dttm`; an index outside
    /// the table is "Unknown".
    fn rev_mark(&self, i: i32, dttm: u32) -> model::RevMark {
        let author = usize::try_from(i).ok().and_then(|i| self.rev_authors.get(i)).cloned().unwrap_or_else(|| "Unknown".to_string());
        model::RevMark { author, date: dttm_to_iso(dttm).unwrap_or_default(), id: None }
    }

    fn commit_font(&mut self) {
        let name = std::mem::take(&mut self.font_name).trim().trim_end_matches(';').trim().to_string();
        if !name.is_empty() && self.fonts.len() < 4096 {
            self.fonts.insert(self.font_cur, name);
            if let Some(cp) = self.font_cp_cur.take() {
                self.font_cps.insert(self.font_cur, cp);
            }
        }
    }

    /// The code page 8-bit text is read in: the current font's, else the default font's, else the
    /// document's (Windows-1252 when none is named).
    fn code_page(&self) -> u16 {
        self.st.cp.or_else(|| self.font_cps.get(&self.deff).copied()).unwrap_or(if self.ansicpg == 0 { 1252 } else { self.ansicpg })
    }

    /// A byte of 8-bit text (`\'hh` or raw): a single-byte code page decodes it at once; a
    /// multibyte one is fed to a streaming decoder ([`Reader::finish_multibyte`]), so a lead byte decodes
    /// together with its trail byte.
    fn byte(&mut self, b: u8) {
        if self.skip_chars > 0 {
            self.skip_chars -= 1;
            return;
        }
        let cp = self.code_page();
        let Some(encoding) = multibyte_encoding(cp) else {
            self.finish_multibyte();
            let c = wordcraft_doc::encoding::code_page_byte(cp, b);
            self.ch(c);
            return;
        };
        // A code page switch ends the previous run (a lead byte left then becomes U+FFFD, as Word
        // writes it) before the new decoder starts.
        if self.mb_cp != cp || self.mb_decoder.is_none() {
            self.finish_multibyte();
            self.mb_cp = cp;
            self.mb_decoder = Some(encoding.new_decoder_without_bom_handling());
        }
        self.decode_run(&[b]);
    }

    /// Feed 8-bit text bytes to the open multibyte decoder and append what it emits. A single
    /// `\'hh` byte decodes into a stack buffer (no heap allocation per byte of escaped text); a
    /// raw run decodes in one call, so a long ASCII run in a Japanese, Chinese or Korean document
    /// costs one append instead of one per byte. The destination's spare capacity is all
    /// `decode_to_*` writes into, so a full buffer is drained and decoding continues.
    fn decode_run(&mut self, bytes: &[u8]) {
        // The decoder is taken out so the text it produced can be appended (which borrows the
        // reader) before it goes back.
        let Some(mut decoder) = self.mb_decoder.take() else { return };
        let mut read = 0;
        if bytes.len() <= 16 {
            // A character is at most 4 UTF-8 bytes, and a byte may also replace a pending lead
            // byte (3 more), so one escaped byte always fits the stack buffer.
            let mut buf = [0u8; 16];
            let (result, got, written, _) = decoder.decode_to_utf8(bytes, &mut buf, false);
            read = got;
            let full = matches!(result, encoding_rs::CoderResult::OutputFull);
            if written > 0
                && let Ok(s) = std::str::from_utf8(&buf[..written])
            {
                self.text(s);
            }
            if !full {
                self.mb_decoder = Some(decoder);
                return;
            }
        }
        let mut out = String::with_capacity(bytes.len().saturating_mul(2));
        let mut rest = bytes.get(read..).unwrap_or_default();
        loop {
            let (result, got, _) = decoder.decode_to_string(rest, &mut out, false);
            rest = rest.get(got..).unwrap_or_default();
            if !matches!(result, encoding_rs::CoderResult::OutputFull) {
                break;
            }
            out.reserve(16.max(rest.len().saturating_mul(2)));
        }
        self.mb_decoder = Some(decoder);
        if !out.is_empty() {
            self.text(&out);
        }
    }

    /// A run of raw 8-bit text from the file (not `\'hh` escapes): a single-byte code page maps
    /// each byte to a char; a multibyte one decodes the whole run in one pass, so a long ASCII run
    /// in a Japanese, Chinese or Korean document costs one decode and one append instead of one
    /// per byte.
    ///
    /// The decoder is created without BOM handling: the code page comes from `\ansicpg`, `\cpg` or
    /// `\fcharset` and must not change. With BOM sniffing, a run that starts `EF BB BF` in GBK,
    /// Big5 or UHC (a valid character pair followed by a lead byte) would be dropped as a UTF-8
    /// BOM and the rest decoded as UTF-8.
    fn run(&mut self, chunk: &[u8]) {
        if self.skip_chars > 0 {
            for b in chunk {
                self.byte(*b);
            }
            return;
        }
        let cp = self.code_page();
        let Some(encoding) = multibyte_encoding(cp) else {
            self.finish_multibyte();
            let mut out = String::with_capacity(chunk.len());
            for b in chunk {
                out.push(wordcraft_doc::encoding::code_page_byte(cp, *b));
            }
            if !out.is_empty() {
                self.text(&out);
            }
            return;
        };
        if self.mb_cp != cp || self.mb_decoder.is_none() {
            self.finish_multibyte();
            self.mb_cp = cp;
            self.mb_decoder = Some(encoding.new_decoder_without_bom_handling());
        }
        self.decode_run(chunk);
    }

    /// Ends the current multibyte run: the streaming decoder emits whatever it still holds (a lone
    /// lead byte becomes U+FFFD, as Word writes it) and is dropped. A code page switch, a single-byte
    /// run, a group end and the end of the document finish the run this way.
    fn finish_multibyte(&mut self) {
        let Some(mut decoder) = self.mb_decoder.take() else { return };
        let mut out = String::with_capacity(16);
        let _ = decoder.decode_to_string(&[], &mut out, true);
        if !out.is_empty() {
            self.text(&out);
        }
    }

    fn commit_style(&mut self) {
        let name = std::mem::take(&mut self.style_name).trim().trim_end_matches(';').trim().to_string();
        let def = std::mem::take(&mut self.style_def_cur);
        if self.styles.len() < 4096 {
            self.styles.insert(self.style_cur, name);
            self.style_defs.insert(self.style_cur, def);
        }
        // The next entry is style 0 (Normal) unless it names a number.
        self.style_cur = 0;
    }

    /// The character formatting style `id` gives its runs in this file: its stylesheet entry's,
    /// then its `\sbasedon` ancestors' (a bounded walk: loops and missing ids end it).
    fn file_style(&self, id: i32) -> StyleDef {
        let mut out = StyleDef::default();
        let mut seen: Vec<i32> = Vec::new();
        let mut cur = Some(id);
        while let Some(i) = cur {
            if seen.len() >= 16 || seen.contains(&i) {
                break;
            }
            seen.push(i);
            let Some(d) = self.style_defs.get(&i) else { break };
            out.bold = out.bold.or(d.bold);
            out.italic = out.italic.or(d.italic);
            out.size = out.size.or(d.size);
            out.font = out.font.or(d.font);
            cur = d.based_on;
        }
        out
    }

    /// The formatting a paragraph of kind `kind` in style `id` gives its runs. A style whose
    /// entry and `\sbasedon` chain set no character formatting (a bare `{\s2 heading 2;}`) has
    /// WordCraft's own (`style_fmt`); otherwise what the chain doesn't set is RTF's default (not
    /// bold, not italic, 12 pt). The font alone falls back per property: WordCraft writes the code
    /// font on each code paragraph, not in its Code entry.
    fn style_base(&self, kind: Kind, id: i32) -> Fmt {
        let mut f = style_fmt(kind);
        let d = self.file_style(id);
        if d.bold.is_some() || d.italic.is_some() || d.size.is_some() || d.font.is_some() {
            f.bold = d.bold.unwrap_or(false);
            f.italic = d.italic.unwrap_or(false);
            f.size = Some(d.size.unwrap_or(12.0));
        }
        // A font is matched the way `\f` sets a run's: a monospaced one is the code font.
        if let Some(name) = d.font.and_then(|i| self.fonts.get(&i)) {
            f.code = model::is_mono(name);
            f.font = (!f.code).then(|| name.clone());
        }
        f
    }

    fn finish_pict(&mut self) {
        if self.pict_half.take().is_some() {
            // Odd hex digit count: drop the dangling nibble.
        }
        let data = std::mem::take(&mut self.pict_hex);
        if self.pict_kind == 0 || data.is_empty() {
            return;
        }
        let sx = if self.pict_sx > 0.0 { self.pict_sx / 100.0 } else { 1.0 };
        let sy = if self.pict_sy > 0.0 { self.pict_sy / 100.0 } else { 1.0 };
        let w = self.pict_w.map(|v| v * sx);
        let h = self.pict_h.map(|v| v * sy);
        if let Some(img) = make_img(data, w, h, "") {
            self.para.inlines.push(Inline::Image(img));
            self.para_has = true;
        }
    }

    fn text(&mut self, s: &str) {
        match self.st.dest {
            Dest::Body => {
                if self.st.hidden {
                    return;
                }
                let mut f = self.st.fmt.clone();
                let r = self.st.rev;
                if r.ins {
                    f.ins = Some(self.rev_mark(r.auth, r.dttm));
                }
                if r.del {
                    f.del = Some(self.rev_mark(r.authdel, r.dttmdel));
                }
                let c = &self.st.chg_c;
                if let Some(words) = c.old.clone() {
                    let mark = self.rev_mark(c.auth, c.date);
                    let old = self.old_fmt(&words);
                    f.fmt_change = Some(Box::new(model::FmtChange { mark, old }));
                }
                self.para.push_text(s, &f);
                self.para_has = true;
            }
            Dest::FontTbl => {
                for c in s.chars() {
                    if c == ';' {
                        self.commit_font();
                    } else {
                        self.font_name.push(c);
                    }
                }
            }
            Dest::ColorTbl => {
                for c in s.chars() {
                    if c == ';' {
                        let (r, g, b, set) = self.color_cur;
                        if self.colors.len() < 4096 {
                            self.colors.push(if set { Some(Rgb(r, g, b)) } else { None });
                        }
                        self.color_cur = (0, 0, 0, false);
                    }
                }
            }
            Dest::StyleSheet => {
                for c in s.chars() {
                    if c == ';' {
                        self.commit_style();
                    } else {
                        self.style_name.push(c);
                    }
                }
            }
            Dest::Info(_) => self.info_text.push_str(s),
            Dest::FldInst => {
                if let Some(f) = self.st.field.and_then(|i| self.fields.get_mut(i))
                    && f.len() < 4096
                {
                    f.push_str(s);
                }
            }
            Dest::Pict => {
                for b in s.bytes() {
                    let v = match b {
                        b'0'..=b'9' => b - b'0',
                        b'a'..=b'f' => b - b'a' + 10,
                        b'A'..=b'F' => b - b'A' + 10,
                        _ => continue,
                    };
                    match self.pict_half.take() {
                        Some(h) => {
                            if self.pict_hex.len() < MAX_PICT {
                                self.pict_hex.push((h << 4) | v);
                            }
                        }
                        None => self.pict_half = Some(v),
                    }
                }
            }
            Dest::Bookmark => {
                if let Some((_, n)) = &mut self.bkmk
                    && n.len() < 256
                {
                    n.push_str(s);
                }
            }
            Dest::RevTbl => {
                for c in s.chars() {
                    if c == ';' {
                        self.commit_author();
                    } else if self.rev_name.len() < 256 {
                        self.rev_name.push(c);
                    }
                }
            }
            Dest::Skip | Dest::ListTable | Dest::ListOverride | Dest::OldProps(_) => {}
        }
    }

    /// The formatting before a tracked character change: the current run's, with the
    /// `\oldcprops` words applied (a `\plain` among them starts from none).
    fn old_fmt(&mut self, words: &[(String, Option<i32>)]) -> Fmt {
        let fmt = Fmt { link: None, ins: None, del: None, fmt_change: None, ..self.st.fmt.clone() };
        let saved = std::mem::replace(&mut self.st, St { dest: Dest::OldProps(false), fmt, ..Default::default() });
        for (w, p) in words {
            self.word(w, *p, false, false);
        }
        let old = std::mem::replace(&mut self.st, saved).fmt;
        Fmt { link: None, ..old }
    }

    fn ch(&mut self, c: char) {
        if self.skip_chars > 0 {
            self.skip_chars -= 1;
            return;
        }
        self.finish_multibyte();
        let mut buf = [0u8; 4];
        self.text(c.encode_utf8(&mut buf));
    }

    /// The paragraph kind `st`'s style names, if it names one.
    fn style_kind(&self, st: &St) -> Option<Kind> {
        let name = self.styles.get(&st.style).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        if let Some(n) = name.strip_prefix("heading").and_then(|r| r.trim().parse::<u8>().ok()) {
            return Some(Kind::Heading(n.clamp(1, 6)));
        }
        if st.style != 0 {
            match name.as_str() {
                "title" => return Some(Kind::Title),
                "quote" | "intense quote" => return Some(Kind::Quote),
                "code" | "html preformatted" | "plain text" => return Some(Kind::Code),
                _ => {}
            }
        }
        None
    }

    fn para_kind(&self, st: &St) -> Kind {
        if let Some(k) = self.style_kind(st) {
            return k;
        }
        if let Some(o) = st.outline.filter(|o| *o < 9) {
            return Kind::Heading((o + 1).min(6));
        }
        Kind::Normal
    }

    /// The paragraph formatting of `st` (kind, alignment, page break, spacing, borders, list).
    fn para_props(&self, st: &St) -> Para {
        let mut p = Para::new(self.para_kind(st));
        p.align = st.align;
        p.page_break = st.pagebb;
        p.spacing = spacing(st);
        p.borders = st.borders.any_visible().then_some(st.borders);
        if let Some(ls) = st.ls {
            let list = self.overrides.get(&ls).and_then(|id| self.lists.get(id));
            let lv = st.ilvl.min(8);
            let ordered = list.and_then(|l| l.get(lv as usize).copied()).unwrap_or(false);
            p.list = Some(ListInfo { ordered, level: lv, start: 1 });
        }
        p
    }

    /// `\par` (or `\sect`): end the paragraph, whose mark is tracked as inserted or deleted when the text
    /// at it is.
    fn end_para_mark(&mut self) {
        let r = self.st.rev;
        self.para.mark_ins = r.ins.then(|| Box::new(self.rev_mark(r.auth, r.dttm)));
        self.para.mark_del = r.del.then(|| Box::new(self.rev_mark(r.authdel, r.dttmdel)));
        self.end_para(false);
    }

    /// End the current paragraph (`\par`, `\cell`).
    fn end_para(&mut self, in_cell: bool) {
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

    fn end_cell(&mut self) {
        self.end_para(true);
        let blocks = std::mem::take(&mut self.cell);
        if self.row.len() < 63 {
            self.row.push(Cell { blocks, ..Default::default() });
        }
    }

    fn end_row(&mut self) {
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

    fn close_table(&mut self) {
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

    fn word(&mut self, w: &str, p: Option<i32>, starred: bool, first_in_group: bool) {
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
                    self.st.fmt.link = link;
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

/// The character formatting the paragraph style of `kind` gives its runs, as the writer's
/// stylesheet and paragraph properties set it.
fn style_fmt(kind: Kind) -> Fmt {
    match kind {
        Kind::Heading(n) => Fmt { bold: true, size: HEAD_SIZES.get(usize::from(n.clamp(1, 6)) - 1).copied(), ..Default::default() },
        Kind::Title => Fmt { size: Some(28.0), ..Default::default() },
        Kind::Quote => Fmt { italic: true, ..Default::default() },
        Kind::Code => Fmt { code: true, size: Some(10.0), ..Default::default() },
        Kind::Normal | Kind::Rule => Fmt::default(),
    }
}

/// `f` with what it doesn't set directly taken from its paragraph style's formatting `base`.
fn styled(base: &Fmt, f: &Fmt) -> Fmt {
    let mut e = f.clone();
    e.bold |= base.bold;
    e.italic |= base.italic;
    if e.size.is_none() {
        e.size = base.size;
    }
    if !e.code && e.font.is_none() {
        e.code = base.code;
    }
    e
}

/// A `\stylesheet` entry's character formatting; `None` where the entry doesn't set it.
#[derive(Clone, Copy, Debug, Default)]
struct StyleDef {
    based_on: Option<i32>,
    bold: Option<bool>,
    italic: Option<bool>,
    size: Option<f32>,
    /// A `\fonttbl` index, named when it's used (the table may follow).
    font: Option<i32>,
}

/// Clear from `f` the formatting a paragraph of kind `kind` gets from its style, whose
/// formatting is `base`: a heading's or title's size and bold when they equal the style's, a
/// quote's italic, a code run's font when it's the style's (and, as before, its size and bold).
fn drop_style_formatting(kind: Kind, base: &Fmt, f: &mut Fmt) {
    match kind {
        Kind::Heading(_) | Kind::Title => {
            if f.size.is_some() && f.size == base.size {
                f.size = None;
            }
            if base.bold {
                f.bold = false;
            }
        }
        Kind::Code => {
            f.size = None;
            f.bold = false;
            let same_font = match (&f.font, &base.font) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (a, b) => a.is_none() && b.is_none(),
            };
            if f.code == base.code && same_font {
                f.code = false;
                f.font = None;
            }
        }
        Kind::Quote => {
            if base.italic {
                f.italic = false;
            }
        }
        Kind::Normal | Kind::Rule => {}
    }
}

/// A character or paragraph formatting word: what old formatting (`\oldcprops`,
/// `\oldpprops`) may hold.
fn is_format_word(w: &str) -> bool {
    matches!(
        w,
        "plain"
            | "b"
            | "i"
            | "ulnone"
            | "strike"
            | "striked"
            | "super"
            | "up"
            | "sub"
            | "dn"
            | "nosupersub"
            | "fs"
            | "f"
            | "cf"
            | "highlight"
            | "chcbpat"
            | "cb"
            | "v"
            | "pard"
            | "ql"
            | "qc"
            | "qr"
            | "qj"
            | "qd"
            | "s"
            | "outlinelevel"
            | "pagebb"
            | "intbl"
            | "ls"
            | "ilvl"
            | "sb"
            | "sa"
            | "sl"
            | "slmult"
            | "li"
            | "lin"
            | "ri"
            | "rin"
            | "fi"
            | "brdrw"
            | "brdrcf"
    ) || (w.starts_with("ul") && w.len() <= 8)
        || w.starts_with("brdr")
}

/// A character formatting word: what an `\oldcprops` group's words are replayed from.
fn is_char_format_word(w: &str) -> bool {
    matches!(
        w,
        "plain"
            | "b"
            | "i"
            | "strike"
            | "striked"
            | "super"
            | "up"
            | "sub"
            | "dn"
            | "nosupersub"
            | "fs"
            | "f"
            | "cf"
            | "highlight"
            | "chcbpat"
            | "cb"
            | "v"
    ) || (w.starts_with("ul") && w.len() <= 8)
}

/// The spacing and indents in points of the paragraph formatting `st`.
fn spacing(st: &St) -> model::Spacing {
    use wordcraft_doc::props::LineSpacing;
    let pt = |v: i32| (v as f32 / 20.0).clamp(-1584.0, 1584.0);
    let gap = |v: Option<i32>| v.map(|v| pt(v).max(0.0));
    let sl = st.sl;
    let line = if sl == 0 {
        None
    } else if sl < 0 {
        Some(LineSpacing::Exactly(pt(sl.saturating_neg()).max(1.0)))
    } else if st.slmult {
        // \sl240\slmult1 is single spacing.
        Some(LineSpacing::Multiple((sl as f32 / 240.0).clamp(0.06, 132.0)))
    } else {
        Some(LineSpacing::AtLeast(pt(sl)))
    };
    model::Spacing { before: gap(st.sb), after: gap(st.sa), line, left: st.li.map(pt), right: st.ri.map(pt), first: st.fi.map(pt) }
}

/// The target of a `HYPERLINK "url"` (or `HYPERLINK "url" \l "bookmark"`, `HYPERLINK \l "bookmark"`) field instruction.
fn hyperlink(instr: &str) -> Option<String> {
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

/// Parse RTF bytes into the flow model.
pub fn parse(bytes: &[u8]) -> Result<Flow, String> {
    parse_with_page(bytes).map(|(f, _, _)| f)
}

/// The flow, the page, and the default font (`\deff`, font 0 when the file names none), which
/// is the font of all text that doesn't select one.
fn parse_with_page(bytes: &[u8]) -> Result<(Flow, PageGeom, Option<String>), String> {
    parse_full(bytes).map(|r| (r.flow, r.page, r.default_font))
}

/// Everything an import takes from an RTF file.
struct Parsed {
    flow: Flow,
    page: PageGeom,
    /// The last section's properties (the others are on the paragraphs that end them).
    last_section: wordcraft_doc::SectionProps,
    default_font: Option<String>,
    /// Each section's headers and footers, in order, the last section's last.
    headers_footers: Vec<Vec<(HeaderFooter, Vec<FBlock>)>>,
}

fn parse_full(bytes: &[u8]) -> Result<Parsed, String> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    if !bytes.get(start..).is_some_and(|b| b.starts_with(b"{\\rtf")) {
        return Err("not an RTF document (it doesn't start with {\\rtf)".into());
    }
    let mut r = Reader::default();
    let mut i = start;
    let mut first_in_group = false;
    let mut starred = false;
    while let Some(&c) = bytes.get(i) {
        match c {
            b'{' => {
                r.open();
                first_in_group = true;
                starred = false;
                i += 1;
                continue;
            }
            b'}' => {
                r.close();
                if r.info_depth.is_some_and(|d| r.depth() < d) {
                    r.info_depth = None;
                }
                first_in_group = false;
                starred = false;
                i += 1;
                continue;
            }
            b'\\' => {
                let Some(&n) = bytes.get(i + 1) else { break };
                if n.is_ascii_alphabetic() {
                    let mut j = i + 1;
                    while bytes.get(j).is_some_and(u8::is_ascii_alphabetic) && j - i <= 32 {
                        j += 1;
                    }
                    let word = std::str::from_utf8(bytes.get(i + 1..j).unwrap_or(b"")).unwrap_or("");
                    let mut k = j;
                    let neg = bytes.get(k) == Some(&b'-');
                    if neg {
                        k += 1;
                    }
                    let d0 = k;
                    while bytes.get(k).is_some_and(u8::is_ascii_digit) && k - d0 < 10 {
                        k += 1;
                    }
                    let param = if k > d0 {
                        let v = std::str::from_utf8(bytes.get(d0..k).unwrap_or(b"0")).ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                        let v = if neg { -v } else { v };
                        // A date (DTTM) is a 32-bit pattern some writers put unsigned.
                        let v = if matches!(word, "revdttm" | "revdttmdel" | "crdate" | "prdate")
                            && v > i64::from(i32::MAX)
                            && v <= i64::from(u32::MAX)
                        {
                            v - (1i64 << 32)
                        } else {
                            v
                        };
                        Some(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
                    } else {
                        if neg {
                            k -= 1;
                        }
                        None
                    };
                    if bytes.get(k) == Some(&b' ') {
                        k += 1;
                    }
                    i = k;
                    if word == "bin" {
                        let n = param.unwrap_or(0).max(0) as usize;
                        let end = i.saturating_add(n).min(bytes.len());
                        if r.st.dest == Dest::Pict
                            && let Some(data) = bytes.get(i..end)
                            && r.pict_hex.len() + data.len() <= MAX_PICT
                        {
                            r.pict_hex.extend_from_slice(data);
                        }
                        i = end;
                        continue;
                    }
                    if word == "info" {
                        r.info_depth = Some(r.depth());
                    }
                    if r.skip_chars > 0
                        && matches!(r.st.dest, Dest::Body | Dest::Info(_) | Dest::FldInst | Dest::Bookmark | Dest::FontTbl)
                        && !matches!(word, "par" | "pard" | "cell" | "row")
                    {
                        r.skip_chars -= 1;
                        first_in_group = false;
                        continue;
                    }
                    r.word(word, param, starred, first_in_group);
                    first_in_group = false;
                    starred = false;
                    continue;
                }
                match n {
                    b'\'' => {
                        let h = bytes.get(i + 2..i + 4).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                        match h {
                            Some(v) => {
                                if r.st.dest == Dest::Pict {
                                    // Hex digits, two at a time, without a String per picture byte.
                                    const HEX: &[u8; 16] = b"0123456789abcdef";
                                    let h = [HEX[usize::from(v >> 4)], HEX[usize::from(v & 0x0f)]];
                                    if let Ok(s) = std::str::from_utf8(&h) {
                                        r.text(s);
                                    }
                                } else {
                                    r.byte(v);
                                }
                                i += 4;
                            }
                            None => i += 2,
                        }
                    }
                    b'*' => {
                        starred = true;
                        i += 2;
                        continue;
                    }
                    b'~' | b' ' => {
                        r.ch('\u{00A0}');
                        i += 2;
                    }
                    b'-' => {
                        r.ch('\u{00AD}');
                        i += 2;
                    }
                    b'_' => {
                        r.ch('\u{2011}');
                        i += 2;
                    }
                    b'\n' | b'\r' => {
                        r.word("par", None, false, false);
                        i += 2;
                    }
                    b'\\' | b'{' | b'}' => {
                        r.ch(n as char);
                        i += 2;
                    }
                    _ => i += 2,
                }
                first_in_group = false;
            }
            b'\r' | b'\n' | 0 => i += 1,
            _ => {
                // A run of plain text.
                let mut j = i;
                while bytes.get(j).is_some_and(|b| !matches!(b, b'{' | b'}' | b'\\' | b'\r' | b'\n' | 0)) {
                    j += 1;
                }
                let chunk = bytes.get(i..j).unwrap_or(b"");
                if r.st.dest == Dest::Pict && r.skip_chars == 0 && chunk.is_ascii() {
                    // Picture hex is not text: it needs no decoder, so neither a pending lead byte
                    // nor the code page's single-byte path applies to it.
                    if let Ok(s) = std::str::from_utf8(chunk) {
                        r.text(s);
                    }
                } else if r.skip_chars == 0 && chunk.is_ascii() && r.mb_decoder.is_none() {
                    // Plain ASCII with no multibyte run open: append it as it is.
                    if let Ok(s) = std::str::from_utf8(chunk) {
                        r.text(s);
                    }
                } else {
                    r.run(chunk);
                }
                i = j;
                first_in_group = false;
            }
        }
    }
    // A header or footer left open by a truncated file ends here, and so does a multibyte run.
    r.finish_multibyte();
    r.end_header_footer();
    if r.para_has || !r.para.inlines.is_empty() {
        r.end_para(false);
    }
    r.close_table();
    if let Some((y, mo, d, h, mi)) = r.created
        && y > 0
    {
        r.meta.created = format!("{y:04}-{:02}-{:02}T{:02}:{:02}:00Z", mo.clamp(1, 12), d.clamp(1, 31), h.min(23), mi.min(59));
    }
    let mut blocks = std::mem::take(&mut r.body);
    // A file that ends with `\sect` ends its last section there: no empty one follows, and
    // header or footer groups after that `\sect` (a section without text) are dropped.
    let mut last_section = r.sec.props(&r.page);
    let mut headers_footers = std::mem::take(&mut r.section_parts);
    if let Some(FBlock::Para(p)) = blocks.last_mut()
        && let Some(s) = p.section.take()
    {
        last_section = *s;
    } else {
        headers_footers.push(std::mem::take(&mut r.hf_parts));
    }
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
    }
    let default_font = r.fonts.get(&r.deff).filter(|n| !model::is_mono(n)).cloned();
    Ok(Parsed { flow: Flow { blocks, meta: r.meta }, page: r.page, last_section, default_font, headers_footers })
}

/// The column grid of a table from its rows' cell right edges: every distinct edge starts a grid
/// column, and each cell spans the grid columns between its left and right edges, as Word lays
/// out RTF rows whose cells don't line up. Returns the widths in points, or none (equal columns)
/// when the edges don't make a grid.
fn column_grid(rows: &mut [Vec<Cell>], rights: &[Vec<i32>]) -> Vec<f32> {
    let mut edges: Vec<i32> = rights.iter().flatten().copied().filter(|&x| x > 0).collect();
    edges.sort_unstable();
    edges.dedup();
    let ok = rights.len() == rows.len()
        && rows
            .iter()
            .zip(rights)
            .all(|(r, x)| r.len() == x.len() && x.windows(2).all(|w| w.first() < w.get(1)) && x.first().is_some_and(|&v| v > 0))
        && !edges.is_empty()
        && edges.len() <= 63;
    if !ok {
        return Vec::new();
    }
    for (row, x) in rows.iter_mut().zip(rights) {
        let mut left = 0;
        for (c, &right) in row.iter_mut().zip(x) {
            let span = edges.iter().filter(|&&e| e > left && e <= right).count().max(1);
            c.colspan = span as u32;
            left = right;
        }
    }
    let mut prev = 0;
    edges
        .iter()
        .map(|&e| {
            let w = (e - prev) as f32 / 20.0;
            prev = e;
            w
        })
        .collect()
}

/// Parse RTF bytes into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    let Parsed { flow, last_section, default_font, headers_footers, .. } = parse_full(bytes)?;
    let mut doc = model::to_doc_in(&flow, last_section);
    // Each section's own headers and footers; those it leaves out stay linked to the previous.
    let mut sets = Vec::with_capacity(headers_footers.len());
    for parts in &headers_footers {
        let (mut headers, mut footers) = (wordcraft_doc::section::HeaderSet::default(), wordcraft_doc::section::HeaderSet::default());
        for (which, blocks) in parts {
            let blocks = model::part_blocks(&mut doc, blocks);
            let id = doc.add_part(if which.footer { wordcraft_doc::PartKind::Footer } else { wordcraft_doc::PartKind::Header }, blocks);
            let set = if which.footer { &mut footers } else { &mut headers };
            match which.pages {
                HfPages::Default => set.default = Some(id),
                HfPages::First => set.first = Some(id),
                HfPages::Even => set.even = Some(id),
            }
        }
        sets.push((headers, footers));
    }
    let mut sets = sets.into_iter();
    for b in &mut doc.body {
        if let wordcraft_doc::Block::Para(p) = &**b
            && p.section.is_some()
            && let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b)
            && let Some(s) = p.section.as_deref_mut()
            && let Some((h, f)) = sets.next()
        {
            (s.headers, s.footers) = (h, f);
        }
    }
    if let Some((h, f)) = sets.next() {
        (doc.last_section.headers, doc.last_section.footers) = (h, f);
    }
    // RTF's paragraph defaults are single spacing and no space around paragraphs; what a file
    // wants beyond that it states on each paragraph.
    let dp = &mut doc.styles.default_para;
    dp.space_before = Some(0.0);
    dp.space_after = Some(0.0);
    dp.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
    if default_font.is_some() {
        doc.styles.default_chr.font = default_font;
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(f: &Flow) -> Vec<String> {
        f.blocks
            .iter()
            .map(|b| match b {
                FBlock::Para(p) => format!("{:?}:{}", p.kind, p.text()),
                FBlock::Table(t) => {
                    format!("table{}x{}:{}", t.rows.len(), t.cols(), t.rows.iter().flatten().map(|c| c.text()).collect::<Vec<_>>().join("|"))
                }
            })
            .collect()
    }

    #[test]
    fn basic_reader() {
        let rtf = br#"{\rtf1\ansi{\fonttbl{\f0 Arial;}{\f1 Courier New;}}{\colortbl;\red255\green0\blue0;}{\stylesheet{\s0 Normal;}{\s1 heading 1;}}
{\info{\title My Doc}{\author Ann}}
\pard\s1 Head\par
\pard Plain {\b bold} \i it\i0  {\cf1 red} caf\'e9 \u8364? and\line next\tab x\par
{\*\unknown skip me}{\field{\*\fldinst{HYPERLINK "http://a.b"}}{\fldrslt{link}}}\par
\trowd\cellx100\cellx200 \pard\intbl a\cell b\cell\row
\pard after\par}"#;
        let f = parse(rtf).unwrap();
        assert_eq!(f.meta.title, "My Doc");
        assert_eq!(f.meta.author, "Ann");
        let t = texts(&f);
        assert_eq!(t[0], "Heading(1):Head");
        assert_eq!(t[1], "Normal:Plain bold it red café € and\nnext\tx");
        assert_eq!(t[2], "Normal:link");
        assert_eq!(t[3], "table1x2:a|b");
        assert_eq!(t[4], "Normal:after");
        let FBlock::Para(p) = &f.blocks[1] else { panic!() };
        assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "bold" && f.bold)));
        assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "red" && f.color == Some(Rgb(255, 0, 0)))));
        let FBlock::Para(l) = &f.blocks[2] else { panic!() };
        assert!(matches!(&l.inlines[0], Inline::Text(_, f) if f.link.as_deref() == Some("http://a.b")));
    }

    #[test]
    fn paragraph_borders() {
        // A heading with a grey rule under it, as hh.ru résumés write their section titles.
        let rtf = br"{\rtf1\ansi{\colortbl;\red0\green0\blue0;\red216\green216\blue216;}
\pard\brdrb\brdrs\brdrw10\brdrcf2 Section\par
\pard Body\par
\pard\brdrt\brdrdb\brdrl\brdrs\brdrr\brdrs Boxed\par
\pard\brdrb\brdrs\par}";
        let f = parse(rtf).unwrap();
        let paras: Vec<&Para> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect();
        let b = paras[0].borders.unwrap();
        let bottom = b.bottom.unwrap();
        assert_eq!((bottom.width, bottom.color), (0.5, Some(Rgb(216, 216, 216))));
        assert!(b.top.is_none());
        assert_eq!(paras[0].kind, Kind::Normal);
        // \pard resets them.
        assert!(paras[1].borders.is_none());
        let b = paras[2].borders.unwrap();
        assert_eq!(b.top.map(|x| x.style), Some(wordcraft_doc::props::BorderStyle::Double));
        assert!(b.left.is_some() && b.right.is_some() && b.bottom.is_none());
        // An empty paragraph with only a bottom border is still a rule.
        assert_eq!(paras[3].kind, Kind::Rule);
        // The document's paragraph carries them.
        let d = import(rtf).unwrap();
        let first = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None }).unwrap();
        assert!(first.props.borders.and_then(|b| b.bottom).is_some());
        // Paragraph borders round-trip through export and import.
        let exported = export(&d);
        assert!(exported.contains("\\brdrb\\brdrs\\brdrw10\\brdrcf1"), "{exported}");
        let back = import(exported.as_bytes()).unwrap();
        let first = back.body.iter().find_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None }).unwrap();
        let b = first.props.borders.unwrap().bottom.unwrap();
        assert_eq!((b.style, b.width, b.color), (wordcraft_doc::props::BorderStyle::Single, 0.5, Some(Rgb(216, 216, 216))));
    }

    #[test]
    fn headers_and_footers() {
        let rtf = br"{\rtf1\ansi\titlepg
{\header \pard\qr Running head\par}
{\headerf \pard First head {\b bold}\par}
{\footer \pard Page \chpgn\par}
\pard Body text\par
\trowd\cellx2000\pard\intbl cell\cell\row
\pard after\par}";
        let d = import(rtf).unwrap();
        let s = &d.last_section;
        assert!(s.title_page);
        let text = |id: Option<u32>| {
            let p = id.and_then(|id| d.parts.get(&id)).unwrap();
            p.blocks
                .iter()
                .filter_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p.text.clone()) } else { None })
                .collect::<Vec<_>>()
                .join("|")
        };
        assert_eq!(text(s.headers.default), "Running head");
        assert_eq!(text(s.headers.first), "First head bold");
        assert!(text(s.footers.default).starts_with("Page"));
        assert!(s.footers.first.is_none() && s.headers.even.is_none());
        // The body keeps only its own text, and its table survives the header groups.
        let f = parse(rtf).unwrap();
        let t = texts(&f);
        assert_eq!(t, ["Normal:Body text", "table1x1:cell", "Normal:after"]);
        // Without \titlepg there is no separate first page.
        let d = import(br"{\rtf1\ansi{\headerf x\par} y\par}").unwrap();
        assert!(!d.last_section.title_page && d.last_section.headers.first.is_some());
        // An unterminated header still ends, and the body text after it isn't lost.
        assert!(import(br"{\rtf1\ansi{\header x").is_ok());
    }

    #[test]
    fn paragraph_spacing_and_indents() {
        use wordcraft_doc::props::LineSpacing;
        let rtf = br"{\rtf1\ansi
\pard\sb120\sa150\sl360\slmult1\li720\ri360\fi-360 a\par
\pard\sl-280 b\par
\pard\sl300\slmult0 c\par
\pard d\par}";
        let f = parse(rtf).unwrap();
        let sp: Vec<model::Spacing> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.spacing) } else { None }).collect();
        assert_eq!(sp[0].before, Some(6.0));
        assert_eq!(sp[0].after, Some(7.5));
        assert_eq!(sp[0].line, Some(LineSpacing::Multiple(1.5)));
        assert_eq!((sp[0].left, sp[0].right, sp[0].first), (Some(36.0), Some(18.0), Some(-18.0)));
        assert_eq!(sp[1].line, Some(LineSpacing::Exactly(14.0)));
        assert_eq!(sp[2].line, Some(LineSpacing::AtLeast(15.0)));
        // \pard resets them.
        assert_eq!(sp[3], model::Spacing::default());
        // The document's defaults are RTF's: single, no space after; the paragraph's own win.
        let d = import(rtf).unwrap();
        assert_eq!(d.styles.default_para.space_after, Some(0.0));
        assert_eq!(d.styles.default_para.line_spacing, Some(LineSpacing::Multiple(1.0)));
        let first = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None }).unwrap();
        assert_eq!(first.props.space_after, Some(7.5));
        assert_eq!(first.props.indent_first, Some(-18.0));
    }

    #[test]
    fn default_font_is_deff_or_font_zero() {
        // Text that never selects a font is in the default font: \deff, else font 0.
        let d = import(br"{\rtf1\ansi{\fonttbl{\f0\fnil Times New Roman Cyr;}{\f1 Arial;}}\plain x\par}").unwrap();
        assert_eq!(d.styles.default_chr.font.as_deref(), Some("Times New Roman Cyr"));
        let d = import(br"{\rtf1\ansi\deff1{\fonttbl{\f0 Times New Roman;}{\f1 Arial;}} x\par}").unwrap();
        assert_eq!(d.styles.default_chr.font.as_deref(), Some("Arial"));
        // No font table: our default stays.
        let d = import(br"{\rtf1\ansi x\par}").unwrap();
        assert_eq!(d.styles.default_chr.font, wordcraft_doc::StyleSheet::builtin().default_chr.font);
    }

    #[test]
    fn unicode_escapes_in_font_names() {
        // Ordinary fallback, hex fallback and \uc0 all give "Café Sans"; the fallback is not kept.
        for name in [r"Caf\u233? Sans", r"Caf\u233\'e9 Sans", r"\uc0 Caf\u233  Sans", r"\uc2 Caf\u233\'e9\'e9 Sans"] {
            let rtf = format!(r"{{\rtf1\ansi\deff0{{\fonttbl{{\f0 {name};}}{{\f1 \u923?ambda;}}}}\plain x {{\f1 y}}\par}}");
            let d = import(rtf.as_bytes()).unwrap();
            assert_eq!(d.styles.default_chr.font.as_deref(), Some("Café Sans"), "{name}");
            // A body run in a Unicode-named font keeps that name.
            let f = parse(rtf.as_bytes()).unwrap();
            let y_font = f.blocks.iter().find_map(|b| match b {
                FBlock::Para(p) => p.inlines.iter().find_map(|i| match i {
                    Inline::Text(t, fmt) if t.trim() == "y" => fmt.font.clone(),
                    _ => None,
                }),
                FBlock::Table(_) => None,
            });
            assert_eq!(y_font.as_deref(), Some("Λambda"), "{name}");
            // Our export writes font names as `\uN?`; reading it back gives the same name.
            let back = import(export(&d).as_bytes()).unwrap();
            assert_eq!(back.styles.default_chr.font.as_deref(), Some("Café Sans"), "{name}");
        }
    }

    #[test]
    fn page_size_and_margins() {
        let d = import(br"{\rtf1\ansi\paperw11906\paperh16838\margl1134\margr850\margt1134\margb1134 x\par}").unwrap();
        let s = &d.last_section;
        assert!((s.page_w - 595.3).abs() < 0.1 && (s.page_h - 841.9).abs() < 0.1, "{} x {}", s.page_w, s.page_h);
        assert!((s.margin_left - 56.7).abs() < 0.1 && (s.margin_right - 42.5).abs() < 0.1);
        assert!(!s.landscape);
        let d = import(br"{\rtf1\ansi\paperw11906\paperh16838\landscape x\par}").unwrap();
        assert!(d.last_section.landscape && d.last_section.page_w > d.last_section.page_h);
        let exported = export(&d);
        assert!(exported.contains("\\landscape"), "exported RTF must include \\landscape: {exported}");
        // Nonsense keeps Word's defaults.
        let d = import(br"{\rtf1\ansi\paperw0\margl99999 x\par}").unwrap();
        assert_eq!(d.last_section, wordcraft_doc::SectionProps::default());
    }

    #[test]
    fn table_grid_from_cell_edges_and_borders() {
        // Rows whose cells don't line up share one grid: edges 2000, 3000, 9000.
        let rtf = br"{\rtf1\ansi
\trowd\cellx2000\cellx9000 \pard\intbl a\cell b\cell\row
\trowd\cellx3000\cellx9000 \pard\intbl c\cell d\cell\row
\pard after\par}";
        let f = parse(rtf).unwrap();
        let FBlock::Table(t) = &f.blocks[0] else { panic!() };
        assert_eq!(t.widths, [100.0, 50.0, 300.0]);
        assert_eq!(t.rows[0].iter().map(|c| c.colspan).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(t.rows[1].iter().map(|c| c.colspan).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(t.cols(), 3);
        // RTF tables have no grid style: each cell draws the borders it names, nothing else.
        assert!(t.borderless);
        assert!(t.rows.iter().flatten().all(|c| c.borders.is_none()));
        // A rule under a cell (a letterhead's line): the bottom side only, its width and colour.
        let f =
            parse(br"{\rtf1\ansi{\colortbl;\red174\green174\blue174;}\trowd\clbrdrb\brdrs\brdrw15\brdrcf1\cellx2000\pard\intbl a\cell\row}").unwrap();
        let FBlock::Table(t) = &f.blocks[0] else { panic!() };
        let b = t.rows[0][0].borders.unwrap();
        assert!(b.top.is_none() && b.left.is_none() && b.right.is_none());
        let bottom = b.bottom.unwrap();
        assert_eq!((bottom.style, bottom.width, bottom.color), (wordcraft_doc::props::BorderStyle::Single, 0.75, Some(Rgb(174, 174, 174))));
        // Every side, in different styles, per cell.
        let f = parse(br"{\rtf1\ansi\trowd\clbrdrt\brdrdb\clbrdrl\brdrs\brdrw10\clbrdrb\brdrdot\clbrdrr\brdrth\cellx2000\cellx4000\pard\intbl a\cell b\cell\row}").unwrap();
        let FBlock::Table(t) = &f.blocks[0] else { panic!() };
        let b = t.rows[0][0].borders.unwrap();
        use wordcraft_doc::props::BorderStyle as S;
        assert_eq!(
            [b.top.map(|x| x.style), b.left.map(|x| x.style), b.bottom.map(|x| x.style), b.right.map(|x| x.style)],
            [Some(S::Double), Some(S::Single), Some(S::Dotted), Some(S::Thick)]
        );
        assert_eq!(b.left.map(|x| x.width), Some(0.5));
        assert!(t.rows[0][1].borders.is_none(), "the second cell names no borders");
        // A border side set to none isn't a border.
        let f = parse(br"{\rtf1\ansi\trowd\clbrdrl\brdrnone\cellx2000\pard\intbl a\cell\row}").unwrap();
        let FBlock::Table(t) = &f.blocks[0] else { panic!() };
        assert!(t.rows[0][0].borders.is_none());
        // The document's cells carry them.
        let d = import(br"{\rtf1\ansi\trowd\clbrdrb\brdrs\cellx2000\pard\intbl a\cell\row}").unwrap();
        let t = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
        assert!(t.props.style.is_none());
        assert!(t.rows[0].cells[0].props.borders.and_then(|b| b.bottom).is_some());
        // Borderless tables export without cell borders and round-trip borderless.
        let d = import(br"{\rtf1\ansi\trowd\cellx2000\pard\intbl a\cell\row}").unwrap();
        let exported = export(&d);
        assert!(!exported.contains("\\clbrdr"), "{exported}");
        let back = import(exported.as_bytes()).unwrap();
        let tb = back.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
        assert!(tb.props.style.is_none());
        assert!(tb.rows[0].cells[0].props.borders.is_none());

        // Cell borders round-trip through export and import.
        let d = import(br"{\rtf1\ansi{\colortbl;\red174\green174\blue174;}\trowd\clbrdrb\brdrdb\brdrw15\brdrcf1\cellx2000\pard\intbl a\cell\row}")
            .unwrap();
        let exported = export(&d);
        assert!(exported.contains("\\clbrdrb\\brdrdb\\brdrw15\\brdrcf1"), "{exported}");
        let back = import(exported.as_bytes()).unwrap();
        let tb = back.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
        let b = tb.rows[0].cells[0].props.borders.unwrap().bottom.unwrap();
        assert_eq!((b.style, b.width, b.color), (wordcraft_doc::props::BorderStyle::Double, 0.75, Some(Rgb(174, 174, 174))));

        // The table sizes to the document's page, not Letter.
        let d = import(br"{\rtf1\ansi\paperw11906\margl1134\margr1134\trowd\cellx4818\cellx9638\pard\intbl a\cell b\cell\row}").unwrap();
        let t = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
        assert_eq!(t.grid, [240.9, 241.0]);
    }

    #[test]
    fn code_pages_from_ansicpg_and_font_charsets() {
        // The document's code page: Russian RTF as accounting systems write it.
        let f = parse(br"{\rtf1\ansi\ansicpg1251\deff0{\fonttbl{\f0 Times New Roman;}}\f0 \'cf\'f0\'e8\'e2\'e5\'f2, \'b8\'eb\'ea\'e0 \'b9 5\par}")
            .unwrap();
        assert_eq!(texts(&f), ["Normal:Привет, ёлка № 5"]);
        // Font charsets switch inside a Windows-1252 document: Western, Cyrillic, Central
        // European, then back to the first font.
        let f = parse(
            br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset204 Arial CYR;}{\f2\fcharset238 Arial CE;}}\f0 caf\'e9 {\f1 \'c4\'e0} \f2 \'a3\'f3d\'9f \f0 na\'efve\par}",
        )
        .unwrap();
        assert_eq!(texts(&f), ["Normal:café Да Łódź naïve"]);
        // The default font's charset applies before any \f; an explicit \cpg wins over \fcharset.
        let f = parse(br"{\rtf1\ansi\deff1{\fonttbl{\f1\fcharset204 Times;}{\f2\fcharset0\cpg1253 Greek;}}\'cc\'e8\'f0 \f2 \'c1\'e2\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:Мир Αβ"]);
        // Raw 8-bit text (not escaped) follows the same code page.
        let f = parse(b"{\\rtf1\\ansi\\ansicpg1251 \xc4\xee\xec\\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:Дом"]);
        // #1653: a Macintosh document (LibreOffice's fdo85889-mac.rtf) is Mac OS Roman, and so is
        // a font with the Mac charset in a Windows document.
        let f = parse(br"{\rtf1\mac \deff0{\fonttbl{\f0\fmodern Helvetica;}}\pard\f0\fs20 \'f1\'f2\'f3\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:ÒÚÛ"]);
        let f = parse(br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset77 Geneva;}}caf\'e9 {\f1 caf\'8e \'d2ok\'d3}\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:café café “ok”"]);
        // \fcharset255 is the standard OEM character set (CP437).
        let f = parse(br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset255 Terminal;}}caf\'e9 {\f1 caf\'82}\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:café café"]);
        // DOS documents: `\pc` is code page 437, `\pca` 850; a font can name one with `\cpg`.
        let f = parse(br"{\rtf1\pc\deff0{\fonttbl{\f0 Courier;}}caf\'82 \'a0 la carte\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:café á la carte"]);
        let f = parse(br"{\rtf1\pca\deff0{\fonttbl{\f0 Courier;}{\f1\cpg437 Courier;}}\'9d {\f1 \'9d}\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:Ø ¥"]);
        // A code page number out of range is ignored, not truncated to another one.
        let f = parse(br"{\rtf1\ansi\ansicpg65973 caf\'e9\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:café"]);
        // The OEM charset (255) is DOS code page 437, like 254.
        let f = parse(br"{\rtf1\ansi\deff0{\fonttbl{\f0\fcharset255 Courier New;}}\f0 caf\'82\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:café"]);
    }

    /// #1653: the East Asian code pages (932 Shift-JIS, 936 GBK, 949 UHC, 950 Big5) decode their
    /// bytes as a run, so a lead byte and its trail byte decode together, escaped or raw.
    #[test]
    fn east_asian_multibyte_code_pages() {
        // Japanese `\ansicpg932` (CP932: 日 = 93 fa, 本 = 96 7b, 語 = 8c ea).
        let f = parse(br"{\rtf1\ansi\ansicpg932 \'93\'fa\'96\'7b\'8c\'ea\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:日本語"]);
        // Raw 8-bit text (not `\'hh`), and raw bytes mixed with one escaped byte: RTF can't carry a
        // raw `{`, `}` or `\`, so 本's trail byte 7b is written `\'7b` here.
        let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x8a\xbf\x8e\x9a\\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:漢字"]);
        let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\xfa\x96\\'7b\x8c\xea\\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:日本語"]);
        // A font names the code page with `\fcharset128` or with `\cpg932`, and `\f0` selects it.
        let f = parse(br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fcharset128 MS Mincho;}}\f0 \'93\'fa\'96\'7b\'8c\'ea\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:日本語"]);
        let f = parse(br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\cpg932 MS Mincho;}}\f0 \'93\'fa\'96\'7b\'8c\'ea\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:日本語"]);
        // Raw 8-bit bytes in the font table (the font's name) decode in the document's code page.
        let d = import(b"{\\rtf1\\ansi\\ansicpg932\\deff0{\\fonttbl{\\f0\\fcharset128 \x96\xbe\x92\xa9;}}\\f0 x\\par}").unwrap();
        assert_eq!(d.styles.default_chr.font.as_deref(), Some("明朝"));
        // Korean `\ansicpg949` (UHC: 한국어 = c7 d1 b1 b9 be ee).
        let f = parse(br"{\rtf1\ansi\ansicpg949 \'c7\'d1\'b1\'b9\'be\'ee\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:한국어"]);
        // Simplified Chinese `\ansicpg936` (GBK: 中文 = d6 d0 ce c4).
        let f = parse(br"{\rtf1\ansi\ansicpg936 \'d6\'d0\'ce\'c4\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:中文"]);
        // Traditional Chinese `\ansicpg950` (Big5: 中文 = a4 a4 a4 e5).
        let f = parse(br"{\rtf1\ansi\ansicpg950 \'a4\'a4\'a4\'e5\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:中文"]);
        // `\uN?` escapes still decode alongside multibyte text (`\u26412?` is 本, its `?` fallback
        // skipped by `\uc1`).
        let f = parse(br"{\rtf1\ansi\ansicpg932 \'93\'fa\u26412?\'8c\'ea x\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:日本語 x"]);
        // An invalid sequence decodes as U+FFFD, as Word writes it, and doesn't lose the rest.
        let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\\par after\\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:\u{FFFD}", "Normal:after"]);
        let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\xfa\x93 x\x96\x8c\\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:日\u{FFFD} x膜"]);
    }

    /// The decoder never sniffs a BOM: the code page is named by `\ansicpg`, `\cpg` or `\fcharset`,
    /// so a run that starts `EF BB BF` is a character pair and a lead byte, not a UTF-8 BOM (which
    /// would switch the rest of the document to UTF-8 and garble it).
    #[test]
    fn a_bom_like_byte_triple_is_text_in_a_multibyte_code_page() {
        // GBK EF BB = 锘, then BF B5 = 康; UHC EF BB = 癤, then BF C0 = 오; Big5 EF BB = 嚜, then
        // BF 40 = 濃.
        let f = parse(br"{\rtf1\ansi\ansicpg936 \'ef\'bb\'bf\'b5\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:锘康"]);
        let f = parse(br"{\rtf1\ansi\ansicpg949 \'ef\'bb\'bf\'c0\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:癤오"]);
        let f = parse(br"{\rtf1\ansi\ansicpg950 \'ef\'bb\'bf\'40\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:嚜濃"]);
    }

    /// A multibyte run longer than any fixed buffer keeps every character: the streaming decoder
    /// holds a lead byte and its trail byte together across decode steps instead of cutting the
    /// character at a buffer size (the review's finding on the byte-capped flush).
    #[test]
    fn a_long_multibyte_run_never_splits_a_character() {
        let mut bytes = b"{\\rtf1\\ansi\\ansicpg932 ".to_vec();
        bytes.push(b'A');
        // 2048 × あ (CP932 82 a0) is 4096 bytes after the one ASCII byte: a fixed 4096-byte buffer
        // would flush with the last あ's lead byte in one step and its trail byte in the next.
        for _ in 0..2048 {
            bytes.extend_from_slice(&[0x82, 0xA0]);
        }
        bytes.extend_from_slice(b"\\par}");
        let f = parse(&bytes).unwrap();
        let expected: String = std::iter::once('A').chain((0..2048).map(|_| 'あ')).collect();
        assert_eq!(texts(&f), [format!("Normal:{expected}")]);
    }

    /// A lead byte with no trail byte at the end of the document becomes U+FFFD, as Word writes a
    /// malformed sequence — the streaming decoder is finalised, not left holding it.
    #[test]
    fn an_incomplete_lead_byte_at_the_end_is_replacement() {
        let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93}").unwrap();
        assert_eq!(texts(&f), ["Normal:\u{FFFD}"]);
    }

    /// #1653: a code page switch (`\f`, `\fcharset`, `\cpg`, `\ansicpg`), a group end and the end
    /// of the document decode the multibyte bytes read so far, before the new code page applies.
    #[test]
    fn multibyte_code_page_switches_flush_pending_bytes() {
        // Japanese (932) → ASCII → Cyrillic (`\fcharset204` with `\cpg1251`): each run decodes in
        // its own code page, the last bytes before the switch included.
        let f = parse(
            br"{\rtf1\ansi\ansicpg932\deff0{\fonttbl{\f0 MS Mincho;}{\f1\fcharset204\cpg1251 Arial CYR;}}\f0 \'93\'fa\'96\'7b\'8c\'ea mixed \f1 \'cf\'f0\'e8\'e2\'e5\'f2\par}",
        )
        .unwrap();
        assert_eq!(texts(&f), ["Normal:日本語 mixed Привет"]);
        // A group end flushes: the group's 932 bytes decode before the outer 1252 text.
        let f =
            parse(br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset128 MS Mincho;}}\f0 {\f1 \'93\'fa\'96\'7b\'8c\'ea}caf\'e9\par}")
                .unwrap();
        assert_eq!(texts(&f), ["Normal:日本語café"]);
        // An `\ansicpg` switch mid-document flushes too.
        let f = parse(br"{\rtf1\ansi\ansicpg932 \'93\'fa\ansicpg1251 \'cf\'f0\'e8\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:日При"]);
        // A run left open at the end of the document isn't lost.
        let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\xfa").unwrap();
        assert_eq!(texts(&f), ["Normal:日"]);
    }

    /// A Windows-1252 document (the default) reads exactly as before: high bytes one at a time.
    #[test]
    fn windows_1252_text_is_unchanged() {
        let f = parse(b"{\\rtf1\\ansi\\ansicpg1252 caf\xe9 na\xefve \x93quoted\x94 \\'80 5\\'b0\\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:café naïve “quoted” € 5°"]);
        // No `\ansicpg`: the default is 1252; the byte values of an East Asian code page keep
        // their 1252 meaning.
        let f = parse(b"{\\rtf1\\ansi \xe9\\'92\\'93 \\'fa\\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:é’“ ú"]);
    }

    #[test]
    fn shppict_imports_one_image_and_skips_legacy_duplicate() -> Result<(), String> {
        let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([40, 80, 160, 255]));
        let mut png = Vec::new();
        pixels.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let hex: String = png.iter().map(|b| format!("{b:02x}")).collect();
        let pict = format!("{{\\pict\\pngblip\\picwgoal1440\\pichgoal720 {hex}}}");
        // Also use a supported PNG as the legacy duplicate: it must still be skipped.
        for legacy in [format!("{{\\pict\\wmetafile8 {hex}}}"), pict.clone()] {
            for picture in [format!("{{\\*\\shppict{pict}}}"), pict.clone()] {
                let f = parse(format!("{{\\rtf1\\ansi Before{picture}{{\\nonshppict{legacy}}}After\\par}}").as_bytes())?;
                let images: Vec<_> = f
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        FBlock::Para(p) => Some(&p.inlines),
                        FBlock::Table(_) => None,
                    })
                    .flatten()
                    .filter_map(|i| match i {
                        Inline::Image(img) => Some(img),
                        _ => None,
                    })
                    .collect();
                assert_eq!(images.len(), 1, "{picture}");
                let img = images.first().ok_or("missing picture")?;
                assert_eq!(img.data.as_ref(), &png);
                assert_eq!(img.ext, "png");
                assert_eq!((img.w, img.h), (72.0, 36.0));
                assert_eq!(texts(&f), vec!["Normal:BeforeAfter"]);
            }
        }
        // Recognizing shppict must not resurrect pictures inside skipped destinations.
        for dest in ["nonshppict", "unknown"] {
            let f = parse(format!("{{\\rtf1{{\\*\\{dest}{{\\*\\shppict{pict}}}}}Before\\par}}").as_bytes())?;
            assert_eq!(texts(&f), vec!["Normal:Before"]);
            assert!(f.blocks.iter().all(|b| match b {
                FBlock::Para(p) => p.inlines.iter().all(|i| !matches!(i, Inline::Image(_))),
                FBlock::Table(_) => false,
            }));
        }
        Ok(())
    }

    #[test]
    fn unicode_in_info_bookmarks_and_field_instructions() {
        // #435: `\uN` (with its `\ucN` fallback skipped) decodes in every text destination.
        let rtf = br#"{\rtf1\ansi\uc1{\info{\title Caf\u233?}{\author Caf\u233\'3f}}{\*\bkmkstart caf\u233?}{\*\bkmkend caf\u233?}{\field{\*\fldinst{HYPERLINK "https://example.test/caf\u233?"}}{\fldrslt{link}}}\par Caf\u233?\par}"#;
        let f = parse(rtf).unwrap();
        assert_eq!(f.meta.title, "Café");
        assert_eq!(f.meta.author, "Café");
        let inl: Vec<&Inline> = f.blocks.iter().flat_map(|b| if let FBlock::Para(p) = b { p.inlines.iter().collect() } else { vec![] }).collect();
        assert!(inl.iter().any(|i| matches!(i, Inline::Anchor(n) if n == "café")), "{inl:?}");
        assert!(
            inl.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "link" && f.link.as_deref() == Some("https://example.test/café"))),
            "{inl:?}"
        );
        assert_eq!(texts(&f).last().map(String::as_str), Some("Normal:Café"));
    }

    /// A hand-written RTF with tracked changes: Alice inserts "added ", Bob deletes "gone", and
    /// "both" is inserted by Alice then deleted by Bob.
    pub(crate) const TRACKED: &[u8] = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}{\*\revtbl {Unknown;}{Alice;}{Bob;}}
\pard Keep {\revised\revauth1\revdttm-1480361314 added }text{\deleted\revauthdel2\revdttmdel-2016341499  gone} end{\revised\revauth1\revdttm-1480361314\deleted\revauthdel2\revdttmdel-2016341499  both}.\par}";

    /// (text, insertion (author, date), deletion (author, date)) of each run of the first paragraph.
    #[allow(clippy::type_complexity)]
    fn tracked_runs(doc: &Document) -> Vec<(String, Option<(String, String)>, Option<(String, String)>)> {
        let rev = |i: Option<u32>, kind: wordcraft_doc::RevisionKind| {
            i.map(|i| {
                let r = doc.revisions.get(i as usize).unwrap();
                assert_eq!(r.kind, kind);
                (r.author.clone(), r.date.clone())
            })
        };
        let Some(wordcraft_doc::Block::Para(p)) = doc.body.first().map(|b| &**b) else { panic!("no paragraph") };
        p.run_ranges()
            .map(|(r, cp)| {
                (p.text[r].to_string(), rev(cp.ins, wordcraft_doc::RevisionKind::Insert), rev(cp.del, wordcraft_doc::RevisionKind::Delete))
            })
            .collect()
    }

    fn expected_tracked() -> Vec<(String, Option<(String, String)>, Option<(String, String)>)> {
        let a = Some(("Alice".to_string(), "2024-03-15T10:30:00Z".to_string()));
        let b = Some(("Bob".to_string(), "2025-01-02T08:05:00Z".to_string()));
        vec![
            ("Keep ".into(), None, None),
            ("added ".into(), a.clone(), None),
            ("text".into(), None, None),
            (" gone".into(), None, b.clone()),
            (" end".into(), None, None),
            (" both".into(), a, b),
            (".".into(), None, None),
        ]
    }

    #[test]
    fn tracked_changes_import_with_authors_and_dates() {
        let doc = import(TRACKED).unwrap();
        assert_eq!(tracked_runs(&doc), expected_tracked());
        // One revision per (kind, author, date).
        assert_eq!(doc.revisions.len(), 2);
    }

    #[test]
    fn tracked_changes_survive_export_and_import() {
        let doc = import(TRACKED).unwrap();
        let rtf = export(&doc);
        assert!(rtf.contains("{\\*\\revtbl {Unknown;}{Alice;}{Bob;}}"), "{rtf}");
        assert!(rtf.contains("\\deleted\\revauthdel2\\revdttmdel-2016341499"), "{rtf}");
        let back = import(rtf.as_bytes()).unwrap();
        assert_eq!(tracked_runs(&back), expected_tracked());
    }

    #[test]
    fn plain_text_export_still_drops_deleted_text() {
        // Only RTF keeps the deletions: the shared flow for other formats leaves them out.
        let doc = import(TRACKED).unwrap();
        let flow = model::from_doc(&doc);
        assert_eq!(texts(&flow), vec!["Normal:Keep added text end."]);
    }

    #[test]
    fn dttm_converts_to_and_from_iso() {
        assert_eq!(dttm_to_iso(-1480361314i32 as u32).as_deref(), Some("2024-03-15T10:30:00Z"));
        assert_eq!(dttm_to_iso(-2016341499i32 as u32).as_deref(), Some("2025-01-02T08:05:00Z"));
        assert_eq!(iso_to_dttm("2024-03-15T10:30:00Z"), Some(-1480361314i32 as u32));
        assert_eq!(iso_to_dttm("2025-01-02T08:05:59.123+02:00"), Some(-2016341499i32 as u32));
        assert_eq!(iso_to_dttm("2000-02-29T00:00:00Z").and_then(dttm_to_iso).as_deref(), Some("2000-02-29T00:00:00Z"));
        // Zero, out-of-range fields and impossible days are no date.
        assert_eq!(dttm_to_iso(0), None);
        assert_eq!(dttm_to_iso(13 << 16 | 1 << 11 | 100 << 20), None);
        assert_eq!(dttm_to_iso(2 << 16 | 30 << 11 | 101 << 20), None);
        assert_eq!(dttm_to_iso(1 << 16 | 1 << 11 | 100 << 20 | 60), None);
        assert_eq!(iso_to_dttm(""), None);
        assert_eq!(iso_to_dttm("1899-12-31T00:00:00Z"), None);
        assert_eq!(iso_to_dttm("2024-02-30T00:00:00Z"), None);
        assert_eq!(iso_to_dttm("2024-1é-01T00:00:00Z"), None);
        assert_eq!(iso_to_dttm("2024-03-15"), iso_to_dttm("2024-03-15T00:00:00Z"));
    }

    #[test]
    fn hostile_revision_tables_and_indices() {
        // An author index past the table, a negative one, and a malformed table.
        let rtf = br"{\rtf1{\*\revtbl {Unknown;}{Ann}{;}{\b;}}{\revised\revauth99999999 a}{\deleted\revauthdel-5\revdttmdel4294967295 b}{\revised\revauth1 c}\par}";
        let doc = import(rtf).unwrap();
        let runs = tracked_runs(&doc);
        assert_eq!(runs.first().and_then(|r| r.1.clone()), Some(("Unknown".to_string(), String::new())));
        assert_eq!(runs.get(1).and_then(|r| r.2.clone()), Some(("Unknown".to_string(), String::new())));
        assert_eq!(runs.get(2).and_then(|r| r.1.clone()), Some(("Ann".to_string(), String::new())));
        // A huge table is capped, and indices past the cap are "Unknown".
        let mut rtf = String::from("{\\rtf1{\\*\\revtbl ");
        for i in 0..20_000 {
            rtf.push_str(&format!("{{A{i};}}"));
        }
        rtf.push_str("}{\\revised\\revauth15000 x}{\\revised\\revauth9999 y}\\par}");
        let doc = import(rtf.as_bytes()).unwrap();
        let runs = tracked_runs(&doc);
        assert_eq!(runs.first().and_then(|r| r.1.clone()).map(|a| a.0).as_deref(), Some("Unknown"));
        assert_eq!(runs.get(1).and_then(|r| r.1.clone()).map(|a| a.0).as_deref(), Some("A9999"));
        // Unterminated table and revision words outside the body never panic.
        for bad in [&br"{\rtf1{\*\revtbl {Al"[..], br"{\rtf1{\*\revtbl \revised\deleted{x}}y\par}", br"{\rtf1\revauth\revdttm\deleted"] {
            let _ = import(bad);
        }
    }

    #[test]
    fn unicode_author_names_survive_import_and_round_trip() {
        // `\uN` in the author table: "José" and "Дмитрий" (not "Jos?" / "???????").
        let rtf = br"{\rtf1\uc1{\*\revtbl {Unknown;}{Jos\u233?;}{\u1044?\u1084?\u1080?\u1090?\u1088?\u1080?\u1081?;}}{\revised\revauth1 a}{\deleted\revauthdel2 b}\par}";
        let authors = |d: &Document| tracked_runs(d).into_iter().map(|r| (r.1.map(|a| a.0), r.2.map(|a| a.0))).collect::<Vec<_>>();
        let want = vec![(Some("José".to_string()), None), (None, Some("Дмитрий".to_string()))];
        let doc = import(rtf).unwrap();
        assert_eq!(authors(&doc), want);
        let back = import(export(&doc).as_bytes()).unwrap();
        assert_eq!(authors(&back), want);
    }

    #[test]
    fn pard_keeps_revision_marks() {
        // Revision marks are character formatting: `\pard` resets paragraph formatting only.
        let rtf = br"{\rtf1{\*\revtbl {Unknown;}{Bob;}}\deleted\revauthdel1 gone\par\pard more\par}";
        let doc = import(rtf).unwrap();
        let Some(wordcraft_doc::Block::Para(p)) = doc.body.get(1).map(|b| &**b) else { panic!("no second paragraph") };
        assert_eq!(p.text, "more");
        let del = p.props_of_char(0).del.and_then(|i| doc.revisions.get(i as usize));
        assert_eq!(del.map(|r| (r.kind, r.author.as_str())), Some((wordcraft_doc::RevisionKind::Delete, "Bob")));
    }

    /// The body's paragraphs.
    fn doc_paras(doc: &Document) -> Vec<&wordcraft_doc::Paragraph> {
        doc.body
            .iter()
            .filter_map(|b| match &**b {
                wordcraft_doc::Block::Para(p) => Some(p),
                _ => None,
            })
            .collect()
    }

    /// A document of `paras` with `revisions`.
    fn doc_with(paras: Vec<wordcraft_doc::Paragraph>, revisions: Vec<wordcraft_doc::Revision>) -> Document {
        let mut doc = Document::new();
        doc.body = paras.into_iter().map(wordcraft_doc::para_block).collect();
        doc.revisions = revisions;
        doc
    }

    /// Kind, author and date of revision `i`.
    fn rev_of(doc: &Document, i: Option<u32>) -> Option<(wordcraft_doc::RevisionKind, String, String)> {
        i.and_then(|i| doc.revisions.get(i as usize)).map(|r| (r.kind, r.author.clone(), r.date.clone()))
    }

    #[test]
    fn character_formatting_change_round_trips() {
        use wordcraft_doc::props::{CharProps, PropChange};
        use wordcraft_doc::{Revision, RevisionKind};
        // "Bold" was made bold by Alice; before, it was plain italic.
        let old = CharProps { italic: Some(true), ..Default::default() };
        let cp = CharProps { bold: Some(true), fmt_change: PropChange::boxed(0, old), ..Default::default() };
        let doc = doc_with(
            vec![wordcraft_doc::Paragraph::with_text("Bold", cp)],
            vec![Revision::new(RevisionKind::Format, "Alice", "2024-03-15T10:30:00Z")],
        );
        let rtf = export(&doc);
        assert!(rtf.contains("\\crauth1\\crdate-1480361314"), "{rtf}");
        assert!(rtf.contains("{\\*\\oldcprops"), "{rtf}");
        let back = import(rtf.as_bytes()).unwrap();
        let paras = doc_paras(&back);
        let p = paras.first().unwrap();
        assert_eq!(p.text, "Bold");
        let cp = p.props_of_char(0);
        assert_eq!(cp.bold, Some(true));
        let change = cp.fmt_change.as_deref().expect("formatting change read back");
        assert_eq!(rev_of(&back, Some(change.rev)), Some((RevisionKind::Format, "Alice".into(), "2024-03-15T10:30:00Z".into())));
        assert_eq!(change.old.italic, Some(true));
        assert_ne!(change.old.bold, Some(true));
    }

    #[test]
    fn paragraph_formatting_change_round_trips() {
        use wordcraft_doc::props::{CharProps, ParaProps, PropChange};
        use wordcraft_doc::{Align, Revision, RevisionKind};
        // Bob centred a right-aligned paragraph.
        let mut p = wordcraft_doc::Paragraph::with_text("Centred", CharProps::default());
        p.props.align = Some(Align::Center);
        p.props.fmt_change = PropChange::boxed(0, ParaProps { align: Some(Align::Right), ..Default::default() });
        let doc = doc_with(vec![p], vec![Revision::new(RevisionKind::Format, "Bob", "2025-01-02T08:05:00Z")]);
        let rtf = export(&doc);
        assert!(rtf.contains("{\\*\\oldpprops"), "{rtf}");
        assert!(rtf.contains("\\prauth1\\prdate-2016341499"), "{rtf}");
        let back = import(rtf.as_bytes()).unwrap();
        let paras = doc_paras(&back);
        let p = paras.first().unwrap();
        assert_eq!(p.text, "Centred");
        assert_eq!(p.props.align, Some(Align::Center));
        let change = p.props.fmt_change.as_deref().expect("paragraph formatting change read back");
        assert_eq!(rev_of(&back, Some(change.rev)), Some((RevisionKind::Format, "Bob".into(), "2025-01-02T08:05:00Z".into())));
        assert_eq!(change.old.align, Some(Align::Right));
    }

    #[test]
    fn paragraph_mark_revisions_round_trip() {
        use wordcraft_doc::props::CharProps;
        use wordcraft_doc::{Revision, RevisionKind};
        // Bob deleted the mark after "One" (joining it to "Two"); Alice inserted the one after "Two".
        let mut one = wordcraft_doc::Paragraph::with_text("One", CharProps::default());
        one.mark.del = Some(0);
        let mut two = wordcraft_doc::Paragraph::with_text("Two", CharProps::default());
        two.mark.ins = Some(1);
        let three = wordcraft_doc::Paragraph::with_text("Three", CharProps::default());
        let doc = doc_with(
            vec![one, two, three],
            vec![
                Revision::new(RevisionKind::Delete, "Bob", "2025-01-02T08:05:00Z"),
                Revision::new(RevisionKind::Insert, "Alice", "2024-03-15T10:30:00Z"),
            ],
        );
        let rtf = export(&doc);
        let back = import(rtf.as_bytes()).unwrap();
        let paras = doc_paras(&back);
        let texts: Vec<&str> = paras.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts.get(..3), Some(&["One", "Two", "Three"][..]), "{rtf}");
        let marks: Vec<_> = paras.iter().take(3).map(|p| (rev_of(&back, p.mark.ins), rev_of(&back, p.mark.del))).collect();
        assert_eq!(
            marks,
            vec![
                (None, Some((RevisionKind::Delete, "Bob".into(), "2025-01-02T08:05:00Z".into()))),
                (Some((RevisionKind::Insert, "Alice".into(), "2024-03-15T10:30:00Z".into())), None),
                (None, None),
            ],
            "{rtf}"
        );
    }

    #[test]
    fn formatting_revisions_read_as_the_spec_writes_them() {
        use wordcraft_doc::{Align, RevisionKind};
        // RTF 1.9.1 "Revision Marks": `\crauth`/`\crdate` with `{\*\oldcprops …}` on a run,
        // `\prauth`/`\prdate` with `{\*\oldpprops …}` on a paragraph, and a `\par` inside a
        // `\revised` group.
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\revtbl {Unknown;}{Alice;}{Bob;}}
\pard\qc\prauth2\prdate-2016341499{\*\oldpprops \pard\ql}Plain {\b\crauth1\crdate-1480361314{\*\oldcprops \plain\i}bold}\par
\pard Next{\revised\revauth1\revdttm-1480361314 \par}After\par}";
        let doc = import(rtf).unwrap();
        let paras = doc_paras(&doc);
        let texts: Vec<&str> = paras.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts.get(..3), Some(&["Plain bold", "Next", "After"][..]));
        let p = paras.first().unwrap();
        assert_eq!(p.props.align, Some(Align::Center));
        let pc = p.props.fmt_change.as_deref().expect("paragraph change");
        assert_eq!(rev_of(&doc, Some(pc.rev)), Some((RevisionKind::Format, "Bob".into(), "2025-01-02T08:05:00Z".into())));
        assert_eq!(pc.old.align, Some(Align::Left));
        assert!(p.props_of_char(0).fmt_change.is_none());
        let cp = p.props_of_char(6);
        assert_eq!(cp.bold, Some(true));
        let cc = cp.fmt_change.as_deref().expect("character change");
        assert_eq!(rev_of(&doc, Some(cc.rev)), Some((RevisionKind::Format, "Alice".into(), "2024-03-15T10:30:00Z".into())));
        assert_eq!(cc.old.italic, Some(true));
        assert_ne!(cc.old.bold, Some(true));
        let next = paras.get(1).unwrap();
        assert_eq!(rev_of(&doc, next.mark.ins), Some((RevisionKind::Insert, "Alice".into(), "2024-03-15T10:30:00Z".into())));
        assert!(paras.first().unwrap().mark.ins.is_none());
        assert!(paras.get(2).unwrap().mark.ins.is_none());
    }

    #[test]
    fn list_item_with_a_formatting_change_keeps_its_list_on_reject() {
        use model::{FmtChange, ListInfo};
        // A numbered item Alice centred (it was left-aligned), and a second plain item.
        let li = Some(ListInfo { ordered: true, level: 0, start: 1 });
        let mark = RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
        let old = Para { list: li, align: Some(Align::Left), ..Default::default() };
        let mut item = Para { list: li, align: Some(Align::Center), fmt_change: Some(Box::new(FmtChange { mark, old })), ..Default::default() };
        item.push_text("Item", &Fmt::default());
        let mut next = Para { list: li, ..Default::default() };
        next.push_text("Next", &Fmt::default());
        let flow = Flow { blocks: vec![FBlock::Para(item), FBlock::Para(next)], meta: Meta::default() };
        let check = |doc: &Document, what: &str| {
            let paras = doc_paras(doc);
            let p = paras.first().unwrap();
            assert!(p.props.numbering.is_some(), "{what}: the item is in a list");
            let change = p.props.fmt_change.as_deref().unwrap_or_else(|| panic!("{what}: change kept"));
            assert_eq!(change.old.align, Some(Align::Left), "{what}");
            // Reject puts the old properties back: the item stays in its list, with its style.
            let rejected = change.old.formatting();
            assert_eq!(rejected.numbering, p.props.numbering, "{what}");
            assert_eq!(rejected.style, p.props.style, "{what}");
        };
        let doc = model::to_doc(&flow);
        check(&doc, "flow to document");
        let rtf = export(&doc);
        check(&import(rtf.as_bytes()).unwrap(), &format!("RTF round trip\n{rtf}"));
    }

    #[test]
    fn paragraph_that_was_a_list_item_gets_its_old_list_back_on_reject() {
        use model::{FmtChange, ListInfo};
        // Alice took the paragraph out of a numbered list: it is no list item now, but was one.
        let mark = RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
        let old = Para { list: Some(ListInfo { ordered: true, level: 0, start: 1 }), ..Default::default() };
        let mut item = Para { fmt_change: Some(Box::new(FmtChange { mark, old })), ..Default::default() };
        item.push_text("Item", &Fmt::default());
        let flow = Flow { blocks: vec![FBlock::Para(item)], meta: Meta::default() };
        let doc = model::to_doc(&flow);
        let paras = doc_paras(&doc);
        let p = paras.first().unwrap();
        assert!(p.props.numbering.is_none());
        let change = p.props.fmt_change.as_deref().expect("change kept");
        assert!(change.old.formatting().numbering.is_some(), "Reject restores the list");
    }

    #[test]
    fn numbered_item_made_bulleted_gets_its_numbered_list_back_on_reject() {
        use model::{FmtChange, ListInfo};
        let mark = RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
        let old = Para { list: Some(ListInfo { ordered: true, level: 0, start: 1 }), ..Default::default() };
        let mut item = Para {
            list: Some(ListInfo { ordered: false, level: 0, start: 1 }),
            fmt_change: Some(Box::new(FmtChange { mark, old })),
            ..Default::default()
        };
        item.push_text("Item", &Fmt::default());
        let flow = Flow { blocks: vec![FBlock::Para(item)], meta: Meta::default() };
        let doc = model::to_doc(&flow);
        let paras = doc_paras(&doc);
        let p = paras.first().unwrap();
        let change = p.props.fmt_change.as_deref().expect("change kept");
        let (now, was) = (p.props.numbering.unwrap(), change.old.formatting().numbering.unwrap());
        assert_ne!(now.num, was.num, "the old list is a list of its own");
    }

    #[test]
    fn heading_formatting_change_has_no_style_formatting_in_the_old_formatting() {
        // A heading's runs repeat its style's bold and size; the old formatting does too.
        let rtf = br"{\rtf1{\stylesheet{\s0 Normal;}{\s1 heading 1;}}{\*\revtbl {Unknown;}{Alice;}}
\pard\s1\b\fs32 {\i\crauth1{\*\oldcprops \plain\b\fs32}Head}\par}";
        let doc = import(rtf).unwrap();
        let paras = doc_paras(&doc);
        let cp = paras.first().unwrap().props_of_char(0);
        let old = cp.fmt_change.as_deref().map(|c| c.old.clone()).expect("change kept");
        assert_eq!((old.bold, old.size), (cp.bold, cp.size));
    }

    #[test]
    fn word_oldcprops_hold_only_the_changed_properties() {
        use wordcraft_doc::RevisionKind;
        // Word for Mac 16.113 saving tracked formatting changes as RTF: "beta" made bold,
        // "gamma" (bold) grown from 12 to 16 pt. `\oldcprops` holds only what changed, with
        // `\crauth`/`\crdate` inside the group; the rest of the old formatting is the run's.
        let rtf = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}{\f1 Arial;}}{\*\revtbl {Unknown;}{Alice;}}
\pard\plain \f1\fs24 alpha {\f1\fs24\b\insrsid12079894\charrsid12079894 {\*\oldcprops \b0\crauth1\crdate-2014675576\insrsid12079894\charrsid12079894}beta} {\b\f1\fs32 {\*\oldcprops \fs24\crauth1\crdate-2014675576\insrsid12079894\charrsid12079894}gamma}\par}";
        let doc = import(rtf).unwrap();
        let paras = doc_paras(&doc);
        let p = paras.first().unwrap();
        assert_eq!(p.text, "alpha beta gamma");
        let word_rev = Some((RevisionKind::Format, "Alice".to_string(), "2026-10-15T22:08:00Z".to_string()));
        // beta: Reject takes the bold off; font and size stay.
        let beta = p.props_of_char(6);
        assert_eq!(beta.bold, Some(true));
        let old = beta.fmt_change.as_deref().expect("beta's change");
        assert_eq!(rev_of(&doc, Some(old.rev)), word_rev);
        assert_ne!(old.old.bold, Some(true), "{:?}", old.old);
        assert_eq!((old.old.font.as_deref(), old.old.size), (beta.font.as_deref(), Some(12.0)), "{:?}", old.old);
        // gamma: Reject puts 12 pt back; bold and font stay.
        let gamma = p.props_of_char(11);
        assert_eq!(gamma.size, Some(16.0));
        let old = gamma.fmt_change.as_deref().expect("gamma's change");
        assert_eq!(rev_of(&doc, Some(old.rev)), word_rev);
        assert_eq!((old.old.bold, old.old.font.as_deref(), old.old.size), (Some(true), gamma.font.as_deref(), Some(12.0)), "{:?}", old.old);
        assert_eq!(gamma.font.as_deref(), Some("Arial"));
    }

    #[test]
    fn word_oldpprops_hold_the_full_old_paragraph_formatting() {
        use wordcraft_doc::{Align, RevisionKind};
        // Word for Mac 16.113: a paragraph centred with tracking on. `\oldpprops` is the full
        // old set from `\pard`; `\prauth`/`\prdate` are on the paragraph.
        let rtf = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}}{\*\revtbl {Unknown;}{Alice;}}
\pard\plain \ltrpar\qc \li0\ri0\sa160\sl278\slmult1\widctlpar\wrapdefault\aspalpha\aspnum\faauto\adjustright\rin0\lin0\itap0\prauth1\prdate-2014675576{\*\oldpprops \pard \ltrpar\ql \li0\ri0\sa160\sl278\slmult1\widctlpar\wrapdefault\aspalpha\aspnum\faauto\adjustright\rin0\lin0\itap0\pararsid12079894}\fs24 Centred\par}";
        let doc = import(rtf).unwrap();
        let paras = doc_paras(&doc);
        let p = paras.first().unwrap();
        assert_eq!(p.text, "Centred");
        assert_eq!(p.props.align, Some(Align::Center));
        let c = p.props.fmt_change.as_deref().expect("paragraph change");
        assert_eq!(rev_of(&doc, Some(c.rev)), Some((RevisionKind::Format, "Alice".into(), "2026-10-15T22:08:00Z".into())));
        assert_eq!(c.old.align, Some(Align::Left));
        assert_eq!(c.old.space_after, p.props.space_after);
        assert_eq!(c.old.line_spacing, p.props.line_spacing);
        assert!(p.props.space_after.is_some());
    }

    #[test]
    fn oldcprops_are_written_as_the_differences() {
        use wordcraft_doc::props::{CharProps, PropChange};
        use wordcraft_doc::{Revision, RevisionKind};
        let rev = || vec![Revision::new(RevisionKind::Format, "Alice", "2024-03-15T10:30:00Z")];
        // Made bold: the group holds `\b0` and who changed it, nothing else.
        let old = CharProps { font: Some("Arial".into()), size: Some(14.0), ..Default::default() };
        let cp =
            CharProps { bold: Some(true), font: Some("Arial".into()), size: Some(14.0), fmt_change: PropChange::boxed(0, old), ..Default::default() };
        let rtf = export(&doc_with(vec![wordcraft_doc::Paragraph::with_text("Bold", cp)], rev()));
        assert!(rtf.contains("{\\*\\oldcprops \\b0\\crauth1\\crdate-1480361314}"), "{rtf}");
        // Several changes, on and off, round-trip.
        let red = wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(255, 0, 0));
        let old = CharProps {
            italic: Some(true),
            underline: Some(wordcraft_doc::props::Underline::Single),
            size: Some(14.0),
            color: Some(red),
            font: Some("Arial".into()),
            ..Default::default()
        };
        let cp = CharProps {
            bold: Some(true),
            size: Some(16.0),
            font: Some("Courier New".into()),
            fmt_change: PropChange::boxed(0, old.clone()),
            ..Default::default()
        };
        let rtf = export(&doc_with(vec![wordcraft_doc::Paragraph::with_text("Mixed", cp)], rev()));
        let back = import(rtf.as_bytes()).unwrap();
        let paras = doc_paras(&back);
        let cp = paras.first().unwrap().props_of_char(0);
        assert_eq!((cp.bold, cp.size), (Some(true), Some(16.0)), "{rtf}");
        let got = &cp.fmt_change.as_deref().expect("change read back").old;
        assert_ne!(got.bold, Some(true), "{rtf}");
        assert_eq!(
            (got.italic, got.underline, got.size, got.color, got.font.as_deref()),
            (old.italic, old.underline, old.size, old.color, old.font.as_deref()),
            "{rtf}"
        );
    }

    #[test]
    fn old_formatting_in_styled_paragraphs_is_the_styles() {
        use model::{FmtChange, Kind};
        let mark = || RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
        // A heading 2 run grown to 20 pt and a code run grown to 14 pt, both from no direct
        // formatting: Reject gives back the style's size, not RTF's default 12 pt. (A code run's
        // own font doesn't reach the flow from a document, so the code font is checked below.)
        let mut head = Para { kind: Kind::Heading(2), ..Default::default() };
        head.push_text(
            "Head",
            &Fmt { size: Some(20.0), fmt_change: Some(Box::new(FmtChange { mark: mark(), old: Fmt::default() })), ..Default::default() },
        );
        let mut code = Para { kind: Kind::Code, ..Default::default() };
        code.push_text(
            "let x",
            &Fmt { size: Some(14.0), fmt_change: Some(Box::new(FmtChange { mark: mark(), old: Fmt::default() })), ..Default::default() },
        );
        let flow = Flow { blocks: vec![FBlock::Para(head), FBlock::Para(code)], meta: Meta::default() };
        let rtf = export(&model::to_doc(&flow));
        let back = parse(rtf.as_bytes()).unwrap();
        let runs: Vec<&Fmt> = back
            .blocks
            .iter()
            .filter_map(|b| match b {
                FBlock::Para(p) => p.inlines.first().and_then(|i| if let Inline::Text(_, f) = i { Some(f) } else { None }),
                _ => None,
            })
            .collect();
        let head = runs.first().expect("heading run");
        // The import takes a heading's direct size as the style's (as on main); the old size
        // written is the style's, so Reject gives it whichever way the import reads it.
        assert!(rtf.contains("{\\*\\oldcprops \\fs32\\crauth1"), "{rtf}");
        let old = &head.fmt_change.as_deref().expect("heading change").old;
        assert!(old.size.is_none_or(|s| s == 16.0), "{old:?}\n{rtf}");
        let code = runs.get(1).expect("code run");
        assert!(rtf.contains("{\\*\\oldcprops \\fs20\\crauth1"), "{rtf}");
        let old = &code.fmt_change.as_deref().expect("code change").old;
        assert!(old.size.is_none_or(|s| s == 10.0), "{old:?}\n{rtf}");
        assert!(old.font.is_none() || old.code, "{old:?}\n{rtf}");
    }

    #[test]
    fn old_formatting_in_a_code_paragraph_has_the_code_font() {
        use model::{FmtChange, Kind};
        // A code run set in Arial from no direct font: the old font written is the code font.
        let mark = RevMark { author: "Alice".into(), date: String::new(), id: None };
        let mut p = Para { kind: Kind::Code, ..Default::default() };
        p.push_text(
            "x",
            &Fmt { font: Some("Arial".into()), fmt_change: Some(Box::new(FmtChange { mark, old: Fmt::default() })), ..Default::default() },
        );
        let mut w = Writer {
            t: Tables { fonts: vec!["Aptos".into()], colors: Vec::new() },
            lists: Vec::new(),
            authors: None,
            base: Fmt::default(),
            sections: Vec::new(),
            sect_i: 0,
        };
        let mut out = String::new();
        w.para(&p, false, None, "\\par", &mut out);
        let mono = w.t.font(model::MONO_FONT);
        assert!(out.contains(&format!("{{\\*\\oldcprops \\f{mono}\\crauth1}}")), "{out}");
    }

    /// The text runs of `rtf`'s paragraphs, in order, with their formatting.
    fn text_runs(rtf: &[u8]) -> Vec<(String, Fmt)> {
        let f = parse(rtf).unwrap();
        f.blocks
            .iter()
            .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
            .flat_map(|p| p.inlines.iter())
            .filter_map(|i| if let Inline::Text(t, f) = i { Some((t.clone(), f.clone())) } else { None })
            .collect()
    }

    /// The formatting of the run whose text is `t`.
    fn run_fmt<'a>(runs: &'a [(String, Fmt)], t: &str) -> &'a Fmt {
        runs.iter().find(|r| r.0 == t).map(|r| &r.1).unwrap_or_else(|| panic!("no run {t:?} in {runs:?}"))
    }

    #[test]
    fn heading_and_code_runs_take_their_style_formatting_on_import() {
        // A stylesheet entry without formatting words (WordCraft's older files, minimal
        // stylesheets): WordCraft's own heading and code formatting stand in for the style's.
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}{\f1 Arial;}{\f2 Courier New;}}{\stylesheet{\s0 Normal;}{\s2 heading 2;}{\s9 Code;}}
\pard\s2\b\fs32 Head\par
\pard\s2\b\fs26 Small\par
\pard\s9\f2\b\fs21 code\par
\pard\s9\f1\fs21 arial\par}";
        let runs = text_runs(rtf);
        let head = run_fmt(&runs, "Head");
        assert_eq!((head.size, head.bold), (None, false));
        // 13 pt isn't heading 2's 16 pt: a size set by hand.
        let small = run_fmt(&runs, "Small");
        assert_eq!((small.size, small.bold), (Some(13.0), false));
        let code = run_fmt(&runs, "code");
        assert_eq!((code.size, code.bold, code.font.as_deref(), code.code), (None, false, None, false));
        // Arial isn't the code font: set by hand.
        let arial = run_fmt(&runs, "arial");
        assert_eq!((arial.size, arial.font.as_deref(), arial.code), (None, Some("Arial"), false));
    }

    #[test]
    fn heading_runs_are_compared_with_the_files_heading_style() {
        // Word writes resolved values: a 13 pt bold Heading 2 has `\b\fs26` in its stylesheet
        // entry and on its runs. Only what differs from the entry is direct formatting.
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}}{\stylesheet{\ql \f0\fs24 Normal;}
{\s2\ql \keepn\outlinelevel1 \b\fs26 \sbasedon0 \snext0 heading 2;}{\s3\ql \keepn\b0\fs24 \sbasedon0 \snext0 heading 3;}}
\pard\s2\b\fs26 Head\par
\pard\s2\b\fs40 Big\par
\pard\s3\b\fs24 Bolded\par}";
        let runs = text_runs(rtf);
        let head = run_fmt(&runs, "Head");
        assert_eq!((head.size, head.bold), (None, false));
        let big = run_fmt(&runs, "Big");
        assert_eq!((big.size, big.bold), (Some(20.0), false));
        // Heading 3 isn't bold in this file: the bold is the user's.
        let bolded = run_fmt(&runs, "Bolded");
        assert_eq!((bolded.size, bolded.bold), (None, true));
    }

    #[test]
    fn style_formatting_is_resolved_through_sbasedon() {
        // LibreOffice's stylesheet: Heading 2 is based on Heading, which sets the boldness and
        // font; Code is based on a style that sets the font.
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Liberation Serif;}{\f1 Liberation Sans;}{\f2 Iosevka;}{\f3 Courier New;}}
{\stylesheet{\s0\f0\fs24 Normal;}{\s1\sbasedon0\snext2\f1\b0\fs28 Heading;}{\s2\sbasedon1\snext3\ql\fs32 Heading 2;}
{\s10\sbasedon0\f2\fs20 Source Base;}{\s9\sbasedon10 Code;}}
\pard\s2\f1\fs32 Plain\par
\pard\s2\f1\b\fs32 Bolded\par
\pard\s9\f2\fs20 base\par
\pard\s9\f3\fs20 courier\par}";
        let runs = text_runs(rtf);
        let plain = run_fmt(&runs, "Plain");
        assert_eq!((plain.size, plain.bold), (None, false));
        // Heading isn't bold, so neither is Heading 2.
        let bolded = run_fmt(&runs, "Bolded");
        assert_eq!((bolded.size, bolded.bold), (None, true));
        let base = run_fmt(&runs, "base");
        assert_eq!((base.size, base.font.as_deref(), base.code), (None, None, false));
        // Code's font is Iosevka here, so Courier New was set by hand.
        let courier = run_fmt(&runs, "courier");
        assert_eq!((courier.size, courier.font.as_deref(), courier.code), (None, None, true));
    }

    #[test]
    fn code_runs_in_the_files_code_font_have_no_direct_font() {
        let rtf =
            br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}{\f1 Courier New;}{\f2 Arial;}}{\stylesheet{\s0 Normal;}{\s9\sbasedon0\sa0\f1\fs21 Code;}}
\pard\s9\f1\fs21 mono\par
\pard\s9\f2\fs21 arial\par}";
        let runs = text_runs(rtf);
        let mono = run_fmt(&runs, "mono");
        assert_eq!((mono.size, mono.font.as_deref(), mono.code), (None, None, false));
        let arial = run_fmt(&runs, "arial");
        assert_eq!((arial.size, arial.font.as_deref(), arial.code), (None, Some("Arial"), false));
    }

    #[test]
    fn hostile_sbasedon_chains_are_harmless() {
        let mut chain = String::new();
        // s100 (heading 5) based on s101 … s139, the last one not bold: past the bound, so
        // heading 5 takes WordCraft's bold.
        for i in 100..140 {
            let name = if i == 100 { "heading 5".to_string() } else { format!("x{i}") };
            let b0 = if i == 139 { "\\b0" } else { "" };
            chain.push_str(&format!("{{\\s{i}\\sbasedon{}{b0} {name};}}", i + 1));
        }
        let rtf = format!(
            r"{{\rtf1{{\stylesheet{{\s0 Normal;}}{{\s1\sbasedon1 heading 1;}}{{\s3\sbasedon4\b0 heading 3;}}{{\s4\sbasedon3 heading 4;}}
{{\s6\sbasedon-5 heading 6;}}{{\s2\sbasedon2147483647\fs-40 heading 2;}}{{\s-2147483648\sbasedon-2147483648 Title;}}{chain}}}
\pard\s1\b\fs40 one\par
\pard\s3\b\fs24 three\par
\pard\s4\b\fs24 four\par
\pard\s6\b\fs22 six\par
\pard\s2\b\fs32 two\par
\pard\s100\b\fs24 five\par
\pard\s-2147483648\fs56 title\par}}"
        );
        let runs = text_runs(rtf.as_bytes());
        // Self-based heading 1: WordCraft's values.
        assert_eq!((run_fmt(&runs, "one").size, run_fmt(&runs, "one").bold), (None, false));
        // A loop between heading 3 and 4: both take heading 3's `\b0`.
        assert!(run_fmt(&runs, "three").bold);
        assert!(run_fmt(&runs, "four").bold);
        assert_eq!((run_fmt(&runs, "six").size, run_fmt(&runs, "six").bold), (None, false));
        assert_eq!((run_fmt(&runs, "two").size, run_fmt(&runs, "two").bold), (None, false));
        assert!(!run_fmt(&runs, "five").bold);
        let _ = run_fmt(&runs, "title");
    }

    #[test]
    fn a_word_365_heading_is_not_bold_so_a_bold_run_there_is_the_users() {
        // Word 365's Heading 1 isn't bold: its entry sets the font and size but no `\b`, so a
        // bold run, or a tracked change to or from bold, is the user's.
        let rtf = br"{\rtf1\ansi{\fonttbl{\f1 Aptos;}{\f2 Aptos Display;}}{\colortbl;\red15\green71\blue97;}
{\stylesheet{\ql \f1\fs24 Normal;}{\s1\ql \keepn\outlinelevel0 \f2\fs40\cf1 \sbasedon0 \snext0 heading 1;}}
{\*\revtbl {Unknown;}{Alice;}}
\pard\s1\f2\fs40 Plain\par
\pard\s1\f2\b\fs40 Bold\par
\pard\s1\f2\fs40 {\b{\*\oldcprops \b0\crauth1}Bolded}{\b0{\*\oldcprops \b\crauth1}Unbolded}\par}";
        let runs = text_runs(rtf);
        let plain = run_fmt(&runs, "Plain");
        assert_eq!((plain.size, plain.bold), (None, false));
        let bold = run_fmt(&runs, "Bold");
        assert_eq!((bold.size, bold.bold), (None, true));
        let bolded = run_fmt(&runs, "Bolded");
        let old = &bolded.fmt_change.as_deref().expect("change kept").old;
        assert_eq!((bolded.bold, old.bold, old.size), (true, false, None));
        let unbolded = run_fmt(&runs, "Unbolded");
        let old = &unbolded.fmt_change.as_deref().expect("change kept").old;
        assert_eq!((unbolded.bold, old.bold), (false, true));
    }

    #[test]
    fn a_style_chain_without_a_size_has_rtfs_default_size() {
        // The entries carry formatting words but no `\fs`: the style's size is RTF's 12 pt.
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}}{\stylesheet{\s0\f0 Normal;}{\s2\sbasedon0\b heading 2;}}
\pard\s2\b\fs24 Twelve\par
\pard\s2\b\fs40 Twenty\par}";
        let runs = text_runs(rtf);
        let twelve = run_fmt(&runs, "Twelve");
        assert_eq!((twelve.size, twelve.bold), (None, false));
        let twenty = run_fmt(&runs, "Twenty");
        assert_eq!((twenty.size, twenty.bold), (Some(20.0), false));
    }

    #[test]
    fn a_code_paragraph_round_trips_without_a_direct_code_font() {
        // WordCraft's Code entry has no `\f` (the code font is on the paragraph): the code font
        // stands in for the style's.
        let mut code = Para { kind: Kind::Code, ..Default::default() };
        code.push_text("let x", &Fmt::default());
        let flow = Flow { blocks: vec![FBlock::Para(code)], meta: Meta::default() };
        let rtf = export(&model::to_doc(&flow));
        let runs = text_runs(rtf.as_bytes());
        let f = run_fmt(&runs, "let x");
        assert_eq!((f.size, f.bold, f.code, f.font.as_deref()), (None, false, false, None), "{rtf}");
    }

    #[test]
    fn a_stylesheet_entry_without_a_number_is_normal() {
        // An entry without `\sN` is style 0; it doesn't replace the entry before it.
        let rtf = br"{\rtf1\ansi{\stylesheet{\s2\b\fs26 heading 2;}{\b0\fs30 Normal;}}
\pard\s2\b\fs26 Head\par
\pard\s2\b\fs30 Other\par}";
        let runs = text_runs(rtf);
        let head = run_fmt(&runs, "Head");
        assert_eq!((head.size, head.bold), (None, false));
        let other = run_fmt(&runs, "Other");
        assert_eq!((other.size, other.bold), (Some(15.0), false));
    }

    #[test]
    fn a_heading_size_set_by_hand_survives_in_the_old_formatting() {
        // Reject puts back a size set by hand: it isn't the file's heading size.
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}}{\stylesheet{\ql \fs24 Normal;}{\s2\ql \b\fs26 \sbasedon0 \snext0 heading 2;}}
{\*\revtbl {Unknown;}{Alice;}}
\pard\s2 {\b\fs26{\*\oldcprops \fs40\crauth1}Shrunk}{\b\fs40{\*\oldcprops \fs26\crauth1}Grown}\par}";
        let runs = text_runs(rtf);
        let shrunk = run_fmt(&runs, "Shrunk");
        assert_eq!(shrunk.size, None);
        let old = &shrunk.fmt_change.as_deref().expect("change kept").old;
        assert_eq!((old.size, old.bold), (Some(20.0), false));
        let grown = run_fmt(&runs, "Grown");
        assert_eq!(grown.size, Some(20.0));
        let old = &grown.fmt_change.as_deref().expect("change kept").old;
        assert_eq!((old.size, old.bold), (None, false));
    }

    #[test]
    fn pard_inside_oldcprops_keeps_the_words_before_it() {
        let doc = import(br"{\rtf1{\*\revtbl {Unknown;}{Alice;}}{\b{\*\oldcprops \i\crauth1\pard\b0}x}\par}").unwrap();
        let paras = doc_paras(&doc);
        let cp = paras.first().unwrap().props_of_char(0);
        let c = cp.fmt_change.as_deref().expect("change kept");
        assert_eq!(c.old.italic, Some(true));
        assert_ne!(c.old.bold, Some(true));
        assert_eq!(rev_of(&doc, Some(c.rev)).map(|r| r.1).as_deref(), Some("Alice"));
    }

    #[test]
    fn a_section_break_under_a_deletion_has_a_deleted_mark() {
        let rtf = br"{\rtf1{\*\revtbl {Unknown;}{Bob;}}\deleted\revauthdel1 one\sect\pard\plain two\par}";
        let doc = import(rtf).unwrap();
        let paras = doc_paras(&doc);
        assert_eq!(paras.first().map(|p| p.text.as_str()), Some("one"));
        assert_eq!(rev_of(&doc, paras.first().and_then(|p| p.mark.del)).map(|r| r.1).as_deref(), Some("Bob"));
    }

    #[test]
    fn hostile_formatting_revisions_never_panic() {
        let text = |rtf: &[u8]| import(rtf).map(|d| doc_paras(&d).iter().map(|p| p.text.clone()).collect::<Vec<_>>());
        // Old properties without an author: the text is kept and nothing else appears.
        assert_eq!(text(br"{\rtf1{\*\oldcprops \b}x\par}").unwrap().first().map(String::as_str), Some("x"));
        // Huge and negative indices and dates.
        let doc = import(br"{\rtf1{\b\crauth2147483647\crdate4294967295{\*\oldcprops \i}x}\pard\prauth-5\prdate4294967295{\*\oldpprops \qc}y\par}")
            .unwrap();
        let paras = doc_paras(&doc);
        let author = |i: Option<u32>| rev_of(&doc, i).map(|r| r.1);
        assert_eq!(paras.first().map(|p| p.text.as_str()), Some("xy"));
        assert_eq!(author(paras.first().and_then(|p| p.props_of_char(0).fmt_change.as_deref().map(|c| c.rev))).as_deref(), Some("Unknown"));
        // Structure inside old properties is ignored: no paragraphs, cells or text come from it.
        assert_eq!(
            text(br"{\rtf1\pard{\*\oldpprops \pard\qc\par\cell\row\trowd\cellx100 junk}z\par}").unwrap().first().map(String::as_str),
            Some("z")
        );
        assert_eq!(text(br"{\rtf1{\b\crauth1{\*\oldcprops {\*\oldcprops \i}\par x}y}\par}").unwrap().first().map(String::as_str), Some("y"));
        // Border words in old properties don't edit the cell border being defined.
        let f = parse(br"{\rtf1\ansi\trowd\clbrdrt\brdrs\brdrw10{\*\oldpprops \brdrdb\brdrw120\brdrcf1}\cellx2000\pard\intbl a\cell\row}").unwrap();
        let FBlock::Table(t) = &f.blocks[0] else { panic!() };
        let top = t.rows[0][0].borders.and_then(|b| b.top).unwrap();
        assert_eq!((top.style, top.width, top.color), (wordcraft_doc::props::BorderStyle::Single, 0.5, None));
        // Unterminated groups and deep nesting.
        for bad in [
            &br"{\rtf1\pard\prauth1\prdate5{\*\oldpprops \pard\qc"[..],
            br"{\rtf1{\crauth1{\*\oldcprops \b",
            br"{\rtf1\crauth\crdate\prauth\prdate{\*\oldcprops}{\*\oldpprops}",
        ] {
            let _ = import(bad);
        }
        let mut deep = String::from("{\\rtf1\\crauth1");
        for _ in 0..20_000 {
            deep.push_str("{\\*\\oldcprops \\b");
        }
        deep.push_str(" x\\par}");
        let _ = import(deep.as_bytes());
    }

    #[test]
    fn sections_keep_their_page_columns_numbering_and_headers() {
        use wordcraft_doc::section::{NumFormat, SectionStart};
        // A4 portrait → landscape restarting at 1 with its own header → continuous two columns
        // → portrait. A `\sectd` inside a footer group is the footer's, not the body's.
        let rtf = br"{\rtf1\ansi\paperw11906\paperh16838\margl1134\margr1134
\sectd\headery708\footery708
{\header \pard One head\par}
{\footer \pard Page {\field{\*\fldinst PAGE}{\fldrslt 1}}\sectd\par}
\pard First\par\sect
\sectd\lndscpsxn\pgwsxn16838\pghsxn11906\headery708\pgnrestart\pgnstarts1\pgnlcrm\titlepg
{\header \pard Two head\par}
{\headerf \pard Two first\par}
{\footer \pard Two foot\sectd\lndscpsxn\par}
\pard Second\par
\trowd\cellx2000\pard\intbl cell\cell\row
\pard{\sect}
\sectd\lndscpsxn\pgwsxn16838\pghsxn11906\headery708\sbknone\pgnrestart\cols2\colsx567\linebetcol
\pard Third\sect
\sectd\headery708\sbkodd
\pard Fourth\par}";
        let d = import(rtf).unwrap();
        let secs = d.sections();
        assert_eq!(secs.len(), 4, "{secs:?}");
        let s: Vec<&wordcraft_doc::SectionProps> = secs.iter().map(|(_, s)| *s).collect();
        // Page size and orientation; the document's margins apply to every section.
        for (i, land) in [(0, false), (1, true), (2, true), (3, false)] {
            assert_eq!(s[i].landscape, land, "section {i}");
            let (w, h) = if land { (841.9, 595.3) } else { (595.3, 841.9) };
            assert!((s[i].page_w - w).abs() < 0.1 && (s[i].page_h - h).abs() < 0.1, "section {i}: {} x {}", s[i].page_w, s[i].page_h);
            assert!((s[i].margin_left - 56.7).abs() < 0.1, "section {i}");
            assert!((s[i].header - 35.4).abs() < 0.1, "section {i}");
        }
        // Break types.
        assert_eq!(
            s.iter().map(|s| s.start).collect::<Vec<_>>(),
            [SectionStart::NextPage, SectionStart::NextPage, SectionStart::Continuous, SectionStart::OddPage]
        );
        // Columns.
        assert_eq!(s[2].columns.count, 2);
        assert!((s[2].columns.space - 28.35).abs() < 0.01 && s[2].columns.separator);
        assert_eq!(s[0].columns.count, 1);
        assert_eq!(s[3].columns.count, 1);
        // Page numbering: restart at 1 (lower roman) in 2 and 3; section 4 carries on.
        assert_eq!(s.iter().map(|s| s.page_num_start).collect::<Vec<_>>(), [None, Some(1), Some(1), None]);
        assert_eq!(s[1].page_num_format, NumFormat::LowerRoman);
        assert_eq!(s[0].page_num_format, NumFormat::Decimal);
        assert!(!s[0].title_page && s[1].title_page && !s[3].title_page);
        // Headers and footers: sections 1 and 2 have their own; 3 and 4 inherit (none set).
        let text = |id: Option<u32>| {
            let p = id.and_then(|id| d.parts.get(&id)).unwrap();
            p.blocks
                .iter()
                .filter_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p.text.clone()) } else { None })
                .collect::<Vec<_>>()
                .join("|")
        };
        assert_eq!(text(s[0].headers.default), "One head");
        assert!(text(s[0].footers.default).starts_with("Page"));
        assert_eq!(text(s[1].headers.default), "Two head");
        assert_eq!(text(s[1].headers.first), "Two first");
        assert_eq!(text(s[1].footers.default), "Two foot");
        assert!(s[0].headers.first.is_none());
        for i in [2, 3] {
            assert_eq!((s[i].headers, s[i].footers), Default::default(), "section {i}");
        }
        // Each section ends on its own paragraph, after the text it holds.
        let body_text = |i: usize| match d.body.get(i).map(|b| &**b) {
            Some(wordcraft_doc::Block::Para(p)) => p.text.clone(),
            _ => "<table>".into(),
        };
        assert_eq!(body_text(secs[0].0 - 1), "First");
        assert_eq!(body_text(secs[1].0 - 1), "<table>");
        assert_eq!(body_text(secs[2].0), "Third");
        assert_eq!(body_text(secs[3].0), "Fourth");
        // A landscape document default doesn't turn a section that gives its own portrait page.
        let d = import(br"{\rtf1\ansi\paperw16838\paperh11906\landscape\sectd a\sect\sectd\pgwsxn11906\pghsxn16838 b\par}").unwrap();
        let secs = d.sections();
        assert_eq!(secs.len(), 2);
        assert!(secs[0].1.landscape && secs[0].1.page_w > secs[0].1.page_h, "{:?}", secs[0].1);
        assert!(!secs[1].1.landscape && secs[1].1.page_w < secs[1].1.page_h, "{:?}", secs[1].1);
    }

    #[test]
    fn export_writes_every_section() {
        use wordcraft_doc::section::{NumFormat, SectionStart};
        use wordcraft_doc::{Block, SectionProps};
        let mut d = wordcraft_doc::Document::from_text("one\ntwo\nthree");
        let s1 = SectionProps { margin_left: 90.0, header: 30.0, footer: 40.0, ..Default::default() };
        let mut s2 = SectionProps { start: SectionStart::Continuous, page_w: 792.0, page_h: 612.0, landscape: true, ..Default::default() };
        s2.columns.count = 2;
        s2.columns.space = 18.0;
        s2.columns.separator = true;
        s2.page_num_start = Some(3);
        s2.page_num_format = NumFormat::LowerRoman;
        let mut s3 = SectionProps { start: SectionStart::OddPage, title_page: true, ..Default::default() };
        s3.page_num_format = NumFormat::UpperLetter;
        for (i, s) in [(0usize, s1.clone()), (1, s2.clone())] {
            if let Some(Block::Para(p)) = d.body.get_mut(i).map(std::sync::Arc::make_mut) {
                p.section = Some(Box::new(s));
            }
        }
        d.last_section = s3.clone();
        let rtf = export(&d);
        assert_eq!(rtf.matches("\\sect\n").count() + rtf.matches("\\sect\\").count(), 2, "{rtf}");
        let back = import(rtf.as_bytes()).unwrap();
        let secs = back.sections();
        assert_eq!(secs.len(), 3, "{rtf}");
        let got: Vec<&SectionProps> = secs.iter().map(|(_, s)| *s).collect();
        for (g, want) in got.iter().zip([&s1, &s2, &s3]) {
            assert_eq!(g.start, want.start);
            assert_eq!(g.landscape, want.landscape);
            assert!((g.page_w - want.page_w).abs() < 0.1 && (g.page_h - want.page_h).abs() < 0.1);
            for (a, b) in [
                (g.margin_left, want.margin_left),
                (g.margin_right, want.margin_right),
                (g.margin_top, want.margin_top),
                (g.margin_bottom, want.margin_bottom),
            ] {
                assert!((a - b).abs() < 0.1);
            }
            assert!((g.header - want.header).abs() < 0.1 && (g.footer - want.footer).abs() < 0.1);
            assert_eq!(g.columns.count, want.columns.count);
            assert_eq!(g.columns.separator, want.columns.separator);
            assert_eq!(g.page_num_start, want.page_num_start);
            assert_eq!(g.page_num_format, want.page_num_format);
            assert_eq!(g.title_page, want.title_page);
        }
        assert!((got[1].columns.space - 18.0).abs() < 0.1);
        // Each section keeps its text.
        assert_eq!(secs.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1, 2]);
        // One section: no section break.
        let one = export(&wordcraft_doc::Document::from_text("a\nb"));
        assert!(!one.contains("\\sect\n") && !one.contains("\\sect\\"), "{one}");
    }

    #[test]
    fn export_keeps_a_single_sections_formatting() {
        use wordcraft_doc::SectionProps;
        let mut d = wordcraft_doc::Document::from_text("a\nb");
        d.last_section = SectionProps { title_page: true, ..Default::default() };
        d.last_section.columns.count = 2;
        d.last_section.columns.space = 20.0;
        let back = import(export(&d).as_bytes()).unwrap();
        assert_eq!(back.sections().len(), 1);
        assert_eq!(back.last_section.columns.count, 2);
        assert!(back.last_section.title_page);
    }

    #[test]
    fn export_keeps_a_section_that_ends_the_body() {
        use wordcraft_doc::{Block, SectionProps};
        let mut d = wordcraft_doc::Document::from_text("a\nb");
        let s1 = SectionProps { landscape: true, page_w: 792.0, page_h: 612.0, ..Default::default() };
        if let Some(Block::Para(p)) = d.body.get_mut(1).map(std::sync::Arc::make_mut) {
            p.section = Some(Box::new(s1));
        }
        d.last_section.columns.count = 3;
        let back = import(export(&d).as_bytes()).unwrap();
        let secs = back.sections();
        assert_eq!(secs.len(), 2);
        assert!(secs[0].1.landscape);
        assert_eq!(secs[1].1.columns.count, 3);
    }

    #[test]
    fn export_writes_no_section_break_inside_a_table() {
        use wordcraft_doc::{Block, SectionProps};
        let mut d = wordcraft_doc::Document::from_text("a");
        let mut t = wordcraft_doc::Table::new(1, 1, 400.0);
        if let Some(Block::Para(p)) =
            t.rows.first_mut().and_then(|r| r.cells.first_mut()).and_then(|c| c.blocks.first_mut()).map(std::sync::Arc::make_mut)
        {
            p.section = Some(Box::new(SectionProps::default()));
        }
        d.body.push(std::sync::Arc::new(Block::Table(t)));
        let rtf = export(&d);
        assert!(!rtf.contains("\\sect\n") && !rtf.contains("\\sect\\") && rtf.matches("\\sectd").count() == 1, "{rtf}");
        assert_eq!(import(rtf.as_bytes()).unwrap().sections().len(), 1);
    }

    #[test]
    fn sections_are_capped_and_a_trailing_sect_adds_no_empty_section() {
        // A file that ends with `\sect` has no empty section after it.
        let d = import(br"{\rtf1\ansi\sectd\sbknone a\sect\sectd\lndscpsxn b\sect}").unwrap();
        let secs = d.sections();
        assert_eq!(secs.len(), 2, "{secs:?}");
        assert!(secs[1].1.landscape && !secs[0].1.landscape);
        // Hostile: a flood of section breaks with headers is capped and never panics.
        let mut rtf = b"{\\rtf1\\ansi".to_vec();
        for _ in 0..(MAX_SECTIONS + 50) {
            rtf.extend_from_slice(b"\\sectd\\cols99999\\colsx-5\\pgnrestart\\pgnstarts-7{\\header h\\par}{\\footerl f\\par}x\\sect");
        }
        rtf.push(b'}');
        let d = import(&rtf).unwrap();
        let secs = d.sections();
        assert_eq!(secs.len(), MAX_SECTIONS);
        // Each section keeps its own header and footer; the ones past the cap replace the last's.
        assert!(d.parts.len() <= 2 * MAX_SECTIONS, "{}", d.parts.len());
        assert!(secs.iter().all(|(_, s)| s.columns.count <= 45 && s.columns.space >= 0.0 && s.page_num_start == Some(0)));
    }

    #[test]
    fn rejects_non_rtf() {
        assert!(parse(b"hello").is_err());
        assert!(parse(b"{\\rtf1").is_ok());
    }

    #[test]
    fn escapes() {
        assert_eq!(esc("a{b}\\c\té😀"), "a\\{b\\}\\\\c\\tab \\u233?\\u-10179?\\u-8704?");
        assert_eq!(esc("col1\u{000E}col2"), "col1\\column col2");
    }

    #[test]
    fn column_break_import() {
        let f = parse(br"{\rtf1\ansi \pard a\column b\par}").unwrap();
        assert_eq!(texts(&f), ["Normal:a\u{000E}b"]);
    }

    #[test]
    fn hyperlink_field_targets() {
        assert_eq!(hyperlink(r#"HYPERLINK "http://example.com" \l "heading""#), Some("http://example.com#heading".into()));
        assert_eq!(hyperlink(r#"HYPERLINK \l "heading""#), Some("#heading".into()));
        assert_eq!(hyperlink(r#"Hyperlink "http://example.com""#), Some("http://example.com".into()));
        assert_eq!(hyperlink(r#"HYPERLINK "http://example.com/path\latest""#), Some("http://example.com/path\\latest".into()));
        assert_eq!(hyperlink(r#"HYPERLINK "http://example.com" \o "Help text""#), Some("http://example.com".into()));
    }

    #[test]
    fn distributed_alignment_round_trips() {
        let rtf = br"{\rtf1\ansi\pard\qd Distributed\par\pard\qj Justified\par}";
        let flow = parse(rtf).unwrap();
        let p0 = match &flow.blocks[0] {
            FBlock::Para(p) => p.align,
            _ => panic!("expected para"),
        };
        let p1 = match &flow.blocks[1] {
            FBlock::Para(p) => p.align,
            _ => panic!("expected para"),
        };
        assert_eq!(p0, Some(Align::Distribute));
        assert_eq!(p1, Some(Align::Justify));

        let doc = import(rtf).unwrap();
        let exported = export(&doc);
        assert!(exported.contains("\\qd"), "expected \\qd in: {exported}");
        assert!(exported.contains("\\qj"), "expected \\qj in: {exported}");
    }

    #[test]
    fn table_cell_vertical_alignment() {
        let rtf = br"{\rtf1\ansi\trowd\clvertalt\cellx1000\clvertalc\cellx2000\clvertalb\cellx3000\intbl top\cell center\cell bottom\cell\row}";
        let doc = import(rtf).unwrap();
        let wordcraft_doc::Block::Table(t) = &*doc.body[0] else { panic!() };
        assert_eq!(t.rows[0].cells[0].props.valign, Some(VAlign::Top));
        assert_eq!(t.rows[0].cells[1].props.valign, Some(VAlign::Center));
        assert_eq!(t.rows[0].cells[2].props.valign, Some(VAlign::Bottom));
        let exported = export(&doc);
        assert!(exported.contains("\\clvertalt"));
        assert!(exported.contains("\\clvertalc"));
        assert!(exported.contains("\\clvertalb"));
    }

    #[test]
    fn rtf_backslash_space_and_special_breaks() {
        let rtf = br"{\rtf1\ansi A\ B\~C\zwbo D\softline E\softpage F\par}";
        let doc = import(rtf).unwrap();
        let p = doc.body.first().unwrap().as_para().unwrap();
        assert_eq!(p.text, "A\u{00A0}B\u{00A0}C\u{200B}D\nE\u{000C}F");
    }
}
