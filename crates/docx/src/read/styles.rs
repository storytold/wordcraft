use super::*;

/// Built-in style names Word stores in lower case, and how the UI shows them.
pub(crate) const LOWER_NAMES: &[(&str, &str)] = &[
    ("caption", "Caption"),
    ("header", "Header"),
    ("footer", "Footer"),
    ("footnote text", "Footnote Text"),
    ("footnote reference", "Footnote Reference"),
    ("endnote text", "Endnote Text"),
    ("endnote reference", "Endnote Reference"),
    ("annotation text", "Annotation Text"),
    ("annotation reference", "Annotation Reference"),
    ("annotation subject", "Annotation Subject"),
    ("table of figures", "Table of Figures"),
    ("normal indent", "Normal Indent"),
    ("line number", "Line Number"),
    ("page number", "Page Number"),
    ("index heading", "Index Heading"),
    ("envelope address", "Envelope Address"),
    ("envelope return", "Envelope Return"),
    ("toa heading", "TOA Heading"),
    ("table of authorities", "Table of Authorities"),
];

/// File style name → display name.
pub(crate) fn ui_style_name(n: &str) -> String {
    if let Some((_, ui)) = LOWER_NAMES.iter().find(|(f, _)| *f == n) {
        return (*ui).to_string();
    }
    for (pre, ui) in [("heading ", "Heading "), ("toc ", "TOC "), ("index ", "Index ")] {
        if let Some(d) = n.strip_prefix(pre)
            && d.len() == 1
            && d.chars().all(|c| c.is_ascii_digit())
        {
            return format!("{ui}{d}");
        }
    }
    n.to_string()
}

impl Reader<'_> {
    pub(super) fn read_styles(&mut self, path: &str) -> Result<(), DocxError> {
        /// Largest `w:latentStyles` kept (Word's has ~380 exceptions, ~40 KB).
        const MAX_LATENT_STYLES_XML: usize = 1 << 20;
        let Some(root) = self.xml(path)? else { return Ok(()) };
        // The default paragraph style must be called `Normal` in our model.
        let has_normal = root.children("w:style").any(|s| s.attr("w:styleId") == Some("Normal"));
        if !has_normal
            && let Some(def) = root
                .children("w:style")
                .find(|s| s.attr("w:type") == Some("paragraph") && s.attr("w:default").is_some_and(|v| v == "1" || v == "true"))
            && let Some(id) = def.attr("w:styleId")
        {
            self.pc.alias.insert(id.to_string(), "Normal".into());
        }
        // Word in other languages names the TOC styles' ids in that language (`toc 1` is `Sumrio1`
        // in Portuguese): they take the engine's ids, which Update Table and the writer's TOC
        // recognise the entries by (#3889). Not when the file or an earlier style has that id.
        let mut taken: Vec<String> = root.children("w:style").take(10_000).filter_map(|s| s.attr("w:styleId")).map(str::to_string).collect();
        for s in root.children("w:style").take(10_000).filter(|s| s.attr("w:type").is_none_or(|t| t == "paragraph")) {
            let (Some(id), Some(name)) = (s.attr("w:styleId"), s.child_val("w:name")) else { continue };
            let canonical = match name.trim().to_ascii_lowercase().as_str() {
                "toc heading" => "TOCHeading".to_string(),
                n => match n.strip_prefix("toc ").and_then(|d| d.parse::<u8>().ok()).filter(|d| (1..=9).contains(d)) {
                    Some(d) => format!("TOC{d}"),
                    None => continue,
                },
            };
            if !taken.contains(&canonical) && !self.pc.alias.contains_key(id) {
                taken.push(canonical.clone());
                self.pc.alias.insert(id.to_string(), canonical);
            }
        }
        let mut sheet = StyleSheet::empty();
        sheet.default_para_present = root.child("w:docDefaults").and_then(|dd| dd.child("w:pPrDefault")).is_some();
        if let Some(dd) = root.child("w:docDefaults") {
            if let Some(r) = dd.child("w:rPrDefault").and_then(|d| d.child("w:rPr")) {
                sheet.default_chr = self.pc.rpr(r);
            }
            if let Some(p) = dd.child("w:pPrDefault").and_then(|d| d.child("w:pPr")) {
                sheet.default_para = self.pc.ppr(p).0;
                // Word makes no paragraph a text frame from the document defaults: the sizeless
                // top-left `w:framePr` Apple Pages writes there would put every paragraph, the
                // headers' and footers' included, at the page's corner (#2360).
                sheet.default_para.frame = None;
            }
        }
        if sheet.default_chr.size.is_none() {
            sheet.default_chr.size = Some(10.0);
        }
        // Built-in styles' behaviour (gallery, priority, visibility), written back as is.
        sheet.latent_styles = root.child("w:latentStyles").map(|l| l.to_xml()).filter(|x| x.len() <= MAX_LATENT_STYLES_XML);
        for s in root.children("w:style").take(10_000) {
            let Some(raw_id) = s.attr("w:styleId").filter(|i| !i.is_empty()) else { continue };
            let id = self.pc.style_id(raw_id);
            let kind = match s.attr("w:type").unwrap_or("paragraph") {
                "character" => StyleKind::Character,
                "table" => StyleKind::Table,
                "numbering" => StyleKind::Numbering,
                _ => StyleKind::Paragraph,
            };
            let name = s.child_val("w:name").map(ui_style_name).unwrap_or_else(|| id.clone());
            let r = |n: &str| s.child_val(n).filter(|v| !v.is_empty()).map(|v| self.pc.style_id(v));
            let mut st = Style {
                id: id.clone(),
                name,
                kind,
                based_on: r("w:basedOn").filter(|b| *b != id),
                next: r("w:next"),
                linked: r("w:link"),
                priority: s.child_val("w:uiPriority").and_then(u32_of),
                quick: flag(s, "w:qFormat").unwrap_or(false),
                hidden: flag(s, "w:semiHidden").unwrap_or(false) || flag(s, "w:hidden").unwrap_or(false),
                builtin: !s.attr("w:customStyle").is_some_and(|v| v == "1" || v == "true"),
                locked: flag(s, "w:locked").unwrap_or(false),
                // Only where the writer couldn't tell from the id (it flags the English ids).
                default: s.attr("w:default").is_some_and(|v| v == "1" || v == "true")
                    && !matches!(
                        (kind, id.as_str()),
                        (StyleKind::Paragraph, "Normal")
                            | (StyleKind::Character, "DefaultParagraphFont")
                            | (StyleKind::Table, "TableNormal")
                            | (StyleKind::Numbering, "NoList")
                    ),
                ..Default::default()
            };
            if let Some(p) = s.child("w:pPr") {
                st.para = self.pc.ppr(p).0;
            }
            if let Some(rp) = s.child("w:rPr") {
                st.chr = self.pc.rpr(rp);
            }
            if kind == StyleKind::Table {
                let parts = self.table_style_parts(s, &mut st);
                st.table = (parts != TableStyleParts::default()).then_some(parts);
            }
            // A style defined twice (some generators write their own Heading 1 after a
            // placeholder one) is the last definition, whole: Word drops the earlier one rather
            // than merging the two (observed with synthetic documents).
            match sheet.styles.iter_mut().find(|s| s.id == id) {
                Some(earlier) => *earlier = st,
                None => sheet.styles.push(st),
            }
        }
        // Styles that give only a list level take the list from the style they are based on
        // (Heading 3: level 3 of Heading 2's outline list).
        let fixed: Vec<Option<wordcraft_doc::props::NumRef>> = sheet
            .styles
            .iter()
            .map(|s| match s.para.numbering {
                Some(n) if n.num == props::LEVEL_ONLY => {
                    let base = s.based_on.as_deref().and_then(|b| props::chain_list(&sheet, b));
                    base.map(|num| wordcraft_doc::props::NumRef { num, level: n.level })
                }
                other => other,
            })
            .collect();
        for (s, n) in sheet.styles.iter_mut().zip(fixed) {
            s.para.numbering = n;
        }
        if sheet.default_para.numbering.is_some_and(|n| n.num == props::LEVEL_ONLY) {
            sheet.default_para.numbering = None;
        }
        // A producer that never writes `w:qFormat` (LibreOffice) still gets Word's usual gallery.
        if !sheet.styles.iter().any(|s| s.quick) {
            sheet.add_default_gallery();
        }
        self.doc.styles = sheet;
        Ok(())
    }

    /// A table style's table-level formatting and conditional formats. The `wholeTable` region
    /// applies to every cell, so its paragraph and run properties go to the style's own (`st`).
    fn table_style_parts(&self, s: &El, st: &mut Style) -> TableStyleParts {
        let mut t = TableStyleParts::default();
        if let Some(tp) = s.child("w:tblPr") {
            if let Some(b) = tp.child("w:tblBorders") {
                t.borders = Some(props::borders(b));
            }
            t.fill = tp.child("w:shd").and_then(|e| self.pc.shd_fill(e));
            t.cell_margins = tp.child("w:tblCellMar").map(props::margins);
            t.band_size = tp.child_val("w:tblStyleRowBandSize").and_then(u32_of).map(|n| n.min(1000));
            t.col_band_size = tp.child_val("w:tblStyleColBandSize").and_then(u32_of).map(|n| n.min(1000));
        }
        for c in s.children("w:tblStylePr").take(64) {
            let Some(part) = c.attr("w:type").and_then(TablePart::from_ooxml) else { continue };
            let tc = c.child("w:tcPr");
            let ppr = c.child("w:pPr").map(|p| self.pc.ppr(p).0);
            let f = PartFormat {
                chr: c.child("w:rPr").map(|r| self.pc.rpr(r)).unwrap_or_default(),
                fill: tc.and_then(|p| p.child("w:shd")).and_then(|e| self.pc.shd_fill(e)),
                borders: tc.and_then(|p| p.child("w:tcBorders")).map(props::borders),
                align: ppr.as_ref().and_then(|p| p.align),
                v_align: tc.and_then(|p| p.child_val("w:vAlign")).map(|v| match v {
                    "center" => VAlign::Center,
                    "bottom" => VAlign::Bottom,
                    _ => VAlign::Top,
                }),
            };
            if part == TablePart::WholeTable {
                // The whole table's paragraph and run properties are the style's own; its
                // borders the table borders.
                if let Some(p) = ppr {
                    st.para.overlay(&p);
                }
                st.chr.overlay(&f.chr);
                if f.fill.is_some() {
                    t.fill = f.fill;
                }
                if let Some(b) = c.child("w:tblPr").and_then(|p| p.child("w:tblBorders")) {
                    t.borders = Some(props::borders(b));
                }
                if f.v_align.is_some() {
                    t.regions.insert(part, PartFormat { v_align: f.v_align, ..PartFormat::default() });
                }
            } else {
                t.set_part(part, f);
            }
        }
        t
    }

    pub(super) fn read_numbering(&mut self, path: &str) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let mut abstracts = Vec::new();
        let mut links: Vec<(u32, String)> = Vec::new();
        for a in root.children("w:abstractNum").take(10_000) {
            let Some(id) = a.attr("w:abstractNumId").and_then(u32_of) else { continue };
            let mut levels: Vec<Level> = (0..9).map(default_level).collect();
            for l in a.children("w:lvl") {
                let i = l.attr("w:ilvl").and_then(u32_of).unwrap_or(0) as usize;
                if let Some(slot) = levels.get_mut(i) {
                    *slot = self.level(l, i);
                }
            }
            if let Some(link) = a.child_val("w:numStyleLink") {
                links.push((id, link.to_string()));
            }
            let hex = |v: &str| v.len() == 8 && v.bytes().all(|b| b.is_ascii_hexdigit());
            abstracts.push(AbstractNum {
                id,
                name: a.child_val("w:name").map(str::to_string),
                levels,
                nsid: a.child_val("w:nsid").filter(|v| hex(v)).map(str::to_ascii_uppercase),
                tmpl: a.child_val("w:tmpl").filter(|v| hex(v)).map(str::to_ascii_uppercase),
                multi_level_type: a
                    .child_val("w:multiLevelType")
                    // `multilevel` is what the writer derives for the nine levels read here.
                    .filter(|v| matches!(*v, "singleLevel" | "hybridMultilevel"))
                    .map(str::to_string),
            });
        }
        let mut nums = Vec::new();
        for n in root.children("w:num").take(10_000) {
            let (Some(id), Some(abs)) = (n.attr("w:numId").and_then(u32_of), n.child_val("w:abstractNumId").and_then(u32_of)) else { continue };
            let (mut start_overrides, mut level_overrides) = (Vec::new(), Vec::new());
            for o in n.children("w:lvlOverride").take(9) {
                let lvl = o.attr("w:ilvl").and_then(u32_of).unwrap_or(0).min(8) as u8;
                if let Some(s) = o.child_val("w:startOverride").and_then(u32_of) {
                    start_overrides.push((lvl, s));
                } else if let Some(s) = o.child("w:lvl").and_then(|l| l.child_val("w:start")).and_then(u32_of) {
                    start_overrides.push((lvl, s));
                }
                // A whole level definition replaces the abstract list's for this list.
                if let Some(l) = o.child("w:lvl") {
                    level_overrides.push((lvl, self.level(l, lvl as usize)));
                }
            }
            nums.push(Num { id, abstract_id: abs, start_overrides, level_overrides });
        }
        // Abstracts that only link to a numbering style take that style's list levels.
        for (aid, style) in links {
            let num = self.doc.styles.get(&self.pc.style_id(&style)).and_then(|s| s.para.numbering).map(|n| n.num);
            let src = num.and_then(|n| nums.iter().find(|x: &&Num| x.id == n)).map(|x| x.abstract_id);
            let levels = src.filter(|s| *s != aid).and_then(|s| abstracts.iter().find(|a: &&AbstractNum| a.id == s)).map(|a| a.levels.clone());
            if let (Some(levels), Some(a)) = (levels, abstracts.iter_mut().find(|a| a.id == aid)) {
                a.levels = levels;
            }
        }
        self.doc.numbering.abstracts = abstracts;
        self.doc.numbering.nums = nums;
        Ok(())
    }

    fn level(&self, l: &El, i: usize) -> Level {
        let mut lv = default_level(i);
        if let Some(s) = l.child_val("w:start").and_then(u32_of) {
            lv.start = s;
        }
        if let Some(f) = l.child_val("w:numFmt") {
            lv.format = NumFormat::from_ooxml(f);
        }
        lv.text = l.child_val("w:lvlText").map(|t| t.chars().take(64).collect()).unwrap_or_default();
        if let Some(j) = l.child_val("w:lvlJc") {
            lv.align = props::align(j);
        }
        if let Some(ind) = l.child("w:pPr").and_then(|p| p.child("w:ind")) {
            if let Some(v) = tw(ind, "w:left").or_else(|| tw(ind, "w:start")) {
                lv.indent = v;
            }
            if let Some(h) = tw(ind, "w:hanging") {
                lv.hanging = h;
            } else if let Some(f) = tw(ind, "w:firstLine") {
                lv.hanging = -f;
            }
        }
        if let Some(r) = l.child("w:rPr") {
            lv.chr = self.pc.rpr(r);
        }
        lv.suffix = match l.child_val("w:suff") {
            Some("space") => LevelSuffix::Space,
            Some("nothing") => LevelSuffix::Nothing,
            _ => LevelSuffix::Tab,
        };
        lv.legal = flag(l, "w:isLgl").unwrap_or(false);
        let restart = l.child_val("w:lvlRestart").and_then(int);
        lv.restart = restart.is_none_or(|v| v != 0);
        lv.restart_after = restart.filter(|v| (1..=9).contains(v)).map(|v| v as u8);
        // The tab stop after the number ("num" tab in the level's paragraph properties).
        lv.tab = l
            .child("w:pPr")
            .and_then(|p| p.child("w:tabs"))
            .and_then(|t| t.children("w:tab").find(|t| t.attr("w:val") == Some("num")))
            .and_then(|t| tw(t, "w:pos"))
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(-1584.0, 1584.0));
        lv.style = l.child_val("w:pStyle").filter(|s| !s.is_empty()).map(|s| self.pc.style_id(s));
        lv
    }
}

/// A level as the file defines it before its own elements: no indents (a level without `w:ind`
/// leaves the label at the paragraph's left edge, as Word shows it), no text.
fn default_level(_i: usize) -> Level {
    Level { indent: 0.0, hanging: 0.0, text: String::new(), ..Level::default() }
}
