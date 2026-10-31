use super::face::{FALLBACK_FAMILY, FontBytes, FontFace, OUTLINE_CACHE_MAX};
#[cfg(not(target_arch = "wasm32"))]
use super::fallback_data::read_fallback_file;
#[cfg(not(target_arch = "wasm32"))]
use super::names::{CatalogEntry, MAX_FILE_FACES, enumerate_faces};
use super::names::{base_style, last_resort_face, make_face, norm, style_italic, style_weight};
use super::{Arc, BezPath, FontDb, MetadataProvider};

/// Drops from `cands` (the faces whose typographic family is `family`) those whose own legacy
/// family name (name ID 1) is another family's, when some keep `family`'s name. Word finds a font
/// by that name: TH SarabunIT๙ gives "TH SarabunPSK" as its typographic family (name ID 16), but
/// a document asking for TH SarabunPSK gets TH SarabunPSK's file, not IT๙'s, whose digits are
/// drawn Thai. A family's styles named after it ("Roboto Light", "Arial Nova Cond") stay.
fn keep_own_faces<T>(cands: &mut Vec<&T>, family: &str, legacy: impl Fn(&T) -> &str) {
    let own = |c: &&T| owns_name(legacy(c), family);
    if cands.iter().any(own) {
        cands.retain(own);
    }
}

/// Is a face with legacy family name `legacy` one of `family`'s own: `family` itself, or a style
/// named after it ("Roboto Light")? See `keep_own_faces`.
pub(super) fn owns_name(legacy: &str, family: &str) -> bool {
    legacy.get(..family.len()).is_some_and(|p| p.eq_ignore_ascii_case(family)) && legacy[family.len()..].chars().next().is_none_or(|c| c == ' ')
}

/// Is `f`, found for `family`, a face claiming the family only by its typographic name (see
/// `keep_own_faces`)?
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn is_claimant(f: &FontFace, family: &str) -> bool {
    f.family.eq_ignore_ascii_case(family) && !owns_name(&f.legacy_family, family)
}

impl FontDb {
    /// `f`, found for `family` and `style`, unless it is a loaded claimant (see `keep_own_faces`,
    /// e.g. a user font) and the family's own faces are installed: they load and the face chosen
    /// among them is returned instead.
    pub(super) fn owner_over_claimant(&self, family: &str, style: &str, f: Arc<FontFace>) -> Arc<FontFace> {
        #[cfg(not(target_arch = "wasm32"))]
        if is_claimant(&f, family) {
            // Look again even when this load added nothing: another thread may have just loaded
            // the family's own faces.
            self.load_cataloged_owner(family);
            if let Some(own) = self.find(family, style).filter(|own| !is_claimant(own, family)) {
                return own;
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (family, style);
        f
    }

    /// [`FontDb::preview_face`]'s `f` for `family`, unless it is a loaded claimant and the
    /// family's own faces are installed: then their preview face, read without loading them, so
    /// the list shows the face [`FontDb::face`] would choose.
    fn preview_owner_over_claimant(&self, family: &str, f: Arc<FontFace>) -> Arc<FontFace> {
        #[cfg(not(target_arch = "wasm32"))]
        if is_claimant(&f, family)
            && self.catalog_has_owner(family)
            && let Some(own) = self.unshared_face(family)
        {
            return own;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = family;
        f
    }

    /// Resolve a family + style to a face, falling back to the closest style of the family, then to
    /// Source Sans 3 Regular. `family` is a typographic family ("Roboto") or a face's legacy family
    /// name ("Roboto Light", whose styles are Light and Light Italic). Installed system fonts are
    /// found by name whatever ran before. An exact full or PostScript name selects its face.
    pub fn face(&self, family: &str, style: &str) -> Arc<FontFace> {
        if let Some(f) = self.find(family, style) {
            return self.owner_over_claimant(family, style, f);
        }
        // Look again even when this load added nothing: another thread may have just loaded it.
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.load_cataloged(family);
            if let Some(f) = self.find(family, style) {
                return self.owner_over_claimant(family, style, f);
            }
        }
        // A localized family name (宋体 → SimSun).
        if let Some(en) = self.english_family(family).filter(|en| !en.eq_ignore_ascii_case(family)) {
            if let Some(f) = self.find(&en, style) {
                return self.owner_over_claimant(&en, style, f);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.load_cataloged(&en);
                if let Some(f) = self.find(&en, style) {
                    return self.owner_over_claimant(&en, style, f);
                }
            }
        }
        self.find(FALLBACK_FAMILY, style)
            .or_else(|| self.find(FALLBACK_FAMILY, "Regular"))
            .or_else(|| self.read_faces().first().cloned())
            .unwrap_or_else(last_resort_face)
    }

    /// The face a font list entry draws `family`'s name in: its Regular face (or the closest
    /// style), without loading the family (#848). A loaded family's face is shared; an installed
    /// one is read into a face of its own that the database doesn't keep, so its data is freed when
    /// the caller drops it. Browsing the font list (hundreds of families) then costs no lasting
    /// memory; only fonts actually used load for good. `None` when no font has that name, or its
    /// file is too large to read just for a preview.
    pub fn preview_face(&self, family: &str) -> Option<Arc<FontFace>> {
        if let Some(f) = self.find(family, "Regular") {
            return Some(self.preview_owner_over_claimant(family, f));
        }
        let english = self.english_family(family).filter(|en| !en.eq_ignore_ascii_case(family));
        if let Some((en, f)) = english.as_deref().and_then(|en| self.find(en, "Regular").map(|f| (en, f))) {
            return Some(self.preview_owner_over_claimant(en, f));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(f) = self.unshared_face(family).or_else(|| english.as_deref().and_then(|en| self.unshared_face(en))) {
            return Some(f);
        }
        None
    }

    /// The installed `family`'s face closest to Regular, read from its file into a face the
    /// database doesn't keep (see [`FontDb::preview_face`]). The family's own face first; when its
    /// file can't be read, a face claiming the family's name (see `keep_own_faces`), as
    /// [`FontDb::face`] then uses it.
    #[cfg(not(target_arch = "wasm32"))]
    fn unshared_face(&self, family: &str) -> Option<Arc<FontFace>> {
        self.unshared_picks(family).into_iter().find_map(|(path, fam, style, legacy)| Self::read_unshared(&path, &fam, &style, &legacy))
    }

    /// The cataloged faces [`FontDb::unshared_face`] tries for `family`, in order, as (path,
    /// family, style, legacy family): the one closest to Regular among the family's own faces,
    /// then among the faces claiming its name when it has both; or among the faces of that legacy
    /// family name.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn unshared_picks(&self, family: &str) -> Vec<(std::path::PathBuf, String, String, String)> {
        let cat = self.read_catalog();
        let named: Vec<&CatalogEntry> = cat.iter().filter(|c| c.family.eq_ignore_ascii_case(family)).collect();
        let mut own = named.clone();
        keep_own_faces(&mut own, family, |c| c.legacy.as_str());
        let groups: Vec<Vec<&CatalogEntry>> = if own.is_empty() {
            vec![cat.iter().filter(|c| c.legacy.eq_ignore_ascii_case(family)).collect()]
        } else if own.len() < named.len() {
            let claimants = named.into_iter().filter(|c| !owns_name(&c.legacy, family)).collect();
            vec![own, claimants]
        } else {
            vec![own]
        };
        let score = |c: &CatalogEntry| {
            if matches!(norm(&c.style).as_str(), "regular" | "normal" | "book" | "roman") {
                -1.0
            } else {
                (style_weight(&c.style) - 400.0).abs() + if style_italic(&c.style) { 1000.0 } else { 0.0 }
            }
        };
        groups
            .into_iter()
            .filter_map(|g| g.into_iter().min_by(|a, b| score(a).total_cmp(&score(b))))
            .map(|c| (c.path.clone(), c.family.clone(), c.style.clone(), c.legacy.clone()))
            .collect()
    }

    /// The face of the font file at `path` with that family, style and legacy family name, read
    /// into a face the database doesn't keep. `None` when the file can't be read or lacks it.
    #[cfg(not(target_arch = "wasm32"))]
    fn read_unshared(path: &std::path::Path, fam: &str, style: &str, legacy: &str) -> Option<Arc<FontFace>> {
        let data = Arc::new(read_fallback_file(path)?);
        // The face chosen, also in a collection holding another face of that family and style.
        let found = enumerate_faces(&data, MAX_FILE_FACES)
            .into_iter()
            .find(|f| f.family.eq_ignore_ascii_case(fam) && f.style.eq_ignore_ascii_case(style) && f.legacy.eq_ignore_ascii_case(legacy))?;
        let mut face = make_face(FontBytes::Owned(data), found.index, found.family, found.style, found.legacy, found.coords)?;
        face.local_names = found.local;
        Some(Arc::new(face))
    }

    /// Bytes of font data held by loaded faces (each file once; compiled-in fonts not counted).
    /// It grows as families load for use; font list previews leave it as is (#848).
    pub fn loaded_font_bytes(&self) -> usize {
        let mut seen: Vec<*const Vec<u8>> = Vec::new();
        let mut total = 0usize;
        for f in self.read_faces().iter() {
            if let FontBytes::Owned(d) = &f.bytes {
                let p = Arc::as_ptr(d);
                if !seen.contains(&p) {
                    seen.push(p);
                    total = total.saturating_add(d.len());
                }
            }
        }
        total
    }

    /// Is `family` available (loaded, or installed on the system)? English and localized family
    /// names and exact full or PostScript face names count.
    pub fn has_family(&self, family: &str) -> bool {
        if self.is_loaded(family) {
            return true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.read_catalog().iter().any(|c| {
            c.family.eq_ignore_ascii_case(family)
                || c.legacy.eq_ignore_ascii_case(family)
                || c.full_name.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(family))
                || c.postscript.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(family))
        }) {
            return true;
        }
        self.english_family(family).is_some()
    }

    /// Is a face named `family` (typographic, legacy, full or PostScript) loaded?
    pub fn is_loaded(&self, family: &str) -> bool {
        self.read_faces().iter().any(|f| f.named(family))
    }

    /// A variable font's axes: (tag, name, min, default, max), empty for static fonts.
    pub fn axes(&self, family: &str, style: &str) -> Vec<(String, String, f32, f32, f32)> {
        let face = self.face(family, base_style(style));
        let Some(f) = face.skrifa() else { return vec![] };
        f.axes()
            .iter()
            .map(|a| {
                let tag = String::from_utf8_lossy(&a.tag().to_be_bytes()).to_string();
                let name = f.localized_strings(a.name_id()).english_or_first().map(|s| s.to_string()).unwrap_or_else(|| tag.clone());
                (tag, name, a.min_value(), a.default_value(), a.max_value())
            })
            .collect()
    }

    /// `Style {wght:650,wdth:90}`: the named style's font at those axis values, made on first use.
    fn instance(&self, family: &str, style: &str) -> Option<Arc<FontFace>> {
        let open = style.find('{')?;
        let base = self.find(family, style[..open].trim())?;
        if base.skrifa().is_none_or(|f| f.axes().is_empty()) {
            return Some(base);
        }
        let mut coords = base.coords.clone();
        for kv in style[open + 1..].trim_end_matches('}').split(',') {
            let (k, v) = kv.split_once(':')?;
            let k = k.trim().as_bytes();
            let v: f32 = v.trim().parse().ok()?;
            if k.len() != 4 {
                return None;
            }
            let tag = [k[0], k[1], k[2], k[3]];
            match coords.iter_mut().find(|(t, _)| *t == tag) {
                Some(c) => c.1 = v,
                None => coords.push((tag, v)),
            }
        }
        let mut f = make_face(base.bytes.clone(), base.index, base.family.clone(), style.to_string(), base.legacy_family.clone(), coords)?;
        f.local_names = base.local_names.clone();
        let f = Arc::new(f);
        self.faces.write().unwrap_or_else(std::sync::PoisonError::into_inner).push(f.clone());
        Some(f)
    }

    /// The loaded face for `family` and `style`, without loading anything: among the faces of
    /// that typographic family (its own faces only, when it has some, see `keep_own_faces`),
    /// else those of that legacy family name, the style asked for or the nearest weight and slant;
    /// else the face with that full or PostScript name. A style with axis settings (`{…}`) is a
    /// variable-font instance, made on first use.
    pub(super) fn find(&self, family: &str, style: &str) -> Option<Arc<FontFace>> {
        if style.contains('{') {
            {
                let faces = self.read_faces();
                let cached: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.named(family) && f.style == style).collect();
                if let Some(f) = cached.iter().find(|f| owns_name(&f.legacy_family, family)) {
                    return Some((*f).clone());
                }
                // A claimant's instance (see `keep_own_faces`) only when the family has no face of
                // its own to make one from.
                let own_base = faces.iter().any(|f| f.family.eq_ignore_ascii_case(family) && owns_name(&f.legacy_family, family));
                if let Some(f) = cached.first().filter(|_| !own_base) {
                    return Some((*f).clone());
                }
            }
            return self.instance(family, style);
        }
        let faces = self.read_faces();
        let mut cands: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).collect();
        keep_own_faces(&mut cands, family, |f| f.legacy_family.as_str());
        if cands.is_empty() {
            // A legacy family name: choose among the faces it names (bold and italic as usual).
            cands = faces.iter().filter(|f| f.legacy_family.eq_ignore_ascii_case(family)).collect();
        }
        if cands.is_empty() {
            // Exact full names keep precedence over PostScript aliases; either selects one face.
            return faces
                .iter()
                .find(|f| f.full_name.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(family)))
                .or_else(|| faces.iter().find(|f| f.postscript.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(family))))
                .cloned();
        }
        let ns = norm(style);
        if let Some(f) = cands.iter().find(|f| norm(&f.style) == ns) {
            return Some((*f).clone());
        }
        let (tw, ti) = (style_weight(style), style_italic(style));
        cands
            .iter()
            .min_by(|a, b| {
                let sa = (a.weight - tw).abs() + if a.italic != ti { 1000.0 } else { 0.0 };
                let sb = (b.weight - tw).abs() + if b.italic != ti { 1000.0 } else { 0.0 };
                sa.total_cmp(&sb)
            })
            .map(|f| (*f).clone())
    }

    /// Glyph outline in font units, y-down (flipped), cached per (face, glyph).
    pub fn outline(&self, face: &FontFace, gid: u32) -> Arc<BezPath> {
        let key = (face.id, gid);
        if let Some(p) = self.outlines.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return p.clone();
        }
        let p = Arc::new(face.glyph_outline(gid));
        let mut cache = self.outlines.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= OUTLINE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, p.clone());
        p
    }
}
