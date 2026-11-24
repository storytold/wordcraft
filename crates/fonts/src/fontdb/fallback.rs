use super::face::{FALLBACK_FAMILY, FontFace};
#[cfg(not(target_arch = "wasm32"))]
use super::fallback_data::{CoverageIndex, IndexedFile, SYS_MISSES_MAX, SYSTEM_FALLBACKS, file_stamp, ranges_hold, read_fallback_file};
use super::fallback_data::{
    DiagramFallbackKey, FALLBACK_CACHE_MAX, RTL_FALLBACKS, RUN_FALLBACK_CACHE_CHARS, RUN_FALLBACK_CACHE_MAX, arabic_block, fallback_order,
};
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

    /// The fallback search, cached per (character, excluded face) until loaded faces or font
    /// generation change.
    fn cached_fallback(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        let generation = self.generation();
        let n = self.read_faces().len();
        {
            let cache = self.fallbacks.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.faces == n
                && cache.generation == generation
                && let Some(hit) = cache.map.get(&(c, exclude))
            {
                return hit.clone();
            }
        }
        let (found, n) = self.search_fallback(c, exclude);
        // Keyed by the face count and generation the answer was chosen from (a system fallback may
        // have loaded fonts, and another thread may have loaded or changed fonts since: an answer
        // from before that change is returned but not cached under the new state).
        if n != self.read_faces().len() || generation != self.generation() {
            return found;
        }
        let mut cache = self.fallbacks.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if n != self.read_faces().len() || generation != self.generation() {
            return found;
        }
        if cache.faces != n || cache.generation != generation || cache.map.len() >= FALLBACK_CACHE_MAX {
            cache.map.clear();
            cache.faces = n;
            cache.generation = generation;
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
        let generation = self.generation();
        let n = self.read_faces().len();
        {
            let cache = self.run_fallbacks.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.faces == n
                && cache.generation == generation
                && let Some(hit) = cache.map.get(&key)
            {
                return hit.clone();
            }
        }
        // Keyed by the face count and generation the search last looked at, not the state after it:
        // a face loaded or changed meanwhile by another thread (one its search, a family named by a
        // document, or a user font change) must not be overwritten with a stale answer.
        let (found, n) = self.search_run(&key.0, exclude, bold, italic);
        if n != self.read_faces().len() || generation != self.generation() {
            return found;
        }
        let mut cache = self.run_fallbacks.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if n != self.read_faces().len() || generation != self.generation() {
            return found;
        }
        if cache.faces != n
            || cache.generation != generation
            || cache.map.len() >= RUN_FALLBACK_CACHE_MAX
            || cache.chars.saturating_add(key.0.len()) > RUN_FALLBACK_CACHE_CHARS
        {
            cache.map.clear();
            cache.faces = n;
            cache.generation = generation;
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
        let before_style = self.read_faces().len();
        let styled = self.face(&base.family, style);
        if before_style == checked {
            checked = self.read_faces().len();
        }
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

    /// First face covering all characters in `chars`, preferring faces matching `prefer_serif`
    /// (so a diagram maintains a consistent fallback face across its nodes matching the
    /// diagram's primary style, #5672). Does not alter body text fallback ordering. Cached until
    /// the loaded faces or font database generation changes, so page redraws do not repeat system scans.
    pub fn fallback_for_diagram(&self, chars: &[char], exclude: u32, prefer_serif: bool) -> Option<Arc<FontFace>> {
        if chars.is_empty() {
            return None;
        }
        let mut set: Vec<char> = chars.to_vec();
        set.sort_unstable();
        set.dedup();
        let key: DiagramFallbackKey = (set, exclude, prefer_serif);
        let generation = self.generation();
        let n = self.read_faces().len();
        {
            let cache = self.diagram_fallbacks.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.faces == n
                && cache.generation == generation
                && let Some(hit) = cache.map.get(&key)
            {
                return hit.clone();
            }
        }
        // Keyed by the face count and generation the search last looked at, not the state after it:
        // a face loaded or added meanwhile (for instance via `add_font`) must not be shadowed by
        // caching an older answer under the newer face count.
        let (found, n) = self.search_diagram(&key.0, exclude, prefer_serif);
        if n != self.read_faces().len() || generation != self.generation() || key.0.len() > RUN_FALLBACK_CACHE_CHARS {
            return found;
        }
        #[cfg(test)]
        if let Some(hook) = self.diagram_publish_hook.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            hook(self);
        }
        let mut cache = self.diagram_fallbacks.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if n != self.read_faces().len() || generation != self.generation() {
            return found;
        }
        if cache.faces != n
            || cache.generation != generation
            || cache.map.len() >= RUN_FALLBACK_CACHE_MAX
            || cache.chars.saturating_add(key.0.len()) > RUN_FALLBACK_CACHE_CHARS
        {
            cache.map.clear();
            cache.faces = n;
            cache.generation = generation;
            cache.chars = 0;
        }
        cache.chars = cache.chars.saturating_add(key.0.len());
        cache.map.insert(key, found.clone());
        found
    }

    /// The face covering a diagram's missing character set, and the face count it was chosen from.
    /// Prefers faces matching `prefer_serif`; when the best loaded face has a mismatched serif
    /// classification, checks system font candidates and retains the loaded face only when no
    /// matching candidate covers the full set (#5672).
    fn search_diagram(&self, chars: &[char], exclude: u32, prefer_serif: bool) -> (Option<Arc<FontFace>>, usize) {
        #[cfg(test)]
        self.diagram_searches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let (mut base, mut checked) = self.loaded_diagram_covering(chars, exclude, prefer_serif);
        #[cfg(not(target_arch = "wasm32"))]
        if base.is_none() || base.as_ref().is_some_and(|f| f.is_serif() != prefer_serif) {
            for c in chars {
                if self.read_faces().iter().any(|f| f.id != exclude && f.covers(*c) && f.is_serif() == prefer_serif) {
                    continue;
                }
                if self.system_fallback_matching(*c, Some((exclude, prefer_serif))) {
                    let (cand, cand_checked) = self.loaded_diagram_covering(chars, exclude, prefer_serif);
                    let matching = cand.as_ref().is_some_and(|f| f.is_serif() == prefer_serif);
                    if let Some(f) = cand {
                        base = Some(f);
                        checked = cand_checked;
                    } else if base.is_none() {
                        checked = cand_checked;
                    }
                    if matching {
                        break;
                    }
                }
            }
            if base.as_ref().is_none_or(|f| f.is_serif() != prefer_serif) {
                self.indexed_run_fallback_matching(chars, Some((exclude, prefer_serif)));
                let (cand, cand_checked) = self.loaded_diagram_covering(chars, exclude, prefer_serif);
                if let Some(f) = cand {
                    base = Some(f);
                    checked = cand_checked;
                } else if base.is_none() {
                    checked = cand_checked;
                }
            }
        }
        #[cfg(test)]
        if let Some(hook) = self.diagram_search_hook.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            hook(self);
        }
        (base, checked)
    }

    /// The first face covering every char in `chars`, preferring matching `prefer_serif`,
    /// and the number of loaded faces it was chosen from.
    fn loaded_diagram_covering(&self, chars: &[char], exclude: u32, prefer_serif: bool) -> (Option<Arc<FontFace>>, usize) {
        let faces = self.read_faces();
        let rtl = chars.iter().any(|c| crate::is_rtl(*c) || arabic_block(*c));
        let complex = chars.iter().copied().find(|c| crate::needs_layout_tables(*c));
        let mut candidates: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.id != exclude && chars.iter().all(|c| f.covers(*c))).collect();
        let rank = |f: &FontFace| {
            if rtl { RTL_FALLBACKS.iter().position(|n| f.family.eq_ignore_ascii_case(n)).unwrap_or(RTL_FALLBACKS.len()) } else { 0 }
        };
        let shaping = |f: &FontFace| match complex {
            Some(c) => (!f.has_layout_tables(), crate::fallback_rank(c, &f.family)),
            None => (false, 0),
        };
        let serif_mismatch = |f: &FontFace| f.is_serif() != prefer_serif;
        candidates.sort_by_cached_key(|f| {
            (rank(f), shaping(f), serif_mismatch(f), !f.family.eq_ignore_ascii_case(FALLBACK_FAMILY), f.italic, (f.weight - 400.0).abs() as i32)
        });
        (candidates.first().map(|f| (*f).clone()), faces.len())
    }

    /// Load a system font covering `c` (preferred fallback families first, then any cataloged
    /// file under 40 MB, both found through the coverage index). Returns true if one was loaded.
    /// Misses are remembered.
    #[cfg(not(target_arch = "wasm32"))]
    fn system_fallback(&self, c: char) -> bool {
        self.system_fallback_matching(c, None)
    }

    /// Like [`Self::system_fallback`], optionally requiring the covering face to not be `exclude`
    /// and to match `prefer_serif` (so a loaded mismatched face does not short-circuit the search
    /// before a matching installed candidate is checked, #5672).
    #[cfg(not(target_arch = "wasm32"))]
    fn system_fallback_matching(&self, c: char, style: Option<(u32, bool)>) -> bool {
        if c.is_control() || c.is_whitespace() {
            return false;
        }
        {
            let sys = self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !sys.enabled || sys.misses.contains(&c) {
                return false;
            }
        }
        let covered = |db: &FontDb| {
            db.read_faces().iter().any(|f| f.covers(c) && style.is_none_or(|(exclude, prefer_serif)| f.id != exclude && f.is_serif() == prefer_serif))
        };
        let cataloged: Vec<String> = self.read_catalog().iter().map(|e| e.family.clone()).collect();
        let rtl = crate::is_rtl(c) || arabic_block(c);
        let preferred = RTL_FALLBACKS.iter().filter(|_| rtl);
        for fam in crate::script_fallbacks(c).iter().chain(preferred).chain(SYSTEM_FALLBACKS.iter()) {
            // Each family is tried once: one that fails to load isn't read again per character.
            if self.is_loaded(fam) || !cataloged.iter().any(|f| f.eq_ignore_ascii_case(fam)) {
                continue;
            }
            // A family none of whose files maps `c` is not loaded for it: it would stay loaded
            // and help nothing (an emoji loaded every CJK family a Mac has first, 500 MB).
            if !self.family_maps(fam, c) {
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
        if style.is_none() {
            if self.indexed_fallback(c) {
                return true;
            }
        } else if self.load_indexed_matching(&[c], false, style) {
            return true;
        }
        if style.is_none() || !self.read_faces().iter().any(|f| f.covers(c)) {
            let mut sys = self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if sys.misses.len() >= SYS_MISSES_MAX {
                sys.misses.clear();
            }
            sys.misses.insert(c);
        }
        false
    }

    /// Does a file of the installed `family` map `c`? Answered by the coverage index, which reads
    /// a file's character maps the first time it is asked about it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn family_maps(&self, family: &str, c: char) -> bool {
        let paths = self.family_paths(family);
        let mut index = self.coverage_index();
        paths.iter().any(|p| index.file_holds(p, u32::from(c)))
    }

    /// The coverage index, its file list brought up to date with the catalog.
    #[cfg(not(target_arch = "wasm32"))]
    fn coverage_index(&self) -> std::sync::MutexGuard<'_, CoverageIndex> {
        let mut index = self.coverage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.catalog_gen.load(Ordering::Acquire);
        if index.generation != Some(generation) {
            // A new scan (or the first search): keep what is known of files still installed,
            // unless a file was replaced since its character maps were read.
            let mut known: HashMap<std::path::PathBuf, IndexedFile> =
                std::mem::take(&mut index.files).into_iter().map(|f| (f.path.clone(), f)).collect();
            let mut paths: Vec<std::path::PathBuf> =
                self.catalog.read().unwrap_or_else(std::sync::PoisonError::into_inner).iter().map(|e| e.path.clone()).collect();
            paths.sort();
            paths.dedup();
            let files = paths.into_iter().map(|path| match known.remove(&path) {
                Some(mut file) => {
                    if file.ranges.is_some() && file.stamp != file_stamp(&file.path) {
                        file.ranges = None;
                    }
                    file
                }
                None => IndexedFile { used: index.loaded.contains(&path), path, ranges: None, stamp: None },
            });
            index.files = files.collect();
            index.generation = Some(generation);
        }
        index
    }

    /// Load the first cataloged font file (in path order) that maps `c`, reading the character
    /// maps of each file not yet indexed as the search reaches it. Returns whether a loaded face
    /// now covers `c`.
    #[cfg(not(target_arch = "wasm32"))]
    fn indexed_fallback(&self, c: char) -> bool {
        self.load_indexed_matching(&[c], false, None)
    }

    /// Load the family of the first cataloged font file (in path order) that maps every char of a
    /// run no loaded face covers whole: all its styles, as path order may reach the Bold file
    /// before the Regular.
    #[cfg(not(target_arch = "wasm32"))]
    fn indexed_run_fallback(&self, chars: &[char]) {
        self.indexed_run_fallback_matching(chars, None);
    }

    /// Like [`Self::indexed_run_fallback`], optionally requiring a loaded face matching
    /// `(exclude, prefer_serif)` to stop scanning (#5672).
    #[cfg(not(target_arch = "wasm32"))]
    fn indexed_run_fallback_matching(&self, chars: &[char], style: Option<(u32, bool)>) {
        if !self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner).enabled || chars.iter().any(|c| c.is_control()) {
            return;
        }
        self.load_indexed_matching(chars, true, style);
    }

    /// Load the cataloged font files (in path order) that map every char of `chars`, reading the
    /// character maps of each file not yet indexed as the search reaches it, until a loaded face
    /// covers them all; with `whole_family`, each such file's family is loaded with it (whatever
    /// of it isn't loaded yet). A collection's faces only together may map them: the search then
    /// goes on. Returns whether a loaded face now covers them all.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_indexed_matching(&self, chars: &[char], whole_family: bool, style: Option<(u32, bool)>) -> bool {
        let covered = |db: &FontDb| {
            db.read_faces().iter().any(|f| {
                style.is_none_or(|(exclude, prefer_serif)| f.id != exclude && f.is_serif() == prefer_serif) && chars.iter().all(|c| f.covers(*c))
            })
        };
        let cps: Vec<u32> = chars.iter().map(|c| u32::from(*c)).collect();
        let maps_all = |ranges: &[(u32, u32)]| cps.iter().all(|cp| ranges_hold(ranges, *cp));
        let mut index = self.coverage_index();
        let CoverageIndex { files, reads, .. } = &mut *index;
        for i in 0..files.len() {
            let Some(file) = files.get_mut(i).filter(|f| !f.used) else { continue };
            if !maps_all(file.ranges(reads)) {
                continue;
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
                if self.add_fonts(fonts) > 0 && covered(self) {
                    return true;
                }
                continue;
            }
            if let Some(data) = read_fallback_file(&file.path)
                && self.add_font(data) > 0
                && covered(self)
            {
                return true;
            }
        }
        false
    }
}
