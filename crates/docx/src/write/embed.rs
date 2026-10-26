//! Writing back charts, SmartArt diagrams, native ink and OLE objects read from a file (see `read::embed`).
//!
//! Each written object gets its kept markup with fresh relationship ids, pointing at copies of the
//! parts it needs. The first object to use a part writes it under its original name (unless that
//! name is one this writer uses for something else: then it's renamed). An object written again
//! (copied and pasted in WordCraft) gets its own renamed copies of every part, so the copies stay
//! independent in Word; past [`MAX_COPY_BYTES`] of such copies, later ones share the first's parts.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use wordcraft_doc::graphic::Embedded;
use wordcraft_doc::para::Float;

use super::PartRels;
use crate::package::{
    CUSTOM_XML_ROOTS, EMBEDDED_PARTS, EmbeddedManifest, FONT_TABLE_PART, MAX_EMBEDDED_PARTS, MAX_EMBEDDED_RELS, VBA_PROJECT_PART, VBA_RELATED,
    rel_is, relative, rt,
};
use crate::units::{MAX_LEN_PT, emu, int};
use crate::xml::{self, El, MAX_DEPTH};

/// Most bytes written for copies of parts (objects pasted more than once).
const MAX_COPY_BYTES: u64 = 256 * 1024 * 1024;
/// Most bytes of kept markup written for all objects (objects can share one kept markup).
const MAX_MARKUP_BYTES: u64 = 128 * 1024 * 1024;
/// What one part copy costs at least against [`MAX_COPY_BYTES`] (its relationships, entry…).
const PART_COST: u64 = 4096;

/// Where the stories' parts live: every story this writer makes is a part in `word/`.
const STORY_PART: &str = "word/document.xml";

/// The embedded-object parts of a package being written.
pub(crate) struct EmbedWriter<'d> {
    manifest: EmbeddedManifest,
    passthrough: &'d BTreeMap<String, Arc<Vec<u8>>>,
    /// Part names in use (lower case): this writer's own, and parts written so far.
    taken: HashSet<String>,
    /// Next suffix to try when renaming a part.
    next: HashMap<String, u32>,
    /// Original part name → name written, for parts written by an object's first copy.
    first: HashMap<String, String>,
    /// Budget left for copies.
    copy_bytes: u64,
    /// Budget left for kept markup.
    markup_bytes: u64,
    /// Parts to write: (name, bytes, content type, relationships).
    pub parts: Vec<(String, Vec<u8>, String, PartRels)>,
    /// Leave out the main document's kept glossary (it's written from the model instead).
    pub drop_glossary: bool,
    /// Kept parts written with these bytes instead of their own (see [`Self::scrub_glossary`]).
    scrubbed: HashMap<String, Vec<u8>>,
    /// A chart past the chart limits has been written (and the loss logged, once per save).
    chart_cut: bool,
}

impl<'d> EmbedWriter<'d> {
    pub fn new(doc: &'d wordcraft_doc::Document) -> EmbedWriter<'d> {
        let manifest = doc.passthrough.get(EMBEDDED_PARTS).map(|m| EmbeddedManifest::parse(m)).unwrap_or_default();
        let mut taken: HashSet<String> = [
            "word/document.xml",
            "word/styles.xml",
            "word/numbering.xml",
            "word/settings.xml",
            "word/theme/theme1.xml",
            "word/footnotes.xml",
            "word/endnotes.xml",
            "word/comments.xml",
            "word/commentsextended.xml",
            "word/commentsids.xml",
            "word/commentsextensible.xml",
            "[content_types].xml",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        taken.insert(VBA_PROJECT_PART.to_ascii_lowercase());
        // The bibliography writer uses these names when sources are present (#532).
        if !doc.sources.is_empty() {
            taken.extend(["customxml/item1.xml".into(), "customxml/itemprops1.xml".into()]);
        }
        // The macro project's parts keep their names (see `write_as`).
        if let Some(m) = doc.passthrough.get(VBA_RELATED) {
            for line in String::from_utf8_lossy(m).lines() {
                if let Some(path) = line.split('\t').nth(1) {
                    taken.insert(path.to_ascii_lowercase());
                }
            }
        }
        EmbedWriter {
            manifest,
            passthrough: &doc.passthrough,
            taken,
            next: HashMap::new(),
            first: HashMap::new(),
            copy_bytes: MAX_COPY_BYTES,
            markup_bytes: MAX_MARKUP_BYTES,
            parts: Vec::new(),
            drop_glossary: false,
            scrubbed: HashMap::new(),
            chart_cut: false,
        }
    }

    /// Write the font table kept from a file, with the fonts it embeds, relating it from `rels`.
    pub fn font_table(&mut self, rels: &mut PartRels) {
        let mut written = Vec::new();
        if let Some(name) = self.copy_tree(FONT_TABLE_PART, &mut HashMap::new(), &mut written) {
            rels.add(rt::FONT_TABLE, &relative(STORY_PART, &name), false);
        }
    }

    /// Reserve the names of the media files this writer writes.
    pub fn reserve_media<'a>(&mut self, files: impl Iterator<Item = &'a String>) {
        for f in files {
            self.taken.insert(format!("word/media/{f}").to_ascii_lowercase());
        }
    }

    /// Names this writer may give its own parts later (headers, footers, package parts).
    fn reserved(lower: &str) -> bool {
        lower.ends_with(".rels")
            || (lower.starts_with("docprops/") && !lower.starts_with("docprops/thumbnail."))
            || lower.starts_with("_rels/")
            || ((lower.starts_with("word/header") || lower.starts_with("word/footer")) && lower.ends_with(".xml"))
    }

    /// A free part name for (a copy of) `orig`: `orig` itself if free, else `stem_N.ext`.
    fn alloc(&mut self, orig: &str) -> String {
        let clean = valid_part_name(orig);
        let orig = clean.as_str();
        // Suffixes cannot escape reserved directories or a .rels extension. Such a kept
        // part needs a different base name, otherwise every candidate would be reserved.
        let orig = if Self::reserved(&orig.to_ascii_lowercase()) { "word/embeddings/part.xml" } else { orig };
        let lower = orig.to_ascii_lowercase();
        if !Self::reserved(&lower) && self.taken.insert(lower) {
            return orig.to_string();
        }
        let file_at = orig.rfind('/').map_or(0, |i| i + 1);
        let (stem, ext) = match orig.rfind('.').filter(|i| *i > file_at) {
            Some(i) => (orig.get(..i).unwrap_or(orig), orig.get(i..).unwrap_or("")),
            None => (orig, ""),
        };
        loop {
            let n = self.next.entry(orig.to_string()).or_insert(1);
            *n = n.saturating_add(1);
            let name = format!("{stem}_{n}{ext}");
            let lower = name.to_ascii_lowercase();
            if !Self::reserved(&lower) && self.taken.insert(lower) {
                return name;
            }
        }
    }

    /// A number `n` for which every `{stem}{n}{ext}` is a free part name, with those names taken.
    pub(super) fn fresh_set(&mut self, stems: &[&str], ext: &str) -> Vec<String> {
        let mut n: u32 = 1;
        loop {
            let names: Vec<String> = stems.iter().map(|s| format!("{s}{n}{ext}")).collect();
            if names.iter().all(|name| {
                let lower = name.to_ascii_lowercase();
                !Self::reserved(&lower) && !self.taken.contains(&lower)
            }) {
                for name in &names {
                    self.taken.insert(name.to_ascii_lowercase());
                }
                return names;
            }
            n = n.saturating_add(1);
        }
    }

    /// A free part name `{stem}{n}{ext}` for a new part, from n = 1 up.
    fn fresh(&mut self, stem: &str, ext: &str) -> String {
        let mut n: u32 = 1;
        loop {
            let name = format!("{stem}{n}{ext}");
            let lower = name.to_ascii_lowercase();
            if !Self::reserved(&lower) && self.taken.insert(lower) {
                return name;
            }
            n = n.saturating_add(1);
        }
    }

    /// Can part `name` be written (bytes and manifest entry both kept)?
    fn available(&self, name: &str) -> bool {
        self.manifest.parts.contains_key(name) && self.passthrough.contains_key(name)
    }

    /// Reattach implicit custom XML roots, sharing their kept parts across both owners.
    pub fn custom_xml_rels(&mut self, owner: &str, rels: &mut PartRels) {
        let Some(roots) = self.passthrough.get(CUSTOM_XML_ROOTS).cloned() else { return };
        let mut memo = std::mem::take(&mut self.first);
        let mut written = Vec::new();
        for line in String::from_utf8_lossy(&roots).lines().take(MAX_EMBEDDED_RELS * 2) {
            let fields: Vec<&str> = line.split('\t').collect();
            let [source, kind, target] = fields.as_slice() else { continue };
            if *source != owner || !rel_is(kind, rt::CUSTOM_XML) {
                continue;
            }
            if let Some(name) = self.copy_tree(target, &mut memo, &mut written) {
                let source_part = if owner == "P" { "" } else { STORY_PART };
                rels.add(kind, &relative(source_part, &name), false);
            }
        }
        self.first = memo;
    }

    /// Reattach the parts kept by `keep_unmodelled` (`UNMODELLED_ROOTS`) to their owner: the
    /// package (`P`) or the main document (`D`).
    pub fn unmodelled_rels(&mut self, owner: &str, rels: &mut PartRels) {
        let Some(roots) = self.passthrough.get(crate::package::UNMODELLED_ROOTS).cloned() else { return };
        let mut memo = std::mem::take(&mut self.first);
        let mut written = Vec::new();
        for line in String::from_utf8_lossy(&roots).lines().take(MAX_EMBEDDED_RELS * 2) {
            let fields: Vec<&str> = line.split('\t').collect();
            let [source, kind, target] = fields.as_slice() else { continue };
            if *source != owner || (self.drop_glossary && owner == "D" && rel_is(kind, rt::GLOSSARY)) {
                continue;
            }
            if let Some(name) = self.copy_tree(target, &mut memo, &mut written) {
                let source_part = if owner == "P" { "" } else { STORY_PART };
                rels.add(kind, &relative(source_part, &name), false);
            }
        }
        self.first = memo;
    }

    /// Whether `unmodelled_rels("D", …)` can write the main document's kept glossary back.
    pub fn kept_glossary(&self) -> bool {
        let Some(roots) = self.passthrough.get(crate::package::UNMODELLED_ROOTS) else { return false };
        String::from_utf8_lossy(roots).lines().take(MAX_EMBEDDED_RELS * 2).any(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            matches!(fields.as_slice(), [source, kind, target] if *source == "D" && rel_is(kind, rt::GLOSSARY) && (self.first.contains_key(*target) || self.available(target)))
        })
    }

    /// Have the main document's kept glossary, and the parts it reaches, written without the
    /// personal information (`names`) and dates the privacy settings remove (#3692). False when a
    /// part's markup can't be rewritten: the glossary is then written from the model.
    pub fn scrub_glossary(&mut self, names: bool, dates: bool) -> bool {
        let Some(roots) = self.passthrough.get(crate::package::UNMODELLED_ROOTS) else { return true };
        let mut todo: Vec<String> = String::from_utf8_lossy(roots)
            .lines()
            .take(MAX_EMBEDDED_RELS * 2)
            .filter_map(|line| match line.split('\t').collect::<Vec<_>>().as_slice() {
                [source, kind, target] if *source == "D" && rel_is(kind, rt::GLOSSARY) => Some(target.to_string()),
                _ => None,
            })
            .collect();
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(p) = todo.pop() {
            if seen.len() >= MAX_EMBEDDED_PARTS || !seen.insert(p.clone()) {
                continue;
            }
            let (Some(info), Some(bytes)) = (self.manifest.parts.get(&p), self.passthrough.get(&p)) else { continue };
            todo.extend(info.rels.iter().filter(|r| !r.external).map(|r| r.target.clone()));
            if !(info.content_type.ends_with("xml") || p.to_ascii_lowercase().ends_with(".xml")) {
                continue;
            }
            match scrub_private(bytes, names, dates) {
                Some(b) => {
                    self.scrubbed.insert(p, b);
                }
                None => {
                    log::warn!(
                        "docx: the glossary's part {p} can't be cleaned of personal information; the glossary is written from its entries, without what WordCraft doesn't model"
                    );
                    return false;
                }
            }
        }
        true
    }

    /// What copying the parts reached from `roots` costs against the copy budget.
    fn tree_cost(&self, roots: &[&str]) -> u64 {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut todo: Vec<&str> = roots.to_vec();
        let mut cost = 0u64;
        while let Some(p) = todo.pop() {
            if !seen.insert(p) || seen.len() > MAX_EMBEDDED_PARTS {
                continue;
            }
            cost = cost.saturating_add(self.passthrough.get(p).map_or(0, |b| b.len() as u64)).saturating_add(PART_COST);
            if let Some(info) = self.manifest.parts.get(p) {
                todo.extend(info.rels.iter().filter(|r| !r.external).map(|r| r.target.as_str()));
            }
        }
        cost
    }

    /// Write part `root` and the parts it reaches, naming them through `memo` (original → written
    /// name; parts already in it aren't written again). Returns `root`'s written name and adds
    /// the indices of the parts written to `written`.
    fn copy_tree(&mut self, root: &str, memo: &mut HashMap<String, String>, written: &mut Vec<usize>) -> Option<String> {
        if let Some(n) = memo.get(root) {
            return Some(n.clone());
        }
        if !self.available(root) {
            return None;
        }
        let name = self.alloc(root);
        memo.insert(root.to_string(), name.clone());
        let mut todo = vec![root.to_string()];
        while let Some(orig) = todo.pop() {
            let (Some(info), Some(bytes)) = (self.manifest.parts.get(&orig).cloned(), self.passthrough.get(&orig)) else { continue };
            let Some(new) = memo.get(&orig).cloned() else { continue };
            let mut rels = PartRels::default();
            for r in &info.rels {
                if r.external {
                    rels.add_with_id(&r.id, &r.kind, &r.target, true);
                    continue;
                }
                let target = match memo.get(&r.target) {
                    Some(t) => t.clone(),
                    None if self.available(&r.target) => {
                        let t = self.alloc(&r.target);
                        memo.insert(r.target.clone(), t.clone());
                        todo.push(r.target.clone());
                        t
                    }
                    // A part that wasn't kept: leave the relationship out.
                    None => continue,
                };
                rels.add_with_id(&r.id, &r.kind, &relative(&new, &target), false);
            }
            let ct = if info.content_type.is_empty() { "application/octet-stream".to_string() } else { info.content_type };
            let bytes = self.scrubbed.get(&orig).cloned().unwrap_or_else(|| bytes.to_vec());
            written.push(self.parts.len());
            self.parts.push((new, bytes, ct, rels));
        }
        Some(name)
    }
}

impl super::Writer<'_> {
    /// The markup for an object kept from a file, with its relationships added to `rels` (the
    /// story part's) and the parts it needs queued for writing. `resize`, for an OLE object, writes
    /// the object's current size and position into its own markup (a chart or diagram's frame is
    /// written by the caller). `None` when it can't be written back (its parts are gone).
    pub(super) fn embedded_xml(&mut self, src: &Embedded, rels: &mut PartRels, resize: Option<(f32, f32, &Float, &str)>) -> Option<String> {
        self.embedded_tree(src, rels, resize).map(|root| root.to_xml())
    }

    /// The same bounded preservation operation, retaining the parsed tree for a caller that
    /// must rebuild an ink AlternateContent drawing frame without reparsing unbound prefixes.
    pub(super) fn embedded_tree(&mut self, src: &Embedded, rels: &mut PartRels, resize: Option<(f32, f32, &Float, &str)>) -> Option<El> {
        let Some(left) = self.embeds.markup_bytes.checked_sub(src.xml.len() as u64) else {
            log::warn!("docx: too much embedded-object markup; the rest is left out");
            return None;
        };
        self.embeds.markup_bytes = left;
        let mut root = xml::parse(src.xml.as_bytes()).ok()?;
        let mut used = Vec::new();
        rel_attr_values(&root, &mut used, 0);
        // Every relationship the markup uses must be writable, or the object would be broken.
        let writable = |id: &String| src.refs.iter().any(|r| r.id == *id && (r.external || self.embeds.available(&r.target)));
        if !used.iter().all(writable) {
            log::warn!("docx: an embedded object's parts are missing; it is left out");
            return None;
        }
        let roots: Vec<&str> = src.refs.iter().filter(|r| !r.external).map(|r| r.target.as_str()).collect();
        // Written before (a copy of this object): give this one its own parts while the budget lasts.
        let again = roots.iter().any(|r| self.embeds.first.contains_key(*r));
        let cost = if again { self.embeds.tree_cost(&roots) } else { 0 };
        let fresh = again && cost <= self.embeds.copy_bytes;
        if fresh {
            self.embeds.copy_bytes -= cost;
        }
        let mut memo = if fresh { HashMap::new() } else { std::mem::take(&mut self.embeds.first) };
        let mut written = Vec::new();
        let mut ids: HashMap<String, String> = HashMap::new();
        for r in &src.refs {
            let new_id = if r.external {
                Some(rels.add(&r.kind, &r.target, true))
            } else {
                self.embeds.copy_tree(&r.target, &mut memo, &mut written).map(|name| rels.add(&r.kind, &relative(STORY_PART, &name), false))
            };
            if let Some(id) = new_id {
                ids.insert(r.id.clone(), id);
            }
        }
        if !fresh {
            self.embeds.first = memo;
        }
        if used.iter().any(|u| !ids.contains_key(u)) {
            log::warn!("docx: an embedded object lost a part; it is left out");
            return None;
        }
        // A SmartArt data part names its drawing's relationship (of the story part) by id.
        for i in written {
            if let Some((_, bytes, ct, _)) = self.embeds.parts.get_mut(i)
                && ct.contains("diagramData")
            {
                *bytes = patch_rel_ids(bytes, &ids);
            }
        }
        set_rel_ids(&mut root, &ids, 0);
        if let Some((w, h, float, alt)) = resize {
            self.resize_object(&mut root, w, h, float, alt, 0);
        }
        Some(root)
    }

    /// The `a:graphic` of a SmartArt graphic written from its model, laid out `w` × `h` points:
    /// new diagram parts (`word/diagrams/{data,layout,quickStyle,colors,drawing}N.xml`) related
    /// from the story part through `rels`.
    pub(super) fn smart_art_graphic(&mut self, spec: &wordcraft_doc::smart_art::SmartArtSpec, w: f32, h: f32, rels: &mut PartRels) -> String {
        use crate::package::rt;
        use crate::smart_art as sa;
        let names = self.embeds.fresh_set(
            &["word/diagrams/data", "word/diagrams/layout", "word/diagrams/quickStyle", "word/diagrams/colors", "word/diagrams/drawing"],
            ".xml",
        );
        let [data, layout, style, colors, drawing] = names.as_slice() else { return String::new() };
        // The drawing first: the data part names it by the story part's relationship id.
        let drawing_id = rels.add(rt::DIAGRAM_DRAWING, &relative(STORY_PART, drawing), false);
        let parts = [
            (data, sa::data_xml(spec, &drawing_id), sa::DATA_CT, rt::DIAGRAM_DATA),
            (layout, sa::layout_xml(spec.layout), sa::LAYOUT_CT, rt::DIAGRAM_LAYOUT),
            (style, sa::quick_style_xml(), sa::STYLE_CT, rt::DIAGRAM_QUICK_STYLE),
            (colors, sa::colors_xml(spec.colors), sa::COLORS_CT, rt::DIAGRAM_COLORS),
            (drawing, sa::drawing_xml(spec, w, h), sa::DRAWING_CT, rt::DIAGRAM_DRAWING),
        ];
        let mut ids = Vec::new();
        for (name, xml, ct, kind) in parts {
            self.embeds.parts.push((name.clone(), xml.into_bytes(), ct.to_string(), PartRels::default()));
            ids.push(rels.add(kind, &relative(STORY_PART, name), false));
        }
        let id = |i: usize| ids.get(i).cloned().unwrap_or_default();
        let mut x = xml::W::default();
        x.open("a:graphic", &[("xmlns:a", "http://schemas.openxmlformats.org/drawingml/2006/main")]);
        x.open("a:graphicData", &[("uri", sa::DIAGRAM_URI)]);
        x.empty(
            "dgm:relIds",
            &[
                ("xmlns:dgm", "http://schemas.openxmlformats.org/drawingml/2006/diagram"),
                ("xmlns:r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"),
                ("r:dm", &id(0)),
                ("r:lo", &id(1)),
                ("r:qs", &id(2)),
                ("r:cs", &id(3)),
            ],
        );
        x.close("a:graphicData");
        x.close("a:graphic");
        x.s
    }

    /// The `a:graphic` of a chart written from its model: a new chart part (`word/charts/chartN.xml`)
    /// related from the story part through `rels`.
    pub(super) fn chart_graphic(&mut self, spec: &wordcraft_doc::chart::ChartSpec, rels: &mut PartRels) -> String {
        let name = self.embeds.fresh("word/charts/chart", ".xml");
        let (xml, cut) = crate::chart_spec::chart_part(spec);
        if cut && !self.embeds.chart_cut {
            self.embeds.chart_cut = true;
            log::warn!("docx: a chart has more series or points than a chart part holds; the rest are left out of the file");
        }
        let bytes = xml.into_bytes();
        self.embeds.parts.push((name.clone(), bytes, crate::chart_spec::CHART_CT.to_string(), PartRels::default()));
        let id = rels.add(crate::package::rt::CHART, &relative(STORY_PART, &name), false);
        let mut w = xml::W::default();
        w.open("a:graphic", &[("xmlns:a", "http://schemas.openxmlformats.org/drawingml/2006/main")]);
        w.open("a:graphicData", &[("uri", crate::chart_spec::CHART_URI)]);
        w.empty(
            "c:chart",
            &[
                ("xmlns:c", "http://schemas.openxmlformats.org/drawingml/2006/chart"),
                ("xmlns:r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"),
                ("r:id", &id),
            ],
        );
        w.close("a:graphicData");
        w.close("a:graphic");
        w.s
    }

    /// Write an OLE object's size (and position, when floating), rotation and flips into its VML
    /// shape and DrawingML picture, with the effect extent covering the rotated bounds as for
    /// other drawings, its alt text into both, and give its drawing a fresh `wp:docPr` id.
    fn resize_object(&mut self, e: &mut El, w: f32, h: f32, float: &Float, alt: &str, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        match e.name.as_str() {
            "v:shape" => {
                if let Some((_, style)) = e.attrs.iter_mut().find(|(k, _)| k == "style") {
                    *style = vml_style_with(style, w, h, float);
                }
                set_attr(e, "alt", alt);
            }
            "wp:extent" => set_size_attrs(e, w, h),
            // A turned object needs an effect extent (filled in below) right after its extent.
            "wp:inline" | "wp:anchor" if !e.els().any(|c| c.name == "wp:effectExtent") && float.spin().deg != 0.0 => {
                if let Some(at) = e.kids.iter().position(|k| matches!(k, xml::Node::El(c) if c.name == "wp:extent")) {
                    e.kids.insert(at + 1, xml::Node::El(El { name: "wp:effectExtent".to_string(), attrs: Vec::new(), kids: Vec::new() }));
                }
            }
            "wp:effectExtent" => {
                // Word's effect extent also covers a rotated object's overhang (its rotated bounds).
                let (px, py) = float.spin_pad(w, h);
                let [el, et, er, eb] = float.effect_extent();
                let ext = [el + px, et + py, er + px, eb + py].map(emu);
                e.attrs.retain(|(k, _)| !matches!(k.as_str(), "l" | "t" | "r" | "b"));
                for (k, v) in ["l", "t", "r", "b"].into_iter().zip(ext) {
                    e.attrs.push((k.to_string(), v));
                }
            }
            // The picture's own frame (`pic:spPr`): its size and the object's rotation and flips.
            "a:xfrm" => {
                e.attrs.retain(|(k, _)| !matches!(k.as_str(), "rot" | "flipH" | "flipV"));
                for (k, v) in super::story::spin_attrs(float.spin()) {
                    e.attrs.push((k.to_string(), v));
                }
                for k in e.kids.iter_mut() {
                    if let xml::Node::El(c) = k
                        && c.name == "a:ext"
                    {
                        set_size_attrs(c, w, h);
                    }
                }
                return;
            }
            "wp:docPr" => {
                let id = self.next_docpr();
                if let Some((_, v)) = e.attrs.iter_mut().find(|(k, _)| k == "id") {
                    *v = id;
                }
                set_attr(e, "descr", alt);
            }
            _ => {}
        }
        for k in e.kids.iter_mut() {
            if let xml::Node::El(c) = k {
                self.resize_object(c, w, h, float, alt, depth + 1);
            }
        }
    }

    /// Ink's own transform need not fill its outer drawing frame. Scale its original offsets
    /// and extents proportionally; an unrelated edit must not stretch the preserved strokes.
    pub(super) fn ink_transform(e: &mut El, from: (f32, f32), to: (f32, f32), float: &Float, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        if e.name == "w14:xfrm" {
            e.attrs.retain(|(k, _)| !matches!(k.as_str(), "rot" | "flipH" | "flipV"));
            for (k, v) in super::story::spin_attrs(float.spin()) {
                e.attrs.push((k.to_string(), v));
            }
            let scale = |old: f32, new: f32| if old.is_finite() && old > 0.0 && new.is_finite() { (new.max(0.0) / old) as f64 } else { 1.0 };
            let (sx, sy) = (scale(from.0, to.0), scale(from.1, to.1));
            let max = MAX_LEN_PT as f64 * 12_700.0;
            for c in e.kids.iter_mut().filter_map(|k| match k {
                xml::Node::El(c) => Some(c),
                _ => None,
            }) {
                if !matches!(c.name.as_str(), "a:off" | "a:ext") {
                    continue;
                }
                for (key, value) in &mut c.attrs {
                    let factor = match key.as_str() {
                        "x" | "cx" => sx,
                        "y" | "cy" => sy,
                        _ => continue,
                    };
                    if factor != 1.0
                        && let Some(n) = int(value)
                    {
                        let min = if c.name == "a:ext" { 0.0 } else { -max };
                        *value = ((n as f64 * factor).clamp(min, max).round() as i64).to_string();
                    }
                }
            }
            return;
        }
        for c in e.kids.iter_mut().filter_map(|k| match k {
            xml::Node::El(c) => Some(c),
            _ => None,
        }) {
            Self::ink_transform(c, from, to, float, depth + 1);
        }
    }

    /// Put a group or canvas member kept from a file (native ink and its picture fallback) at
    /// `frame` ([x, y, w, h] in points, in the group's space, as the group is written): its
    /// `w14:xfrm` and the fallback's `a:xfrm` get that offset and extent and the member's turn.
    /// It rewrites every `w14:xfrm` and `a:xfrm` under `e` and doesn't descend into them: right
    /// for the forms Word writes (the content part's own transform and one `pic:pic` fallback's
    /// `pic:spPr/a:xfrm`); a fallback holding several transforms (a group) would get one frame.
    pub(super) fn place_member(e: &mut El, frame: [f32; 4], float: &Float, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        if matches!(e.name.as_str(), "w14:xfrm" | "a:xfrm") {
            e.attrs.retain(|(k, _)| !matches!(k.as_str(), "rot" | "flipH" | "flipV"));
            for (k, v) in super::story::spin_attrs(float.spin()) {
                e.attrs.push((k.to_string(), v));
            }
            let [x, y, w, h] = frame.map(|v| v.clamp(-MAX_LEN_PT, MAX_LEN_PT));
            e.kids.retain(|k| !matches!(k, xml::Node::El(c) if matches!(c.name.as_str(), "a:off" | "a:ext")));
            let el = |name: &str, attrs: [(&str, String); 2]| {
                xml::Node::El(El { name: name.to_string(), attrs: attrs.map(|(k, v)| (k.to_string(), v)).to_vec(), kids: Vec::new() })
            };
            e.kids.insert(0, el("a:ext", [("cx", emu(w.max(0.0))), ("cy", emu(h.max(0.0)))]));
            e.kids.insert(0, el("a:off", [("x", emu(x)), ("y", emu(y))]));
            return;
        }
        for c in e.kids.iter_mut().filter_map(|k| match k {
            xml::Node::El(c) => Some(c),
            _ => None,
        }) {
            Self::place_member(c, frame, float, depth + 1);
        }
    }
}

/// Set an extent's `cx` and `cy` (those it has) to `w` × `h` points.
fn set_size_attrs(e: &mut El, w: f32, h: f32) {
    for (k, v) in e.attrs.iter_mut() {
        match k.as_str() {
            "cx" => *v = emu(w.max(0.0)),
            "cy" => *v = emu(h.max(0.0)),
            _ => {}
        }
    }
}

/// Set attribute `k` of `e` to `v` (added when missing; left out when both are empty).
fn set_attr(e: &mut El, k: &str, v: &str) {
    match e.attrs.iter_mut().find(|(n, _)| n == k) {
        Some((_, old)) => *old = v.to_string(),
        None if !v.is_empty() => e.attrs.push((k.to_string(), v.to_string())),
        None => {}
    }
}

fn rel_attr_values(e: &El, out: &mut Vec<String>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for (k, v) in &e.attrs {
        if xml::is_rel_attr(&e.name, k) && !out.contains(v) {
            out.push(v.clone());
        }
    }
    for c in e.els() {
        rel_attr_values(c, out, depth + 1);
    }
}

fn set_rel_ids(e: &mut El, ids: &HashMap<String, String>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let name = e.name.clone();
    for (k, v) in e.attrs.iter_mut() {
        if xml::is_rel_attr(&name, k)
            && let Some(n) = ids.get(v.as_str())
        {
            *v = n.clone();
        }
    }
    for k in e.kids.iter_mut() {
        if let xml::Node::El(c) = k {
            set_rel_ids(c, ids, depth + 1);
        }
    }
}

/// `bytes` with each `relId="old"` naming a remapped relationship rewritten to its new id.
fn patch_rel_ids(bytes: &[u8], ids: &HashMap<String, String>) -> Vec<u8> {
    const KEY: &[u8] = b"relId=\"";
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some(at) = rest.windows(KEY.len()).position(|w| w == KEY) {
        let (head, tail) = rest.split_at(at + KEY.len());
        out.extend_from_slice(head);
        let end = tail.iter().position(|b| *b == b'"').unwrap_or(tail.len());
        let (old, after) = tail.split_at(end);
        match std::str::from_utf8(old).ok().and_then(|o| ids.get(o)) {
            Some(new) => out.extend_from_slice(new.as_bytes()),
            None => out.extend_from_slice(old),
        }
        rest = after;
    }
    out.extend_from_slice(rest);
    out
}

/// `bytes` (kept XML) without the personal information and dates the privacy settings remove, as
/// the writer leaves them out of the parts it writes: annotation authors become "Author" and
/// comment initials "A" (`names`); annotation dates and UTC dates go (`dates`); and with `names`,
/// a people part's authors become "Author" and their presence information goes. Attributes are
/// matched by namespace, so any prefix is caught. `None` when the markup can't be rewritten
/// (malformed, or not UTF-8).
fn scrub_private(bytes: &[u8], names: bool, dates: bool) -> Option<Vec<u8>> {
    use std::borrow::Cow;

    use quick_xml::events::Event;
    use quick_xml::events::attributes::Attribute;
    use quick_xml::name::{QName, ResolveResult};
    use quick_xml::reader::NsReader;
    use quick_xml::writer::Writer;

    const WML: &[u8] = b"http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const WML_STRICT: &[u8] = b"http://purl.oclc.org/ooxml/wordprocessingml/main";
    const W15: &[u8] = b"http://schemas.microsoft.com/office/word/2012/wordml";
    // UTF-16 (or other wide encodings) would pass through unread.
    if bytes.contains(&0) || bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return None;
    }
    let ns_is = |r: &ResolveResult, ns: &[u8]| matches!(r, ResolveResult::Bound(n) if n.as_ref() == ns);
    let wml = |r: &ResolveResult| ns_is(r, WML) || ns_is(r, WML_STRICT);
    let mut r = NsReader::from_reader(bytes);
    r.config_mut().check_end_names = false;
    let mut out = Writer::new(Vec::with_capacity(bytes.len()));
    let mut buf = Vec::new();
    let mut skip = Vec::new();
    loop {
        buf.clear();
        let (res, ev) = r.read_resolved_event_into(&mut buf).ok()?;
        let (in_wml, in_w15) = (wml(&res), ns_is(&res, W15));
        let ev = match ev {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) if names && in_w15 && e.local_name().as_ref() == b"presenceInfo" => {
                if matches!(ev, Event::Start(_)) {
                    let end = e.name().as_ref().to_vec();
                    r.read_to_end_into(QName(&end), &mut skip).ok()?;
                    skip.clear();
                }
                continue;
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                let mut el = e.to_owned();
                el.clear_attributes();
                for a in e.attributes().with_checks(false) {
                    let a = a.ok()?;
                    let (ares, local) = r.resolver_mut().resolve_attribute(a.key);
                    // An unprefixed attribute of a WordprocessingML element is read as its own.
                    let wml_attr = wml(&ares) || (matches!(ares, ResolveResult::Unbound) && in_wml);
                    let value: Option<&[u8]> = match local.as_ref() {
                        b"author" if names && (wml_attr || ns_is(&ares, W15)) => Some(super::ANONYMOUS_AUTHOR.as_bytes()),
                        b"initials" if names && wml_attr => Some(super::ANONYMOUS_INITIALS.as_bytes()),
                        b"date" if dates && wml_attr => None,
                        b"dateUtc" if dates => None,
                        _ => {
                            el.push_attribute(a);
                            continue;
                        }
                    };
                    if let Some(v) = value {
                        el.push_attribute(Attribute { key: a.key, value: Cow::Borrowed(v) });
                    }
                }
                if matches!(ev, Event::Start(_)) { Event::Start(el) } else { Event::Empty(el) }
            }
            ev => ev,
        };
        out.write_event(ev).ok()?;
    }
    Some(out.into_inner())
}

/// A VML `style` with the object's size, rotation and flips, and its offset when it floats
/// (`position:absolute`).
fn vml_style_with(style: &str, w: f32, h: f32, float: &Float) -> String {
    let absolute = style
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .any(|(k, v)| k.trim().eq_ignore_ascii_case("position") && v.trim().eq_ignore_ascii_case("absolute"));
    let mut out: Vec<String> = style
        .split(';')
        .filter(|d| {
            let key = d.split_once(':').map_or("", |(k, _)| k).trim().to_ascii_lowercase();
            !(d.trim().is_empty()
                || matches!(key.as_str(), "width" | "height" | "rotation" | "flip")
                || (absolute && matches!(key.as_str(), "left" | "top" | "margin-left" | "margin-top")))
        })
        .map(|d| d.trim().to_string())
        .collect();
    if absolute {
        out.push(format!("margin-left:{}pt", pt(float.x)));
        out.push(format!("margin-top:{}pt", pt(float.y)));
    }
    out.push(format!("width:{}pt", pt(w.max(0.0))));
    out.push(format!("height:{}pt", pt(h.max(0.0))));
    // VML `rotation`: degrees clockwise; `flip`: `x` and/or `y` (as `read::story::vml_float` reads them).
    let spin = float.spin();
    if spin.deg != 0.0 {
        out.push(format!("rotation:{}", pt(spin.deg)));
    }
    let flip: Vec<&str> = [(spin.flip_h, "x"), (spin.flip_v, "y")].into_iter().filter(|(on, _)| *on).map(|(_, f)| f).collect();
    if !flip.is_empty() {
        out.push(format!("flip:{}", flip.join(" ")));
    }
    out.join(";")
}

/// Points for a VML style: finite, bounded, at most two decimals.
fn pt(v: f32) -> String {
    let v = wordcraft_geom::finite(v).clamp(-crate::units::MAX_LEN_PT, crate::units::MAX_LEN_PT);
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `name` as a valid OPC part name: no backslashes, empty or dot segments, segments ending in a
/// dot, or encoded separators.
fn valid_part_name(name: &str) -> String {
    let mut out = Vec::new();
    for seg in name.split('/') {
        let mut seg = seg.replace('\\', "_");
        for enc in ["%2f", "%5c"] {
            while let Some(i) = seg.to_ascii_lowercase().find(enc) {
                seg.replace_range(i..i + 3, "_");
            }
        }
        if seg.is_empty() || seg == "." || seg == ".." {
            continue;
        }
        // A segment can't end in a dot either (`word/a.`).
        if seg.ends_with('.') {
            seg = format!("{}_", seg.trim_end_matches('.'));
        }
        out.push(seg);
    }
    if out.is_empty() { "word/embeddings/part.xml".to_string() } else { out.join("/") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charts_cut_to_the_limits_on_save_say_so_once() {
        // A model deserialised as is can hold more than a chart part keeps; the save left the rest
        // out without a word (review of #3872).
        use std::sync::Arc;
        use wordcraft_doc::chart::{ChartSeries, ChartSpec, ChartType, MAX_SERIES};
        use wordcraft_doc::graphic::{Graphic, GraphicKind};
        use wordcraft_doc::{CharProps, InlineObject, Pos};
        let chart = |series: usize| {
            let spec = ChartSpec {
                kind: ChartType::Column,
                categories: vec!["a".into(), "b".into()],
                series: (0..series).map(|i| ChartSeries { name: format!("S{i}"), values: vec![Some(1.0); 2] }).collect(),
                title: None,
                legend: None,
                data_labels: false,
            };
            let graphic = Graphic { kind: GraphicKind::Chart, w: 100.0, h: 100.0, chart: Some(Arc::new(spec)), ..Default::default() };
            InlineObject::Graphic { w: 100.0, h: 100.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) }
        };
        let save = |charts: Vec<InlineObject>| {
            let mut doc = wordcraft_doc::Document::new();
            for c in charts {
                doc.insert_object(&Pos::body(0, 0), c, &CharProps::default()).unwrap();
            }
            crate::test_log::warnings(|| crate::write(&doc).unwrap()).1
        };
        let cut = |w: &Vec<String>| w.iter().filter(|w| w.contains("chart")).count();
        let warned = save(vec![chart(MAX_SERIES + 1), chart(MAX_SERIES + 5), chart(2)]);
        assert_eq!(cut(&warned), 1, "one warning for the save, not one per chart: {warned:?}");
        let warned = save(vec![chart(3)]);
        assert_eq!(cut(&warned), 0, "nothing left out: {warned:?}");
    }

    #[test]
    fn data_part_drawing_ids_are_rewritten_once() {
        let ids: HashMap<String, String> = [("rId9".to_string(), "rId2".to_string()), ("rId2".to_string(), "rId3".to_string())].into_iter().collect();
        let out = patch_rel_ids(br#"<x relId="rId9"/><y relId="rId2"/><z relId="rId7"/><w relId="unterminated"#, &ids);
        assert_eq!(String::from_utf8(out).unwrap(), r#"<x relId="rId2"/><y relId="rId3"/><z relId="rId7"/><w relId="unterminated"#);
    }

    #[test]
    fn vml_style_takes_the_new_size_and_offset() {
        let inline = vml_style_with("width:100pt;height:50pt;mso-wrap-style:square", 120.5, 60.0, &Float::default());
        assert_eq!(inline, "mso-wrap-style:square;width:120.5pt;height:60pt");
        let float = Float { x: 10.0, y: 20.25, ..Float::default() };
        let abs = vml_style_with("position:absolute;left:5pt;margin-left:3pt;top:1pt;width:1pt;height:1pt", 2.0, 3.0, &float);
        assert_eq!(abs, "position:absolute;margin-left:10pt;margin-top:20.25pt;width:2pt;height:3pt");
        assert_eq!(pt(f32::NAN), "0");
    }

    #[test]
    fn vml_style_takes_the_rotation_and_flips() {
        let mut float = Float::default();
        float.set_spin(wordcraft_geom::Spin::new(-30.5, true, true));
        let s = vml_style_with("width:1pt;rotation:10;flip:x;height:1pt;z-index:3", 2.0, 3.0, &float);
        assert_eq!(s, "z-index:3;width:2pt;height:3pt;rotation:329.5;flip:x y");
        // Unturned: no rotation or flip left behind.
        assert_eq!(vml_style_with("rotation:90;flip:y;width:1pt", 2.0, 3.0, &Float::default()), "width:2pt;height:3pt");
    }

    #[test]
    fn kept_part_names_are_valid_opc_names() {
        let doc = wordcraft_doc::Document::default();
        let mut w = EmbedWriter::new(&doc);
        for orig in ["word/a\\b.bin", "word/..\\..\\x.bin", "word/a/../b%2Fc.bin", "word/a//./b%5Cc.bin", "word/a./b.bin", "word/a..."] {
            let name = w.alloc(orig);
            assert!(!name.contains('\\'), "{name}");
            assert!(!name.to_ascii_lowercase().contains("%2f") && !name.to_ascii_lowercase().contains("%5c"), "{name}");
            assert!(name.split('/').all(|s| !s.is_empty() && !s.ends_with('.')), "{name}");
        }
    }
}
