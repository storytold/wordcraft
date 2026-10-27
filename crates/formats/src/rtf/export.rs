use super::date::{iso_to_dttm, rtf_date};
use super::styles::{style_fmt, styled};
use super::*;

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

pub(super) struct Tables {
    pub(super) fonts: Vec<String>,
    pub(super) colors: Vec<Rgb>,
}

impl Tables {
    pub(super) fn font(&mut self, name: &str) -> usize {
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

pub(super) struct Writer {
    pub(super) t: Tables,
    /// List definitions: per list, the ordered flag per level.
    pub(super) lists: Vec<[bool; 9]>,
    /// Authors of tracked changes (`\revtbl` entries after "Unknown"), when the text has any.
    pub(super) authors: Option<Vec<String>>,
    /// The character formatting the current paragraph's style gives its runs (heading bold and
    /// size, code font…): what a run's formatting without a direct value resolves to.
    pub(super) base: Fmt,
    /// The body's sections in order (the last is the document's); empty when it has just one.
    pub(super) sections: Vec<wordcraft_doc::SectionProps>,
    /// The section being written.
    pub(super) sect_i: usize,
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

pub(super) const HEAD_SIZES: [f32; 6] = [20.0, 16.0, 14.0, 12.0, 12.0, 11.0];

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
        fn font(f: &Fmt) -> Option<&str> {
            if f.code { Some(model::MONO_FONT) } else { f.font.as_deref() }
        }
        let old_font = font(old);
        if old_font != font(cur) {
            // No font is the document's default font, font 0 (Normal's).
            let i = old_font.map_or(0, |n| self.t.font(n));
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

    pub(super) fn para(&mut self, p: &Para, intbl: bool, ls: Option<(usize, String)>, end: &str, out: &mut String) {
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
