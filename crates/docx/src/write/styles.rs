use super::{Document, Level, LevelSuffix, ParaProps, Style, StyleKind, TablePart, VAlign, W, props, xml};

/// Built-in names Word keeps in lower case in styles.xml.
fn file_style_name(name: &str) -> String {
    if let Some((f, _)) = crate::read::LOWER_NAMES.iter().find(|(_, ui)| ui.eq_ignore_ascii_case(name)) {
        return (*f).to_string();
    }
    for (pre, f) in [("Heading ", "heading "), ("TOC ", "toc "), ("Index ", "index ")] {
        if let Some(d) = name.strip_prefix(pre)
            && d.len() == 1
            && d.chars().all(|c| c.is_ascii_digit())
        {
            return format!("{f}{d}");
        }
    }
    name.to_string()
}

pub(super) fn styles_xml(doc: &Document) -> Vec<u8> {
    let s = &doc.styles;
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:styles", &ns_refs);
    w.open("w:docDefaults", &[]);
    w.open("w:rPrDefault", &[]);
    w.open("w:rPr", &[]);
    if s.default_chr.size.is_none() {
        // The reader reads an omitted `w:sz` as 10 pt, but the resolver falls back to
        // DEFAULT_FONT_SIZE, so writing nothing would change the size on reopening.
        let mut chr = s.default_chr.clone();
        chr.size = Some(wordcraft_doc::props::DEFAULT_FONT_SIZE);
        props::rpr_inner(&mut w, &chr);
    } else {
        props::rpr_inner(&mut w, &s.default_chr);
    }
    w.close("w:rPr");
    w.close("w:rPrDefault");
    if s.default_para_present || s.default_para != wordcraft_doc::props::ParaProps::default() {
        w.open("w:pPrDefault", &[]);
        w.open("w:pPr", &[]);
        props::ppr_inner(&mut w, &s.default_para, false, None);
        w.close("w:pPr");
        w.close("w:pPrDefault");
    }
    w.close("w:docDefaults");
    if let Some(latent) = s.latent_styles.as_deref() {
        w.raw(latent);
    }
    // Kinds whose default style the file named (`w:default`); the English ids stand in otherwise.
    let flagged: Vec<StyleKind> = s.styles.iter().filter(|st| st.default).map(|st| st.kind).collect();
    for st in &s.styles {
        style_xml(&mut w, st, &flagged);
    }
    w.close("w:styles");
    w.into_bytes()
}

fn style_xml(w: &mut W, st: &Style, flagged: &[StyleKind]) {
    let ty = match st.kind {
        StyleKind::Paragraph => "paragraph",
        StyleKind::Character => "character",
        StyleKind::Table => "table",
        StyleKind::Numbering => "numbering",
    };
    let english_default = matches!(
        (st.kind, st.id.as_str()),
        (StyleKind::Paragraph, "Normal")
            | (StyleKind::Character, "DefaultParagraphFont")
            | (StyleKind::Table, "TableNormal")
            | (StyleKind::Numbering, "NoList")
    );
    let is_default = st.default || (english_default && !flagged.contains(&st.kind));
    let mut a = vec![("w:type", ty), ("w:styleId", st.id.as_str())];
    if is_default {
        a.push(("w:default", "1"));
    }
    if !st.builtin {
        a.push(("w:customStyle", "1"));
    }
    w.open("w:style", &a);
    w.val("w:name", &file_style_name(if st.name.is_empty() { &st.id } else { &st.name }));
    if let Some(b) = &st.based_on {
        w.val("w:basedOn", b);
    }
    if let Some(b) = &st.next {
        w.val("w:next", b);
    }
    if let Some(b) = &st.linked {
        w.val("w:link", b);
    }
    if let Some(p) = st.priority {
        w.val("w:uiPriority", &p.to_string());
    }
    if st.hidden {
        w.empty("w:semiHidden", &[]);
        w.empty("w:unhideWhenUsed", &[]);
    }
    if st.quick {
        w.empty("w:qFormat", &[]);
    }
    if st.locked {
        w.empty("w:locked", &[]);
    }
    if !st.para.is_empty() && st.kind != StyleKind::Character {
        w.open("w:pPr", &[]);
        props::ppr_inner(w, &st.para, false, None);
        w.close("w:pPr");
    }
    if props::has_rpr(&st.chr) && st.kind != StyleKind::Numbering {
        w.open("w:rPr", &[]);
        props::rpr_inner(w, &st.chr);
        w.close("w:rPr");
    }
    if let Some(t) = &st.table {
        if t.borders.is_some() || t.cell_margins.is_some() || t.band_size.is_some() || t.col_band_size.is_some() {
            w.open("w:tblPr", &[]);
            if let Some(n) = t.band_size {
                w.val("w:tblStyleRowBandSize", &n.min(1000).to_string());
            }
            if let Some(n) = t.col_band_size {
                w.val("w:tblStyleColBandSize", &n.min(1000).to_string());
            }
            if let Some(b) = &t.borders {
                props::borders(w, "w:tblBorders", b, Some("w:insideH"), &[]);
            }
            if let Some(m) = &t.cell_margins {
                props::margins(w, "w:tblCellMar", m);
            }
            w.close("w:tblPr");
        }
        // The conditional formatting parts (ECMA-376 §17.7.6): paragraph and run properties,
        // then cell borders, shading and vertical alignment. The whole table's text formatting
        // is the style's own (above).
        for part in TablePart::ALL {
            let f = t.part(part);
            let ppr = ParaProps { align: f.align, ..ParaProps::default() };
            let has_rpr = props::has_rpr(&f.chr);
            // The whole table's borders are its table borders (`w:tblPr` above).
            let borders = f.borders.filter(|_| part != TablePart::WholeTable);
            let has_tc = f.fill.is_some() || borders.is_some() || f.v_align.is_some();
            if !has_rpr && !has_tc && f.align.is_none() {
                continue;
            }
            w.open("w:tblStylePr", &[("w:type", part.ooxml())]);
            if f.align.is_some() {
                w.open("w:pPr", &[]);
                props::ppr_inner(w, &ppr, false, None);
                w.close("w:pPr");
            }
            if has_rpr {
                w.open("w:rPr", &[]);
                props::rpr_inner(w, &f.chr);
                w.close("w:rPr");
            }
            if has_tc {
                w.open("w:tcPr", &[]);
                if let Some(bs) = borders {
                    props::borders(w, "w:tcBorders", &bs, Some("w:insideH"), &[]);
                }
                if let Some(c) = f.fill {
                    w.empty("w:shd", &[("w:val", "clear"), ("w:color", "auto"), ("w:fill", &c.hex())]);
                }
                if let Some(v) = f.v_align {
                    w.val(
                        "w:vAlign",
                        match v {
                            VAlign::Top => "top",
                            VAlign::Center => "center",
                            VAlign::Bottom => "bottom",
                        },
                    );
                }
                w.close("w:tcPr");
            }
            w.close("w:tblStylePr");
        }
    }
    w.close("w:style");
}

pub(super) fn numbering_xml(doc: &Document) -> Vec<u8> {
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:numbering", &ns_refs);
    let mut nsids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for a in &doc.numbering.abstracts {
        w.open("w:abstractNum", &[("w:abstractNumId", &a.id.to_string())]);
        // CT_AbstractNum order: nsid, multiLevelType, tmpl, name… A copied definition repeats
        // its source's nsid; only the first keeps it, so Word doesn't take them for one list.
        if let Some(n) = a.nsid.as_deref().filter(|n| nsids.insert(n)) {
            w.val("w:nsid", n);
        }
        let derived = if a.levels.len() > 1 { "multilevel" } else { "singleLevel" };
        w.val("w:multiLevelType", a.multi_level_type.as_deref().unwrap_or(derived));
        if let Some(t) = &a.tmpl {
            w.val("w:tmpl", t);
        }
        if let Some(n) = &a.name {
            w.val("w:name", n);
        }
        for (i, l) in a.levels.iter().take(9).enumerate() {
            write_level(&mut w, i, l);
        }
        w.close("w:abstractNum");
    }
    for n in &doc.numbering.nums {
        w.open("w:num", &[("w:numId", &n.id.to_string())]);
        w.val("w:abstractNumId", &n.abstract_id.to_string());
        for lvl in 0..9u8 {
            let start = n.start_overrides.iter().find(|(l, _)| (*l).min(8) == lvl).map(|(_, s)| *s);
            let level = n.level_overrides.iter().find(|(l, _)| (*l).min(8) == lvl).map(|(_, l)| l);
            if start.is_none() && level.is_none() {
                continue;
            }
            w.open("w:lvlOverride", &[("w:ilvl", &lvl.to_string())]);
            if let Some(start) = start {
                w.val("w:startOverride", &start.to_string());
            }
            if let Some(level) = level {
                write_level(&mut w, lvl as usize, level);
            }
            w.close("w:lvlOverride");
        }
        w.close("w:num");
    }
    w.close("w:numbering");
    w.into_bytes()
}

/// One `w:lvl` (list level `i`).
fn write_level(w: &mut W, i: usize, l: &Level) {
    w.open("w:lvl", &[("w:ilvl", &i.to_string())]);
    w.val("w:start", &l.start.to_string());
    w.val("w:numFmt", l.format.ooxml());
    if !l.restart {
        w.val("w:lvlRestart", "0");
    } else if let Some(k) = l.restart_after.filter(|k| (1..=9).contains(k)) {
        w.val("w:lvlRestart", &k.to_string());
    }
    if let Some(s) = &l.style {
        w.val("w:pStyle", s);
    }
    if l.legal {
        w.empty("w:isLgl", &[]);
    }
    match l.suffix {
        LevelSuffix::Tab => {}
        LevelSuffix::Space => w.val("w:suff", "space"),
        LevelSuffix::Nothing => w.val("w:suff", "nothing"),
    }
    w.val("w:lvlText", &l.text);
    w.val("w:lvlJc", props::align_val(l.align));
    w.open("w:pPr", &[]);
    if let Some(t) = l.tab.filter(|t| t.is_finite()) {
        w.open("w:tabs", &[]);
        w.empty("w:tab", &[("w:val", "num"), ("w:pos", &crate::units::twips(t.clamp(-1584.0, 1584.0)))]);
        w.close("w:tabs");
    }
    let ind = crate::units::twips(l.indent);
    if l.hanging >= 0.0 {
        w.empty("w:ind", &[("w:left", &ind), ("w:hanging", &crate::units::twips(l.hanging))]);
    } else {
        w.empty("w:ind", &[("w:left", &ind), ("w:firstLine", &crate::units::twips(-l.hanging))]);
    }
    w.close("w:pPr");
    props::rpr(w, &l.chr);
    w.close("w:lvl");
}
