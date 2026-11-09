//! Optional document text properties from [MS-OLEPS] §2.21 and §2.25.1.
//! Broken metadata must not prevent opening the document or allocate from unchecked sizes.

use std::io::{Read, Seek};

use wordcraft_doc::CoreProps;

const MAX_STREAM: u64 = 2 << 20;
const MAX_STRING: usize = 64 << 10;
const SUMMARY_FMTID: &[u8] = &[0xe0, 0x85, 0x9f, 0xf2, 0xf9, 0x4f, 0x68, 0x10, 0xab, 0x91, 0x08, 0x00, 0x2b, 0x27, 0xb3, 0xd9];

pub(crate) fn read<F: Read + Seek>(comp: &mut cfb::CompoundFile<F>) -> Option<CoreProps> {
    let stream = comp.open_stream("/\u{5}SummaryInformation").ok()?;
    if stream.len() > MAX_STREAM {
        return None;
    }
    let mut bytes = Vec::new();
    stream.take(MAX_STREAM + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_STREAM {
        return None;
    }
    parse(&bytes)
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?))
}

fn parse(bytes: &[u8]) -> Option<CoreProps> {
    if bytes.get(..2)? != [0xfe, 0xff] || bytes.get(28..44)? != SUMMARY_FMTID || u32_at(bytes, 24)? < 1 {
        return None;
    }
    if u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?) > 1 {
        return None;
    }
    let offset = u32_at(bytes, 44)? as usize;
    if offset < 48 || !offset.is_multiple_of(4) {
        return None;
    }
    let size = u32_at(bytes, offset)? as usize;
    let set = bytes.get(offset..offset.checked_add(size)?)?;
    let count = u32_at(set, 4)? as usize;
    if count > 4096 {
        return None;
    }
    let table_end = 8 + count * 8;
    let table = set.get(8..table_end)?;
    // Offsets are relative to the property set. Bound each value by the next property so a
    // forged string length cannot read another property's bytes as part of the title.
    let property = |id: u32| -> Option<&[u8]> {
        let entry = table.as_chunks::<8>().0.iter().find(|entry| u32_at(entry.as_slice(), 0) == Some(id))?.as_slice();
        let start = u32_at(entry, 4)? as usize;
        if start < table_end || !start.is_multiple_of(4) {
            return None;
        }
        let end = table
            .as_chunks::<8>()
            .0
            .iter()
            .filter_map(|e| u32_at(e, 4))
            .map(|n| n as usize)
            .filter(|n| *n > start)
            .min()
            .unwrap_or(set.len())
            .min(set.len());
        set.get(start..end)
    };
    let codepage = property(1)
        .and_then(|cp| {
            let vt = u32_at(cp, 0)?;
            if vt == 2 || vt == 3 { Some(u16::from_le_bytes(cp.get(4..6)?.try_into().ok()?)) } else { None }
        })
        .unwrap_or(1252);
    let get = |id| property(id).and_then(|value| text(value, codepage)).unwrap_or_default();
    let get_filetime = |id| property(id).and_then(filetime_iso).unwrap_or_default();
    Some(CoreProps {
        title: get(2),
        subject: get(3),
        creator: get(4),
        keywords: get(5),
        description: get(6),
        last_modified_by: get(8),
        revision: get(9).parse().unwrap_or_default(),
        created: get_filetime(12),
        modified: get_filetime(13),
        ..Default::default()
    })
}

/// A `VT_FILETIME` property value as an ISO-8601 UTC string (`YYYY-MM-DDTHH:MM:SSZ`), or `None`
/// if it is not a FILETIME, is zero, or predates the 1601 FILETIME epoch.
fn filetime_iso(value: &[u8]) -> Option<String> {
    let kind = u32_at(value, 0)?;
    if kind != 64 {
        return None;
    }
    let ft_low = u32_at(value, 4)? as u64;
    let ft_high = u32_at(value, 8)? as u64;
    let ft = (ft_high << 32) | ft_low;
    if ft == 0 {
        return None;
    }
    const EPOCH_DIFFERENCE: u64 = 116_444_736_000_000_000;
    let unix_100ns = ft.checked_sub(EPOCH_DIFFERENCE)?;
    let unix_secs = i64::try_from(unix_100ns / 10_000_000).ok()?;
    let (days, sod) = (unix_secs.div_euclid(86_400), unix_secs.rem_euclid(86_400));
    let (h, mi, s) = (sod / 3600, sod / 60 % 60, sod % 60);
    let (y, m, d) = civil(days);
    // `{y:04}` rather than `%Y`, which writes a `+` before years past 9999.
    Some(format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"))
}

/// Days since 1970-01-01 → proleptic Gregorian (year, month, day).
///
/// A FILETIME can land past `NaiveDate`'s ±262143-year range, where the old hand-rolled converter
/// still produced a date; dropping to `None` there would change the output. The Gregorian calendar
/// repeats every 400 years (146_097 days), so chrono converts the day within one cycle and whole
/// cycles go back into the year (an `i128` so the extremes can't overflow), keeping chrono as the
/// only calendar logic and reproducing the old result exactly. 1970-01-01 is `num_days_from_ce` 719_163.
fn civil(days: i64) -> (i64, u32, u32) {
    use chrono::Datelike;
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` is 719_163..865_259, always a valid CE day.
    match i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(d) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), d.month(), d.day())
        }
        None => (1970, 1, 1), // unreachable
    }
}

fn text(value: &[u8], codepage: u16) -> Option<String> {
    let kind = u32_at(value, 0)?;
    let count = u32_at(value, 4)? as usize;
    let size = match kind {
        30 => count,                 // CodePageString length is in bytes, even for code page 1200.
        31 => count.checked_mul(2)?, // UnicodeString length is in UTF-16 units.
        _ => return None,
    };
    if size > MAX_STRING {
        return None;
    }
    let bytes = value.get(8..8usize.checked_add(size)?)?;
    if kind == 31 || codepage == 1200 {
        if !bytes.len().is_multiple_of(2) {
            return None;
        }
        let units = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes([b[0], b[1]])).take_while(|u| *u != 0);
        Some(char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect())
    } else {
        let bytes = bytes.split(|b| *b == 0).next().unwrap_or_default();
        if codepage == 65001 {
            Some(String::from_utf8_lossy(bytes).into_owned())
        } else if wordcraft_doc::encoding::SINGLE_BYTE_CODE_PAGES.contains(&codepage) {
            Some(bytes.iter().map(|b| wordcraft_doc::encoding::windows_byte(codepage, *b)).collect())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::filetime_iso;

    /// A `FILETIME` property (VT_FILETIME = 64) as `YYYY-MM-DDTHH:MM:SSZ`. The values are pinned
    /// to what the old hand-rolled `civil` produced.
    fn value(ft: u64) -> Vec<u8> {
        let mut v = 64u32.to_le_bytes().to_vec();
        v.extend_from_slice(&((ft & 0xffff_ffff) as u32).to_le_bytes());
        v.extend_from_slice(&((ft >> 32) as u32).to_le_bytes());
        v
    }

    #[test]
    fn filetime_to_iso() {
        // 2020-01-01T00:00:00Z and the same day at 13:45:09.
        assert_eq!(filetime_iso(&value(132_223_104_000_000_000)).as_deref(), Some("2020-01-01T00:00:00Z"));
        assert_eq!(filetime_iso(&value(132_223_599_090_000_000)).as_deref(), Some("2020-01-01T13:45:09Z"));
        // Year 10000 keeps the plain zero-padded year (no leading `+` that chrono's `%Y` would add).
        assert_eq!(filetime_iso(&value(2_650_467_744_000_000_000)).as_deref(), Some("10000-01-01T00:00:00Z"));
        // The largest FILETIME (u64::MAX) lands in year 60056: past year 9999 (so the plain year
        // matters) and the old `civil` converted it without ever dropping to the epoch. chrono's
        // own range (year 262143) isn't reachable here — a u64 FILETIME caps at unix seconds
        // ~1.83e12 — but the 400-year cycle keeps the result exact regardless.
        assert_eq!(filetime_iso(&value(u64::MAX)).as_deref(), Some("60056-05-28T05:36:10Z"));
        // Zero, a non-FILETIME kind and times before the 1601 epoch are rejected, not a panic.
        assert_eq!(filetime_iso(&value(0)), None);
        assert_eq!(filetime_iso(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(filetime_iso(&value(1)), None);
    }
}
