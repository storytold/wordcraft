use super::face::{FALLBACK_FAMILY, FontFace};
#[cfg(not(target_arch = "wasm32"))]
use super::fallback_data::{CoverageIndex, IndexedFile, SYS_MISSES_MAX, SYSTEM_FALLBACKS, coverage_ranges, ranges_hold, read_fallback_file};
use super::fallback_data::{FALLBACK_CACHE_MAX, RTL_FALLBACKS, RUN_FALLBACK_CACHE_CHARS, RUN_FALLBACK_CACHE_MAX, arabic_block, fallback_order};
use super::{Arc, FontDb};
#[cfg(not(target_arch = "wasm32"))]
use super::{HashMap, Ordering};

impl FontDb {
    /// First face (fallback family first, then load order) that covers `c`; on native, system
    /// fonts are cataloged and loaded lazily the first time no loaded face covers a character.
    pub fn fallback_for(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        self.fallback_styled(c, exclude, false, false)
    }

    /// [`Self::fallback_for`], preferring a bold and/or italic face of the chosen family (so
    /// bold Persian text in a font without Persian letters stays bold).
    pub fn fallback_styled(&self, c: char, exclude: u32, bold: bool, italic: bool) -> Option<Arc<FontFace>> {
        let base = self.cached_fallback(c, exclude)?;
        if !bold && !italic {
            return Some(base);
        }
        let style = match (bold, italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            _ => "Italic",
        };
        let styled = self.face(&base.family, style);
        Some(if styled.family.eq_ignore_ascii_case(&base.family) && styled.covers(c) && styled.id != exclude { styled } else { base })
    }

    /// The fallback search, cached per (character, excluded face) until more faces load.
    fn cached_fallback(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        let n = self.read_faces().len();
        {
            let cache = self.fallbacks.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.faces == n
                && let Some(hit) = cache.map.get(&(c, exclude))
            {
                return hit.clone();
            }
        }
        let (found, n) = self.search_fallback(c, exclude);
        // Keyed by the face count the answer was chosen from (a system fallback may have loaded
        // fonts, and another thread may have loaded more since: an answer from before that load
        // is returned but not cached under the new count).
        if n != self.read_faces().len() {
            return found;
        }
        let mut cache = self.fallbacks.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.faces != n || cache.map.len() >= FALLBACK_CACHE_MAX {
            cache.map.clear();
            cache.faces = n;
        }
        cache.map.insert((c, exclude), found.clone());
        found
    }

    /// The fallback face for `c`, and the number of loaded faces it was chosen from.
    fn search_fallback(&self, c: char, exclude: u32) -> (Option<Arc<FontFace>>, usize) {
        if let Some(face) = self.circle_fallback(c, exclude) {
            return (Some(face), self.read_faces().len());
        }
        let (found, n) = self.loaded_fallback(c, exclude);
        if found.is_some() {
            return (found, n);
        }
        self.system_search(c, exclude)
    }

    fn circle_fallback(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        if c != '\u{25CF}' {
            return None;
        }
        // Word uses this text face for an uncovered black circle, before a UI sans.
        const FAMILY: &str = "Times New Roman";
        let find = || self.find(FAMILY, "Regular").filter(|f| f.id != exclude && f.covers(c));
        if let Some(face) = find() {
            return Some(face);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let enabled = self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner).enabled;
            if enabled {
                self.load_cataloged(FAMILY);
            }
        }
        find()
    }

    /// [`Self::search_fallback`] when no loaded face covers `c`: load a system font that does,
    /// one search at a time.
    #[cfg(not(target_arch = "wasm32"))]
    fn system_search(&self, c: char, exclude: u32) -> (Option<Arc<FontFace>>, usize) {
        let _searching = self.searching.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // A search that held the lock first may have loaded a font covering `c`: choose among the
        // loaded faces again before loading more.
        let (found, n) = self.loaded_fallback(c, exclude);
        if found.is_none() && self.system_fallback(c) {
            return self.loaded_fallback(c, exclude);
        }
        (found, n)
    }

    #[cfg(target_arch = "wasm32")]
    fn system_search(&self, _c: char, _exclude: u32) -> (Option<Arc<FontFace>>, usize) {
        (None, self.read_faces().len())
    }

    /// The best loaded face covering `c`, and the number of loaded faces it was chosen from.
    fn loaded_fallback(&self, c: char, exclude: u32) -> (Option<Arc<FontFace>>, usize) {
        let faces = self.read_faces();
        let mut order: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.id != exclude).collect();
        // Right-to-left letters prefer a font made for the script, so a word isn't split across
        // fonts (that breaks Arabic joining). Emoji prefer a colour emoji font.
        let rank = |f: &FontFace| {
            if crate::is_rtl(c) || arabic_block(c) {
                RTL_FALLBACKS.iter().position(|n| f.family.eq_ignore_ascii_case(n)).unwrap_or(RTL_FALLBACKS.len())
            } else if crate::is_emoji(c) {
                crate::fallback_rank(c, &f.family)
            } else {
                0
            }
        };
        // Complex scripts (Myanmar, Khmer, Indic…): a font with OpenType layout tables, preferring
        // the script's own families, so syllables reorder and stack (#604).
        let complex = crate::needs_layout_tables(c);
        let shaping = |f: &FontFace| if complex { (!f.has_layout_tables(), crate::fallback_rank(c, &f.family)) } else { (false, 0) };
        order.sort_by_key(|f| (rank(f), shaping(f), !f.family.eq_ignore_ascii_case(FALLBACK_FAMILY), f.italic, (f.weight - 400.0).abs() as i32));
        (order.into_iter().find(|f| f.covers(c)).cloned(), faces.len())
    }

    /// One fallback face for a run: the first face in the fallback order that covers **every**
    /// given character, so a whole run draws in a single face instead of mixing faces character
    /// by character. East Asian runs: a bundled Japanese Mincho for JIS-covered ideographs beside
    /// a Simplified-Chinese sans for the rest. A missing Thai font (#1549): the Thai letters in
    /// one face and the Latin letters and digits beside them in another, at another size; a Thai
    /// face covers them all, so the run is given every character, Latin ones included.
    /// Complex-script characters prefer the script's own families with OpenType layout tables, as
    /// [`Self::fallback_for`] does. Like it, a system font is loaded lazily the first time no
    /// loaded face covers a character. `None` when no single face covers them all, and the caller
    /// keeps the per-character fallback.
    ///
    /// Cached per (character set, excluded face, bold, italic) and dropped when the loaded faces
    /// change, like [`Self::fallback_for`], so relayout doesn't repeat the search.
    pub fn fallback_for_run(&self, chars: &[char], exclude: u32, bold: bool, italic: bool) -> Option<Arc<FontFace>> {
        if chars.is_empty() {
            return None;
        }
        let mut set: Vec<char> = chars.to_vec();
        set.sort_unstable();
        set.dedup();
        let key = (set, exclude, bold, italic);
        let n = self.read_faces().len();
        {
            let cache = self.run_fallbacks.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.faces == n
                && let Some(hit) = cache.map.get(&key)
            {
                return hit.clone();
            }
        }
        // Keyed by the face count the search last looked at, not the count after it: a face loaded
        // meanwhile by another thread (one its search, or a family named by a document) must make
        // a miss count as stale, not as the answer for the new count.
        let (found, n) = self.search_run(&key.0, exclude, bold, italic);
        let mut cache = self.run_fallbacks.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.faces != n || cache.map.len() >= RUN_FALLBACK_CACHE_MAX || cache.chars.saturating_add(key.0.len()) > RUN_FALLBACK_CACHE_CHARS {
            cache.map.clear();
            cache.faces = n;
            cache.chars = 0;
        }
        cache.chars = cache.chars.saturating_add(key.0.len());
        cache.map.insert(key, found.clone());
        found
    }

    /// The face for a run, and the number of loaded faces the search last chose from.
    fn search_run(&self, chars: &[char], exclude: u32, bold: bool, italic: bool) -> (Option<Arc<FontFace>>, usize) {
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let (mut base, mut checked) = self.loaded_covering(chars, exclude);
        #[cfg(not(target_arch = "wasm32"))]
        if base.is_none() {
            // A character no loaded face covers: load a system font for it, then retry (the run's
            // other characters may already be covered by loaded faces).
            for c in chars {
                if self.read_faces().iter().any(|f| f.id != exclude && f.covers(*c)) {
                    continue;
                }
                if self.system_fallback(*c) {
                    (base, checked) = self.loaded_covering(chars, exclude);
                    if base.is_some() {
                        break;
                    }
                }
            }
            // Each character has a face now, but the families a script prefers may lack the
            // run's other letters (Noto Sans Thai has no Latin): look for one installed font
            // that maps the whole run.
            if base.is_none() {
                // Look again even when this search loaded nothing: another thread's may just have
                // loaded the font (and taken its files off the index).
                self.indexed_run_fallback(chars);
                (base, checked) = self.loaded_covering(chars, exclude);
            }
        }
        let Some(base) = base else { return (None, checked) };
        if !bold && !italic {
            return (Some(base), checked);
        }
        let style = match (bold, italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            _ => "Italic",
        };
        let styled = self.face(&base.family, style);
        let face = if styled.family.eq_ignore_ascii_case(&base.family) && chars.iter().all(|c| styled.covers(*c)) && styled.id != exclude {
            styled
        } else {
            base
        };
        (Some(face), checked)
    }

    /// The first face in the fallback order that covers every char, and the number of loaded
    /// faces it was chosen from.
    fn loaded_covering(&self, chars: &[char], exclude: u32) -> (Option<Arc<FontFace>>, usize) {
        let faces = self.read_faces();
        let rtl = chars.iter().any(|c| crate::is_rtl(*c) || arabic_block(*c));
        // A complex-script character picks the families its script prefers (the lowest one: the
        // run's characters come sorted).
        let complex = chars.iter().copied().find(|c| crate::needs_layout_tables(*c));
        let found = fallback_order(&faces, exclude, rtl, complex).into_iter().find(|f| chars.iter().all(|c| f.covers(*c))).cloned();
        (found, faces.len())
    }

    /// Load a system font covering `c` (preferred fallback families first, then any cataloged
    /// file under 40 MB, found through the coverage index). Returns true if one was loaded.
    /// Misses are remembered.
    #[cfg(not(target_arch = "wasm32"))]
    fn system_fallback(&self, c: char) -> bool {
        if c.is_control() || c.is_whitespace() {
            return false;
        }
        {
            let sys = self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !sys.enabled || sys.misses.contains(&c) {
                return false;
            }
        }
        let covered = |db: &FontDb| db.read_faces().iter().any(|f| f.covers(c));
        let cataloged: Vec<String> = self.read_catalog().iter().map(|e| e.family.clone()).collect();
        let rtl = crate::is_rtl(c) || arabic_block(c);
        let preferred = RTL_FALLBACKS.iter().filter(|_| rtl);
        for fam in crate::script_fallbacks(c).iter().chain(preferred).chain(SYSTEM_FALLBACKS.iter()) {
            // Each family is tried once: one that fails to load isn't read again per character.
            if self.is_loaded(fam) || !cataloged.iter().any(|f| f.eq_ignore_ascii_case(fam)) {
                continue;
            }
            if !self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner).tried.insert(fam) {
                continue;
            }
            self.load_cataloged(fam);
            if covered(self) {
                return true;
            }
        }
        if self.indexed_fallback(c) {
            return true;
        }
        let mut sys = self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if sys.misses.len() >= SYS_MISSES_MAX {
            sys.misses.clear();
        }
        sys.misses.insert(c);
        false
    }

    /// Load the first cataloged font file (in path order) that maps `c`, reading each file not
    /// yet indexed as the search reaches it. Returns whether a loaded face now covers `c`.
    #[cfg(not(target_arch = "wasm32"))]
    fn indexed_fallback(&self, c: char) -> bool {
        self.load_indexed(&[c], false)
    }

    /// Load the family of the first cataloged font file (in path order) that maps every char of a
    /// run no loaded face covers whole: all its styles, as path order may reach the Bold file
    /// before the Regular.
    #[cfg(not(target_arch = "wasm32"))]
    fn indexed_run_fallback(&self, chars: &[char]) {
        if !self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner).enabled || chars.iter().any(|c| c.is_control()) {
            return;
        }
        self.load_indexed(chars, true);
    }

    /// Load the cataloged font files (in path order) that map every char of `chars`, reading each
    /// file not yet indexed as the search reaches it, until a loaded face covers them all; with
    /// `whole_family`, each such file's family is loaded with it (whatever of it isn't loaded
    /// yet). A collection's faces only together may map them: the search then goes on. Returns
    /// whether a loaded face now covers them all.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_indexed(&self, chars: &[char], whole_family: bool) -> bool {
        let covered = |db: &FontDb| db.read_faces().iter().any(|f| chars.iter().all(|c| f.covers(*c)));
        let cps: Vec<u32> = chars.iter().map(|c| u32::from(*c)).collect();
        let maps_all = |ranges: &[(u32, u32)]| cps.iter().all(|cp| ranges_hold(ranges, *cp));
        let mut index = self.coverage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.catalog_gen.load(Ordering::Acquire);
        if index.generation != Some(generation) {
            // A new scan (or the first search): keep what is known of files still installed.
            let mut known: HashMap<std::path::PathBuf, IndexedFile> =
                std::mem::take(&mut index.files).into_iter().map(|f| (f.path.clone(), f)).collect();
            let mut paths: Vec<std::path::PathBuf> =
                self.catalog.read().unwrap_or_else(std::sync::PoisonError::into_inner).iter().map(|e| e.path.clone()).collect();
            paths.sort();
            paths.dedup();
            let files = paths
                .into_iter()
                .map(|path| known.remove(&path).unwrap_or_else(|| IndexedFile { used: index.loaded.contains(&path), path, ranges: None }));
            index.files = files.collect();
            index.generation = Some(generation);
        }
        let CoverageIndex { files, reads, .. } = &mut *index;
        for i in 0..files.len() {
            let Some(file) = files.get_mut(i).filter(|f| !f.used) else { continue };
            let mut data = None;
            match &file.ranges {
                Some(r) if !maps_all(r) => continue,
                Some(_) => {}
                None => {
                    *reads += 1;
                    data = read_fallback_file(&file.path);
                    let ranges = data.as_deref().map(coverage_ranges).unwrap_or_default();
                    let hit = maps_all(&ranges);
                    file.ranges = Some(ranges);
                    if !hit {
                        continue;
                    }
                }
            }
            // The file maps them: load it (once; its faces are loaded fonts from now on).
            file.used = true;
            if whole_family && let Some(name) = self.read_catalog().iter().find(|e| e.path == file.path).map(|e| e.family.clone()) {
                // With the rest of its family not loaded yet: path order may reach the Bold file
                // before the Regular, and a single-character fallback may have loaded one style.
                let this = file.path.clone();
                let family = self.family_paths(&name);
                let mut load = Vec::new();
                for f in files.iter_mut().filter(|f| f.path == this || !f.used && family.contains(&f.path)) {
                    f.used = true;
                    load.push(f.path.clone());
                }
                let fonts: Vec<(Vec<u8>, Option<String>)> = load.iter().filter_map(|p| read_fallback_file(p)).map(|b| (b, None)).collect();
                *reads += fonts.len();
                if self.add_fonts(fonts) > 0 && covered(self) {
                    return true;
                }
                continue;
            }
            if data.is_none() {
                *reads += 1;
                data = read_fallback_file(&file.path);
            }
            if let Some(data) = data
                && self.add_font(data) > 0
                && covered(self)
            {
                return true;
            }
        }
        false
    }
}
