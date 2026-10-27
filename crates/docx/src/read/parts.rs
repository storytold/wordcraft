use super::*;

impl Reader<'_> {
    /// Keep the parts the VBA project at `project` relates to, with their relationship types and
    /// content types (see [`VBA_RELATED`]). Parts outside `word/`, relationship parts, the project
    /// itself and missing targets are skipped (logged); at most [`MAX_VBA_RELATED`] are kept.
    pub(super) fn read_vba_related(&mut self, project: &str) {
        let pkg = self.pkg;
        let types = self.content_types.get_or_insert_with(|| ContentTypes::read(pkg));
        let mut manifest = String::new();
        let mut kept = 0usize;
        for rel in self.pkg.rels(project).list.iter().filter(|r| !r.external) {
            // Part names compare case-insensitively (OPC): `/WORD/x` names the part the writer
            // stores as `word/x`.
            let target = rel.target.as_str();
            let path = match (target.get(..5), target.get(5..)) {
                (Some(dir), Some(rest)) if dir.eq_ignore_ascii_case("word/") => format!("word/{rest}"),
                _ => target.to_string(),
            };
            let path = path.as_str();
            let ct = types.of(path).unwrap_or("application/octet-stream");
            let usable = path.strip_prefix("word/").is_some_and(|t| !t.is_empty())
                && !path.to_ascii_lowercase().ends_with(".rels")
                && !path.eq_ignore_ascii_case(project)
                && !path.eq_ignore_ascii_case(VBA_PROJECT_PART)
                && ![rel.kind.as_str(), path, ct].iter().any(|f| f.is_empty() || f.contains(['\t', '\r', '\n']));
            if !usable {
                log::warn!("docx: not carrying VBA-related part {path:?}");
                continue;
            }
            let Some(bytes) = self.pkg.get_shared(path) else {
                log::warn!("docx: VBA-related part {path} is missing; keeping the project without it");
                continue;
            };
            if kept >= MAX_VBA_RELATED {
                log::warn!("docx: more than {MAX_VBA_RELATED} VBA-related parts; the rest are dropped");
                break;
            }
            kept += 1;
            manifest.push_str(&format!("{}\t{path}\t{ct}\n", rel.kind));
            self.doc.passthrough.entry(path.to_string()).or_insert_with(|| bytes.clone());
        }
        if !manifest.is_empty() {
            self.doc.passthrough.insert(VBA_RELATED.into(), Arc::new(manifest.into_bytes()));
        }
    }

    /// The fonts the font table classes as roman (`w:font/w:family w:val="roman"`).
    pub(super) fn read_font_families(&mut self, path: &str) {
        let Ok(Some(table)) = self.xml(path) else { return };
        self.doc.settings.roman_fonts = table
            .children("w:font")
            .filter(|f| f.child("w:family").and_then(|v| v.attr("w:val")) == Some("roman"))
            .filter_map(|f| f.attr("w:name"))
            .filter(|n| !n.is_empty() && n.len() <= MAX_FONT_NAME)
            .take(MAX_TABLE_FONTS)
            .map(str::to_string)
            .collect();
    }

    pub(super) fn xml(&self, path: &str) -> Result<Option<El>, DocxError> {
        self.pkg.xml(path)
    }

    pub fn revision(&mut self, kind: RevisionKind, e: &El) -> u32 {
        self.revision_named(kind, e, None)
    }

    pub(super) fn revision_named(&mut self, kind: RevisionKind, e: &El, move_name: Option<String>) -> u32 {
        let author = e.attr("w:author").unwrap_or("").chars().take(256).collect::<String>();
        let date = e.attr("w:date").unwrap_or("").chars().take(64).collect::<String>();
        let k = match kind {
            RevisionKind::Insert => 0,
            RevisionKind::Delete => 1,
            RevisionKind::Format => 2,
        };
        let key = (k, author.clone(), date.clone(), move_name.clone());
        if let Some(i) = self.revs.get(&key) {
            return *i;
        }
        let i = self.doc.revisions.len() as u32;
        self.doc.revisions.push(Revision { kind, author, date, move_name });
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
        let bytes = self.pkg.get_shared(&rel.target)?;
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
        self.doc.media.insert(key.clone(), bytes.clone());
        self.media_by_path.insert(rel.target.clone(), key.clone());
        Some(key)
    }

    /// A chart or diagram part, parsed once however often it's referenced (`None` when unreadable).
    pub(super) fn graphic_part(&mut self, path: &str) -> Option<Arc<El>> {
        if let Some(p) = self.graphic_parts.get(path) {
            return p.clone();
        }
        let p = self.xml(path).ok().flatten().map(Arc::new);
        self.graphic_parts.insert(path.to_string(), p.clone());
        p
    }

    pub fn load_header_footer(&mut self, path: &str, footer: bool) -> Option<u32> {
        if let Some(id) = self.hf_by_path.get(path) {
            return Some(*id);
        }
        if self.doc.parts.len() >= MAX_PARTS || self.hf_depth >= MAX_HF_DEPTH {
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
        sc.in_hf = true;
        let mut blocks = Blocks::new();
        self.hf_depth += 1;
        self.read_blocks(&mut sc, &root, &rels, &mut blocks, 0);
        self.hf_depth = self.hf_depth.saturating_sub(1);
        self.flush_pending(&mut sc, &mut blocks);
        if let Some(p) = self.doc.parts.get_mut(&id)
            && !blocks.is_empty()
        {
            p.blocks = blocks;
        }
        Some(id)
    }

    pub(super) fn read_theme(&mut self, path: &str) {
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
            self.pc.theme_colors = colors.clone();
            self.doc.settings.theme_colors = colors;
        }
        self.theme_fills = shape_style::ThemeFills::read(el.child("a:fmtScheme"));
        if let Some(fs) = el.child("a:fontScheme") {
            let face =
                |n: &str| fs.child(n).and_then(|f| f.child("a:latin")).and_then(|l| l.attr("typeface")).filter(|t| !t.is_empty()).map(str::to_string);
            if let Some(f) = face("a:majorFont") {
                self.doc.settings.major_font = f;
            }
            if let Some(f) = face("a:minorFont") {
                self.doc.settings.minor_font = f;
            }
            let cs = |n: &str| fs.child(n).and_then(|f| f.child("a:cs")).and_then(|c| c.attr("typeface")).unwrap_or_default().to_string();
            self.pc.major_cs = cs("a:majorFont");
            self.pc.minor_cs = cs("a:minorFont");
            // Per-script fonts (`a:font script="Arab"`); Office's themes list about 30.
            let scripts = |n: &str| -> Vec<(String, String)> {
                let Some(f) = fs.child(n) else { return Vec::new() };
                f.els()
                    .filter(|e| e.name == "a:font")
                    .filter_map(|e| Some((e.attr("script")?, e.attr("typeface")?)))
                    .filter(|(s, t)| s.len() <= 8 && !t.is_empty() && t.len() < 256)
                    .take(256)
                    .map(|(s, t)| (s.to_string(), t.to_string()))
                    .collect()
            };
            self.pc.major_scripts = scripts("a:majorFont");
            self.pc.minor_scripts = scripts("a:minorFont");
        }
        self.pc.major_font = self.doc.settings.major_font.clone();
        self.pc.minor_font = self.doc.settings.minor_font.clone();
        self.pc.theme = self.doc.settings.theme_colors.clone();
    }

    /// `w:settings/w:clrSchemeMapping`: which scheme colour text1, background1… stand for. Also
    /// `w:themeFontLang/@w:bidi`, the language whose script picks the `…Bidi` theme fonts.
    pub(super) fn read_color_mapping(&mut self, path: &str) {
        let Ok(Some(root)) = self.xml(path) else { return };
        self.pc.bidi_script = root.child("w:themeFontLang").and_then(|l| l.attr("w:bidi")).filter(|v| !v.trim().is_empty()).map(props::lang_script);
        let Some(m) = root.child("w:clrSchemeMapping") else { return };
        let names = [
            "dark1",
            "light1",
            "dark2",
            "light2",
            "accent1",
            "accent2",
            "accent3",
            "accent4",
            "accent5",
            "accent6",
            "hyperlink",
            "followedHyperlink",
        ];
        let attrs = [
            "w:t1",
            "w:bg1",
            "w:t2",
            "w:bg2",
            "w:accent1",
            "w:accent2",
            "w:accent3",
            "w:accent4",
            "w:accent5",
            "w:accent6",
            "w:hyperlink",
            "w:followedHyperlink",
        ];
        let mut map = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        for (slot, a) in map.iter_mut().zip(attrs) {
            if let Some(i) = m.attr(a).and_then(|v| names.iter().position(|n| *n == v)) {
                *slot = i;
            }
        }
        self.pc.color_map = Some(map);
    }
}
