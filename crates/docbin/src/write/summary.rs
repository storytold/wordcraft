//! The `\u{5}SummaryInformation` stream ([MS-OLEPS] §2.21): title, subject, author, keywords,
//! comments, last author, revision and the creation and save times.

use wordcraft_doc::CoreProps;

/// `FMTID_SummaryInformation` as stored (little-endian GUID fields).
const SUMMARY_FMTID: [u8; 16] = [0xe0, 0x85, 0x9f, 0xf2, 0xf9, 0x4f, 0x68, 0x10, 0xab, 0x91, 0x08, 0x00, 0x2b, 0x27, 0xb3, 0xd9];
const VT_I2: u32 = 2;
const VT_LPSTR: u32 = 30;
const VT_FILETIME: u32 = 64;
/// Longest string property written (characters).
const MAX_CHARS: usize = 32 << 10;

/// The text of `s` in code page 1252, when every character has a byte there.
fn cp1252(s: &str) -> Option<Vec<u8>> {
    s.chars().map(|c| (0u8..=255).find(|&b| wordcraft_doc::encoding::cp1252(b) == c && (b >= 0x20 || c == '\t' || c == '\n' || c == '\r'))).collect()
}

/// The stream's bytes. Strings are Windows-1252 when they all fit it, else UTF-16 (code page
/// 1200).
pub(crate) fn stream(core: &CoreProps) -> Vec<u8> {
    let strings: Vec<(u32, String)> =
        [(2u32, &core.title), (3, &core.subject), (4, &core.creator), (5, &core.keywords), (6, &core.description), (8, &core.last_modified_by)]
            .into_iter()
            .filter(|(_, s)| !s.is_empty())
            .map(|(id, s)| (id, s.chars().take(MAX_CHARS).collect()))
            .chain((core.revision > 0).then(|| (9u32, core.revision.to_string())))
            .collect();
    let ansi = strings.iter().all(|(_, s)| cp1252(s).is_some());
    let codepage: u16 = if ansi { 1252 } else { 1200 };

    let mut props: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut cp = VT_I2.to_le_bytes().to_vec();
    cp.extend_from_slice(&codepage.to_le_bytes());
    cp.extend_from_slice(&[0, 0]);
    props.push((1, cp));
    for (id, s) in &strings {
        let mut bytes = if ansi { cp1252(s).unwrap_or_default() } else { s.encode_utf16().flat_map(u16::to_le_bytes).collect() };
        bytes.extend_from_slice(if ansi { &[0][..] } else { &[0, 0][..] });
        let mut v = VT_LPSTR.to_le_bytes().to_vec();
        v.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        v.extend_from_slice(&bytes);
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
        props.push((*id, v));
    }
    for (id, iso) in [(12u32, &core.created), (13, &core.modified)] {
        if let Some(ft) = filetime(iso) {
            let mut v = VT_FILETIME.to_le_bytes().to_vec();
            v.extend_from_slice(&(ft as u32).to_le_bytes());
            v.extend_from_slice(&((ft >> 32) as u32).to_le_bytes());
            props.push((id, v));
        }
    }

    // PropertySet: size, count, (id, offset) pairs, then the values.
    let table = 8 + props.len() * 8;
    let mut set = Vec::new();
    let mut values = Vec::new();
    let mut index = Vec::new();
    for (id, v) in &props {
        index.extend_from_slice(&id.to_le_bytes());
        index.extend_from_slice(&((table + values.len()) as u32).to_le_bytes());
        values.extend_from_slice(v);
    }
    set.extend_from_slice(&((table + values.len()) as u32).to_le_bytes());
    set.extend_from_slice(&(props.len() as u32).to_le_bytes());
    set.extend_from_slice(&index);
    set.extend_from_slice(&values);

    // PropertySetStream header: byte order, version, system id, CLSID, one set, its FMTID and
    // offset.
    let mut out = vec![0xFE, 0xFF, 0, 0];
    out.extend_from_slice(&0x0002_0006u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&SUMMARY_FMTID);
    out.extend_from_slice(&48u32.to_le_bytes());
    out.extend_from_slice(&set);
    out
}

/// A FILETIME (100 ns since 1601) from an ISO 8601 date.
fn filetime(iso: &str) -> Option<u64> {
    let t = super::parse_time(iso)?.and_utc().timestamp();
    let ft = (t.checked_add(11_644_473_600)?).checked_mul(10_000_000)?;
    u64::try_from(ft).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn summary_reads_back() {
        let core = wordcraft_doc::CoreProps {
            title: "Annual Report".into(),
            creator: "Zoë Writer".into(),
            keywords: "a, b".into(),
            revision: 3,
            created: "2024-05-06T07:08:09Z".into(),
            ..Default::default()
        };
        let read = crate::metadata::parse_for_tests(&super::stream(&core)).unwrap();
        assert_eq!(read.title, "Annual Report");
        assert_eq!(read.creator, "Zoë Writer");
        assert_eq!(read.keywords, "a, b");
        assert_eq!(read.revision, 3);
        assert_eq!(read.created, "2024-05-06T07:08:09Z");
        let unicode = wordcraft_doc::CoreProps { title: "Отчёт 报告".into(), ..Default::default() };
        assert_eq!(crate::metadata::parse_for_tests(&super::stream(&unicode)).unwrap().title, "Отчёт 报告");
    }
}
