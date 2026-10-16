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
            if vt == 2 || vt == 3 {
                Some(u16::from_le_bytes(cp.get(4..6)?.try_into().ok()?))
            } else {
                None
            }
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

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    (y, m, d)
}

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
    let unix_secs = unix_100ns / 10_000_000;
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = (unix_secs % 86_400) as u32;
    let (h, mi, s) = (secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60);
    let (y, m, d) = civil(days);
    Some(format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"))
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
