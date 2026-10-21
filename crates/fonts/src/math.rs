//! The OpenType `MATH` table (constants, italic corrections, corner kerning, accent attachment, glyph size
//! variants and assemblies) read into owned, bounds-checked data, and math font resolution.
//!
//! Spec: OpenType 1.9 `MATH` table. Every read is checked; a malformed table yields `None` or
//! partial data, never a panic.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kurbo::Shape;

use crate::{FaceRef, FontDb, FontFace};

/// `MathConstants` value-record fields, in table order (after the four 16-bit leading fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum C {
    MathLeading = 0,
    AxisHeight,
    AccentBaseHeight,
    FlattenedAccentBaseHeight,
    SubscriptShiftDown,
    SubscriptTopMax,
    SubscriptBaselineDropMin,
    SuperscriptShiftUp,
    SuperscriptShiftUpCramped,
    SuperscriptBottomMin,
    SuperscriptBaselineDropMax,
    SubSuperscriptGapMin,
    SuperscriptBottomMaxWithSubscript,
    SpaceAfterScript,
    UpperLimitGapMin,
    UpperLimitBaselineRiseMin,
    LowerLimitGapMin,
    LowerLimitBaselineDropMin,
    StackTopShiftUp,
    StackTopDisplayStyleShiftUp,
    StackBottomShiftDown,
    StackBottomDisplayStyleShiftDown,
    StackGapMin,
    StackDisplayStyleGapMin,
    StretchStackTopShiftUp,
    StretchStackBottomShiftDown,
    StretchStackGapAboveMin,
    StretchStackGapBelowMin,
    FractionNumeratorShiftUp,
    FractionNumeratorDisplayStyleShiftUp,
    FractionDenominatorShiftDown,
    FractionDenominatorDisplayStyleShiftDown,
    FractionNumeratorGapMin,
    FractionNumDisplayStyleGapMin,
    FractionRuleThickness,
    FractionDenominatorGapMin,
    FractionDenomDisplayStyleGapMin,
    SkewedFractionHorizontalGap,
    SkewedFractionVerticalGap,
    OverbarVerticalGap,
    OverbarRuleThickness,
    OverbarExtraAscender,
    UnderbarVerticalGap,
    UnderbarRuleThickness,
    UnderbarExtraDescender,
    RadicalVerticalGap,
    RadicalDisplayStyleVerticalGap,
    RadicalRuleThickness,
    RadicalExtraAscender,
    RadicalKernBeforeDegree,
    RadicalKernAfterDegree,
}

const VALUE_COUNT: usize = 51;

/// One part of a glyph assembly (font units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPart {
    pub gid: u32,
    pub start_connector: f32,
    pub end_connector: f32,
    pub full_advance: f32,
    pub extender: bool,
}

/// Size variants of a glyph (smallest first, with their advance along the stretch direction)
/// and an optional assembly for bigger sizes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Construction {
    pub variants: Vec<(u32, f32)>,
    pub parts: Vec<GlyphPart>,
}

/// Corner of a glyph used for height-dependent script kerning.
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum MathCorner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

#[derive(Clone, Debug)]
struct KernSteps {
    heights: Vec<f32>,
    values: Vec<f32>,
}

const MAX_KERN_HEIGHTS: usize = 256;
const MAX_KERN_GLYPHS: usize = 4096;

/// A parsed `MATH` table. Values are font units.
#[derive(Clone, Debug, Default)]
pub struct MathTable {
    pub script_percent: f32,
    pub script_script_percent: f32,
    pub delimited_sub_formula_min_height: f32,
    pub display_operator_min_height: f32,
    values: Vec<f32>,
    pub radical_degree_bottom_raise_percent: f32,
    pub italics: HashMap<u32, f32>,
    kerns: HashMap<u32, [Option<KernSteps>; 4]>,
    pub top_accent: HashMap<u32, f32>,
    pub min_connector_overlap: f32,
    pub vert: HashMap<u32, Construction>,
    pub horiz: HashMap<u32, Construction>,
}

impl MathTable {
    pub fn get(&self, c: C) -> f32 {
        self.values.get(c as usize).copied().unwrap_or(0.0)
    }

    /// Corner kerning at `height` above the glyph baseline, in font units.
    pub fn kern(&self, gid: u32, corner: MathCorner, height: f32) -> f32 {
        if !height.is_finite() {
            return 0.0;
        }
        let Some(steps) = self.kerns.get(&gid).and_then(|corners| corners.get(corner as usize)).and_then(Option::as_ref) else { return 0.0 };
        let index = steps.heights.partition_point(|h| height >= *h);
        steps.values.get(index).copied().unwrap_or(0.0)
    }
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

fn i16_at(b: &[u8], off: usize) -> Option<i16> {
    u16_at(b, off).map(|v| v as i16)
}

/// A table at `base + offset` (offset 0 = absent).
fn sub(b: &[u8], base: usize, offset_at: usize) -> Option<usize> {
    let o = u16_at(b, offset_at)? as usize;
    if o == 0 {
        return None;
    }
    let p = base.checked_add(o)?;
    (p < b.len()).then_some(p)
}

/// Glyphs of a Coverage table in coverage-index order.
fn coverage(b: &[u8], at: usize) -> Vec<u32> {
    let mut out = Vec::new();
    match u16_at(b, at) {
        Some(1) => {
            let n = u16_at(b, at + 2).unwrap_or(0) as usize;
            for i in 0..n {
                match u16_at(b, at + 4 + i * 2) {
                    Some(g) => out.push(g as u32),
                    None => break,
                }
            }
        }
        Some(2) => {
            let n = u16_at(b, at + 2).unwrap_or(0) as usize;
            for i in 0..n {
                let r = at + 4 + i * 6;
                let (Some(s), Some(e), Some(idx)) = (u16_at(b, r), u16_at(b, r + 2), u16_at(b, r + 4)) else { break };
                if e < s {
                    continue;
                }
                for g in s..=e {
                    let k = idx as usize + (g - s) as usize;
                    if k >= 70_000 {
                        break;
                    }
                    if out.len() <= k {
                        out.resize(k + 1, u32::MAX);
                    }
                    if let Some(slot) = out.get_mut(k) {
                        *slot = g as u32;
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// `coverageOffset, count, MathValueRecord[count]` → glyph → value.
fn value_map(b: &[u8], at: usize) -> HashMap<u32, f32> {
    let mut m = HashMap::new();
    let Some(cov) = sub(b, at, at) else { return m };
    let glyphs = coverage(b, cov);
    let n = u16_at(b, at + 2).unwrap_or(0) as usize;
    for (i, g) in glyphs.iter().enumerate().take(n) {
        if let Some(v) = i16_at(b, at + 4 + i * 4)
            && *g != u32::MAX
        {
            m.insert(*g, v as f32);
        }
    }
    m
}

fn constructions(b: &[u8], variants: usize, cov_at: usize, count: usize, offsets_at: usize) -> HashMap<u32, Construction> {
    let mut m = HashMap::new();
    let Some(cov) = sub(b, variants, cov_at) else { return m };
    let glyphs = coverage(b, cov);
    for (i, g) in glyphs.iter().enumerate().take(count) {
        let Some(c) = sub(b, variants, offsets_at + i * 2) else { continue };
        let mut con = Construction::default();
        let nv = u16_at(b, c + 2).unwrap_or(0) as usize;
        for k in 0..nv.min(256) {
            let r = c + 4 + k * 4;
            let (Some(vg), Some(adv)) = (u16_at(b, r), u16_at(b, r + 2)) else { break };
            con.variants.push((vg as u32, adv as f32));
        }
        if let Some(a) = sub(b, c, c) {
            let np = u16_at(b, a + 4).unwrap_or(0) as usize;
            for k in 0..np.min(64) {
                let r = a + 6 + k * 10;
                let (Some(pg), Some(s), Some(e), Some(full), Some(fl)) =
                    (u16_at(b, r), u16_at(b, r + 2), u16_at(b, r + 4), u16_at(b, r + 6), u16_at(b, r + 8))
                else {
                    break;
                };
                con.parts.push(GlyphPart {
                    gid: pg as u32,
                    start_connector: s as f32,
                    end_connector: e as f32,
                    full_advance: full as f32,
                    extender: fl & 1 != 0,
                });
            }
        }
        if *g != u32::MAX {
            m.insert(*g, con);
        }
    }
    m
}

fn kern_steps(b: &[u8], at: usize) -> Option<KernSteps> {
    let count = u16_at(b, at)? as usize;
    if count > MAX_KERN_HEIGHTS {
        return None;
    }
    b.get(at..at.checked_add(2 + (2 * count + 1) * 4)?)?;
    let heights: Vec<f32> = (0..count).map(|i| i16_at(b, at + 2 + i * 4).map(f32::from)).collect::<Option<_>>()?;
    if heights.windows(2).any(|pair| matches!(pair, [low, high] if low > high)) {
        return None;
    }
    let values = (0..=count).map(|i| i16_at(b, at + 2 + (count + i) * 4).map(f32::from)).collect::<Option<_>>()?;
    Some(KernSteps { heights, values })
}

// Validate counts and ranges before expanding untrusted coverage.
fn kern_coverage(b: &[u8], at: usize, limit: usize) -> Option<Vec<u32>> {
    if limit > MAX_KERN_GLYPHS {
        return None;
    }
    let mut out = Vec::new();
    if limit == 0 {
        return Some(out);
    }
    let count = u16_at(b, at + 2)? as usize;
    match u16_at(b, at)? {
        1 => {
            if count != limit {
                return None;
            }
            for i in 0..count {
                let glyph = u16_at(b, at + 4 + i * 2)? as u32;
                if out.last().is_some_and(|last| glyph <= *last) {
                    return None;
                }
                out.push(glyph);
            }
        }
        2 => {
            if count > limit {
                return None;
            }
            let mut previous = None;
            for i in 0..count {
                let r = at + 4 + i * 6;
                let start = u16_at(b, r)?;
                let end = u16_at(b, r + 2)?;
                let index = u16_at(b, r + 4)? as usize;
                if end < start || index != out.len() || previous.is_some_and(|last| start <= last) {
                    return None;
                }
                let len = usize::from(end - start) + 1;
                if len > limit - out.len() {
                    return None;
                }
                // Disjoint ranges expand to at most the declared, capped glyph count.
                out.extend((start..=end).map(u32::from));
                previous = Some(end);
            }
        }
        _ => return None,
    }
    (out.len() == limit).then_some(out)
}

fn kern_map(b: &[u8], at: usize) -> HashMap<u32, [Option<KernSteps>; 4]> {
    let mut out = HashMap::new();
    let count = u16_at(b, at + 2).unwrap_or(0) as usize;
    if count == 0 || count > MAX_KERN_GLYPHS {
        return out;
    }
    let Some(cov) = sub(b, at, at) else { return out };
    for (i, gid) in kern_coverage(b, cov, count).unwrap_or_default().into_iter().enumerate() {
        let corners = std::array::from_fn(|corner| sub(b, at, at + 4 + i * 8 + corner * 2).and_then(|offset| kern_steps(b, offset)));
        out.insert(gid, corners);
    }
    out
}

/// Parse a `MATH` table.
pub fn parse(b: &[u8]) -> Option<MathTable> {
    if u16_at(b, 0)? != 1 {
        return None;
    }
    let mut t = MathTable::default();
    let c = sub(b, 0, 4)?;
    t.script_percent = i16_at(b, c)? as f32;
    t.script_script_percent = i16_at(b, c + 2)? as f32;
    t.delimited_sub_formula_min_height = u16_at(b, c + 4)? as f32;
    t.display_operator_min_height = u16_at(b, c + 6)? as f32;
    for i in 0..VALUE_COUNT {
        t.values.push(i16_at(b, c + 8 + i * 4).unwrap_or(0) as f32);
    }
    t.radical_degree_bottom_raise_percent = i16_at(b, c + 8 + VALUE_COUNT * 4).unwrap_or(60) as f32;
    if let Some(gi) = sub(b, 0, 6) {
        if let Some(ic) = sub(b, gi, gi) {
            t.italics = value_map(b, ic);
        }
        if let Some(ta) = sub(b, gi, gi + 2) {
            t.top_accent = value_map(b, ta);
        }
        if let Some(ki) = sub(b, gi, gi + 6) {
            t.kerns = kern_map(b, ki);
        }
    }
    if let Some(v) = sub(b, 0, 8) {
        t.min_connector_overlap = u16_at(b, v).unwrap_or(0) as f32;
        let nv = u16_at(b, v + 6).unwrap_or(0) as usize;
        let nh = u16_at(b, v + 8).unwrap_or(0) as usize;
        t.vert = constructions(b, v, v + 2, nv, v + 10);
        t.horiz = constructions(b, v, v + 4, nh, v + 10 + nv * 2);
    }
    Some(t)
}

/// The face's `MATH` table (parsed once per face).
pub fn table(face: &FontFace) -> Option<Arc<MathTable>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, Option<Arc<MathTable>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(t) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&face.id()) {
        return t.clone();
    }
    let t = face
        .skrifa()
        .and_then(|f| f.table_data(skrifa::raw::types::Tag::new(b"MATH")).map(|d| d.as_bytes().to_vec()))
        .and_then(|bytes| parse(&bytes))
        .map(Arc::new);
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(face.id(), t.clone());
    t
}

/// Ink bounds of a glyph in font units, y up: (x_min, y_min, x_max, y_max). `None` for blank glyphs.
pub fn glyph_bounds(face: &FontFace, gid: u32) -> Option<(f32, f32, f32, f32)> {
    let p = FontDb::global().outline(face, gid);
    if p.elements().is_empty() {
        return None;
    }
    let r = p.bounding_box();
    // Outlines are cached y-down.
    let b = (r.x0 as f32, -r.y1 as f32, r.x1 as f32, -r.y0 as f32);
    (b.0.is_finite() && b.1.is_finite() && b.2.is_finite() && b.3.is_finite()).then_some(b)
}

/// Math fonts to use, in order, for a requested math font that isn't installed: open fonts
/// with a `MATH` table that are close to Cambria Math in weight and proportion first.
pub const MATH_SUBSTITUTES: &[&str] = &[
    "Cambria Math",
    "STIX Two Math",
    "XITS Math",
    "Libertinus Math",
    "TeX Gyre Termes Math",
    "TeX Gyre Pagella Math",
    "Latin Modern Math",
    "DejaVu Math TeX Gyre",
    "Asana Math",
    "Noto Sans Math",
];

const MATH_CACHE_MAX: usize = 1024;

/// Math names longer than this (bytes) are resolved but never remembered.
const MATH_KEY_MAX: usize = 256;

/// Resolutions tagged with the [`FontDb::generation`] they were made at.
type MathMemo = (u64, HashMap<String, (FaceRef, Option<Arc<MathTable>>)>);

fn math_cache() -> &'static Mutex<MathMemo> {
    static CACHE: OnceLock<Mutex<MathMemo>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new((0, HashMap::new())))
}

/// The face to set math in: `family` when installed and it has a `MATH` table, else the first
/// installed substitute that has one, else `family` (or its text substitute) without one.
/// Remembered until user fonts are added or removed ([`FontDb::generation`]).
pub fn resolve_math(family: &str) -> (FaceRef, Option<Arc<MathTable>>) {
    let cache = math_cache();
    let db = FontDb::global();
    let generation = db.generation();
    {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        if c.0 != generation {
            *c = (generation, HashMap::new());
        }
        if let Some(v) = c.1.get(family) {
            return v.clone();
        }
    }
    let mut found = None;
    for fam in std::iter::once(family).chain(MATH_SUBSTITUTES.iter().copied()) {
        if !db.has_family(fam) {
            continue;
        }
        let face = db.face(fam, "Regular");
        if face.family.eq_ignore_ascii_case(fam)
            && let Some(t) = table(&face)
        {
            found = Some((FaceRef::of(&face), Some(t)));
            break;
        }
    }
    let v = found.unwrap_or_else(|| (crate::word::resolve(family, false, false).face, None));
    if family.len() <= MATH_KEY_MAX {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        if c.0 == generation {
            if c.1.len() >= MATH_CACHE_MAX {
                c.1.clear();
            }
            c.1.insert(family.to_string(), v.clone());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(family: &str) -> bool {
        math_cache().lock().unwrap_or_else(|e| e.into_inner()).1.contains_key(family)
    }

    /// `font` (an sfnt) with a minimal valid `MATH` table added: enough for `parse` to accept.
    fn with_math_table(font: &[u8]) -> Vec<u8> {
        let n = u16::from_be_bytes([font[4], font[5]]) as usize;
        let mut dir: Vec<[u8; 16]> = (0..n).map(|i| font[12 + i * 16..28 + i * 16].try_into().unwrap()).collect();
        let mut math = vec![0u8, 1, 0, 0, 0, 10, 0, 0, 0, 0];
        math.resize(10 + 8 + VALUE_COUNT * 4 + 2, 0);
        let data_start = 12 + (n + 1) * 16;
        let end = font.len() + 16;
        for e in &mut dir {
            let off = u32::from_be_bytes([e[8], e[9], e[10], e[11]]) + 16;
            e[8..12].copy_from_slice(&off.to_be_bytes());
        }
        let mut entry = [0u8; 16];
        entry[..4].copy_from_slice(b"MATH");
        entry[8..12].copy_from_slice(&(end as u32).to_be_bytes());
        entry[12..16].copy_from_slice(&(math.len() as u32).to_be_bytes());
        let at = dir.iter().position(|e| e[..4] > entry[..4]).unwrap_or(dir.len());
        dir.insert(at, entry);
        let mut out = font[..12].to_vec();
        out[4..6].copy_from_slice(&((n + 1) as u16).to_be_bytes());
        // Search fields are only hints for lookups; skrifa binary-searches the directory, so
        // keep them consistent.
        let entry_selector = (usize::BITS - 1 - (n + 1).leading_zeros()) as u16;
        let search_range = (1u16 << entry_selector) * 16;
        out[6..8].copy_from_slice(&search_range.to_be_bytes());
        out[8..10].copy_from_slice(&entry_selector.to_be_bytes());
        out[10..12].copy_from_slice(&(((n + 1) as u16) * 16 - search_range).to_be_bytes());
        for e in &dir {
            out.extend_from_slice(e);
        }
        debug_assert_eq!(out.len(), data_start);
        out.extend_from_slice(&font[12 + n * 16..]);
        out.extend_from_slice(&math);
        out
    }

    #[test]
    fn math_resolution_follows_installed_and_removed_fonts() {
        use crate::fontdb::user::tests::renamed;
        // 13 characters, as `renamed` requires.
        let fam = "Zz Math Gen 1";
        let db = FontDb::global();
        assert_ne!(resolve_math(fam).0.family, fam);
        let font = with_math_table(&renamed("SourceSans3-Regular.ttf", fam));
        db.install_font("ZzMathGen.ttf", &font).unwrap();
        let (face, table) = resolve_math(fam);
        assert_eq!(face.family, fam, "a math font added after the first lookup must be used");
        assert!(table.is_some());
        db.remove_user_font(fam).unwrap();
        assert_ne!(resolve_math(fam).0.family, fam, "a removed font must stop being used");
    }

    #[test]
    fn math_cache_stays_bounded() {
        for i in 0..(MATH_CACHE_MAX + 50) {
            let _ = resolve_math(&format!("Zz Bound Probe {i}"));
        }
        assert!(math_cache().lock().unwrap_or_else(|e| e.into_inner()).1.len() <= MATH_CACHE_MAX);
        let long = "L".repeat(10_000);
        let _ = resolve_math(&long);
        assert!(!cached(&long), "absurdly long names must not be cached");
    }

    #[test]
    fn height_kerns_are_bounded() {
        let mut bytes = vec![0u8; 22];
        bytes[..2].copy_from_slice(&2u16.to_be_bytes());
        for (i, value) in [0i16, 300, -100, -300, 50].into_iter().enumerate() {
            let at = 2 + i * 4;
            bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        let mut corners = std::array::from_fn(|_| None);
        corners[MathCorner::BottomRight as usize] = kern_steps(&bytes, 0);
        let mut table = MathTable::default();
        table.kerns.insert(7, corners);
        for (height, expected) in [(-1.0, -100.0), (0.0, -300.0), (299.0, -300.0), (300.0, 50.0), (10000.0, 50.0)] {
            assert_eq!(table.kern(7, MathCorner::BottomRight, height), expected);
        }
        assert_eq!(table.kern(7, MathCorner::TopLeft, 0.0), 0.0);
        assert_eq!(table.kern(8, MathCorner::BottomRight, 0.0), 0.0);
        for height in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(table.kern(7, MathCorner::BottomRight, height), 0.0);
        }
        for len in 0..bytes.len() {
            assert!(kern_steps(&bytes[..len], 0).is_none());
        }
        bytes[2..4].copy_from_slice(&301i16.to_be_bytes());
        assert!(kern_steps(&bytes, 0).is_none(), "descending heights are ignored");
        bytes[..2].copy_from_slice(&((MAX_KERN_HEIGHTS + 1) as u16).to_be_bytes());
        assert!(kern_steps(&bytes, 0).is_none(), "hostile counts are bounded");
    }

    fn coverage_bytes(format: u16, records: &[u16]) -> Vec<u8> {
        std::iter::once(format).chain(records.iter().copied()).flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn kern_coverage_bounds_ranges() {
        let bytes = coverage_bytes(2, &[1, 0, (MAX_KERN_GLYPHS - 1) as u16, 0]);
        let glyphs = kern_coverage(&bytes, 0, MAX_KERN_GLYPHS).unwrap();
        assert_eq!(glyphs.len(), MAX_KERN_GLYPHS);
        assert_eq!(glyphs.first(), Some(&0));
        assert_eq!(glyphs.last(), Some(&((MAX_KERN_GLYPHS - 1) as u32)));
        assert!(kern_coverage(&bytes, 0, usize::MAX).is_none());
        assert_eq!(kern_coverage(&bytes, 0, 0), Some(vec![]));
        let bytes = coverage_bytes(2, &[u16::MAX, 0, u16::MAX, 0]);
        assert!(kern_coverage(&bytes, 0, MAX_KERN_GLYPHS).is_none());

        let bytes = coverage_bytes(1, &[3, 11, 12, 13]);
        assert_eq!(kern_coverage(&bytes, 0, 3), Some(vec![11, 12, 13]));
        let bytes = coverage_bytes(2, &[2, 7, 8, 0, 12, 14, 2]);
        assert_eq!(kern_coverage(&bytes, 0, 5), Some(vec![7, 8, 12, 13, 14]));
        assert!(kern_coverage(&bytes, 0, 3).is_none());
    }

    #[test]
    fn kern_coverage_rejects_ranges() {
        for records in [
            vec![1, 7, 8, u16::MAX],     // Sparse coverage index.
            vec![2, 7, 8, 0, 8, 9, 2],   // Overlapping glyph ranges.
            vec![2, 7, 8, 0, 12, 13, 0], // Repeated coverage index.
            vec![1, 8, 7, 0],            // Reversed range.
            vec![1, 7, 8],               // Truncated range.
        ] {
            let bytes = coverage_bytes(2, &records);
            assert!(kern_coverage(&bytes, 0, 8).is_none(), "{records:?}");
        }
    }

    fn kern_math_bytes(count: u16, coverage: &[u8]) -> Vec<u8> {
        const GLYPH_INFO: usize = 256;
        const KERN_INFO: usize = GLYPH_INFO + 8;
        let cov = KERN_INFO + 4 + count as usize * 8;
        let mut bytes = vec![0; cov + coverage.len()];
        let mut put = |at, value: u16| bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
        put(0, 1);
        put(4, 10);
        put(6, GLYPH_INFO as u16);
        put(GLYPH_INFO + 6, 8);
        put(KERN_INFO, (cov - KERN_INFO) as u16);
        put(KERN_INFO + 2, count);
        bytes[cov..].copy_from_slice(coverage);
        bytes
    }

    #[test]
    fn kern_parse_rejects_ranges() {
        for (count, format, records) in [
            (1, 2, vec![2, 7, 7, 0, 8, 8, 1]),   // Too many ranges.
            (3, 2, vec![1, 7, 8, 0]),            // Too few glyphs.
            (1, 2, vec![1, 7, 8, 0]),            // Too many glyphs.
            (3, 1, vec![2, 7, 8]),               // Glyph count mismatch.
            (4, 2, vec![2, 7, 8, 0, 8, 9, 2]),   // Overlap.
            (4, 2, vec![2, 7, 8, 0, 12, 13, 0]), // Index mismatch.
        ] {
            let coverage = coverage_bytes(format, &records);
            let table = parse(&kern_math_bytes(count, &coverage)).unwrap();
            assert!(table.kerns.is_empty(), "{count}: {records:?}");
        }
    }

    #[test]
    fn parse_hostile_kern_ranges() {
        let records: Vec<u16> = std::iter::once(u16::MAX).chain((0..u16::MAX).flat_map(|_| [0, u16::MAX, 0])).collect();
        let coverage = coverage_bytes(2, &records);
        for count in [0, 1, (MAX_KERN_GLYPHS + 1) as u16] {
            let bytes = kern_math_bytes(count, &coverage);
            let table = parse(&bytes).unwrap();
            assert!(table.kerns.is_empty());
        }
        let coverage = coverage_bytes(2, &[2, 7, 8, 0, 12, 12, 2]);
        let table = parse(&kern_math_bytes(3, &coverage)).unwrap();
        let mut glyphs: Vec<_> = table.kerns.keys().copied().collect();
        glyphs.sort_unstable();
        assert_eq!(glyphs, [7, 8, 12]);
    }

    #[test]
    fn garbage_tables_never_panic() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0, 1]).is_none());
        let mut b = vec![0u8, 1, 0, 0, 0, 10, 0, 0xff, 0xff, 0xff];
        b.resize(40, 0xff);
        let _ = parse(&b);
        for n in 0..64u8 {
            let b: Vec<u8> = (0..300u32).map(|i| (i as u8).wrapping_mul(n).wrapping_add(7)).collect();
            let mut b = b;
            b[0] = 0;
            b[1] = 1;
            let _ = parse(&b);
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn system_math_font_has_a_table() {
        // Only meaningful where an open math font is installed (macOS ships STIX Two Math).
        let (face, t) = resolve_math("Cambria Math");
        eprintln!("math font: {} (MATH table: {})", face.family, t.is_some());
        if let Some(t) = t {
            assert!(t.get(C::AxisHeight) > 0.0, "{}", face.family);
            assert!(t.get(C::FractionRuleThickness) > 0.0);
            let paren = face.glyph_for('(');
            assert!(t.vert.get(&paren).is_some_and(|c| !c.variants.is_empty()), "paren variants in {}", face.family);
        }
    }
}
