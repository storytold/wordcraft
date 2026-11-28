//! A bytes-only check of a ZIP package's declared entry count, run before anything opens the ZIP
//! (#3660). The logic lives in `craft-zip`; this wraps its error as a `String`.

/// Refuse `bytes` when the end record `zip` would use declares more than `max_entries` entries.
pub fn check(bytes: &[u8], max_entries: u64) -> Result<(), String> {
    craft_zip::check_entry_count(bytes, max_entries).map_err(|e| e.to_string())
}

/// A ZIP64 package declaring `entries` entries: regression input for the readers' tests.
#[cfg(any(test, feature = "test-support"))]
pub use craft_zip::zip64_entry_count_sample;

#[cfg(test)]
mod tests {
    use super::{check, zip64_entry_count_sample};

    const EOCD: &[u8; 4] = b"PK\x05\x06";
    const CENTRAL_HEADER: &[u8; 4] = b"PK\x01\x02";

    /// A plain package's tail: one central header at 0, then an end record declaring `entries`.
    fn plain(entries: u16, comment: &[u8]) -> Vec<u8> {
        let mut f = CENTRAL_HEADER.to_vec();
        f.resize(46, 0);
        f.extend_from_slice(EOCD);
        f.extend_from_slice(&[0, 0, 0, 0]);
        f.extend_from_slice(&entries.to_le_bytes());
        f.extend_from_slice(&entries.to_le_bytes());
        f.extend_from_slice(&46u32.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        f.extend_from_slice(comment);
        f
    }

    #[test]
    fn a_zip64_end_record_declaring_too_many_entries_is_refused() {
        // #3660: `zip` would reserve and parse them all before the reader's cap.
        let sample = zip64_entry_count_sample(20_001);
        assert!(check(&sample, 20_000).is_err_and(|e| e.contains("20001 entries")));
        assert!(check(&zip64_entry_count_sample(5), 20_000).is_ok());
        // Junk before the package (a self-extracting stub) doesn't hide it.
        let mut prefixed = vec![0x55u8; 4096];
        prefixed.extend_from_slice(&sample);
        assert!(check(&prefixed, 20_000).is_err());
    }

    #[test]
    fn a_plain_end_record_is_checked_as_zip_reads_it() {
        assert!(check(&plain(3, b""), 20_000).is_ok());
        assert!(check(&plain(30_000, b"a comment"), 20_000).is_err_and(|e| e.contains("30000 entries")));
        // A record whose comment runs past the end is not the one `zip` uses.
        let mut cut = plain(30_000, b"");
        cut.truncate(cut.len() - 2);
        cut.extend_from_slice(&500u16.to_le_bytes());
        assert!(check(&cut, 20_000).is_ok());
        assert!(check(b"not a package", 20_000).is_ok());
        assert!(check(b"", 20_000).is_ok());
    }

    #[test]
    fn a_fake_end_record_after_the_real_one_does_not_hide_it() {
        // `zip` skips a last record with no central header before it and uses the one before.
        let mut f = zip64_entry_count_sample(1_000_000);
        f.extend_from_slice(EOCD);
        f.extend_from_slice(&[0, 0, 0, 0]);
        f.extend_from_slice(&7u16.to_le_bytes());
        f.extend_from_slice(&7u16.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&((f.len() - 12) as u32).to_le_bytes());
        f.extend_from_slice(&0u16.to_le_bytes());
        assert!(check(&f, 20_000).is_err());
    }

    #[test]
    fn many_end_records_are_refused() {
        // Each declares one entry and has no central header before it, so `zip` would try them all.
        let record = [0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let f: Vec<u8> = (0..20).flat_map(|_| EOCD.iter().copied().chain(record)).collect();
        assert!(check(&f, 20_000).is_err_and(|e| e.contains("end of central directory records")));
    }

    /// The pre-cutover implementation, kept as a differential oracle.
    mod old {
        const EOCD: &[u8; 4] = b"PK\x05\x06";
        const EOCD64_LOCATOR: &[u8; 4] = b"PK\x06\x07";
        const EOCD64: &[u8; 4] = b"PK\x06\x06";
        const CENTRAL_HEADER: &[u8; 4] = b"PK\x01\x02";
        /// End records looked at, at most. A package has one; a few more can turn up inside stored data.
        const MAX_END_RECORDS: usize = 16;

        /// Refuse `bytes` when the end record `zip` would use declares more than `max_entries` entries.
        ///
        /// `zip` searches the file backwards for an end of central directory record. A ZIP64 one points
        /// through its locator at the ZIP64 end record, which `zip` looks for from the locator's offset
        /// on: every such record there is checked, and as `zip` may fall back to an earlier end record
        /// when one doesn't parse, the search goes on. A plain one is used when it declares no entries, or
        /// a central directory header lies between its directory offset and itself: it is the last one
        /// looked at.
        pub fn check(bytes: &[u8], max_entries: u64) -> Result<(), String> {
            let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
            let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            let u64_at = |at: usize| bytes.get(at..at.checked_add(8)?).and_then(|b| <[u8; 8]>::try_from(b).ok()).map(u64::from_le_bytes);
            let refuse = |n: u64| -> Result<(), String> {
                if n > max_entries { Err(format!("the package declares {n} entries, more than {max_entries}")) } else { Ok(()) }
            };
            let mut end = bytes.len();
            let mut seen = 0;
            while let Some(at) = bytes.get(..end).and_then(|b| b.windows(4).rposition(|w| w == EOCD)) {
                end = at;
                seen += 1;
                if seen > MAX_END_RECORDS {
                    return Err(format!("more than {MAX_END_RECORDS} end of central directory records"));
                }
                // `zip` skips a record whose comment runs past the end of the file.
                let Some(comment) = u16_at(at + 20) else { continue };
                if at + 22 + usize::from(comment) > bytes.len() {
                    continue;
                }
                let (Some(on_disk), Some(total), Some(cd_offset)) = (u16_at(at + 8), u16_at(at + 10), u32_at(at + 16)) else { continue };
                let zip64 = total == u16::MAX || cd_offset == u32::MAX;
                if zip64
                    && let Some(locator) = at.checked_sub(20)
                    && bytes.get(locator..locator + 4) == Some(EOCD64_LOCATOR.as_slice())
                {
                    let (Some(eocd64), Some(disks)) = (u64_at(locator + 8), u32_at(locator + 16)) else { continue };
                    let Some(from) = usize::try_from(eocd64).ok().filter(|&f| f < locator) else { continue };
                    if disks > 1 {
                        continue;
                    }
                    let records = bytes.get(from..locator).unwrap_or_default().windows(4).enumerate().filter(|(_, w)| *w == EOCD64);
                    for (i, _) in records {
                        let p = from + i;
                        refuse(u64_at(p + 24).unwrap_or(0).max(u64_at(p + 32).unwrap_or(0)))?;
                    }
                    continue;
                }
                if total == 0 {
                    return Ok(());
                }
                let cd = cd_offset as usize;
                if cd < at && bytes.get(cd..at).is_some_and(|b| b.windows(4).any(|w| w == CENTRAL_HEADER)) {
                    return refuse(u64::from(on_disk.max(total)));
                }
            }
            Ok(())
        }
    }

    fn sentinel_sample() -> Vec<u8> {
        // ZIP64 sample whose plain end record claims one entry and a real directory offset but
        // sets the directory size to u32::MAX, the ZIP64 marker `zip` honours.
        let mut f = zip64_entry_count_sample(30_000);
        let n = f.len();
        f[n - 14..n - 12].copy_from_slice(&1u16.to_le_bytes());
        f[n - 12..n - 10].copy_from_slice(&1u16.to_le_bytes());
        f[n - 10..n - 6].copy_from_slice(&u32::MAX.to_le_bytes());
        f[n - 6..n - 2].copy_from_slice(&31u32.to_le_bytes());
        f
    }

    fn corpus() -> Vec<Vec<u8>> {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 24) as u8
                })
                .collect()
        };
        let mut truncated = zip64_entry_count_sample(10);
        truncated.truncate(truncated.len() - 5);
        vec![
            vec![],
            plain(3, b""),
            plain(30_000, b"c"),
            zip64_entry_count_sample(1),
            zip64_entry_count_sample(10),
            zip64_entry_count_sample(30_000),
            zip64_entry_count_sample(5_000_000),
            truncated,
            rnd(1),
            rnd(100),
            rnd(5000),
        ]
    }

    #[test]
    fn agrees_with_the_old_implementation_except_for_the_sentinel() {
        for (i, input) in corpus().iter().enumerate() {
            for max in [0u64, 9, 10, 20_000] {
                assert_eq!(check(input, max).is_ok(), old::check(input, max).is_ok(), "input {i}, max {max}");
            }
        }
    }

    #[test]
    fn a_directory_size_sentinel_alone_cannot_bypass_the_count_limit() {
        let f = sentinel_sample();
        assert!(old::check(&f, 20_000).is_ok(), "the old check missed it");
        assert!(check(&f, 20_000).is_err_and(|e| e.contains("30000 entries")));
    }

    #[test]
    fn no_truncation_of_a_zip64_package_panics() {
        for f in [zip64_entry_count_sample(10), sentinel_sample()] {
            for end in 0..=f.len().min(4096) {
                let _ = check(&f[..end], 5);
            }
            for start in f.len().saturating_sub(300)..f.len() {
                let _ = check(&f[start..], 5);
            }
        }
    }
}
