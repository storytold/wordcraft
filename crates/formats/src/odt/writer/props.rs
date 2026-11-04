//! Style properties written for text and paragraphs (ODF 1.3 §17.6, §17.8): character
//! formatting, scripts and languages, spacing, tab stops, alignment and borders.

use super::{Align, Border, BorderStyle, Borders, Fmt, Kind, LineSpacing, Rgb, model, pt, x};

/// `style:text-properties` attributes for what only [`Fmt::extra`] holds (underline and strike
/// are written with the rest); `size` is the text's size, which a raised position is a share of.
pub(in crate::odt) fn extra_props(x: &wordcraft_doc::props::CharProps, size: Option<f32>) -> String {
    let mut p = String::new();
    let on = |v: Option<bool>| v == Some(true);
    if on(x.caps) {
        p.push_str(" fo:text-transform=\"uppercase\"");
    }
    if on(x.small_caps) {
        p.push_str(" fo:font-variant=\"small-caps\"");
    }
    if on(x.hidden) {
        p.push_str(" text:display=\"none\"");
    }
    if let Some(k) = x.kern.filter(|k| k.is_finite()) {
        p.push_str(&format!(" style:letter-kerning=\"{}\"", k > 0.0));
    }
    if let Some(s) = x.spacing.filter(|s| s.is_finite() && *s != 0.0) {
        p.push_str(&format!(" fo:letter-spacing=\"{}\"", pt(s)));
    }
    if let Some(s) = x.scale.filter(|s| s.is_finite() && *s > 0.0 && *s != 100.0) {
        p.push_str(&format!(" style:text-scale=\"{}%\"", s.round()));
    }
    if let Some(m) = x.emphasis {
        use wordcraft_doc::props::EmphasisMark as E;
        p.push_str(match m {
            E::None => " style:text-emphasize=\"none\"",
            E::Dot => " style:text-emphasize=\"dot above\"",
            E::Comma => " style:text-emphasize=\"accent above\"",
            E::Circle => " style:text-emphasize=\"circle above\"",
            E::UnderDot => " style:text-emphasize=\"dot below\"",
        });
    }
    if let Some(r) = x.position.filter(|r| r.is_finite() && *r != 0.0) {
        let size = size.filter(|s| s.is_finite() && *s >= 1.0).unwrap_or(12.0);
        let pct = r / size * 100.0;
        p.push_str(&format!(" style:text-position=\"{}% 100%\"", (pct * 100.0).round() / 100.0));
    }
    if on(x.outline) {
        p.push_str(" style:text-outline=\"true\"");
    }
    if on(x.shadow) {
        p.push_str(" fo:text-shadow=\"1pt 1pt\"");
    }
    if on(x.emboss) {
        p.push_str(" style:font-relief=\"embossed\"");
    } else if on(x.engrave) {
        p.push_str(" style:font-relief=\"engraved\"");
    }
    p.push_str(&script_attrs(x));
    if let Some(l) = x.lang.as_deref() {
        let mut parts = l.split(['-', '_']);
        let ok = |s: &&str| (1..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric());
        if let Some(lang) = parts.next().filter(ok) {
            p.push_str(&format!(" fo:language=\"{lang}\""));
            if let Some(country) = parts.next().filter(ok) {
                p.push_str(&format!(" fo:country=\"{country}\""));
            }
        }
    }
    p
}

/// `style:text-properties` attributes for character properties `cp` (a style's look).
pub(in crate::odt) fn props_text(cp: &wordcraft_doc::props::CharProps) -> String {
    use wordcraft_doc::props::{TextColor, Underline, VertAlign};
    let f = Fmt {
        bold: cp.bold == Some(true),
        italic: cp.italic == Some(true),
        underline: cp.underline.is_some_and(|u| u != Underline::None),
        strike: cp.strike == Some(true),
        sup: cp.vert_align == Some(VertAlign::Superscript),
        sub: cp.vert_align == Some(VertAlign::Subscript),
        color: match cp.color {
            Some(TextColor::Rgb(c)) => Some(c),
            _ => None,
        },
        size: cp.size,
        font: cp.font.as_deref().map(std::sync::Arc::from),
        extra: Some(Box::new(cp.clone())),
        ..Default::default()
    };
    let mut p = text_props(&f);
    // What a style turns off that the style it is based on may turn on: said so, or the parent's
    // bold, italic, underline or strike-through would show through.
    if cp.bold == Some(false) {
        p.push_str(" fo:font-weight=\"normal\" style:font-weight-asian=\"normal\"");
        if cp.bold_cs.is_none() {
            p.push_str(" style:font-weight-complex=\"normal\"");
        }
    }
    if cp.italic == Some(false) {
        p.push_str(" fo:font-style=\"normal\" style:font-style-asian=\"normal\"");
        if cp.italic_cs.is_none() {
            p.push_str(" style:font-style-complex=\"normal\"");
        }
    }
    if cp.underline == Some(Underline::None) {
        p.push_str(" style:text-underline-style=\"none\"");
    }
    if cp.strike == Some(false) && cp.double_strike != Some(true) {
        p.push_str(" style:text-line-through-style=\"none\"");
    }
    p
}

/// `fo:text-align` for `a` (ODF 1.3 §20.223).
pub(in crate::odt) fn align_attr(a: Option<Align>) -> &'static str {
    match a {
        Some(Align::Center) => " fo:text-align=\"center\"",
        Some(Align::Right) => " fo:text-align=\"end\"",
        Some(Align::Justify) | Some(Align::Distribute) => " fo:text-align=\"justify\"",
        Some(Align::Left) => " fo:text-align=\"start\"",
        None => "",
    }
}

/// `fo:language`/`fo:country` (with `suffix`: `-asian`, `-complex` on `style:`) for BCP 47 tag
/// `tag`.
pub(in crate::odt) fn lang_attrs(tag: &str, suffix: &str) -> String {
    let mut parts = tag.split(['-', '_']);
    let ok = |s: &&str| (1..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric());
    let ns = if suffix.is_empty() { "fo" } else { "style" };
    let mut p = String::new();
    if let Some(lang) = parts.next().filter(ok) {
        p.push_str(&format!(" {ns}:language{suffix}=\"{lang}\""));
        if let Some(country) = parts.next().filter(ok) {
            p.push_str(&format!(" {ns}:country{suffix}=\"{country}\""));
        }
    }
    p
}

/// The Asian and complex-script attributes of `x`: fonts, complex size, weight, posture and the
/// two languages.
pub(in crate::odt) fn script_attrs(x: &wordcraft_doc::props::CharProps) -> String {
    let mut p = String::new();
    if let Some(f) = x.font_ea.as_deref() {
        p.push_str(&format!(" style:font-family-asian=\"'{}'\"", crate::odt::writer::x(&f.replace('\'', ""))));
    }
    if let Some(f) = x.font_cs.as_deref() {
        p.push_str(&format!(" style:font-family-complex=\"'{}'\"", crate::odt::writer::x(&f.replace('\'', ""))));
    }
    if let Some(s) = x.size_cs.filter(|s| s.is_finite()) {
        p.push_str(&format!(" style:font-size-complex=\"{}\"", pt(s)));
    }
    if let Some(b) = x.bold_cs {
        p.push_str(if b { " style:font-weight-complex=\"bold\"" } else { " style:font-weight-complex=\"normal\"" });
    }
    if let Some(i) = x.italic_cs {
        p.push_str(if i { " style:font-style-complex=\"italic\"" } else { " style:font-style-complex=\"normal\"" });
    }
    if let Some(l) = x.lang_ea.as_deref() {
        p.push_str(&lang_attrs(l, "-asian"));
    }
    if let Some(l) = x.lang_bidi.as_deref() {
        p.push_str(&lang_attrs(l, "-complex"));
    }
    p
}

pub(in crate::odt) fn text_props(f: &Fmt) -> String {
    let mut p = String::new();
    // Complex-script weight and posture of their own are written with the other script extras.
    let own_cs = |pick: fn(&wordcraft_doc::props::CharProps) -> Option<bool>| f.extra.as_deref().and_then(pick).is_some();
    if f.bold {
        p.push_str(" fo:font-weight=\"bold\" style:font-weight-asian=\"bold\"");
        if !own_cs(|x| x.bold_cs) {
            p.push_str(" style:font-weight-complex=\"bold\"");
        }
    }
    if f.italic {
        p.push_str(" fo:font-style=\"italic\" style:font-style-asian=\"italic\"");
        if !own_cs(|x| x.italic_cs) {
            p.push_str(" style:font-style-complex=\"italic\"");
        }
    }
    let extra = f.extra.as_deref();
    if f.underline {
        use wordcraft_doc::props::Underline as U;
        let (style, kind, width, mode) = match extra.and_then(|e| e.underline) {
            Some(U::Double) => ("solid", "double", "auto", ""),
            Some(U::Thick) => ("solid", "single", "bold", ""),
            Some(U::Dotted) => ("dotted", "single", "auto", ""),
            Some(U::Dash) => ("dash", "single", "auto", ""),
            Some(U::DotDash) => ("dot-dash", "single", "auto", ""),
            Some(U::DotDotDash) => ("dot-dot-dash", "single", "auto", ""),
            Some(U::Wave) => ("wave", "single", "auto", ""),
            Some(U::DoubleWave) => ("wave", "double", "auto", ""),
            Some(U::Words) => ("solid", "single", "auto", " style:text-underline-mode=\"skip-white-space\""),
            _ => ("solid", "single", "auto", ""),
        };
        let color =
            extra.and_then(|e| e.underline_color).map(|c| format!("#{}", c.hex().to_ascii_lowercase())).unwrap_or_else(|| "font-color".into());
        p.push_str(&format!(
            " style:text-underline-style=\"{style}\" style:text-underline-type=\"{kind}\" style:text-underline-width=\"{width}\" style:text-underline-color=\"{color}\"{mode}"
        ));
    }
    if f.strike || extra.is_some_and(|e| e.double_strike == Some(true)) {
        p.push_str(" style:text-line-through-style=\"solid\"");
        if extra.is_some_and(|e| e.double_strike == Some(true)) {
            p.push_str(" style:text-line-through-type=\"double\"");
        }
    }
    if let Some(e) = extra {
        if (f.sup || f.sub) && e.position.is_some() {
            // One `style:text-position` only (a repeated attribute isn't well-formed XML):
            // superscript or subscript, written below, wins over a raised or lowered position.
            let e = wordcraft_doc::props::CharProps { position: None, ..e.clone() };
            p.push_str(&extra_props(&e, f.size));
        } else {
            p.push_str(&extra_props(e, f.size));
        }
    }
    // A border around the text (Word's `w:bdr`): LibreOffice's text border, which ODF 1.3 has no
    // attribute of its own for (`loext:border`, `loext:padding`).
    if let Some(b) = extra.and_then(|e| e.border).filter(|b| b.is_visible()) {
        p.push_str(&format!(" loext:border=\"{}\" loext:padding=\"{}\"", odf_border(Some(b)), pt(b.space.clamp(0.0, 31.0))));
    }
    if f.sup {
        p.push_str(" style:text-position=\"super 58%\"");
    } else if f.sub {
        p.push_str(" style:text-position=\"sub 58%\"");
    }
    if let Some(c) = f.color {
        p.push_str(&format!(" fo:color=\"#{}\"", c.hex().to_ascii_lowercase()));
    }
    if let Some(c) = f.background {
        p.push_str(&format!(" fo:background-color=\"#{}\"", c.hex().to_ascii_lowercase()));
    }
    if let Some(s) = f.size {
        p.push_str(&format!(" fo:font-size=\"{}\"", pt(s)));
    }
    if f.code {
        p.push_str(&format!(" fo:font-family=\"'{}'\" style:font-family-generic=\"modern\" style:font-pitch=\"fixed\"", model::MONO_FONT));
    } else if let Some(fam) = &f.font {
        p.push_str(&format!(" fo:font-family=\"'{}'\"", x(&fam.replace('\'', ""))));
    }
    p
}

/// `style:paragraph-properties` attributes for the spacing, indents and line spacing `sp` sets
/// (ODF 1.3 §20.204–209, §20.225, §20.317).
pub(in crate::odt) fn spacing_props(sp: &model::Spacing) -> String {
    let mut p = String::new();
    for (k, v) in [
        ("fo:margin-top", sp.before),
        ("fo:margin-bottom", sp.after),
        ("fo:margin-left", sp.left),
        ("fo:margin-right", sp.right),
        ("fo:text-indent", sp.first),
    ] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            p.push_str(&format!(" {k}=\"{}\"", pt(v)));
        }
    }
    match sp.line {
        Some(LineSpacing::Multiple(m)) if m.is_finite() => p.push_str(&format!(" fo:line-height=\"{}%\"", (m * 100.0).round().max(1.0))),
        Some(LineSpacing::Exactly(h)) if h.is_finite() => p.push_str(&format!(" fo:line-height=\"{}\"", pt(h))),
        Some(LineSpacing::AtLeast(h)) if h.is_finite() => p.push_str(&format!(" style:line-height-at-least=\"{}\"", pt(h))),
        _ => {}
    }
    p
}

/// `style:tab-stops` (ODF 1.3 §17.7) for `tabs`, measured from the margin (the package's
/// settings say so); bar and clearing stops have no ODF form and are left out.
pub(in crate::odt) fn tab_stops(tabs: &[wordcraft_doc::props::TabStop]) -> String {
    use wordcraft_doc::props::{TabAlign, TabLeader};
    let mut s = String::from("<style:tab-stops>");
    for t in tabs.iter().filter(|t| t.pos.is_finite()) {
        let kind = match t.align {
            TabAlign::Left => "",
            TabAlign::Center => " style:type=\"center\"",
            TabAlign::Right => " style:type=\"right\"",
            TabAlign::Decimal => " style:type=\"char\" style:char=\".\"",
            TabAlign::Bar | TabAlign::Clear => continue,
        };
        let leader = match t.leader {
            TabLeader::None => "",
            TabLeader::Dot => " style:leader-style=\"dotted\" style:leader-text=\".\"",
            TabLeader::Hyphen => " style:leader-style=\"solid\" style:leader-text=\"-\"",
            TabLeader::Underscore => " style:leader-style=\"solid\" style:leader-text=\"_\"",
            TabLeader::MiddleDot => " style:leader-style=\"dotted\" style:leader-text=\"·\"",
        };
        s.push_str(&format!("<style:tab-stop style:position=\"{}\"{kind}{leader}/>", pt(t.pos)));
    }
    s.push_str("</style:tab-stops>");
    s
}

/// An ODF style name (an XML NCName, ODF 1.3 §19.75) for document style id `id`: letters,
/// digits and `-`, `.` kept, anything else as `_xHH_` (its code point in hex), and a leading
/// digit, `-` or `.` escaped too.
pub(in crate::odt) fn odf_name(id: &str) -> String {
    if let Some((name, _)) = generated_style(id) {
        return name;
    }
    let mut out = String::new();
    for (i, c) in id.chars().enumerate() {
        let ok = c.is_alphanumeric() || (i > 0 && (c == '-' || c == '.' || c.is_ascii_digit()));
        if ok && !(i == 0 && c.is_ascii_digit()) {
            out.push(c);
        } else {
            out.push_str(&format!("_x{:X}_", c as u32));
        }
    }
    if out.is_empty() { "Custom".into() } else { out }
}

/// `fo:border-*` and `fo:padding-*` attributes for paragraph borders `b`, when a side shows.
pub(in crate::odt) fn border_attrs(b: Option<Borders>) -> String {
    let mut p = String::new();
    // A side without a line may still hold padding (read from ODF), which keeps its room.
    let padded = |l: &Border| !l.is_visible() && l.space.is_finite() && l.space > 0.0;
    let Some(b) = b.filter(|b| b.any_visible() || [b.top, b.left, b.bottom, b.right].iter().flatten().any(padded)) else { return p };
    for (side, line) in [("top", b.top), ("left", b.left), ("bottom", b.bottom), ("right", b.right)] {
        let shown = line.filter(|l| l.is_visible());
        p.push_str(&format!(" fo:border-{side}=\"{}\"", odf_border(shown)));
        if let Some(l) = shown {
            p.push_str(&format!(" fo:padding-{side}=\"{}\"", pt(l.space.clamp(0.0, 31.0))));
        } else if let Some(l) = line.filter(padded) {
            p.push_str(&format!(" fo:padding-{side}=\"{}\"", pt(l.space.min(1584.0))));
        }
    }
    p
}

/// An ODF border value (ODF 1.3 §20.183) for `b`; `none` for no line.
pub(in crate::odt) fn odf_border(b: Option<Border>) -> String {
    let Some(b) = b.filter(|b| b.is_visible() && b.width.is_finite()) else { return "none".into() };
    let style = match b.style {
        BorderStyle::Double | BorderStyle::Triple => "double",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Dashed => "dashed",
        BorderStyle::DotDash => "dot-dash",
        _ => "solid",
    };
    let color = b.color.unwrap_or(Rgb(0, 0, 0)).hex().to_ascii_lowercase();
    format!("{} {style} #{color}", pt(b.width.clamp(0.05, 12.0)))
}

pub(in crate::odt) fn parent_style(k: Kind) -> &'static str {
    match k {
        Kind::Heading(1) => "Heading_20_1",
        Kind::Heading(2) => "Heading_20_2",
        Kind::Heading(3) => "Heading_20_3",
        Kind::Heading(4) => "Heading_20_4",
        Kind::Heading(5) => "Heading_20_5",
        Kind::Heading(_) => "Heading_20_6",
        Kind::Title => "Title",
        Kind::Quote => "Quotations",
        Kind::Code => "Preformatted_20_Text",
        Kind::Normal | Kind::Rule => "Standard",
    }
}

/// The LibreOffice style (name and display name) standing for WordCraft's table of contents,
/// table of figures and index heading styles, which LibreOffice's indexes are generated in:
/// TOC 1–9 are Contents 1–9, TOC Heading is Contents Heading, Table of Figures is Figure Index 1.
pub(in crate::odt) fn generated_style(id: &str) -> Option<(String, String)> {
    let n = id.strip_prefix("TOC").and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n));
    Some(match (id, n) {
        (_, Some(n)) => (format!("Contents_20_{n}"), format!("Contents {n}")),
        ("TOCHeading", _) => ("Contents_20_Heading".into(), "Contents Heading".into()),
        ("TableofFigures", _) => ("Figure_20_Index_20_1".into(), "Figure Index 1".into()),
        _ => return None,
    })
}

/// The WordCraft style (id and name) a LibreOffice index style `odt` stands for (the reverse
/// of [`generated_style`]).
pub(in crate::odt) fn generating_style(odt: &str) -> Option<(String, String)> {
    let n = odt.strip_prefix("Contents_20_").and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=10).contains(n));
    Some(match (odt, n) {
        (_, Some(n)) => (format!("TOC{}", n.min(9)), format!("TOC {}", n.min(9))),
        ("Contents_20_Heading", _) => ("TOCHeading".into(), "TOC Heading".into()),
        ("Figure_20_Index_20_1", _) => ("TableofFigures".into(), "Table of Figures".into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superscript_raised_text_has_one_text_position() {
        for (sup, sub) in [(true, false), (false, true)] {
            let f = Fmt {
                sup,
                sub,
                size: Some(12.0),
                extra: Some(Box::new(wordcraft_doc::props::CharProps { position: Some(3.0), ..Default::default() })),
                ..Default::default()
            };
            let p = text_props(&f);
            assert_eq!(p.matches("style:text-position=").count(), 1, "{p}");
        }
        // Without superscript or subscript the position is still written.
        let f = Fmt {
            size: Some(12.0),
            extra: Some(Box::new(wordcraft_doc::props::CharProps { position: Some(3.0), ..Default::default() })),
            ..Default::default()
        };
        assert!(text_props(&f).contains("style:text-position=\"25% 100%\""), "{}", text_props(&f));
    }
}
