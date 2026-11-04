//! A bytes-only check of an OLE compound file ([MS-CFB]) before the `cfb` crate opens it
//! (#3642). Every reader of `.doc`, `.xls` and encrypted Office packages calls [`check`] first.

/// Spare FAT sectors a compound file may list past those its sectors need: as many as the
/// header's DIFAT holds.
pub const SPARE_FAT_SECTORS: usize = 109;

/// Check a compound file's FAT layout before `cfb` reads it ([MS-CFB] §2.2 header, §2.5
/// DIFAT). `cfb` appends a whole sector of FAT entries for every FAT sector the DIFAT lists and
/// only validates the FAT once it is built, so a small header naming one sector over and over
/// makes it allocate gigabytes. Here the DIFAT is walked the way `cfb` walks it (header entries,
/// then the DIFAT chain, at most one step per sector; trailing zero and free entries trimmed
/// the same way) and the file is refused when it lists one FAT sector twice, or more FAT sectors
/// than its sectors need plus [`SPARE_FAT_SECTORS`] spare ones (writers keep spare FAT sectors,
/// #5109), so the FAT `cfb` builds is about the size the file needs. Anything else that is wrong
/// is left for `cfb` to report.
pub fn check(bytes: &[u8]) -> Result<(), String> {
    const MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    const FREE: u32 = 0xFFFF_FFFF;
    const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
    const HEADER_ENTRIES: usize = 109;
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.len() < 512 || bytes.get(..8) != Some(MAGIC.as_slice()) {
        return Ok(());
    }
    let sector_len: usize = match u16_at(0x1A) {
        Some(3) => 512,
        Some(4) => 4096,
        _ => return Ok(()),
    };
    if bytes.len() < sector_len {
        return Ok(());
    }
    // `cfb` counts the sectors after the header sector, the last one possibly partial.
    let num_sectors = bytes.len().div_ceil(sector_len) - 1;
    let num_fat_sectors = u32_at(0x2C).unwrap_or(0) as usize;
    let mut difat: Vec<u32> = (0..HEADER_ENTRIES).filter_map(|i| u32_at(0x4C + i * 4)).collect();
    let mut next = u32_at(0x44).unwrap_or(END_OF_CHAIN);
    let mut steps = 0usize;
    while next != END_OF_CHAIN && next != FREE {
        steps += 1;
        // `cfb` refuses a chain sector past the file or seen before; a chain longer than the
        // file has sectors must have done one of the two.
        if steps > num_sectors || next as usize >= num_sectors {
            return Err(format!("the DIFAT chain is longer than the file or leaves it (sector {next})"));
        }
        let at = (next as usize + 1) * sector_len;
        let entries = sector_len / 4 - 1;
        difat.extend((0..entries).map(|i| u32_at(at + i * 4).unwrap_or(FREE)));
        next = u32_at(at + entries * 4).unwrap_or(END_OF_CHAIN);
    }
    while difat.len() > HEADER_ENTRIES && difat.len() > num_fat_sectors && difat.last() == Some(&0) {
        difat.pop();
    }
    while difat.last() == Some(&FREE) {
        difat.pop();
    }
    // Each listed FAT sector adds a whole sector of entries. A FAT sector holds the entries of
    // `sector_len / 4` sectors, so a file needs no more FAT sectors than that covers. It may keep
    // spare ones, their entries free: an encrypted package of 78 sectors listed 4 where one covers
    // them, and was refused while the guard allowed a single spare (#5109). Spare sectors are
    // allowed up to a fixed number, so the FAT `cfb` builds stays about the size the file needs
    // (at most 109 sectors more, under half a megabyte), not the size of the file itself. None
    // may be listed twice. A free entry in the middle is left for `cfb` to refuse.
    let listed: Vec<u32> = difat.into_iter().filter(|&s| s != FREE).collect();
    let needed = num_sectors.div_ceil(sector_len / 4);
    if listed.len() > needed.saturating_add(SPARE_FAT_SECTORS) {
        return Err(format!("the DIFAT lists {} FAT sectors in a file of {num_sectors} sectors", listed.len()));
    }
    let mut seen = std::collections::HashSet::with_capacity(listed.len());
    if let Some(dup) = listed.iter().find(|&&s| !seen.insert(s)) {
        return Err(format!("the DIFAT lists FAT sector {dup} twice"));
    }
    Ok(())
}

/// A compound file of about 1 MB whose DIFAT chain lists one FAT sector 2048 × 127 times:
/// without [`check`], `cfb` builds a FAT of about 130 MB for it. Regression input for the
/// readers' tests: built only for tests, this crate's and (through the `test-support` feature,
/// which they turn on as a dev-dependency) the readers', never into the app.
#[cfg(any(test, feature = "test-support"))]
pub fn repeating_difat_sample() -> Vec<u8> {
    const S: usize = 512;
    const CHAIN: u32 = 2048; // DIFAT sectors 1..=CHAIN, sector 0 is the "FAT"
    let mut f = vec![0u8; S * (CHAIN as usize + 2)];
    let mut put = |at: usize, v: &[u8]| {
        if let Some(dst) = f.get_mut(at..at + v.len()) {
            dst.copy_from_slice(v);
        }
    };
    put(0, &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    put(0x18, &0x3Eu16.to_le_bytes());
    put(0x1A, &3u16.to_le_bytes());
    put(0x1C, &0xFFFEu16.to_le_bytes());
    put(0x1E, &9u16.to_le_bytes());
    put(0x20, &6u16.to_le_bytes());
    put(0x2C, &u32::MAX.to_le_bytes()); // no zero trimming
    put(0x38, &4096u32.to_le_bytes());
    put(0x44, &1u32.to_le_bytes());
    put(0x48, &CHAIN.to_le_bytes());
    // The 109 header entries are all 0 already; each chain sector's entries too.
    for d in 1..=CHAIN {
        let next = if d == CHAIN { 0xFFFF_FFFE } else { d + 1 };
        put((d as usize + 2) * S - 4, &next.to_le_bytes());
    }
    f
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_repeating_difat_is_refused_and_a_small_file_passes() {
        assert!(super::check(&super::repeating_difat_sample()).is_err_and(|e| e.contains("FAT sectors in a file of")));
        assert!(super::check(b"not a compound file").is_ok());
    }

    /// A 512-byte-sector file of `sectors` sectors after the header, whose DIFAT lists `fat` (the
    /// rest free): the first 109 in the header, the rest in one DIFAT sector, the file's last.
    fn header_difat(sectors: usize, fat: &[u32]) -> Vec<u8> {
        let mut f = vec![0u8; 512 * (sectors + 1)];
        f[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
        f[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
        f[0x2C..0x30].copy_from_slice(&(fat.len() as u32).to_le_bytes());
        f[0x44..0x48].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        for i in 0..109 {
            let v = fat.get(i).copied().unwrap_or(u32::MAX);
            f[0x4C + i * 4..0x50 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        if fat.len() > 109 {
            let chain = sectors - 1;
            f[0x44..0x48].copy_from_slice(&(chain as u32).to_le_bytes());
            f[0x48..0x4C].copy_from_slice(&1u32.to_le_bytes());
            let at = (chain + 1) * 512;
            for i in 0..127 {
                let v = fat.get(109 + i).copied().unwrap_or(u32::MAX);
                f[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            f[at + 508..at + 512].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        }
        f
    }

    #[test]
    fn a_fat_sector_listed_twice_or_too_many_spare_ones_are_refused() {
        assert!(super::check(&header_difat(10, &[0, 0])).is_err_and(|e| e.contains("FAT sector 0 twice")));
        // 2000 sectors need 16 FAT sectors; 109 spare ones are allowed past them.
        let listed = |n: u32| -> Vec<u32> { (0..n).collect() };
        assert!(super::check(&header_difat(2000, &listed(16 + 109))).is_ok());
        assert!(super::check(&header_difat(2000, &listed(16 + 110))).is_err_and(|e| e.contains("126 FAT sectors in a file of 2000 sectors")));
        // A file listing nearly every one of its sectors as a FAT sector: `cfb` would build a FAT
        // the size of the file.
        assert!(super::check(&header_difat(230, &listed(229))).is_err_and(|e| e.contains("229 FAT sectors")));
        assert!(super::check(&header_difat(10, &[0])).is_ok());
        assert!(super::check(&header_difat(200, &[0, 1])).is_ok());
    }

    /// #5109: an encrypted package of 78 sectors listing 4 FAT sectors, where one covers them,
    /// was refused ("the DIFAT lists 4 FAT sectors in a file of 78 sectors"). Spare FAT sectors
    /// are allowed, up to [`super::SPARE_FAT_SECTORS`] of them.
    #[test]
    fn spare_fat_sectors_are_allowed() {
        assert!(super::check(&header_difat(78, &[0, 1, 2, 3])).is_ok());
        assert!(super::check(&header_difat(10, &[0, 1, 2])).is_ok());
        let ten: Vec<u32> = (0..10).collect();
        assert!(super::check(&header_difat(10, &ten)).is_ok());
    }
}
