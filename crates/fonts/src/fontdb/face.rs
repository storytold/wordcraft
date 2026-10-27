use super::names::{LocalName, east_asian_font};
use super::*;

/// The family used when a requested family is unknown (the UI sans).
pub const FALLBACK_FAMILY: &str = "Source Sans 3";

/// The bundled fonts (OFL), as font file bytes. Japanese fonts are not bundled: they come from
/// the optional craft-fonts build input ([`crate::japanese_document_fonts`]).
pub fn bundled() -> &'static [&'static [u8]] {
    BUNDLED
}

pub(super) static BUNDLED: &[&[u8]] = &[
    include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSans3-Semibold.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSans3-Bold.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSans3-It.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSerif4-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSerif4-It.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSerif4-Semibold.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSerif4-Bold.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSerif4-BoldIt.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf"),
];

#[derive(Clone)]
pub(super) enum FontBytes {
    Static(&'static [u8]),
    Owned(Arc<Vec<u8>>),
}

/// One loaded font face.
pub struct FontFace {
    pub(super) id: u32,
    /// Typographic family name (e.g. "Source Sans 3").
    pub family: String,
    /// Typographic style name (e.g. "Semibold", "Italic").
    pub style: String,
    /// The family name Word lists and stores for the face (name ID 1 style, e.g. "Source Sans 3
    /// Semibold"): the typographic family for Regular, Bold and Italic faces, the family plus the
    /// other style words for the rest. See [`legacy_family`].
    pub legacy_family: String,
    /// The family's names in other languages (宋体 for SimSun), from the `name` table. Lookups by
    /// family name accept them; the font list shows them in a matching interface language.
    pub local_names: Vec<LocalName>,
    /// Exact OpenType full name (name ID 4), distinct from family and style names.
    pub(super) full_name: Option<String>,
    /// Exact OpenType PostScript name (name ID 6).
    pub(super) postscript: Option<String>,
    /// usWeightClass-style weight (400 = regular).
    pub weight: f32,
    pub italic: bool,
    pub(super) bytes: FontBytes,
    pub(super) index: u32,
    pub upem: f64,
    /// Ascender in font units (positive = up).
    pub ascent: f64,
    /// Descender in font units (positive = down).
    pub descent: f64,
    /// Cap height and x height in font units (estimated from the ascent when the font has no OS/2
    /// values).
    pub cap_height: f64,
    pub x_height: f64,
    pub shaper: harfrust::ShaperData,
    /// Variable fonts: the named instance's axis settings (user units; empty = default instance),
    /// the normalised location and the shaper's view of it.
    pub coords: Vec<([u8; 4], f32)>,
    pub(super) location: Location,
    pub instance: Option<harfrust::ShaperInstance>,
    /// Basic Multilingual Plane coverage bitset, built on first use.
    pub(super) bmp: std::sync::OnceLock<Box<[u64]>>,
    // Some legacy CJK fonts assemble their contours with TrueType instructions.
    pub(super) outline_hinter: std::sync::OnceLock<Option<HintingInstance>>,
    /// Hinted draws so far, those that ran too long, and whether that turned hinting off
    /// ([`FontFace::count_hinted`]).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) hint_draws: std::sync::atomic::AtomicU32,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) hint_slow: std::sync::atomic::AtomicU32,
    pub(super) hinting_off: std::sync::atomic::AtomicBool,
    /// A hinted draw slower than this is a strike ([`HINT_STRIKE`]; tests shorten it).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) hint_strike: std::time::Duration,
}

/// Limits on what a face's TrueType instructions may run over (#3692). skrifa's interpreter budget
/// counts instructions, not the points each one moves (SHZ, IUP… walk a whole zone), so a font with
/// huge zones turns a short program into billions of point moves. Real fonts stay far below: a
/// twilight zone of tens to hundreds of points, a stack of up to ~2,000 values, a few thousand
/// points a glyph. Past them, glyphs are drawn unhinted.
const MAX_HINT_TWILIGHT: u16 = 1024;

const MAX_HINT_STACK: u16 = 4096;

/// skrifa's buffer for one hinted glyph (points, stack, twilight zone, CVT and storage).
const MAX_HINT_GLYPH_BYTES: usize = 256 * 1024;

/// A hinted glyph slower than this is a strike against its face. A face under the limits above can
/// still take seconds for one glyph, so [`MIN_HINT_STRIKES`] strikes that are also at least one
/// in [`HINT_SLOW_RATIO`] of its hinted draws turn hinting off: a face that is slow throughout
/// stops after a few glyphs, while the odd draw slowed by a busy machine leaves a face hinted.
///
/// Wall-clock, not interpreter work: skrifa's own budget (a million instructions a program, not
/// settable) counts instructions, not the points each one moves, and it exposes no count of the
/// work done, so time is the only measure there is (#3827). Whether such a face stays hinted can
/// therefore depend on the machine, by design: only faces that need the interpreter to assemble
/// their contours (legacy CJK fonts such as DFKai-SB) are hinted at all, and those typically hint
/// a glyph in microseconds, so 50 ms is far above an honest face on a loaded machine; a face
/// that trips it is drawn unhinted (its outlines possibly misassembled) rather than freezing the
/// app. Native only (no clock on wasm).
#[cfg(not(target_arch = "wasm32"))]
pub(super) const HINT_STRIKE: std::time::Duration = std::time::Duration::from_millis(50);

#[cfg(not(target_arch = "wasm32"))]
const MIN_HINT_STRIKES: u32 = 3;

#[cfg(not(target_arch = "wasm32"))]
const HINT_SLOW_RATIO: u32 = 16;

/// A cheap `Copy` handle to a face. Faces are never unloaded, so the handle lives for the rest of
/// the process; unlike cloning an `Arc`, copying it touches no shared reference count (which made
/// parallel composition scale negatively).
#[derive(Clone, Copy)]
pub struct FaceRef(pub(crate) &'static FontFace);

impl FaceRef {
    /// The handle for a loaded face (memoised per face).
    pub fn of(face: &Arc<FontFace>) -> FaceRef {
        static LEAKED: RwLock<Vec<Option<&FontFace>>> = RwLock::new(Vec::new());
        let id = face.id as usize;
        if let Some(Some(f)) = LEAKED.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(id) {
            return FaceRef(f);
        }
        let mut w = LEAKED.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if w.len() <= id {
            w.resize(id + 1, None);
        }
        let f: &'static FontFace = w[id].unwrap_or_else(|| {
            let keep: &'static Arc<FontFace> = Box::leak(Box::new(face.clone()));
            keep
        });
        w[id] = Some(f);
        FaceRef(f)
    }
    pub fn get(self) -> &'static FontFace {
        self.0
    }
}

impl std::ops::Deref for FaceRef {
    type Target = FontFace;
    fn deref(&self) -> &FontFace {
        self.0
    }
}

impl std::fmt::Debug for FaceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq for FaceRef {
    fn eq(&self, o: &FaceRef) -> bool {
        self.0.id == o.0.id
    }
}

impl std::fmt::Debug for FontFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FontFace({} {})", self.family, self.style)
    }
}

impl FontFace {
    pub fn data(&self) -> &[u8] {
        match &self.bytes {
            FontBytes::Static(b) => b,
            FontBytes::Owned(v) => v.as_slice(),
        }
    }
    pub fn skrifa(&self) -> Option<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(self.data(), self.index).ok()
    }
    pub fn hb(&self) -> Option<harfrust::FontRef<'_>> {
        harfrust::FontRef::from_index(self.data(), self.index).ok()
    }
    /// Where in the design space glyphs, metrics and outlines come from.
    pub fn location(&self) -> LocationRef<'_> {
        (&self.location).into()
    }
    /// Is this a named instance of a variable font?
    pub fn is_variable(&self) -> bool {
        !self.coords.is_empty()
    }
    /// Face index within the font file (collections); 0 for plain fonts.
    pub fn index(&self) -> u32 {
        self.index
    }
    /// Unique id of this face within the process.
    pub fn id(&self) -> u32 {
        self.id
    }
    /// Counts a hinted draw, `slow` when it ran past [`HINT_STRIKE`]; true when it is the one that
    /// turns hinting off.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn count_hinted(&self, slow: bool) -> bool {
        let draws = self.hint_draws.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if !slow {
            return false;
        }
        let slow = self.hint_slow.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        slow >= MIN_HINT_STRIKES && slow.saturating_mul(HINT_SLOW_RATIO) >= draws && !self.hinting_off.swap(true, Ordering::Relaxed)
    }
    /// Glyph `gid`'s outline in font units, y-down (flipped), uncached: for faces the database
    /// doesn't keep (font list previews); others go through [`FontDb::outline`]'s cache.
    pub fn glyph_outline(&self, gid: u32) -> BezPath {
        self.draw_outline(gid, usize::MAX).path
    }
    /// [`FontFace::glyph_outline`], or None when the outline has more than `max_elements` path
    /// elements: drawing stops copying them at the limit, so a huge glyph is never built whole.
    pub fn glyph_outline_capped(&self, gid: u32, max_elements: usize) -> Option<BezPath> {
        let pen = self.draw_outline(gid, max_elements);
        (!pen.over).then_some(pen.path)
    }
    pub(super) fn draw_outline(&self, gid: u32, max_elements: usize) -> FlipPen {
        let mut pen = FlipPen { path: BezPath::new(), room: max_elements, over: false, hinted: false };
        if let Some(f) = self.skrifa()
            && let Some(g) = f.outline_glyphs().get(GlyphId::new(gid))
        {
            let hinter =
                self.hinter().filter(|_| !self.hinting_off.load(Ordering::Relaxed) && g.draw_memory_size(Hinting::Embedded) <= MAX_HINT_GLYPH_BYTES);
            #[cfg(not(target_arch = "wasm32"))]
            let started = std::time::Instant::now();
            let settings = hinter.map_or_else(|| DrawSettings::unhinted(Size::unscaled(), self.location()), |h| DrawSettings::hinted(h, false));
            pen.hinted = hinter.is_some();
            let mut drawn = g.draw(settings, &mut pen);
            #[cfg(not(target_arch = "wasm32"))]
            if hinter.is_some() && self.count_hinted(started.elapsed() > self.hint_strike) {
                log::warn!("Hinting glyph {gid} in {:?} took {:?}: drawing the face unhinted", self.family, started.elapsed());
            }
            // A draw that was hinted when hinting went off (by itself, or by another thread's draw
            // meanwhile) is the kind that was slow or misassembled: draw it again unhinted, so
            // the outline cache never keeps it (#3692).
            if pen.hinted && self.hinting_off.load(Ordering::Relaxed) {
                pen = FlipPen { path: BezPath::new(), room: max_elements, over: false, hinted: false };
                drawn = g.draw(DrawSettings::unhinted(Size::unscaled(), self.location()), &mut pen);
            }
            if let Err(e) = drawn {
                log::warn!("Cannot draw glyph {gid} in {:?}: {e}", self.family);
                pen.path = BezPath::new();
            }
        }
        pen
    }
    /// The face's TrueType interpreter, for the fonts that need one to assemble their contours and
    /// whose zones are within the limits ([`MAX_HINT_TWILIGHT`]).
    pub(super) fn hinter(&self) -> Option<&HintingInstance> {
        self.outline_hinter
            .get_or_init(|| {
                use skrifa::raw::TableProvider;
                let f = self.skrifa()?;
                let outlines = f.outline_glyphs();
                if !outlines.require_interpreter() {
                    return None;
                }
                let zones = f.maxp().ok().map(|m| (m.max_twilight_points().unwrap_or(0), m.max_stack_elements().unwrap_or(0)));
                if !zones.is_some_and(|(twilight, stack)| twilight <= MAX_HINT_TWILIGHT && stack <= MAX_HINT_STACK) {
                    log::warn!("{:?} asks for hinting zones of {zones:?}: drawing it unhinted", self.family);
                    return None;
                }
                // At UPEM pixels, hinted coordinates remain in font units. This
                // executes contour assembly without tying the cache to view zoom.
                HintingInstance::new(
                    &outlines,
                    Size::new(self.upem as f32),
                    self.location(),
                    HintingOptions { engine: Engine::Interpreter, target: Target::Mono },
                )
                .map_err(|e| log::warn!("Cannot initialize outline interpreter for {:?}: {e}", self.family))
                .ok()
            })
            .as_ref()
    }
    /// Is `family` this face's typographic, legacy, full or PostScript name?
    pub fn named(&self, family: &str) -> bool {
        self.family.eq_ignore_ascii_case(family)
            || self.legacy_family.eq_ignore_ascii_case(family)
            || self.full_name.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(family))
            || self.postscript.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(family))
    }
    /// Does the face map `c` to a glyph?
    pub fn covers(&self, c: char) -> bool {
        let cp = c as u32;
        if cp < 0x1_0000 {
            let bits = self.bmp.get_or_init(|| {
                let mut b = vec![0u64; 1024].into_boxed_slice();
                if let Some(f) = self.skrifa() {
                    for (cp, _) in f.charmap().mappings() {
                        if cp < 0x1_0000 {
                            b[(cp / 64) as usize] |= 1 << (cp % 64);
                        }
                    }
                }
                b
            });
            return bits[(cp / 64) as usize] & (1 << (cp % 64)) != 0;
        }
        self.skrifa().is_some_and(|f| f.charmap().map(c).is_some())
    }
    /// Does the face carry OpenType glyph substitution (GSUB)? Complex scripts (Myanmar, Khmer,
    /// Indic…) need it to reorder and stack; fonts with only Apple's AAT tables draw them unshaped.
    pub fn has_layout_tables(&self) -> bool {
        use skrifa::raw::TableProvider;
        self.skrifa().is_some_and(|f| f.gsub().is_ok())
    }
    /// Units per em.
    pub fn units_per_em(&self) -> f64 {
        self.upem
    }
    /// (ascent, descent) in font units, both positive.
    pub fn vertical_metrics(&self) -> (f64, f64) {
        (self.ascent, self.descent)
    }
    /// Every mapped character and its glyph id, sorted by code point (the Glyphs panel).
    pub fn chars(&self) -> Vec<(char, u32)> {
        let Some(f) = self.skrifa() else { return vec![] };
        let mut v: Vec<(char, u32)> = f.charmap().mappings().filter_map(|(cp, g)| char::from_u32(cp).map(|c| (c, g.to_u32()))).collect();
        v.sort_unstable_by_key(|x| x.0);
        v.dedup_by_key(|x| x.0);
        v
    }
    /// Advance width of glyph `gid` in font units.
    pub fn advance(&self, gid: u32) -> f64 {
        self.skrifa()
            .and_then(|f| f.glyph_metrics(Size::unscaled(), self.location()).advance_width(GlyphId::new(gid)))
            .map(|a| a as f64)
            .unwrap_or(self.upem * 0.5)
    }
    /// Glyph id for `c` (0 = .notdef).
    pub fn glyph_for(&self, c: char) -> u32 {
        self.skrifa().and_then(|f| f.charmap().map(c)).map(|g| g.to_u32()).unwrap_or(0)
    }
    /// Is this an East Asian (Chinese, Japanese or Korean) font? Its OS/2 code pages say so
    /// (Shift-JIS, GB2312, Wansung, Big5, Johab), or, without them, it maps common ideographs.
    /// Word applies such a font to East Asian text too (`w:eastAsia`).
    pub fn is_east_asian(&self) -> bool {
        self.skrifa().is_some_and(|f| east_asian_font(&f))
    }
}

pub(super) static NEXT_ID: AtomicU32 = AtomicU32::new(1);

pub(super) const OUTLINE_CACHE_MAX: usize = 50_000;

/// Collects an outline flipped to y-down, keeping at most `room` path elements (`over` once it
/// had to drop one).
pub(super) struct FlipPen {
    pub(super) path: BezPath,
    room: usize,
    over: bool,
    /// Drawn with the face's TrueType interpreter.
    pub(super) hinted: bool,
}

impl FlipPen {
    /// Take one element's room; false (and `over`) when there is none left.
    fn take(&mut self) -> bool {
        match self.room.checked_sub(1) {
            Some(room) if !self.over => {
                self.room = room;
                true
            }
            _ => {
                self.over = true;
                false
            }
        }
    }
}

impl OutlinePen for FlipPen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.take() {
            self.path.move_to((x as f64, -y as f64));
        }
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if self.take() {
            self.path.line_to((x as f64, -y as f64));
        }
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        if self.take() {
            self.path.quad_to((cx0 as f64, -cy0 as f64), (x as f64, -y as f64));
        }
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if self.take() {
            self.path.curve_to((cx0 as f64, -cy0 as f64), (cx1 as f64, -cy1 as f64), (x as f64, -y as f64));
        }
    }
    fn close(&mut self) {
        if self.take() {
            self.path.close_path();
        }
    }
}
