//! Windows: pins made from the old advertised Start menu shortcut, mended after an update.
//!
//! Up to the release that made the Start menu shortcut a plain one (`packaging/windows/wordcraft.wxs`),
//! the installer made an advertised shortcut. Pinning it to the taskbar (or to Start, on Windows 10)
//! copies that shortcut into `User Pinned`, and the copy names the package by its ProductCode and
//! takes its icon from `%WINDIR%\Installer\{ProductCode}`. Every update is a major upgrade with a new
//! ProductCode, so the update to the plain shortcut still left that pin blank and dead. This replaces
//! such a pin, in place and under the same name, with a plain shortcut to the running `wordcraft.exe`,
//! so the pin keeps its place and works again without being pinned anew. The installer runs it right
//! after an install (`--repair-taskbar-pins`), and the app at every start, in case the installer's
//! run was skipped.
//!
//! The shortcut is read and written by hand, after the Shell Link format ([MS-SHLLINK]): reading one
//! needs only its header and string data, and the plain shortcut written here is a header, a
//! `LinkInfo` with the exe's path, its folder and its icon. No COM, so no `unsafe`.
//!
//! [MS-SHLLINK]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/

use std::path::Path;

/// The installer's shortcut description (`Description` in `wordcraft.wxs`), which the shortcut keeps
/// as its name string: how a pinned copy of WordCraft's own advertised shortcut is told apart from
/// any other advertised shortcut in the same folder.
const DESCRIPTION: &str = "WordCraft word processor";

const HEADER_SIZE: usize = 0x4C;
/// `00021401-0000-0000-C000-000000000046`, as stored.
const LINK_CLSID: [u8; 16] = [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];

const HAS_LINK_TARGET_ID_LIST: u32 = 0x0000_0001;
const HAS_LINK_INFO: u32 = 0x0000_0002;
const HAS_NAME: u32 = 0x0000_0004;
const HAS_WORKING_DIR: u32 = 0x0000_0010;
const HAS_ICON_LOCATION: u32 = 0x0000_0040;
const IS_UNICODE: u32 = 0x0000_0080;
/// The shortcut is a Windows Installer (advertised) one.
const HAS_DARWIN_ID: u32 = 0x0000_1000;

/// A shortcut larger than this is not one of ours; it is skipped unread.
const MAX_LINK_BYTES: u64 = 1 << 20;

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The shortcut's link flags, when `data` starts with a Shell Link header.
fn link_flags(data: &[u8]) -> Option<u32> {
    (u32_at(data, 0)? as usize == HEADER_SIZE && data.get(4..20)? == LINK_CLSID).then_some(())?;
    u32_at(data, 0x14)
}

/// The shortcut's name string (its description), when it has one.
fn name_string(data: &[u8]) -> Option<String> {
    let flags = link_flags(data)?;
    if flags & HAS_NAME == 0 {
        return None;
    }
    let mut at = HEADER_SIZE;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        at = at.checked_add(2)?.checked_add(usize::from(u16_at(data, at)?))?;
    }
    if flags & HAS_LINK_INFO != 0 {
        at = at.checked_add(u32_at(data, at)? as usize)?;
    }
    let count = usize::from(u16_at(data, at)?);
    let text = data.get(at + 2..)?;
    if flags & IS_UNICODE != 0 {
        let units: Vec<u16> = text.get(..count * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).ok()
    } else {
        Some(String::from_utf8_lossy(text.get(..count)?).into_owned())
    }
}

/// Whether `data` is a copy of WordCraft's own advertised Start menu shortcut.
pub(crate) fn is_old_wordcraft_pin(data: &[u8]) -> bool {
    link_flags(data).is_some_and(|f| f & HAS_DARWIN_ID != 0) && name_string(data).as_deref() == Some(DESCRIPTION)
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// A string-data entry: its length in UTF-16 units, then the units, no terminator.
fn string_data(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().take(usize::from(u16::MAX)).collect();
    let mut out = (units.len() as u16).to_le_bytes().to_vec();
    out.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    out
}

/// A plain shortcut to `exe`: started in its folder, with the exe's own icon and the installer's
/// description. `exe` is an absolute local path (`C:\…\wordcraft.exe`).
pub(crate) fn plain_link(exe: &str) -> Vec<u8> {
    let folder = exe.rsplit_once('\\').map_or("", |(dir, _)| dir);
    let mut out = Vec::with_capacity(512);
    out.extend((HEADER_SIZE as u32).to_le_bytes());
    out.extend(LINK_CLSID);
    out.extend((HAS_LINK_INFO | HAS_NAME | HAS_WORKING_DIR | HAS_ICON_LOCATION | IS_UNICODE).to_le_bytes());
    out.extend(0x20u32.to_le_bytes()); // FILE_ATTRIBUTE_ARCHIVE
    out.extend([0u8; 24]); // creation, access and write times
    out.extend(0u32.to_le_bytes()); // file size
    out.extend(0i32.to_le_bytes()); // icon index
    out.extend(1u32.to_le_bytes()); // SW_SHOWNORMAL
    out.extend([0u8; 12]); // hot key, reserved
    out.extend(link_info(exe));
    out.extend(string_data(DESCRIPTION));
    out.extend(string_data(folder));
    out.extend(string_data(exe));
    out.extend(0u32.to_le_bytes()); // no extra data blocks
    out
}

/// `LinkInfo` for a file on a fixed local drive, with the Unicode path as well as the ANSI one
/// (non-ASCII characters in the ANSI one are `?`; Windows reads the Unicode one).
fn link_info(exe: &str) -> Vec<u8> {
    const HEADER: u32 = 0x24;
    // VolumeID: its size, DRIVE_FIXED, no serial number, the label (empty) right after.
    let volume: Vec<u8> = [0x11u32, 3, 0, 0x10].iter().flat_map(|v| v.to_le_bytes()).chain([0]).collect();
    let mut ansi: Vec<u8> = exe.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).collect();
    ansi.push(0);
    let unicode = utf16z(exe);
    let volume_at = HEADER;
    let base_at = volume_at + volume.len() as u32;
    let suffix_at = base_at + ansi.len() as u32;
    let base_unicode_at = suffix_at + 1;
    let suffix_unicode_at = base_unicode_at + unicode.len() as u32;
    let size = suffix_unicode_at + 2;
    let mut out = Vec::with_capacity(size as usize);
    for v in [size, HEADER, 1 /* VolumeIDAndLocalBasePath */, volume_at, base_at, 0, suffix_at, base_unicode_at, suffix_unicode_at] {
        out.extend(v.to_le_bytes());
    }
    out.extend(volume);
    out.extend(ansi);
    out.push(0); // CommonPathSuffix: empty
    out.extend(unicode);
    out.extend([0, 0]); // CommonPathSuffixUnicode: empty
    out
}

/// Replace each pinned copy of the old advertised shortcut in `folder` with a plain one to `exe`,
/// under the same name. The number replaced.
pub(crate) fn repair_folder(folder: &Path, exe: &str) -> std::io::Result<usize> {
    let mut mended = 0;
    let entries = match std::fs::read_dir(folder) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
            continue;
        }
        if !entry.metadata().is_ok_and(|m| m.is_file() && m.len() <= MAX_LINK_BYTES) {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else { continue };
        if !is_old_wordcraft_pin(&data) {
            continue;
        }
        // Written beside it, then moved over it: a pin is never left half written.
        let mut temp = path.clone().into_os_string();
        temp.push(".new");
        std::fs::write(&temp, plain_link(exe))?;
        if let Err(e) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
        mended += 1;
    }
    Ok(mended)
}

/// Mend this account's taskbar and Start pins of the old shortcut, pointing them at the running exe.
#[cfg(windows)]
pub(crate) fn repair() -> std::io::Result<usize> {
    let Some(appdata) = std::env::var_os("APPDATA") else { return Ok(0) };
    let exe = std::env::current_exe()?;
    let Some(exe) = exe.to_str() else { return Ok(0) };
    let pinned = Path::new(&appdata).join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned");
    let mut mended = 0;
    for place in ["TaskBar", "StartMenu"] {
        mended += repair_folder(&pinned.join(place), exe)?;
    }
    Ok(mended)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\יונתן\AppData\Local\Programs\ArtCraft\WordCraft\wordcraft.exe";

    /// The shape of an advertised shortcut as Windows Installer writes it: a target ID list and
    /// the name string, with HasDarwinID (its Darwin data block is not needed to tell it apart).
    fn advertised(name: &str) -> Vec<u8> {
        let mut out = plain_link(EXE);
        let flags = HAS_LINK_TARGET_ID_LIST | HAS_NAME | IS_UNICODE | HAS_DARWIN_ID;
        out.truncate(HEADER_SIZE);
        out[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        out.extend(4u16.to_le_bytes());
        out.extend([2, 0, 0, 0]); // an ID list of one empty item, then its terminator
        out.extend(string_data(name));
        out.extend(0u32.to_le_bytes());
        out
    }

    /// The Unicode LocalBasePath of a plain shortcut: where Windows starts it from.
    fn target(data: &[u8]) -> String {
        let info = &data[HEADER_SIZE..];
        let at = u32_at(info, 0x1C).unwrap() as usize;
        let units: Vec<u16> = info[at..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn a_pin_of_the_old_advertised_shortcut_is_recognised() {
        assert!(is_old_wordcraft_pin(&advertised(DESCRIPTION)));
    }

    #[test]
    fn other_shortcuts_are_left_alone() {
        assert!(!is_old_wordcraft_pin(&advertised("Adobe Acrobat")), "another app's advertised shortcut");
        assert!(!is_old_wordcraft_pin(&plain_link(EXE)), "already plain");
        assert!(!is_old_wordcraft_pin(b"not a shortcut"));
        assert!(!is_old_wordcraft_pin(&advertised(DESCRIPTION)[..HEADER_SIZE + 3]), "cut short");
    }

    #[test]
    fn the_plain_shortcut_starts_the_exe_in_its_folder_with_its_icon() {
        let link = plain_link(EXE);
        assert_eq!(link_flags(&link), Some(HAS_LINK_INFO | HAS_NAME | HAS_WORKING_DIR | HAS_ICON_LOCATION | IS_UNICODE));
        assert_eq!(target(&link), EXE);
        assert_eq!(name_string(&link).as_deref(), Some(DESCRIPTION));
        // After the name: the working folder, then the icon location.
        let info_size = u32_at(&link, HEADER_SIZE).unwrap() as usize;
        let mut at = HEADER_SIZE + info_size;
        let mut strings = Vec::new();
        for _ in 0..3 {
            let n = usize::from(u16_at(&link, at).unwrap());
            let units: Vec<u16> = link[at + 2..at + 2 + n * 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            strings.push(String::from_utf16(&units).unwrap());
            at += 2 + n * 2;
        }
        assert_eq!(strings, [DESCRIPTION, r"C:\Users\יונתן\AppData\Local\Programs\ArtCraft\WordCraft", EXE]);
        assert_eq!(&link[at..], &[0, 0, 0, 0], "ends with the terminal block");
    }

    #[test]
    fn repair_replaces_only_the_old_pin_and_keeps_its_name() {
        let dir = std::env::temp_dir().join(format!("wordcraft-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("WordCraft.lnk"), advertised(DESCRIPTION)).unwrap();
        std::fs::write(dir.join("Acrobat.lnk"), advertised("Adobe Acrobat")).unwrap();
        std::fs::write(dir.join("notes.txt"), advertised(DESCRIPTION)).unwrap();
        assert_eq!(repair_folder(&dir, EXE).unwrap(), 1);
        assert_eq!(std::fs::read(dir.join("WordCraft.lnk")).unwrap(), plain_link(EXE));
        assert_eq!(std::fs::read(dir.join("Acrobat.lnk")).unwrap(), advertised("Adobe Acrobat"));
        assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), advertised(DESCRIPTION), "not a shortcut file");
        assert_eq!(repair_folder(&dir, EXE).unwrap(), 0, "nothing left to mend");
        assert!(!dir.join("WordCraft.lnk.new").exists());
        assert_eq!(repair_folder(&dir.join("missing"), EXE).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
