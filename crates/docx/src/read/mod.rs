//! DOCX → [`Document`].

mod props;
mod story;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use wordcraft_doc::numbering::{AbstractNum, Level, LevelSuffix, Num};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::styles::{Style, StyleKind, StyleSheet, TableStyleParts};
use wordcraft_doc::{Blocks, Comment, Document, PartKind, Revision, RevisionKind};

use crate::DocxError;
use crate::package::{Package, Rels, rt};
use crate::units::{flag, int, on_off, tw, u32_of};
use crate::xml::El;
pub(crate) use props::PropCtx;
use story::StoryCtx;

/// Most parts (headers, notes, comments, text boxes) we create from one file.
const MAX_PARTS: usize = 50_000;

pub(crate) struct Reader<'p> {
    pkg: &'p Package,
    pub doc: Document,
    pub pc: PropCtx,
    revs: HashMap<(u8, String, String), u32>,
    media_by_path: HashMap<String, String>,
    hf_by_path: HashMap<String, u32>,
    pub footnotes: HashMap<i64, u32>,
    pub endnotes: HashMap<i64, u32>,
    pub comment_map: HashMap<String, u32>,
    pub comments_ended: HashSet<u32>,
    pub bookmarks: HashMap<String, String>,
}

/// Read a `.docx` package.
pub fn read(bytes: &[u8]) -> Result<Document, DocxError> {
    let pkg = Package::open(bytes)?;
    let root_rels = pkg.rels("");
    let main = root_rels.by_type(rt::OFFICE_DOC).filter(|r| !r.external).map(|r| r.target.clone()).unwrap_or_else(|| "word/document.xml".to_string());
    let Some(root) = pkg.xml(&main)? else {
        return Err(DocxError::MissingPart(main));
    };
    if root.name != "w:document" {
        return Err(DocxError::NotWord(format!("root element is {}", root.name)));
    }
    let rels = pkg.rels(&main);
    let mut doc = Document::new();
    doc.body.clear();
    let mut r = Reader {
        pkg: &pkg,
        doc,
        pc: PropCtx::default(),
        revs: HashMap::new(),
        media_by_path: HashMap::new(),
        hf_by_path: HashMap::new(),
        footnotes: HashMap::new(),
        endnotes: HashMap::new(),
        comment_map: HashMap::new(),
        comments_ended: HashSet::new(),
        bookmarks: HashMap::new(),
    };
    r.pc.major_font = r.doc.settings.major_font.clone();
    r.pc.minor_font = r.doc.settings.minor_font.clone();
    let part = |t: &str| rels.by_type(t).filter(|x| !x.external).map(|x| x.target.clone());

    if let Some(p) = part(rt::THEME) {
        r.read_theme(&p);
    }
    // Secondary parts are optional: a broken one is skipped (logged), not fatal.
    let lenient = |what: &str, res: Result<(), DocxError>| {
        if let Err(e) = res {
            log::warn!("docx: ignoring unreadable {what}: {e}");
        }
    };
    if let Some(p) = part(rt::STYLES) {
        lenient("styles", r.read_styles(&p));
    }
    if let Some(p) = part(rt::NUMBERING) {
        lenient("numbering", r.read_numbering(&p));
    }
    if let Some(p) = part(rt::SETTINGS) {
        lenient("settings", r.read_settings(&p));
    }
    if let Some(p) = part(rt::FOOTNOTES) {
        lenient("footnotes", r.read_notes(&p, true));
    }
    if let Some(p) = part(rt::ENDNOTES) {
        lenient("endnotes", r.read_notes(&p, false));
    }
    if let Some(p) = part(rt::COMMENTS) {
        let ex = part(rt::COMMENTS_EX);
        lenient("comments", r.read_comments(&p, ex.as_deref()));
    }

    if let Some(bg) = root.child("w:background").and_then(|b| b.attr("w:color")).and_then(Rgb::parse) {
        r.doc.settings.page_color = Some(bg);
    }
    if let Some(body) = root.child("w:body") {
        let mut sc = StoryCtx::default();
        let mut blocks = Blocks::new();
        r.read_blocks(&mut sc, body, &rels, &mut blocks, 0);
        r.flush_pending(&mut sc, &mut blocks);
        r.doc.body = blocks;
        if let Some(s) = body.child("w:sectPr") {
            r.doc.last_section = r.read_section(s, &rels);
        }
    }

    let core = root_rels.by_type(rt::CORE).map(|x| x.target.clone()).unwrap_or_else(|| "docProps/core.xml".into());
    r.read_core(&core);
    if let Some(c) = root_rels.by_type(rt::CUSTOM)
        && let Some(b) = pkg.get(&c.target)
    {
        r.doc.passthrough.insert("docProps/custom.xml".into(), Arc::new(b.to_vec()));
    }
    let mut doc = r.doc;
    doc.ensure_nonempty();
    Ok(doc)
}

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
    fn xml(&self, path: &str) -> Result<Option<El>, DocxError> {
        self.pkg.xml(path)
    }

    pub fn revision(&mut self, kind: RevisionKind, e: &El) -> u32 {
        let author = e.attr("w:author").unwrap_or("").chars().take(256).collect::<String>();
        let date = e.attr("w:date").unwrap_or("").chars().take(64).collect::<String>();
        let k = match kind {
            RevisionKind::Insert => 0,
            RevisionKind::Delete => 1,
            RevisionKind::Format => 2,
        };
        let key = (k, author.clone(), date.clone());
        if let Some(i) = self.revs.get(&key) {
            return *i;
        }
        let i = self.doc.revisions.len() as u32;
        self.doc.revisions.push(Revision { kind, author, date });
        self.revs.insert(key, i);
        i
    }

    /// Media key for an image relationship (loads the bytes once).
    pub fn media_for(&mut self, rels: &Rels, id: &str) -> Option<String> {
        let rel = rels.by_id(id)?;
        if rel.external {
            return None;
        }
        if let Some(k) = self.media_by_path.get(&rel.target) {
            return Some(k.clone());
        }
        let bytes = self.pkg.get(&rel.target)?;
        let base: String =
            rel.target.rsplit('/').next().unwrap_or("image").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).collect();
        let base = if base.is_empty() || base.starts_with('.') { format!("image{base}") } else { base };
        let mut key = base.clone();
        let mut n = 1;
        while self.doc.media.contains_key(&key) {
            n += 1;
            key = match base.rsplit_once('.') {
                Some((stem, ext)) => format!("{stem}_{n}.{ext}"),
                None => format!("{base}_{n}"),
            };
        }
        self.doc.media.insert(key.clone(), Arc::new(bytes.to_vec()));
        self.media_by_path.insert(rel.target.clone(), key.clone());
        Some(key)
    }

    pub fn load_header_footer(&mut self, path: &str, footer: bool) -> Option<u32> {
        if let Some(id) = self.hf_by_path.get(path) {
            return Some(*id);
        }
        if self.doc.parts.len() >= MAX_PARTS {
            return None;
        }
        // Reserve the id first so a (malformed) self-reference can't recurse.
        let kind = if footer { PartKind::Footer } else { PartKind::Header };
        let id = self.doc.add_part(kind, Blocks::new());
        self.hf_by_path.insert(path.to_string(), id);
        let root = match self.xml(path) {
            Ok(Some(r)) => r,
            _ => return Some(id),
        };
        let rels = self.pkg.rels(path);
        let mut sc = StoryCtx::default();
        let mut blocks = Blocks::new();
        self.read_blocks(&mut sc, &root, &rels, &mut blocks, 0);
        self.flush_pending(&mut sc, &mut blocks);
        if let Some(p) = self.doc.parts.get_mut(&id)
            && !blocks.is_empty()
        {
            p.blocks = blocks;
        }
        Some(id)
    }

    fn read_theme(&mut self, path: &str) {
        let Ok(Some(t)) = self.xml(path) else { return };
        if let Some(n) = t.attr("name").filter(|n| !n.is_empty()) {
            self.doc.settings.theme_name = n.to_string();
        }
        let Some(el) = t.child("a:themeElements") else { return };
        if let Some(cs) = el.child("a:clrScheme") {
            let names = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];
            let mut colors = self.doc.settings.theme_colors.clone();
            colors.resize(12, Rgb::BLACK);
            for (i, n) in names.iter().enumerate() {
                let Some(c) = cs.child(&format!("a:{n}")) else { continue };
                let rgb = c
                    .child("a:srgbClr")
                    .and_then(|x| x.attr("val"))
                    .and_then(Rgb::parse)
                    .or_else(|| c.child("a:sysClr").and_then(|x| x.attr("lastClr")).and_then(Rgb::parse));
                if let (Some(rgb), Some(slot)) = (rgb, colors.get_mut(i)) {
                    *slot = rgb;
                }
            }
            self.doc.settings.theme_colors = colors;
        }
        if let Some(fs) = el.child("a:fontScheme") {
            let face =
                |n: &str| fs.child(n).and_then(|f| f.child("a:latin")).and_then(|l| l.attr("typeface")).filter(|t| !t.is_empty()).map(str::to_string);
            if let Some(f) = face("a:majorFont") {
                self.doc.settings.major_font = f;
            }
            if let Some(f) = face("a:minorFont") {
                self.doc.settings.minor_font = f;
            }
        }
        self.pc.major_font = self.doc.settings.major_font.clone();
        self.pc.minor_font = self.doc.settings.minor_font.clone();
    }

    fn read_styles(&mut self, path: &str) -> Result<(), DocxError> {
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
        let mut sheet = StyleSheet::empty();
        if let Some(dd) = root.child("w:docDefaults") {
            if let Some(r) = dd.child("w:rPrDefault").and_then(|d| d.child("w:rPr")) {
                sheet.default_chr = self.pc.rpr(r);
            }
            if let Some(p) = dd.child("w:pPrDefault").and_then(|d| d.child("w:pPr")) {
                sheet.default_para = self.pc.ppr(p).0;
            }
        }
        if sheet.default_chr.size.is_none() {
            sheet.default_chr.size = Some(10.0);
        }
        for s in root.children("w:style").take(10_000) {
            let Some(raw_id) = s.attr("w:styleId").filter(|i| !i.is_empty()) else { continue };
            let id = self.pc.style_id(raw_id);
            if sheet.get(&id).is_some() {
                continue;
            }
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
                ..Default::default()
            };
            if let Some(p) = s.child("w:pPr") {
                st.para = self.pc.ppr(p).0;
            }
            if let Some(rp) = s.child("w:rPr") {
                st.chr = self.pc.rpr(rp);
            }
            if kind == StyleKind::Table {
                let parts = self.table_style_parts(s);
                st.table = (parts != TableStyleParts::default()).then_some(parts);
            }
            sheet.styles.push(st);
        }
        self.doc.styles = sheet;
        Ok(())
    }

    fn table_style_parts(&self, s: &El) -> TableStyleParts {
        let mut t = TableStyleParts::default();
        if let Some(b) = s.child("w:tblPr").and_then(|p| p.child("w:tblBorders")) {
            t.borders = Some(props::borders(b));
        }
        for c in s.children("w:tblStylePr") {
            let fill = c.child("w:tcPr").and_then(|p| p.child("w:shd")).and_then(props::shd_fill);
            let chr = c.child("w:rPr").map(|r| self.pc.rpr(r)).unwrap_or_default();
            match c.attr("w:type") {
                Some("firstRow") => {
                    t.header_fill = fill;
                    t.header_chr = chr;
                }
                Some("band1Horz") => t.band_fill = fill,
                Some("firstCol") => t.first_col_chr = chr,
                Some("lastRow") => {
                    t.total_chr = chr;
                    t.total_border_top = c.child("w:tcPr").and_then(|p| p.child("w:tcBorders")).and_then(|b| b.child("w:top")).map(props::border);
                }
                _ => {}
            }
        }
        t
    }

    fn read_numbering(&mut self, path: &str) -> Result<(), DocxError> {
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
            abstracts.push(AbstractNum { id, name: a.child_val("w:name").map(str::to_string), levels });
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
        lv.restart = l.child_val("w:lvlRestart").and_then(int).is_none_or(|v| v != 0);
        lv.style = l.child_val("w:pStyle").filter(|s| !s.is_empty()).map(|s| self.pc.style_id(s));
        lv
    }

    fn read_settings(&mut self, path: &str) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let s = &mut self.doc.settings;
        for k in root.els() {
            match k.name.as_str() {
                "w:trackRevisions" => s.track_changes = on_off(k),
                "w:defaultTabStop" => {
                    if let Some(v) = tw(k, "w:val").filter(|v| *v > 0.0) {
                        s.default_tab = v.clamp(1.0, 1584.0);
                    }
                }
                "w:evenAndOddHeaders" => s.even_odd_headers = on_off(k),
                "w:mirrorMargins" => s.mirror_margins = on_off(k),
                "w:autoHyphenation" => s.auto_hyphenation = on_off(k),
                "w:footnotePr" => {
                    if let Some(f) = k.child_val("w:numFmt") {
                        s.footnote_format = NumFormat::from_ooxml(f);
                    }
                }
                "w:endnotePr" => {
                    if let Some(f) = k.child_val("w:numFmt") {
                        s.endnote_format = NumFormat::from_ooxml(f);
                    }
                }
                "w:documentProtection" => {
                    let enforced = k.attr("w:enforcement").is_some_and(|v| !matches!(v, "0" | "false" | "off"));
                    if enforced && let Some(e) = k.attr("w:edit") {
                        s.protection = Some(e.to_string());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read_notes(&mut self, path: &str, foot: bool) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let rels = self.pkg.rels(path);
        let tag = if foot { "w:footnote" } else { "w:endnote" };
        for n in root.children(tag) {
            if n.attr("w:type").is_some_and(|t| t != "normal") {
                continue;
            }
            let Some(fid) = n.attr("w:id").and_then(int) else { continue };
            if self.doc.parts.len() >= MAX_PARTS {
                break;
            }
            let mut sc = StoryCtx::default();
            let mut blocks = Blocks::new();
            self.read_blocks(&mut sc, n, &rels, &mut blocks, 0);
            self.flush_pending(&mut sc, &mut blocks);
            let id = self.doc.add_part(if foot { PartKind::Footnote } else { PartKind::Endnote }, blocks);
            if foot {
                self.footnotes.insert(fid, id);
            } else {
                self.endnotes.insert(fid, id);
            }
        }
        Ok(())
    }

    fn read_comments(&mut self, path: &str, ext: Option<&str>) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let rels = self.pkg.rels(path);
        let mut by_para: HashMap<String, u32> = HashMap::new();
        let mut next_id = 0u32;
        let mut entries: BTreeMap<u32, Comment> = BTreeMap::new();
        for c in root.children("w:comment") {
            if self.doc.parts.len() >= MAX_PARTS {
                break;
            }
            let file_id = c.attr("w:id").unwrap_or("").to_string();
            if self.comment_map.contains_key(&file_id) {
                continue;
            }
            let id = match file_id.parse::<u32>() {
                Ok(v) if !entries.contains_key(&v) => v,
                _ => {
                    while entries.contains_key(&next_id) || self.comment_map.values().any(|v| *v == next_id) {
                        next_id += 1;
                    }
                    next_id
                }
            };
            let mut sc = StoryCtx::default();
            let mut blocks = Blocks::new();
            self.read_blocks(&mut sc, c, &rels, &mut blocks, 0);
            self.flush_pending(&mut sc, &mut blocks);
            let part = self.doc.add_part(PartKind::Comment, blocks);
            if let Some(pid) = c.children("w:p").last().and_then(|p| p.attr("w14:paraId")) {
                by_para.insert(pid.to_string(), id);
            }
            entries.insert(
                id,
                Comment {
                    author: c.attr("w:author").unwrap_or("").to_string(),
                    initials: c.attr("w:initials").unwrap_or("").to_string(),
                    date: c.attr("w:date").unwrap_or("").to_string(),
                    parent: None,
                    resolved: false,
                    part,
                },
            );
            self.comment_map.insert(file_id, id);
        }
        if let Some(ext) = ext
            && let Ok(Some(x)) = self.xml(ext)
        {
            for e in x.els().filter(|e| e.local() == "commentEx") {
                let Some(cid) = e.attr("w15:paraId").and_then(|p| by_para.get(p)).copied() else { continue };
                let parent = e.attr("w15:paraIdParent").and_then(|p| by_para.get(p)).copied().filter(|p| *p != cid);
                let done = e.attr("w15:done").is_some_and(|v| v == "1" || v == "true");
                if let Some(c) = entries.get_mut(&cid) {
                    c.parent = parent;
                    c.resolved = done;
                }
            }
        }
        self.doc.comments = entries;
        Ok(())
    }

    fn read_core(&mut self, path: &str) {
        let Ok(Some(root)) = self.xml(path) else { return };
        let c = &mut self.doc.core;
        for k in root.els() {
            let t = k.text().trim().to_string();
            match k.name.as_str() {
                "dc:title" => c.title = t,
                "dc:subject" => c.subject = t,
                "dc:creator" => c.creator = t,
                "cp:keywords" => c.keywords = t,
                "dc:description" => c.description = t,
                "cp:lastModifiedBy" => c.last_modified_by = t,
                "cp:revision" => c.revision = u32_of(&t).unwrap_or(0),
                "dcterms:created" => c.created = t,
                "dcterms:modified" => c.modified = t,
                "cp:category" => c.category = t,
                _ => {}
            }
        }
    }
}

fn default_level(i: usize) -> Level {
    Level { indent: 36.0 * (i as f32 + 1.0), text: String::new(), ..Level::default() }
}
