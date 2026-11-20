//! Installed stand-ins for Hancom fonts that are not installed.
//!
//! WordCraft finds fonts by their family name; when one is missing it draws Hangul in a gothic
//! face even for Myeongjo (serif) text (measured with WordCraft 0.10; it now also takes a DOCX
//! font table's `w:altName`, which this converter doesn't write). Text in another face takes
//! another width, so lines and pages break elsewhere than in Hancom's viewer. The converter therefore swaps a missing font for an installed one of
//! the same style (brush, serif or sans by the name) and records the swaps in the DOCX
//! ([`PROPERTY`]) so a new HWPX written from the document can put the original names back.
//! Installed fonts are looked up in WordCraft's font database, on every system.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::{BTreeMap, HashSet};

/// The DOCX custom property holding the swaps: a JSON object, original name -> stand-in.
pub(crate) const PROPERTY: &str = "HWPX font substitutions";

const BRUSH_HINTS: [&str; 6] = ["궁서", "gungsuh", "해서", "행서", "초서", "붓"];
const SERIF_HINTS: [&str; 8] = ["명조", "바탕", "신명", "myeongjo", "batang", "serif", "mincho", "song"];
const SANS_HINTS: [&str; 9] = ["고딕", "돋움", "굴림", "헤드라인", "gothic", "dotum", "gulim", "sans", "산세리프"];
/// Stand-ins in order of preference: the first one installed is used. Naver's Nanum fonts (free,
/// also for commercial use; never bundled) come first, so users are pointed to them ([`notice`]).
/// Gothic text takes NanumGothic, else Pretendard (OFL-1.1, also free; installed as "Pretendard"
/// or, the variable build, "Pretendard Variable"); with neither, the font is left to WordCraft's
/// own fallback. Pretendard has no serif face, so the other lists don't take it.
const BRUSH_SUBSTITUTES: [&str; 4] = ["궁서", "나눔명조", "함초롬바탕", "바탕"];
const SERIF_SUBSTITUTES: [&str; 3] = ["나눔명조", "함초롬바탕", "바탕"];
const SANS_SUBSTITUTES: [&str; 3] = ["나눔고딕", "Pretendard", "Pretendard Variable"];
/// The Nanum fonts the stand-ins prefer, and where to get them.
const NANUM: [[&str; 2]; 2] = [["나눔고딕", "nanumgothic"], ["나눔명조", "nanummyeongjo"]];
const NANUM_SITE: &str = "hangeul.naver.com";
/// The Gothic alternative to NanumGothic, by its names, and where to get it.
const PRETENDARD: [&str; 2] = ["pretendard", "pretendard variable"];
const PRETENDARD_SITE: &str = "github.com/orioncactus/pretendard";
/// Is `family` installed (English or localized name), or added to WordCraft?
fn installed(family: &str) -> bool {
    wordcraft_fonts::FontDb::global().has_family(family)
}

/// An installed stand-in for `face` when it is missing and of a known style.
fn substitute(face: &str, installed: impl Fn(&str) -> bool) -> Option<String> {
    let low = face.to_lowercase();
    if face.is_empty() || installed(face) {
        return None;
    }
    let pool: &[&str] = if BRUSH_HINTS.iter().any(|h| low.contains(h)) {
        &BRUSH_SUBSTITUTES
    } else if SERIF_HINTS.iter().any(|h| low.contains(h)) {
        &SERIF_SUBSTITUTES
    } else if SANS_HINTS.iter().any(|h| low.contains(h)) {
        &SANS_SUBSTITUTES
    } else {
        return None;
    };
    pool.iter().find(|s| installed(s)).map(|s| s.to_string())
}

/// The font Hangul is written in when a document names no East Asian font for its text, and its
/// own font has no Hangul: the one this system draws Hangul with (WordCraft's fallback for '가':
/// Malgun Gothic on Windows, Apple SD Gothic Neo on macOS, ...), by its Korean name when it has
/// one, as Hangul documents name fonts. `None` when the font has Hangul, or no font here has.
pub(crate) fn hangul_default(font: &str) -> Option<String> {
    let db = wordcraft_fonts::FontDb::global();
    let has_hangul = |f: &str| db.has_family(f) && db.face(f, "Regular").covers('가');
    let system = || {
        let face = db.fallback_for('가', u32::MAX)?;
        let korean = face.local_names.iter().find(|n| n.lang.starts_with("ko")).map(|n| n.name.clone());
        Some(korean.unwrap_or_else(|| face.family.clone()))
    };
    hangul_default_with(font, has_hangul, system)
}

/// [`hangul_default`], with what has Hangul and the system's Hangul font given.
fn hangul_default_with(font: &str, has_hangul: impl Fn(&str) -> bool, system: impl FnOnce() -> Option<String>) -> Option<String> {
    if !font.is_empty() && has_hangul(font) {
        return None;
    }
    system()
}

/// Swap the missing fonts of a header's font table (language -> id -> name) for installed
/// stand-ins; the swaps made, original -> stand-in.
pub(crate) fn substitute_all(fonts: &mut BTreeMap<String, BTreeMap<i64, String>>) -> BTreeMap<String, String> {
    let mut swaps = BTreeMap::new();
    for list in fonts.values_mut() {
        for name in list.values_mut() {
            if let Some(s) = substitute(name, installed) {
                swaps.insert(std::mem::replace(name, s.clone()), s);
            }
        }
    }
    swaps
}

/// What to tell a user whose document uses fonts that are not installed while the stand-ins
/// aren't installed either: `None` when nothing was swapped, or NanumMyeongjo and a Gothic
/// stand-in (NanumGothic or Pretendard) are there.
pub fn notice(doc: &wordcraft_doc::Document) -> Option<String> {
    let swapped = doc.custom_props.iter().any(|p| p.name == PROPERTY);
    (swapped && lacks_stand_ins(installed)).then(|| {
        format!(
            "Some fonts of this document are not installed and are drawn with similar ones. For the closest look, install the free Naver Nanum fonts (NanumGothic, NanumMyeongjo) from {NANUM_SITE}. For Gothic text, the free Pretendard font ({PRETENDARD_SITE}) also works."
        )
    })
}

/// Is NanumMyeongjo missing, or both NanumGothic and Pretendard (each looked up by its names)?
fn lacks_stand_ins(installed: impl Fn(&str) -> bool) -> bool {
    let has = |names: &[&str]| names.iter().any(|n| installed(n));
    let [gothic, myeongjo] = &NANUM;
    !has(myeongjo) || (!has(gothic) && !has(&PRETENDARD))
}

/// The swaps as the property's JSON object.
pub(crate) fn to_json(swaps: &BTreeMap<String, String>) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let pairs: Vec<String> = swaps.iter().map(|(k, v)| format!("{}: {}", q(k), q(v))).collect();
    format!("{{{}}}", pairs.join(", "))
}

/// The swaps from the property's JSON object, original -> stand-in.
pub(crate) fn swaps(json: &str) -> BTreeMap<String, String> {
    strings(json).as_chunks::<2>().0.iter().map(|[o, s]| (o.clone(), s.clone())).collect()
}

/// The stand-ins mapped back to the names they replaced, from the property's JSON object; a
/// stand-in used for several fonts is left out (it cannot be told which one it was).
pub(crate) fn originals(json: &str) -> BTreeMap<String, String> {
    let strings = strings(json);
    let mut back: BTreeMap<String, String> = BTreeMap::new();
    let mut shared: HashSet<String> = HashSet::new();
    for [orig, sub] in strings.as_chunks::<2>().0 {
        if back.insert(sub.clone(), orig.clone()).is_some() {
            shared.insert(sub.clone());
        }
    }
    back.retain(|k, _| !shared.contains(k));
    back
}

/// The strings of a JSON object, in order: key, value, key, value ...
fn strings(json: &str) -> Vec<String> {
    let mut strings: Vec<String> = Vec::new();
    let mut chars = json.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => s.extend(chars.next()),
                '"' => break,
                c => s.push(c),
            }
        }
        strings.push(s);
    }
    strings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> impl Fn(&str) -> bool {
        let names: HashSet<String> = names.iter().map(|s| s.to_lowercase()).collect();
        move |f: &str| names.contains(&f.to_lowercase())
    }

    #[test]
    fn stand_ins_follow_the_style_and_map_back() {
        let fonts = set(&["함초롬바탕", "맑은 고딕"]);
        assert_eq!(substitute("휴먼명조", &fonts).as_deref(), Some("함초롬바탕"));
        assert_eq!(substitute("함초롬바탕", &fonts), None, "installed: kept");
        assert_eq!(substitute("Some Font", &fonts), None, "unknown style: kept");
        let with_nanum = set(&["함초롬바탕", "맑은 고딕", "나눔명조", "나눔고딕"]);
        assert_eq!(substitute("휴먼명조", &with_nanum).as_deref(), Some("나눔명조"), "Nanum first");
        assert_eq!(substitute("HY헤드라인M", &with_nanum).as_deref(), Some("나눔고딕"));
        assert!(!lacks_stand_ins(&with_nanum));
        assert!(lacks_stand_ins(&fonts), "no Nanum: point to it");
        assert!(!lacks_stand_ins(set(&["nanumgothic", "nanummyeongjo"])), "found by its English name too");
        let swaps: BTreeMap<String, String> = [("휴먼명조", "함초롬바탕"), ("한양신명조", "함초롬바탕"), ("HY헤드라인M", "맑은 고딕")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let back = originals(&to_json(&swaps));
        assert_eq!(back.get("맑은 고딕").map(String::as_str), Some("HY헤드라인M"));
        assert!(!back.contains_key("함초롬바탕"), "two fonts share it: not mapped back");
    }

    /// A run's font with Hangul is kept; one without takes the system's Hangul font; with none
    /// on the system, the run's font stays (no default).
    #[test]
    fn hangul_default_keeps_a_font_with_hangul() {
        let has = |f: &str| f == "나눔고딕";
        assert_eq!(hangul_default_with("나눔고딕", has, || Some("맑은 고딕".into())), None);
        assert_eq!(hangul_default_with("Aptos", has, || Some("맑은 고딕".into())).as_deref(), Some("맑은 고딕"));
        assert_eq!(hangul_default_with("", has, || Some("맑은 고딕".into())).as_deref(), Some("맑은 고딕"));
        assert_eq!(hangul_default_with("Aptos", has, || None), None);
    }

    /// Gothic text takes NanumGothic, else Pretendard (either build), and nothing else: with
    /// neither, the font is left to WordCraft's own fallback. The notice isn't given when
    /// Pretendard stands in for NanumGothic.
    #[test]
    fn gothic_takes_nanum_then_pretendard_only() {
        assert_eq!(substitute("HY헤드라인M", set(&["나눔고딕", "Pretendard"])).as_deref(), Some("나눔고딕"));
        assert_eq!(substitute("HY헤드라인M", set(&["Pretendard", "맑은 고딕"])).as_deref(), Some("Pretendard"));
        assert_eq!(substitute("HY헤드라인M", set(&["Pretendard Variable"])).as_deref(), Some("Pretendard Variable"));
        assert_eq!(substitute("HY헤드라인M", set(&["맑은 고딕", "함초롬돋움", "돋움"])), None, "the others aren't taken");
        assert_eq!(substitute("휴먼명조", set(&["Pretendard"])), None, "no serif face");
        assert!(!lacks_stand_ins(set(&["Pretendard", "나눔명조"])));
        assert!(lacks_stand_ins(set(&["Pretendard"])), "NanumMyeongjo still missing");
    }
}
