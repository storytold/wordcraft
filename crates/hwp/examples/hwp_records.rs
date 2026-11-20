//! List the paragraph records of an HWP 5.0 section, for checking what a save wrote:
//! `cargo run -p wordcraft-hwp --example hwp_records -- in.hwp [section|docinfo|raw<section>] [count]`.
//!
//! One line per paragraph header: record number, level, the high bit of the character count,
//! the count, control mask, shape and style ids, record counts and the tags of its children
//! (relative to HWPTAG_BEGIN), and whether it is the last paragraph of its list.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::io::Read;
use std::process::ExitCode;

fn u32_at(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0)
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).unwrap_or(0)
}

/// Largest stream read, and largest it may inflate to (as the crate's own reader allows).
const MAX_STREAM: usize = 256 * 1024 * 1024;

fn stream(c: &mut cfb::CompoundFile<std::fs::File>, name: &str) -> Result<Vec<u8>, String> {
    let s = c.open_stream(name).map_err(|e| format!("{name}: {e}"))?;
    let mut v = Vec::new();
    s.take(MAX_STREAM as u64 + 1).read_to_end(&mut v).map_err(|e| format!("{name}: {e}"))?;
    if v.len() > MAX_STREAM {
        return Err(format!("{name}: larger than {MAX_STREAM} bytes"));
    }
    Ok(v)
}

fn run(path: &str, section: &str, count: usize) -> Result<(), String> {
    let mut c = cfb::open(path).map_err(|e| format!("{path}: {e}"))?;
    let head = stream(&mut c, "/FileHeader")?;
    // "docinfo", a section number, or "raw<n>" for every record of section n
    let raw_mode = section.strip_prefix("raw");
    let name = match (section, raw_mode) {
        ("docinfo", _) => "/DocInfo".to_string(),
        (_, Some(n)) => format!("/BodyText/Section{n}"),
        _ => format!("/BodyText/Section{section}"),
    };
    let raw = stream(&mut c, &name)?;
    let data = if u32_at(&head, 36) & 1 != 0 {
        miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, MAX_STREAM).map_err(|e| format!("inflate: {:?}", e.status))?
    } else {
        raw
    };
    let version: Vec<String> = head.get(32..36).unwrap_or(&[]).iter().rev().map(u8::to_string).collect();
    let mut recs: Vec<(u32, u32, &[u8])> = Vec::new();
    let mut i = 0;
    while i + 4 <= data.len() {
        let h = u32_at(&data, i);
        i += 4;
        let mut size = (h >> 20) as usize;
        if size == 0xFFF {
            size = u32_at(&data, i) as usize;
            i += 4;
        }
        let end = i.saturating_add(size).min(data.len());
        recs.push((h & 0x3FF, (h >> 10) & 0x3FF, data.get(i..end).unwrap_or(&[])));
        i = i.saturating_add(size);
    }
    println!("version {} records {}", version.join("."), recs.len());
    if section == "docinfo" || raw_mode.is_some() {
        // every record: tag (from HWPTAG_BEGIN), level, size and its first bytes
        for (k, (tag, level, d)) in recs.iter().enumerate().take(count) {
            let hex: Vec<String> = d.iter().take(if count <= 5 { usize::MAX } else { 80 }).map(|b| format!("{b:02x}")).collect();
            println!("#{k} T{} L{level} len={} {}", tag.wrapping_sub(0x10), d.len(), hex.join(""));
        }
        return Ok(());
    }
    let mut shown = 0;
    for (k, &(tag, level, d)) in recs.iter().enumerate() {
        if tag != 0x10 + 50 {
            continue;
        }
        let rest = recs.get(k + 1..).unwrap_or(&[]);
        let kids: Vec<u32> = rest.iter().take_while(|r| r.1 > level).filter(|r| r.1 == level + 1).map(|r| r.0.wrapping_sub(0x10)).collect();
        let last = rest.iter().find(|r| r.1 <= level).is_none_or(|r| r.1 < level || r.0 != 0x10 + 50);
        let n = u32_at(d, 0);
        println!(
            "#{k} L{level} hi={} n={} mask={:#x} ps={} st={} br={} cs={} rt={} ls={} len={} last={last} kids={kids:?}",
            n >> 31,
            n & 0x7FFF_FFFF,
            u32_at(d, 4),
            u16_at(d, 8),
            d.get(10).copied().unwrap_or(0),
            d.get(11).copied().unwrap_or(0),
            u16_at(d, 12),
            u16_at(d, 14),
            u16_at(d, 16),
            d.len()
        );
        shown += 1;
        if shown >= count {
            break;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: hwp_records <in.hwp> [section] [count]");
        return ExitCode::FAILURE;
    };
    let section = args.get(1).map(String::as_str).unwrap_or("0");
    let count = args.get(2).and_then(|c| c.parse().ok()).unwrap_or(usize::MAX);
    match run(path, section, count) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
