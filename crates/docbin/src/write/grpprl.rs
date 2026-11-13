//! Model properties → grpprls of sprms ([MS-DOC] §2.6): character (CHPX), paragraph (PAPX) and
//! section (SEPX) formatting. The opcodes are the ones the reader decodes (`fmt.rs`,
//! `sections.rs`, `table.rs`), so a saved file reads back with the same properties.

use wordcraft_doc::Rgb;
use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CharProps, Highlight, LineSpacing, ParaProps, TabAlign, TabLeader, TextColor, Underline, VertAlign,
};
use wordcraft_doc::section::{NumFormat, PageVAlign, SectionProps, SectionStart};

/// A grpprl being built: Prls appended one by one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Grpprl(pub(crate) Vec<u8>);

impl Grpprl {
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    fn op(&mut self, op: u16) {
        self.0.extend_from_slice(&op.to_le_bytes());
    }

    pub(crate) fn u8(&mut self, op: u16, v: u8) {
        self.op(op);
        self.0.push(v);
    }

    pub(crate) fn u16(&mut self, op: u16, v: u16) {
        self.op(op);
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn i16(&mut self, op: u16, v: i16) {
        self.u16(op, v as u16);
    }

    pub(crate) fn u32(&mut self, op: u16, v: u32) {
        self.op(op);
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    /// A Prl whose operand is written as given (its size implied by the opcode's spra, or a
    /// leading size byte the caller includes).
    pub(crate) fn raw(&mut self, op: u16, operand: &[u8]) {
        self.op(op);
        self.0.extend_from_slice(operand);
    }

    /// A variable-length Prl (spra 6): a size byte, then the operand.
    pub(crate) fn var(&mut self, op: u16, operand: &[u8]) {
        self.op(op);
        self.0.push(u8::try_from(operand.len()).unwrap_or(u8::MAX));
        self.0.extend_from_slice(operand.get(..255).unwrap_or(operand));
    }

    pub(crate) fn extend(&mut self, other: &Grpprl) {
        self.0.extend_from_slice(&other.0);
    }

    /// Keep whole Prls up to `max` bytes: a grpprl that doesn't fit where it is stored (a CHPX
    /// holds 255 bytes) loses its last properties rather than the file its shape.
    pub(crate) fn truncate_to(&mut self, max: usize) {
        if self.0.len() <= max {
            return;
        }
        let mut keep = 0usize;
        for prl in crate::sprm::iter(&self.0) {
            let next = keep + 2 + prl.operand.len();
            if next > max {
                break;
            }
            keep = next;
        }
        self.0.truncate(keep);
    }
}

// Character sprms ([MS-DOC] §2.6.1).
pub(crate) const C_RMARK_DEL: u16 = 0x0800;
pub(crate) const C_RMARK_INS: u16 = 0x0801;
pub(crate) const C_IBST_RMARK: u16 = 0x4804;
pub(crate) const C_DTTM_RMARK: u16 = 0x6805;
pub(crate) const C_IBST_RMARK_DEL: u16 = 0x4863;
pub(crate) const C_DTTM_RMARK_DEL: u16 = 0x6864;
pub(crate) const C_ISTD: u16 = 0x4A30;
const C_BOLD: u16 = 0x0835;
const C_ITALIC: u16 = 0x0836;
const C_STRIKE: u16 = 0x0837;
const C_OUTLINE: u16 = 0x0838;
const C_SHADOW: u16 = 0x0839;
const C_SMALL_CAPS: u16 = 0x083A;
const C_CAPS: u16 = 0x083B;
const C_VANISH: u16 = 0x083C;
const C_IMPRINT: u16 = 0x0854;
pub(crate) const C_SPEC: u16 = 0x0855;
const C_EMBOSS: u16 = 0x0858;
const C_RTL: u16 = 0x085A;
const C_BOLD_BI: u16 = 0x085C;
const C_ITALIC_BI: u16 = 0x085D;
const C_NO_PROOF: u16 = 0x0875;
const C_COMPLEX: u16 = 0x0882;
const C_HIGHLIGHT: u16 = 0x2A0C;
const C_KUL: u16 = 0x2A3E;
const C_ICO: u16 = 0x2A42;
const C_ISS: u16 = 0x2A48;
const C_DSTRIKE: u16 = 0x2A53;
const C_HPS: u16 = 0x4A43;
const C_HPS_POS: u16 = 0x4845;
const C_HPS_KERN: u16 = 0x484B;
const C_CHAR_SCALE: u16 = 0x4852;
const C_LID_BI: u16 = 0x485F;
const C_LID0_80: u16 = 0x486D;
const C_LID1_80: u16 = 0x486E;
const C_LID0: u16 = 0x4873;
const C_LID1: u16 = 0x4874;
const C_FTC0: u16 = 0x4A4F;
const C_FTC1: u16 = 0x4A50;
const C_FTC2: u16 = 0x4A51;
const C_FTC_BI: u16 = 0x4A5E;
const C_HPS_BI: u16 = 0x4A61;
pub(crate) const C_PIC_LOCATION: u16 = 0x6A03;
const C_CV: u16 = 0x6870;
const C_CV_UL: u16 = 0x6877;
const C_DXA_SPACE: u16 = 0x8840;
const C_SHD: u16 = 0xCA71;

/// What [`chpx`] needs besides the properties: the font table, the style table, and the
/// tracked-change authors.
pub(crate) trait CharCtx {
    /// Index of `name` in the font table (added on first use).
    fn font(&mut self, name: &str) -> u16;
    /// The istd of a character style id; `None` when the stylesheet doesn't have it.
    fn char_style(&self, id: &str) -> Option<u16>;
    /// The author index and `DTTM` of revision `rev` (`Document::revisions`).
    fn revision(&mut self, rev: u32) -> Option<(u16, u32)>;
}

/// The CHPX grpprl for `p`: only the properties it sets, in the order Word applies them (the
/// style first, then toggles, then values). Inherited (`None`) properties are left out.
pub(crate) fn chpx(p: &CharProps, ctx: &mut dyn CharCtx) -> Grpprl {
    let mut g = Grpprl::default();
    if let Some(istd) = p.style.as_deref().and_then(|s| ctx.char_style(s)) {
        g.u16(C_ISTD, istd);
    }
    // Tracked changes: the mark, the author and the time ([MS-DOC] sprmCFRMarkIns and friends).
    if let Some((ibst, dttm)) = p.ins.and_then(|r| ctx.revision(r)) {
        g.u8(C_RMARK_INS, 1);
        g.u16(C_IBST_RMARK, ibst);
        g.u32(C_DTTM_RMARK, dttm);
    }
    if let Some((ibst, dttm)) = p.del.and_then(|r| ctx.revision(r)) {
        g.u8(C_RMARK_DEL, 1);
        g.u16(C_IBST_RMARK_DEL, ibst);
        g.u32(C_DTTM_RMARK_DEL, dttm);
    }
    for (op, v) in [
        (C_BOLD, p.bold),
        (C_ITALIC, p.italic),
        (C_STRIKE, p.strike),
        (C_OUTLINE, p.outline),
        (C_SHADOW, p.shadow),
        (C_SMALL_CAPS, p.small_caps),
        (C_CAPS, p.caps),
        (C_VANISH, p.hidden),
        (C_IMPRINT, p.engrave),
        (C_EMBOSS, p.emboss),
        (C_RTL, p.rtl),
        (C_BOLD_BI, p.bold_cs),
        (C_ITALIC_BI, p.italic_cs),
        (C_NO_PROOF, p.no_proof),
        (C_COMPLEX, p.cs),
    ] {
        if let Some(on) = v {
            g.u8(op, u8::from(on));
        }
    }
    if let Some(d) = p.double_strike {
        g.u8(C_DSTRIKE, u8::from(d));
    }
    if let Some(u) = p.underline {
        g.u8(C_KUL, kul(u));
    }
    if let Some(c) = p.underline_color {
        g.u32(C_CV_UL, colorref(c));
    }
    match p.color {
        Some(TextColor::Auto) => g.u8(C_ICO, 0),
        Some(TextColor::Rgb(c)) => {
            // The palette colour for Word 97, then the exact colour for every later version.
            g.u8(C_ICO, nearest_ico(c));
            g.u32(C_CV, colorref(c));
        }
        None => {}
    }
    if let Some(h) = p.highlight {
        g.u8(C_HIGHLIGHT, highlight_ico(h));
    }
    if let Some(s) = p.size {
        g.u16(C_HPS, half_points(s));
    }
    if let Some(s) = p.size_cs {
        g.u16(C_HPS_BI, half_points(s));
    }
    if let Some(v) = p.vert_align {
        g.u8(
            C_ISS,
            match v {
                VertAlign::Baseline => 0,
                VertAlign::Superscript => 1,
                VertAlign::Subscript => 2,
            },
        );
    }
    if let Some(pos) = p.position {
        g.i16(C_HPS_POS, (pos * 2.0).round().clamp(-3168.0, 3168.0) as i16);
    }
    if let Some(sp) = p.spacing {
        g.i16(C_DXA_SPACE, twips(sp));
    }
    if let Some(sc) = p.scale {
        g.u16(C_CHAR_SCALE, sc.round().clamp(1.0, 600.0) as u16);
    }
    if let Some(k) = p.kern {
        g.u16(C_HPS_KERN, half_points(k.max(0.0)));
    }
    if let Some(f) = p.font.as_deref() {
        let i = ctx.font(f);
        g.u16(C_FTC0, i);
        // Text outside ASCII but not East Asian takes the third font; Word sets both together.
        g.u16(C_FTC2, i);
    }
    if let Some(f) = p.font_ea.as_deref() {
        let i = ctx.font(f);
        g.u16(C_FTC1, i);
    }
    if let Some(f) = p.font_cs.as_deref() {
        let i = ctx.font(f);
        g.u16(C_FTC_BI, i);
    }
    if let Some(lid) = p.lang.as_deref().and_then(lcid) {
        g.u16(C_LID0_80, lid);
        g.u16(C_LID0, lid);
    }
    if let Some(lid) = p.lang_ea.as_deref().and_then(lcid) {
        g.u16(C_LID1_80, lid);
        g.u16(C_LID1, lid);
    }
    if let Some(lid) = p.lang_bidi.as_deref().and_then(lcid) {
        g.u16(C_LID_BI, lid);
    }
    if let Some(c) = p.shading {
        g.var(C_SHD, &shd(Some(c)));
    }
    g
}

/// Points → half-points, as the size sprms store them.
fn half_points(pt: f32) -> u16 {
    (pt * 2.0).round().clamp(2.0, 3276.0) as u16
}

/// Points → twips, clamped to what an `XAS`/`YAS` holds.
pub(crate) fn twips(pt: f32) -> i16 {
    if !pt.is_finite() {
        return 0;
    }
    (pt * 20.0).round().clamp(-31_680.0, 31_680.0) as i16
}

/// A `COLORREF` (0x00BBGGRR, stored R, G, B, 0).
pub(crate) fn colorref(c: Rgb) -> u32 {
    u32::from_le_bytes([c.0, c.1, c.2, 0])
}

/// The automatic colour (`fAuto` in the high byte).
pub(crate) const CV_AUTO: u32 = 0xFF00_0000;

/// A 10-byte `Shd` ([MS-DOC] §2.9.248): automatic foreground, `fill` as background, clear
/// pattern; no fill is an all-automatic one.
pub(crate) fn shd(fill: Option<Rgb>) -> [u8; 10] {
    let mut b = [0u8; 10];
    b[0..4].copy_from_slice(&CV_AUTO.to_le_bytes());
    let back = fill.map_or(CV_AUTO, colorref);
    b[4..8].copy_from_slice(&back.to_le_bytes());
    b
}

/// Word's 16-colour palette ([MS-DOC] §2.9.135 Ico), index 1..=16.
const ICO: [Rgb; 16] = [
    Rgb(0, 0, 0),
    Rgb(0, 0, 255),
    Rgb(0, 255, 255),
    Rgb(0, 255, 0),
    Rgb(255, 0, 255),
    Rgb(255, 0, 0),
    Rgb(255, 255, 0),
    Rgb(255, 255, 255),
    Rgb(0, 0, 128),
    Rgb(0, 128, 128),
    Rgb(0, 128, 0),
    Rgb(128, 0, 128),
    Rgb(128, 0, 0),
    Rgb(128, 128, 0),
    Rgb(128, 128, 128),
    Rgb(192, 192, 192),
];

/// The palette entry nearest `c` (by squared distance), 1-based.
pub(crate) fn nearest_ico(c: Rgb) -> u8 {
    let d = |p: &Rgb| {
        let dr = i32::from(p.0) - i32::from(c.0);
        let dg = i32::from(p.1) - i32::from(c.1);
        let db = i32::from(p.2) - i32::from(c.2);
        dr * dr + dg * dg + db * db
    };
    ICO.iter().enumerate().min_by_key(|(_, p)| d(p)).map_or(1, |(i, _)| i as u8 + 1)
}

/// A highlight colour as the `Ico` `sprmCHighlight` takes.
fn highlight_ico(h: Highlight) -> u8 {
    match h {
        Highlight::None => 0,
        Highlight::Black => 1,
        Highlight::Blue => 2,
        Highlight::Turquoise => 3,
        Highlight::BrightGreen => 4,
        Highlight::Pink => 5,
        Highlight::Red => 6,
        Highlight::Yellow => 7,
        Highlight::White => 8,
        Highlight::DarkBlue => 9,
        Highlight::Teal => 10,
        Highlight::Green => 11,
        Highlight::Violet => 12,
        Highlight::DarkRed => 13,
        Highlight::DarkYellow => 14,
        Highlight::Gray50 => 15,
        Highlight::Gray25 => 16,
    }
}

/// An underline as a `Kul` ([MS-DOC] §2.9.133).
fn kul(u: Underline) -> u8 {
    match u {
        Underline::None => 0x00,
        Underline::Single => 0x01,
        Underline::Words => 0x02,
        Underline::Double => 0x03,
        Underline::Dotted => 0x04,
        Underline::Thick => 0x06,
        Underline::Dash => 0x07,
        Underline::DotDash => 0x09,
        Underline::DotDotDash => 0x0A,
        Underline::Wave => 0x0B,
        Underline::DoubleWave => 0x2B,
    }
}

/// The Windows language id (LCID) of a BCP 47 tag, for the languages WordCraft proofs and the
/// other common ones; `None` for a tag not in the table (the text keeps the style's language).
pub(crate) fn lcid(tag: &str) -> Option<u16> {
    let t = tag.trim().to_ascii_lowercase().replace('_', "-");
    Some(match t.as_str() {
        "ar-sa" | "ar" => 0x0401,
        "bg-bg" | "bg" => 0x0402,
        "ca-es" | "ca" => 0x0403,
        "zh-tw" => 0x0404,
        "cs-cz" | "cs" => 0x0405,
        "da-dk" | "da" => 0x0406,
        "de-de" | "de" => 0x0407,
        "el-gr" | "el" => 0x0408,
        "en-us" | "en" => 0x0409,
        "es-es" | "es" => 0x0C0A,
        "fi-fi" | "fi" => 0x040B,
        "fr-fr" | "fr" => 0x040C,
        "he-il" | "he" => 0x040D,
        "hu-hu" | "hu" => 0x040E,
        "is-is" | "is" => 0x040F,
        "it-it" | "it" => 0x0410,
        "ja-jp" | "ja" => 0x0411,
        "ko-kr" | "ko" => 0x0412,
        "nl-nl" | "nl" => 0x0413,
        "nb-no" | "nb" | "no" => 0x0414,
        "pl-pl" | "pl" => 0x0415,
        "pt-br" => 0x0416,
        "ro-ro" | "ro" => 0x0418,
        "ru-ru" | "ru" => 0x0419,
        "hr-hr" | "hr" => 0x041A,
        "sk-sk" | "sk" => 0x041B,
        "sv-se" | "sv" => 0x041D,
        "th-th" | "th" => 0x041E,
        "tr-tr" | "tr" => 0x041F,
        "id-id" | "id" => 0x0421,
        "uk-ua" | "uk" => 0x0422,
        "sl-si" | "sl" => 0x0424,
        "et-ee" | "et" => 0x0425,
        "lv-lv" | "lv" => 0x0426,
        "lt-lt" | "lt" => 0x0427,
        "fa-ir" | "fa" => 0x0429,
        "vi-vn" | "vi" => 0x042A,
        "hi-in" | "hi" => 0x0439,
        "ne-np" | "ne" => 0x0461,
        "zh-cn" | "zh" => 0x0804,
        "de-ch" => 0x0807,
        "en-gb" => 0x0809,
        "es-mx" => 0x080A,
        "pt-pt" | "pt" => 0x0816,
        "sr-latn-rs" => 0x241A,
        "sr-cyrl-rs" | "sr" => 0x281A,
        "en-au" => 0x0C09,
        "en-ca" => 0x1009,
        "fr-ca" => 0x0C0C,
        "es-uy" => 0x380A,
        "es-419" => 0x580A,
        _ => return None,
    })
}

// Paragraph sprms ([MS-DOC] §2.6.2).
const P_JC80: u16 = 0x2403;
const P_JC: u16 = 0x2461;
const P_KEEP: u16 = 0x2405;
const P_KEEP_FOLLOW: u16 = 0x2406;
const P_PAGE_BREAK_BEFORE: u16 = 0x2407;
const P_NO_LINE_NUMB: u16 = 0x240C;
const P_IN_TABLE: u16 = 0x2416;
const P_TTP: u16 = 0x2417;
const P_NO_AUTO_HYPH: u16 = 0x242A;
const P_WIDOW: u16 = 0x2431;
const P_BIDI: u16 = 0x2441;
const P_OUTLVL: u16 = 0x2640;
const P_CONTEXTUAL: u16 = 0x246D;
const P_ILVL: u16 = 0x260A;
const P_ILFO: u16 = 0x460B;
const P_DXC_RIGHT: u16 = 0x4455;
const P_DXC_LEFT: u16 = 0x4456;
const P_DXC_FIRST: u16 = 0x4457;
const P_DYA_LINE: u16 = 0x6412;
const P_ITAP: u16 = 0x6649;
const P_DXA_RIGHT80: u16 = 0x840E;
const P_DXA_LEFT80: u16 = 0x840F;
const P_DXA_FIRST80: u16 = 0x8411;
const P_DXA_RIGHT: u16 = 0x845D;
const P_DXA_LEFT: u16 = 0x845E;
const P_DXA_FIRST: u16 = 0x8460;
const P_DYA_BEFORE: u16 = 0xA413;
const P_DYA_AFTER: u16 = 0xA414;
const P_CHG_TABS_PAPX: u16 = 0xC60D;
const P_SHD: u16 = 0xC64D;
const P_BRC: [u16; 5] = [0xC64E, 0xC64F, 0xC650, 0xC651, 0xC652];
const P_BRC80: [u16; 5] = [0x6424, 0x6425, 0x6426, 0x6427, 0x6428];

/// What [`papx`] needs besides the properties: the list (LFO) a model num id became.
pub(crate) trait ParaCtx {
    /// The 1-based LFO index of model num `id` (0 = no list).
    fn lfo(&self, num: u32) -> u16;
}

/// The PAPX grpprl for `p` (its style is the PAPX's istd, not a sprm).
pub(crate) fn papx(p: &ParaProps, ctx: &dyn ParaCtx) -> Grpprl {
    let mut g = Grpprl::default();
    if let Some(a) = p.align {
        let jc = match a {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
            Align::Justify => 3,
            Align::Distribute => 4,
        };
        g.u8(P_JC80, jc);
        g.u8(P_JC, jc);
    }
    for (v, op80, op) in
        [(p.indent_left, P_DXA_LEFT80, P_DXA_LEFT), (p.indent_right, P_DXA_RIGHT80, P_DXA_RIGHT), (p.indent_first, P_DXA_FIRST80, P_DXA_FIRST)]
    {
        if let Some(v) = v {
            g.i16(op80, twips(v));
            g.i16(op, twips(v));
        }
    }
    for (v, op) in [(p.indent_left_chars, P_DXC_LEFT), (p.indent_right_chars, P_DXC_RIGHT), (p.indent_first_chars, P_DXC_FIRST)] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            g.i16(op, (v * 100.0).round().clamp(-31_680.0, 31_680.0) as i16);
        }
    }
    if let Some(v) = p.space_before {
        g.u16(P_DYA_BEFORE, twips(v.max(0.0)) as u16);
    }
    if let Some(v) = p.space_after {
        g.u16(P_DYA_AFTER, twips(v.max(0.0)) as u16);
    }
    if let Some(ls) = p.line_spacing {
        // LSPD: dyaLine, then fMultLinespace.
        let (line, mult): (i16, i16) = match ls {
            LineSpacing::Multiple(m) => ((m * 240.0).round().clamp(1.0, 31_680.0) as i16, 1),
            LineSpacing::AtLeast(pt) => (twips(pt.max(0.0)), 0),
            LineSpacing::Exactly(pt) => (twips(pt.max(0.05)).saturating_neg(), 0),
        };
        let mut op = line.to_le_bytes().to_vec();
        op.extend_from_slice(&mult.to_le_bytes());
        g.raw(P_DYA_LINE, &op);
    }
    for (op, v) in [
        (P_KEEP, p.keep_lines),
        (P_KEEP_FOLLOW, p.keep_next),
        (P_PAGE_BREAK_BEFORE, p.page_break_before),
        (P_WIDOW, p.widow_control),
        (P_CONTEXTUAL, p.contextual_spacing),
        (P_NO_AUTO_HYPH, p.suppress_hyphens),
        (P_NO_LINE_NUMB, p.suppress_line_numbers),
        (P_BIDI, p.bidi),
    ] {
        if let Some(on) = v {
            g.u8(op, u8::from(on));
        }
    }
    if let Some(l) = p.outline_level {
        g.u8(P_OUTLVL, l.min(9));
    }
    if let Some(n) = p.numbering {
        g.u8(P_ILVL, n.level.min(8));
        g.u16(P_ILFO, if n.num == 0 { 0 } else { ctx.lfo(n.num) });
    }
    if let Some(c) = p.shading {
        g.var(P_SHD, &shd(Some(c)));
    }
    if let Some(b) = &p.borders {
        para_borders(&mut g, b);
    }
    if let Some(tabs) = &p.tabs {
        tab_stops(&mut g, tabs);
    }
    g
}

/// The paragraph's borders: the Word 97 form (palette colour) and the exact one, side by side.
fn para_borders(g: &mut Grpprl, b: &Borders) {
    for (i, side) in [b.top, b.left, b.bottom, b.right, b.between].iter().enumerate() {
        let Some(side) = side else { continue };
        if let (Some(&op80), Some(&op)) = (P_BRC80.get(i), P_BRC.get(i)) {
            g.raw(op80, &brc80(side));
            g.var(op, &brc(side));
        }
    }
}

/// A border's `brcType` ([MS-DOC] §2.9.22).
fn brc_type(s: BorderStyle) -> u8 {
    match s {
        BorderStyle::None => 0x00,
        BorderStyle::Single => 0x01,
        BorderStyle::Thick => 0x02,
        BorderStyle::Double => 0x03,
        BorderStyle::Dotted => 0x06,
        BorderStyle::Dashed => 0x07,
        BorderStyle::DotDash => 0x08,
        BorderStyle::Triple => 0x0A,
        BorderStyle::Wave => 0x14,
        BorderStyle::ThinThickSmallGap => 0x0B,
        BorderStyle::ThickThinSmallGap => 0x0C,
        BorderStyle::ThinThickThinSmallGap => 0x0D,
        BorderStyle::ThinThickMediumGap => 0x0E,
        BorderStyle::ThickThinMediumGap => 0x0F,
        BorderStyle::ThinThickThinMediumGap => 0x10,
        BorderStyle::ThinThickLargeGap => 0x11,
        BorderStyle::ThickThinLargeGap => 0x12,
        BorderStyle::ThinThickThinLargeGap => 0x13,
    }
}

/// A `Brc80` ([MS-DOC] §2.9.18): width in eighths of a point, type, palette colour, space.
pub(crate) fn brc80(b: &Border) -> [u8; 4] {
    if b.style == BorderStyle::None {
        return [0; 4];
    }
    let dpt = (b.width * 8.0).round().clamp(2.0, 255.0) as u8;
    let ico = b.color.map_or(0, nearest_ico);
    let space = b.space.round().clamp(0.0, 31.0) as u8;
    [dpt, brc_type(b.style), ico, space]
}

/// A `Brc` ([MS-DOC] §2.9.16): `COLORREF`, width in eighths of a point, type, space and flags.
pub(crate) fn brc(b: &Border) -> [u8; 8] {
    if b.style == BorderStyle::None {
        return [0; 8];
    }
    let cv = b.color.map_or(0, colorref);
    let [c0, c1, c2, c3] = cv.to_le_bytes();
    let dpt = (b.width * 8.0).round().clamp(2.0, 255.0) as u8;
    let space = b.space.round().clamp(0.0, 31.0) as u8;
    [c0, c1, c2, c3, dpt, brc_type(b.style), space, 0]
}

/// The paragraph's own tab stops as a `PChgTabsPapxOperand`: the cleared stops in `rgdxaDel`,
/// the others (with their alignment and leader) in `rgdxaAdd`, each list ascending.
fn tab_stops(g: &mut Grpprl, tabs: &[wordcraft_doc::props::TabStop]) {
    let mut del: Vec<i16> = tabs.iter().filter(|t| t.align == TabAlign::Clear).map(|t| twips(t.pos)).collect();
    let mut add: Vec<(i16, u8)> = tabs
        .iter()
        .filter(|t| t.align != TabAlign::Clear)
        .map(|t| {
            let jc = match t.align {
                TabAlign::Center => 1,
                TabAlign::Right => 2,
                TabAlign::Decimal => 3,
                TabAlign::Bar => 4,
                _ => 0,
            };
            let tlc = match t.leader {
                TabLeader::None => 0,
                TabLeader::Dot => 1,
                TabLeader::Hyphen => 2,
                TabLeader::Underscore => 3,
                TabLeader::MiddleDot => 5,
            };
            (twips(t.pos), jc | (tlc << 3))
        })
        .collect();
    del.sort_unstable();
    del.dedup();
    add.sort_by_key(|t| t.0);
    add.dedup_by_key(|t| t.0);
    // The operand's size byte holds 255: 64 stops each way is Word's own limit and fits.
    del.truncate(64);
    add.truncate(64);
    if del.is_empty() && add.is_empty() {
        return;
    }
    let mut op = vec![del.len() as u8];
    for d in &del {
        op.extend_from_slice(&d.to_le_bytes());
    }
    op.push(add.len() as u8);
    for (x, _) in &add {
        op.extend_from_slice(&x.to_le_bytes());
    }
    op.extend(add.iter().map(|(_, tbd)| *tbd));
    g.var(P_CHG_TABS_PAPX, &op);
}

/// The table membership of a paragraph inside a table cell (`sprmPFInTable`, `sprmPItap`); the
/// row-end mark adds `sprmPFTtp`.
pub(crate) fn in_table(g: &mut Grpprl, row_end: bool) {
    g.u8(P_IN_TABLE, 1);
    if row_end {
        g.u8(P_TTP, 1);
    }
    g.u32(P_ITAP, 1);
}

// Section sprms ([MS-DOC] §2.6.4).
const S_BKC: u16 = 0x3009;
const S_TITLE_PAGE: u16 = 0x300A;
const S_CCOLUMNS: u16 = 0x500B;
const S_DXA_COLUMNS: u16 = 0x900C;
const S_NFC_PGN: u16 = 0x300E;
const S_PGN_RESTART: u16 = 0x3011;
const S_LNC: u16 = 0x3013;
const S_NLNN_MOD: u16 = 0x5015;
const S_DXA_LNN: u16 = 0x9016;
const S_HDR_TOP: u16 = 0xB017;
const S_HDR_BOTTOM: u16 = 0xB018;
const S_VJC: u16 = 0x301A;
const S_LNN_MIN: u16 = 0x501B;
const S_PGN_START: u16 = 0x501C;
const S_ORIENTATION: u16 = 0x301D;
const S_XA_PAGE: u16 = 0xB01F;
const S_YA_PAGE: u16 = 0xB020;
const S_DXA_LEFT: u16 = 0xB021;
const S_DXA_RIGHT: u16 = 0xB022;
const S_DYA_TOP: u16 = 0x9023;
const S_DYA_BOTTOM: u16 = 0x9024;
const S_DZA_GUTTER: u16 = 0xB025;
const S_EVENLY_SPACED: u16 = 0x3005;
const S_BIDI: u16 = 0x3228;

/// The SEPX grpprl of a section: every page property, so the section reads the same whatever
/// the reader's defaults.
pub(crate) fn sepx(s: &SectionProps) -> Grpprl {
    let mut g = Grpprl::default();
    g.u8(
        S_BKC,
        match s.start {
            SectionStart::Continuous => 0,
            SectionStart::NextColumn => 1,
            SectionStart::NextPage => 2,
            SectionStart::EvenPage => 3,
            SectionStart::OddPage => 4,
        },
    );
    if s.title_page {
        g.u8(S_TITLE_PAGE, 1);
    }
    let cols = s.columns.count.clamp(1, 44);
    if cols > 1 {
        g.u16(S_CCOLUMNS, (cols - 1) as u16);
        g.u16(S_DXA_COLUMNS, twips(s.columns.space.max(0.0)) as u16);
        g.u8(S_EVENLY_SPACED, 1);
    }
    if let Some(start) = s.page_num_start {
        g.u8(S_PGN_RESTART, 1);
        g.u16(S_PGN_START, start.min(0x7FFF) as u16);
    }
    if s.page_num_format != NumFormat::Decimal {
        g.u8(S_NFC_PGN, nfc(s.page_num_format));
    }
    if let Some(ln) = &s.line_numbers {
        g.u8(
            S_LNC,
            match ln.restart {
                wordcraft_doc::section::LineNumberRestart::Page => 0,
                wordcraft_doc::section::LineNumberRestart::Section => 1,
                wordcraft_doc::section::LineNumberRestart::Continuous => 2,
            },
        );
        g.u16(S_NLNN_MOD, ln.count_by.clamp(1, 100) as u16);
        g.i16(S_DXA_LNN, twips(ln.distance.max(0.0)));
        g.u16(S_LNN_MIN, ln.start.saturating_sub(1).min(0x7FFF) as u16);
    }
    g.u8(S_ORIENTATION, if s.landscape { 2 } else { 1 });
    g.u16(S_XA_PAGE, twips(s.page_w.max(7.2)) as u16);
    g.u16(S_YA_PAGE, twips(s.page_h.max(7.2)) as u16);
    g.u16(S_DXA_LEFT, twips(s.margin_left.max(0.0)) as u16);
    g.u16(S_DXA_RIGHT, twips(s.margin_right.max(0.0)) as u16);
    g.i16(S_DYA_TOP, twips(s.margin_top));
    g.i16(S_DYA_BOTTOM, twips(s.margin_bottom));
    g.u16(S_DZA_GUTTER, twips(s.gutter.max(0.0)) as u16);
    g.u16(S_HDR_TOP, twips(s.header.max(0.0)) as u16);
    g.u16(S_HDR_BOTTOM, twips(s.footer.max(0.0)) as u16);
    let vjc = match s.valign {
        PageVAlign::Top => 0,
        PageVAlign::Center => 1,
        PageVAlign::Justify => 2,
        PageVAlign::Bottom => 3,
    };
    if vjc != 0 {
        g.u8(S_VJC, vjc);
    }
    if s.rtl {
        g.u8(S_BIDI, 1);
    }
    g
}

/// A number format as an `MSONFC` ([MS-OSHARED] §2.2.1.3): its place in the ST_NumberFormat
/// order up to 0x27, decimal past it (the formats Word 97 never had), 0xFF for none.
pub(crate) fn nfc(f: NumFormat) -> u8 {
    match f {
        NumFormat::None => 0xFF,
        f => NumFormat::ALL.iter().position(|x| *x == f).filter(|&i| i <= 0x27).map_or(0, |i| i as u8),
    }
}
