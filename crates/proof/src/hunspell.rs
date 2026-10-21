//! Hunspell dictionaries installed on the system, read at runtime.
//!
//! WordCraft doesn't bundle large spelling dictionaries. For languages with an affix dictionary
//! (Polish, Russian), it reads the standard Hunspell `.aff`/`.dic` pair installed by the operating system,
//! a package manager or LibreOffice, with [`crate::affix::AffixDict::from_hunspell`]. When none is
//! installed the language has no spelling check (no marks, no errors), like Word without the
//! language's proofing tools.
//!
//! Search order: the directory in `WORDCRAFT_DICTIONARIES` (or one set with
//! [`set_dictionary_dir`]), then the usual system locations. On the web there are none.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::affix::AffixDict;

/// Largest `.aff` or `.dic` file read (real ones are a few MB).
const MAX_FILE: u64 = 64 << 20;

/// An extra directory searched first (a setting), set by the front end.
static EXTRA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Why an installed dictionary failed to load, by `.dic` name (`pt_BR`): the file and the reason.
/// Without it a file that is there but unreadable looks the same as no file (#2830).
static LOAD_ERRORS: RwLock<Vec<(String, PathBuf, String)>> = RwLock::new(Vec::new());

/// Remember (or, for `Ok`, forget) why the dictionary `key` found at `dic` failed to load, and
/// hand the result back unchanged.
pub fn note_load<T>(key: &str, dic: &Path, result: Result<T, String>) -> Result<T, String> {
    let mut errors = LOAD_ERRORS.write().unwrap_or_else(|e| e.into_inner());
    errors.retain(|(k, _, _)| k != key);
    if let Err(e) = &result {
        errors.push((key.to_string(), dic.to_path_buf(), e.clone()));
    }
    result
}

/// Forget the load error of the dictionary `key` (none found, or it is about to be searched for again).
pub fn clear_load_error(key: &str) {
    LOAD_ERRORS.write().unwrap_or_else(|e| e.into_inner()).retain(|(k, _, _)| k != key);
}

/// The `.dic` file and the reason of the last failed load of the dictionary `key`, if any.
pub fn load_error(key: &str) -> Option<(PathBuf, String)> {
    LOAD_ERRORS.read().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _, _)| k == key).map(|(_, p, e)| (p.clone(), e.clone()))
}

/// Search `dir` before the system locations (a "dictionaries folder" setting); `None` clears it.
/// Takes effect for dictionaries not loaded yet.
pub fn set_dictionary_dir(dir: Option<PathBuf>) {
    *EXTRA_DIR.write().unwrap_or_else(|e| e.into_inner()) = dir;
    LOAD_ERRORS.write().unwrap_or_else(|e| e.into_inner()).clear(); // they belong to the old search path
}

/// The LibreOffice extension directories a Hunspell dictionary name may live in: LibreOffice's
/// IDs are per dictionary — Polish is `dict-pl`, Brazilian Portuguese `dict-pt-BR` — so both
/// the primary-language folder and the dashed full name are tried.
fn lo_ext_dirs(name: &str) -> Vec<String> {
    let primary = name.split(['-', '_']).next().unwrap_or(name);
    let dashed = name.replace('_', "-");
    let first = format!("dict-{primary}");
    let full = format!("dict-{dashed}");
    if full == first { vec![first] } else { vec![first, full] }
}

/// Directories that may hold Hunspell dictionaries, most specific first. `lo_exts`, when a
/// language is being searched for, adds LibreOffice's extension folders for it (`dict-pt-BR`).
pub fn search_dirs(lo_exts: Option<&[String]>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = EXTRA_DIR.read().unwrap_or_else(|e| e.into_inner()).clone() {
        dirs.push(d);
    }
    if cfg!(target_arch = "wasm32") {
        return dirs;
    }
    if let Some(d) = std::env::var_os("WORDCRAFT_DICTIONARIES").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(d));
    }
    // LibreOffice keeps each language under `…/share/extensions/<dict-xx>`.
    let lo_dirs =
        |prefix: &str| -> Vec<PathBuf> { lo_exts.map_or_else(Vec::new, |exts| exts.iter().map(|e| PathBuf::from(prefix).join(e)).collect()) };
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        for var in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                dirs.push(base.join("WordCraft").join("dictionaries"));
                dirs.push(base.join("hunspell"));
            }
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                dirs.extend(lo_dirs(&base.join("LibreOffice").join("share").join("extensions").to_string_lossy()));
            }
        }
        // If LibreOffice is installed on another drive (e.g. D:\Program Files\LibreOffice), check drives C..Z
        for drive in b'C'..=b'Z' {
            let drive_char = drive as char;
            for pf in ["Program Files", "Program Files (x86)"] {
                let lo_base = PathBuf::from(format!(r"{drive_char}:\{pf}\LibreOffice\share\extensions"));
                if lo_base.is_dir() {
                    dirs.extend(lo_dirs(&lo_base.to_string_lossy()));
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            dirs.push(h.join("Library/Spelling"));
        }
        dirs.push(PathBuf::from("/Library/Spelling"));
        dirs.push(PathBuf::from("/opt/homebrew/share/hunspell"));
        dirs.push(PathBuf::from("/usr/local/share/hunspell"));
        dirs.extend(lo_dirs("/Applications/LibreOffice.app/Contents/Resources/extensions"));
    } else {
        if let Some(h) = &home {
            dirs.push(h.join(".local/share/hunspell"));
        }
        for d in ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/usr/local/share/myspell"] {
            dirs.push(PathBuf::from(d));
        }
        for d in ["/usr/lib/libreoffice/share/extensions", "/opt/libreoffice/share/extensions"] {
            dirs.extend(lo_dirs(d));
        }
    }
    dirs
}

/// The first `<name>.aff`/`<name>.dic` pair found for any of `names` (`pl_PL`, `pl`), searching
/// [`search_dirs`] (including LibreOffice's extension folder for the first name).
pub fn find(names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    let lo = names.first().copied().map(lo_ext_dirs);
    search_dirs(lo.as_deref()).into_iter().find_map(|dir| find_in(&dir, names))
}

/// The first `<name>.aff`/`<name>.dic` pair for any of `names` in `dir`.
pub fn find_in(dir: &Path, names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    names.iter().find_map(|n| {
        let aff = dir.join(format!("{n}.aff"));
        let dic = dir.join(format!("{n}.dic"));
        (aff.is_file() && dic.is_file()).then_some((aff, dic))
    })
}

/// Read a Hunspell pair from disk (size-capped), decoded per the `.aff`'s `SET` line.
pub fn load(aff: &Path, dic: &Path) -> Result<AffixDict, String> {
    let read = |p: &Path| -> Result<Vec<u8>, String> {
        let len = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?.len();
        if len > MAX_FILE {
            return Err(format!("{}: too large ({len} bytes)", p.display()));
        }
        std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let (aff_text, dic_text) = decode(&read(aff)?, &read(dic)?)?;
    let source = format!("Hunspell dictionary {}", dic.display());
    AffixDict::from_hunspell(&aff_text, &dic_text, &[source.as_str()])
}

/// ISO 8859-1: every byte is the code point of the same value (the original VERO pt_BR uses it).
fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// ISO 8859-2 code points 0xA0..=0xFF (0x00..=0x9F map to themselves).
const ISO_8859_2_HIGH: &str = "\u{a0}Ą˘Ł¤ĽŚ§¨ŠŞŤŹ\u{ad}ŽŻ°ą˛ł´ľśˇ¸šşťź˝žżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙";

fn decode_latin2(bytes: &[u8]) -> String {
    let high: Vec<char> = ISO_8859_2_HIGH.chars().collect();
    bytes.iter().map(|&b| if b < 0xa0 { char::from(b) } else { high.get(usize::from(b - 0xa0)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// KOI8-R code points 0x80..=0xFF (0x00..=0x7F are ASCII): the encoding of older Russian
/// dictionaries.
const KOI8_R_HIGH: &str =
    "─│┌┐└┘├┤┬┴┼▀▄█▌▐░▒▓⌠■∙√≈≤≥\u{a0}⌡°²·÷═║╒ё╓╔╕╖╗╘╙╚╛╜╝╞╟╠╡Ё╢╣╤╥╦╧╨╩╪╫╬©юабцдефгхийклмнопярстужвьызшэщчъЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";

/// Windows-1251 code points 0x80..=0xFF (0x98 is unassigned): the other one Russian dictionaries
/// use.
const CP1251_HIGH: &str = "ЂЃ‚ѓ„…†‡€‰Љ‹ЊЌЋЏђ‘’“”•–—\u{fffd}™љ›њќћџ\u{a0}ЎўЈ¤Ґ¦§Ё©Є«¬\u{ad}®Ї°±Ііґµ¶·ё№є»јЅѕїАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдежзийклмнопрстуфхцчшщъыьэюя";

/// Decode a single-byte encoding whose bytes 0x80..=0xFF are `high` (ASCII below).
fn decode_high(bytes: &[u8], high: &str) -> String {
    let high: Vec<char> = high.chars().collect();
    bytes.iter().map(|&b| if b < 0x80 { char::from(b) } else { high.get(usize::from(b - 0x80)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// Decode an `.aff`/`.dic` pair to UTF-8 from the encoding the `.aff`'s `SET` line names
/// (UTF-8 when there is none; ISO 8859-2 is the other one Polish dictionaries use, ISO 8859-1 the
/// one the original Portuguese VERO uses, KOI8-R and Windows-1251 the ones older Russian dictionaries use).
pub fn decode(aff: &[u8], dic: &[u8]) -> Result<(String, String), String> {
    let head = String::from_utf8_lossy(aff.get(..aff.len().min(8192)).unwrap_or_default()).to_string();
    let set = head.lines().find_map(|l| l.trim_start_matches('\u{feff}').strip_prefix("SET ")).map(str::trim).unwrap_or("UTF-8").to_ascii_uppercase();
    match set.as_str() {
        "UTF-8" | "UTF8" => {
            let text = |b: &[u8], what: &str| String::from_utf8(b.to_vec()).map_err(|e| format!("{what}: not UTF-8: {e}"));
            Ok((text(aff, ".aff")?, text(dic, ".dic")?))
        }
        "ISO8859-1" | "ISO-8859-1" => Ok((decode_latin1(aff), decode_latin1(dic))),
        "ISO8859-2" | "ISO-8859-2" => Ok((decode_latin2(aff), decode_latin2(dic))),
        "KOI8-R" | "KOI8R" => Ok((decode_high(aff, KOI8_R_HIGH), decode_high(dic, KOI8_R_HIGH))),
        "CP1251" | "WINDOWS-1251" | "MICROSOFT-CP1251" => Ok((decode_high(aff, CP1251_HIGH), decode_high(dic, CP1251_HIGH))),
        other => Err(format!("unsupported encoding {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_latin2_and_utf8() {
        let aff = b"SET ISO8859-2\nTRY a\n";
        let (a, d) = decode(aff, b"2\n\xbf\xf3\xb3w\n").unwrap();
        assert!(a.contains("TRY a"));
        assert_eq!(d, "2\nżółw\n");
        let (_, d) = decode("SET UTF-8\n".as_bytes(), "żółw".as_bytes()).unwrap();
        assert_eq!(d, "żółw");
        assert!(decode(b"SET SHIFT_JIS\n", b"").is_err());
        assert!(decode(b"SET UTF-8\n", b"\xff\xfe").is_err());
    }

    /// #2830: a dictionary that is there but can't be read leaves its reason behind; a good
    /// load or no file clears it.
    #[test]
    fn load_error_is_remembered_until_the_next_good_load() {
        let key = "test_unreadable_2830";
        assert_eq!(load_error(key), None);
        let dic = Path::new("/dicts/test.dic");
        assert!(note_load(key, dic, decode(b"SET SHIFT_JIS\n", b"")).is_err());
        let (file, reason) = load_error(key).expect("error kept");
        assert_eq!(file, dic);
        assert_eq!(reason, "unsupported encoding SHIFT_JIS");
        assert!(note_load(key, dic, decode(b"SET UTF-8\n", b"")).is_ok());
        assert_eq!(load_error(key), None);
        let _ = note_load::<()>(key, dic, Err("too large".into()));
        clear_load_error(key);
        assert_eq!(load_error(key), None);
    }

    #[test]
    fn decodes_latin1() {
        // `orçamento esperávamos` in ISO 8859-1: ç = 0xE7, á = 0xE1.
        let (a, d) = decode(b"SET ISO8859-1\nTRY \xe7\n", b"1\nor\xe7amento/A\nesper\xe1vamos\n").unwrap();
        assert!(a.contains("TRY ç"), "{a}");
        assert_eq!(d, "1\norçamento/A\nesperávamos\n");
        assert!(decode(b"SET ISO-8859-1\n", b"\xe7").is_ok());
    }

    #[test]
    fn decodes_russian_single_byte_encodings() {
        // `ёлка` in KOI8-R (0xA3 0xCC 0xCB 0xC1) and in Windows-1251 (0xB8 0xEB 0xEA 0xE0).
        let (a, d) = decode(b"SET KOI8-R\nTRY \xcf\xc5\n", b"1\n\xa3\xcc\xcb\xc1/A\n").unwrap();
        assert!(a.contains("TRY ое"), "{a}");
        assert_eq!(d, "1\nёлка/A\n");
        let (_, d) = decode(b"SET microsoft-cp1251\n", b"1\n\xb8\xeb\xea\xe0 \xc4\xee\xec\n").unwrap();
        assert_eq!(d, "1\nёлка Дом\n");
        assert_eq!(decode_high(&[0x98], CP1251_HIGH), "\u{fffd}");
    }

    #[test]
    fn libreoffice_extension_dirs_follow_the_language() {
        // LibreOffice's IDs are per dictionary: Polish is `dict-pl`, Brazilian Portuguese
        // `dict-pt-BR`, so both the primary-language folder and the dashed full name are tried.
        assert_eq!(super::lo_ext_dirs("cs_CZ"), ["dict-cs", "dict-cs-CZ"]);
        assert_eq!(super::lo_ext_dirs("id_ID"), ["dict-id", "dict-id-ID"]);
        assert_eq!(super::lo_ext_dirs("nl_NL"), ["dict-nl", "dict-nl-NL"]);
        assert_eq!(super::lo_ext_dirs("pl_PL"), ["dict-pl", "dict-pl-PL"]);
        assert_eq!(super::lo_ext_dirs("pt_BR"), ["dict-pt", "dict-pt-BR"]);
        assert_eq!(super::lo_ext_dirs("de-DE"), ["dict-de", "dict-de-DE"]);
        let dirs = search_dirs(Some(&super::lo_ext_dirs("pt_BR")));
        assert!(dirs.iter().any(|d| d.ends_with("dict-pt-BR")), "{dirs:?}");
        assert!(search_dirs(None).iter().all(|d| !d.to_string_lossy().contains("dict-")), "no language: no LibreOffice guess");
    }

    #[test]
    fn finds_and_loads_a_pair_in_a_directory() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hunspell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(find_in(&dir, &["xx_XX"]).is_none(), "no files: nothing found");
        std::fs::write(dir.join("xx_XX.aff"), "SET UTF-8\nSFX A Y 1\nSFX A 0 y .\n").unwrap();
        assert!(find_in(&dir, &["xx_XX"]).is_none(), "an .aff alone isn't a dictionary");
        std::fs::write(dir.join("xx_XX.dic"), "1\nkot/A\n").unwrap();
        let (aff, dic) = find_in(&dir, &["yy", "xx_XX"]).unwrap();
        let d = load(&aff, &dic).unwrap();
        assert!(d.check("kot") && d.check("koty") && !d.check("kotx"));
        assert!(load(&dir.join("missing.aff"), &dic).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
