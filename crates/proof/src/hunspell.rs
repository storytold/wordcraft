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

/// Search `dir` before the system locations (a "dictionaries folder" setting); `None` clears it.
/// Takes effect for dictionaries not loaded yet.
pub fn set_dictionary_dir(dir: Option<PathBuf>) {
    *EXTRA_DIR.write().unwrap_or_else(|e| e.into_inner()) = dir;
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
/// (UTF-8 when there is none; ISO 8859-2 is the other one Polish dictionaries use, KOI8-R and
/// Windows-1251 the ones older Russian dictionaries use).
///
/// Under a UTF-8 `SET` without `FLAG UTF-8`, flags are still single bytes, as in Hunspell: the
/// Hungarian dictionary writes its affix flags and flag aliases as bytes above 0x7F that aren't
/// UTF-8. Those fields are read byte by byte (each byte becomes U+0000..U+00FF, one flag per
/// byte); the words, affixes and conditions around them are UTF-8.
pub fn decode(aff: &[u8], dic: &[u8]) -> Result<(String, String), String> {
    let head = String::from_utf8_lossy(aff.get(..aff.len().min(8192)).unwrap_or_default()).to_string();
    let directive = |name: &str| {
        head.lines().find_map(|l| {
            let l = l.trim_start_matches('\u{feff}');
            l.strip_prefix(name).filter(|rest| rest.starts_with([' ', '\t'])).map(|rest| rest.trim().to_ascii_uppercase())
        })
    };
    let set = directive("SET").unwrap_or_else(|| "UTF-8".into());
    match set.as_str() {
        "UTF-8" | "UTF8" => {
            if matches!(directive("FLAG").as_deref(), Some("UTF-8" | "UTF8")) {
                let text = |b: &[u8], what: &str| String::from_utf8(b.to_vec()).map_err(|e| format!("{what}: not UTF-8: {e}"));
                return Ok((text(aff, ".aff")?, text(dic, ".dic")?));
            }
            Ok((decode_utf8_aff(aff)?, decode_utf8_dic(dic)?))
        }
        "ISO8859-2" | "ISO-8859-2" => Ok((decode_latin2(aff), decode_latin2(dic))),
        "KOI8-R" | "KOI8R" => Ok((decode_high(aff, KOI8_R_HIGH), decode_high(dic, KOI8_R_HIGH))),
        "CP1251" | "WINDOWS-1251" | "MICROSOFT-CP1251" => Ok((decode_high(aff, CP1251_HIGH), decode_high(dic, CP1251_HIGH))),
        other => Err(format!("unsupported encoding {other}")),
    }
}

/// Directives whose one argument is a flag.
const FLAG_DIRECTIVES: &[&str] = &[
    "CIRCUMFIX",
    "COMPOUNDBEGIN",
    "COMPOUNDEND",
    "COMPOUNDFLAG",
    "COMPOUNDFORBIDFLAG",
    "COMPOUNDLAST",
    "COMPOUNDFIRST",
    "COMPOUNDMIDDLE",
    "COMPOUNDPERMITFLAG",
    "COMPOUNDROOT",
    "FORBIDDENWORD",
    "FORCEUCASE",
    "GENERATE",
    "HU_KOTOHANGZO",
    "KEEPCASE",
    "LEMMA_PRESENT",
    "NEEDAFFIX",
    "NOSUGGEST",
    "ONLYINCOMPOUND",
    "ONLYROOT",
    "PSEUDOROOT",
    "SUBSTANDARD",
    "WARN",
];

/// Directives that only describe the dictionary; a non-UTF-8 byte in them (the Hungarian
/// dictionary's `NAME` is ISO 8859-2) is replaced instead of failing the load.
const METADATA_DIRECTIVES: &[&str] = &["NAME", "HOME", "VERSION", "LANG", "AM"];

/// One byte per char: a flag field read as single-byte flags.
fn bytes_as_flags(b: &[u8]) -> String {
    b.iter().map(|&x| char::from(x)).collect()
}

/// `text/flags`: the text as UTF-8 and the flags byte by byte (`\/` is a literal slash).
fn text_and_flags(field: &[u8], what: &str) -> Result<String, String> {
    let mut slash = None;
    let mut i = 0;
    while let Some(&b) = field.get(i) {
        if b == b'\\' && field.get(i + 1) == Some(&b'/') {
            i += 2;
            continue;
        }
        if b == b'/' {
            slash = Some(i);
            break;
        }
        i += 1;
    }
    let (text, flags) = match slash {
        Some(k) => (field.get(..k).unwrap_or_default(), field.get(k..).unwrap_or_default()),
        None => (field, &[][..]),
    };
    let mut out = std::str::from_utf8(text).map_err(|e| format!("{what}: not UTF-8: {e}"))?.to_string();
    out.push_str(&bytes_as_flags(flags));
    Ok(out)
}

/// Split a line into fields on spaces and tabs, keeping the separators (so a line rebuilt from
/// its parts is the original).
fn fields(line: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_sep = line.first().is_some_and(|b| matches!(b, b' ' | b'\t'));
    for (i, b) in line.iter().enumerate() {
        let sep = matches!(b, b' ' | b'\t');
        if sep != in_sep {
            out.push(line.get(start..i).unwrap_or_default());
            start = i;
            in_sep = sep;
        }
    }
    out.push(line.get(start..).unwrap_or_default());
    out
}

/// A UTF-8 `.aff` with single-byte flags (see [`decode`]).
fn decode_utf8_aff(aff: &[u8]) -> Result<String, String> {
    let mut out = String::with_capacity(aff.len());
    for (n, line) in aff.split(|&b| b == b'\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if let Ok(s) = std::str::from_utf8(line)
            && s.is_ascii()
        {
            out.push_str(s);
            continue;
        }
        let parts = fields(line);
        // Field index among the non-separator parts.
        let words: Vec<usize> =
            parts.iter().enumerate().filter(|(_, p)| !p.first().is_some_and(|b| matches!(b, b' ' | b'\t'))).map(|(i, _)| i).collect();
        let dir = words
            .first()
            .and_then(|&i| parts.get(i))
            .map(|p| String::from_utf8_lossy(p).trim_start_matches('\u{feff}').to_string())
            .unwrap_or_default();
        let what = format!(".aff line {}", n + 1);
        if dir.starts_with('#') || METADATA_DIRECTIVES.contains(&dir.as_str()) {
            out.push_str(&String::from_utf8_lossy(line));
            continue;
        }
        // A trailing `# comment` (the Hungarian `AF` lines number themselves) is never a word or
        // a flag; from there on the line is kept as text.
        let comment_at = words.iter().skip(1).position(|&w| parts.get(w).is_some_and(|f| f.first() == Some(&b'#'))).map(|k| k + 1);
        for (i, p) in parts.iter().enumerate() {
            let field = words.iter().position(|&w| w == i);
            let in_comment = comment_at.is_some_and(|c| words.get(c).is_some_and(|&w| i >= w));
            let decoded = match (dir.as_str(), field) {
                _ if in_comment => String::from_utf8_lossy(p).into_owned(),
                ("AF" | "PFX" | "SFX" | "COMPOUNDRULE", Some(1)) => bytes_as_flags(p),
                ("PFX" | "SFX", Some(3)) | ("CHECKCOMPOUNDPATTERN", Some(1 | 2)) => text_and_flags(p, &what)?,
                (d, Some(1)) if FLAG_DIRECTIVES.contains(&d) => bytes_as_flags(p),
                _ => std::str::from_utf8(p).map_err(|e| format!("{what}: not UTF-8: {e}"))?.to_string(),
            };
            out.push_str(&decoded);
        }
    }
    Ok(out)
}

/// A UTF-8 `.dic` with single-byte flags: the word is UTF-8, the flags after `/` are bytes.
fn decode_utf8_dic(dic: &[u8]) -> Result<String, String> {
    let mut out = String::with_capacity(dic.len());
    for (n, line) in dic.split(|&b| b == b'\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if let Ok(s) = std::str::from_utf8(line)
            && s.split_once('/').is_none_or(|(_, flags)| flags.is_ascii())
        {
            out.push_str(s);
            continue;
        }
        let what = format!(".dic line {}", n + 1);
        // The first field is `word/flags`; morphological fields after it are UTF-8 text.
        let end = line.iter().position(|b| matches!(b, b' ' | b'\t')).unwrap_or(line.len());
        out.push_str(&text_and_flags(line.get(..end).unwrap_or_default(), &what)?);
        let rest = line.get(end..).unwrap_or_default();
        out.push_str(std::str::from_utf8(rest).map_err(|e| format!("{what}: not UTF-8: {e}"))?);
    }
    Ok(out)
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

    #[test]
    fn utf8_dictionaries_keep_single_byte_flags() {
        // The Hungarian dictionary is `SET UTF-8` but writes flags as single bytes above 0x7F,
        // which aren't UTF-8; each byte is one flag. Its `NAME` line is ISO 8859-2.
        let aff = b"SET UTF-8\nNAME Magyar helyes\xedr\xe1si\nAF 1\nAF \xd5a # 1\nSFX \xd5 Y 1\nSFX \xd5 0 ok/\xe9 . 7\nSFX \xe9 Y 1\nSFX \xe9 0 ban .\nSFX a Y 1\nSFX a 0 at .\n";
        let dic = "2\nház/1\t1\nkert/\u{e9}\n".as_bytes();
        let (a, d) = decode(aff, dic).unwrap();
        assert!(a.contains("AF \u{d5}a # 1") && a.contains("SFX \u{d5} 0 ok/\u{e9} . 7"));
        assert!(d.contains("kert/\u{c3}\u{a9}"), "a UTF-8 é in the flags is two one-byte flags");
        let dict = crate::affix::AffixDict::from_hunspell(&a, &d, &[]).unwrap();
        assert!(dict.check("ház") && dict.check("házok") && dict.check("házokban") && dict.check("házat"));
        assert!(!dict.check("házban"));
        // `FLAG UTF-8` keeps the whole file UTF-8, and words must still be UTF-8.
        let (_, d) = decode(b"SET UTF-8\nFLAG UTF-8\n", "kert/\u{e9}\n".as_bytes()).unwrap();
        assert_eq!(d, "kert/\u{e9}\n");
        assert!(decode(b"SET UTF-8\nSFX a Y 1\nSFX a 0 \xe9 .\n", b"").is_err(), "an affix must be UTF-8");
        assert!(decode(b"SET UTF-8\n", b"1\nh\xe1z/a\n").is_err(), "a stem must be UTF-8");
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
