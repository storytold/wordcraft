use super::*;

const FAMILY: &str = "Userfont Sans";

/// A bundled Source Sans 3 file renamed `family` (as long as the original name).
pub(crate) fn renamed(file: &str, family: &str) -> Vec<u8> {
    assert_eq!(family.len(), "Source Sans 3".len());
    let mut data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(family)), (b"Source Sans 3".to_vec(), family.as_bytes().to_vec())] {
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

fn db() -> FontDb {
    FontDb::with_font_dirs(vec![])
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wc-userfonts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The tables of a TrueType/OpenType font: (tag, data), in directory order.
fn tables(font: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let n = u16::from_be_bytes([font[4], font[5]]) as usize;
    (0..n)
        .map(|i| {
            let r = &font[12 + i * 16..28 + i * 16];
            let off = u32::from_be_bytes(r[8..12].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(r[12..16].try_into().unwrap()) as usize;
            (r[..4].try_into().unwrap(), font[off..off + len].to_vec())
        })
        .collect()
}

/// `font` as a WOFF 1.0 file with every table stored uncompressed (the format allows it).
fn woff1(font: &[u8]) -> Vec<u8> {
    let mut t = tables(font);
    t.sort_by_key(|x| x.0);
    let mut out = Vec::new();
    let dir_end = 44 + 20 * t.len();
    let mut body = Vec::new();
    let mut dir = Vec::new();
    let mut sfnt_size = 12 + 16 * t.len();
    for (tag, data) in &t {
        let off = dir_end + body.len();
        dir.extend_from_slice(tag);
        dir.extend_from_slice(&(off as u32).to_be_bytes());
        dir.extend_from_slice(&(data.len() as u32).to_be_bytes());
        dir.extend_from_slice(&(data.len() as u32).to_be_bytes());
        dir.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(data);
        body.resize(body.len().next_multiple_of(4), 0);
        sfnt_size += data.len().next_multiple_of(4);
    }
    out.extend_from_slice(b"wOFF");
    out.extend_from_slice(&font[..4]);
    out.extend_from_slice(&((dir_end + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&(t.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(sfnt_size as u32).to_be_bytes());
    out.extend_from_slice(&[0, 1, 0, 0]); // version 1.0
    out.extend_from_slice(&[0; 20]); // no metadata, no private data
    out.extend(dir);
    out.extend(body);
    out
}

/// `data` as a brotli stream of uncompressed meta-blocks (RFC 7932 §9.2).
fn brotli_stored(data: &[u8]) -> Vec<u8> {
    struct Bits {
        out: Vec<u8>,
        acc: u64,
        n: u32,
    }
    impl Bits {
        fn put(&mut self, v: u64, bits: u32) {
            self.acc |= v << self.n;
            self.n += bits;
            while self.n >= 8 {
                self.out.push(self.acc as u8);
                self.acc >>= 8;
                self.n -= 8;
            }
        }
        fn align(&mut self) {
            if self.n > 0 {
                self.put(0, 8 - self.n);
            }
        }
    }
    let mut b = Bits { out: Vec::new(), acc: 0, n: 0 };
    b.put(0, 1); // WBITS = 16
    for chunk in data.chunks(1 << 16) {
        b.put(0, 1); // ISLAST = 0
        b.put(0, 2); // MNIBBLES = 4
        b.put(chunk.len() as u64 - 1, 16);
        b.put(1, 1); // ISUNCOMPRESSED
        b.align();
        b.out.extend_from_slice(chunk);
    }
    b.put(1, 1); // ISLAST
    b.put(1, 1); // ISLASTEMPTY
    b.align();
    b.out
}

/// WOFF2's UIntBase128.
fn base128(mut v: u32, out: &mut Vec<u8>) {
    let mut bytes = vec![(v & 0x7f) as u8];
    v >>= 7;
    while v > 0 {
        bytes.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(bytes.into_iter().rev());
}

/// `font` (TrueType) as a WOFF2 file with no table transformed (glyf/loca use the null
/// transform, version 3) and the table data in stored brotli blocks.
fn woff2(font: &[u8]) -> Vec<u8> {
    let t = tables(font);
    let mut dir = Vec::new();
    let mut stream = Vec::new();
    for (tag, data) in &t {
        let null = if tag == b"glyf" || tag == b"loca" { 3 << 6 } else { 0 };
        dir.push(63 | null);
        dir.extend_from_slice(tag);
        base128(data.len() as u32, &mut dir);
        stream.extend_from_slice(data);
    }
    let sfnt: usize = 12 + 16 * t.len() + t.iter().map(|(_, d)| d.len().next_multiple_of(4)).sum::<usize>();
    woff2_file(&font[..4], t.len(), &dir, &stream, sfnt)
}

/// A WOFF2 file: header, the table directory `dir` of `n` tables, `stream` in stored brotli blocks.
fn woff2_file(flavor: &[u8], n: usize, dir: &[u8], stream: &[u8], sfnt: usize) -> Vec<u8> {
    let compressed = brotli_stored(stream);
    let mut out = Vec::new();
    out.extend_from_slice(b"wOF2");
    out.extend_from_slice(flavor);
    let total = (48 + dir.len() + compressed.len()).next_multiple_of(4);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(sfnt as u32).to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0, 1, 0, 0]);
    out.extend_from_slice(&[0; 20]);
    out.extend_from_slice(dir);
    out.extend(compressed);
    out.resize(total, 0);
    out
}

/// A WOFF2 file holding a transformed `glyf` (and `loca`) whose one glyph has two contours of
/// `points` points each (and no coordinates: the decoder finds it damaged in the end).
fn woff2_glyph_with(points: u16) -> Vec<u8> {
    let [hi, lo] = points.to_be_bytes();
    woff2_glyph_with_contours(2, &[253, hi, lo].repeat(2), 2 * points as usize)
}

/// As [`woff2_glyph_with`], for a glyph whose contour count is `n` and whose point-count stream
/// is `counts`, with `total` flags.
fn woff2_glyph_with_contours(n: u16, counts: &[u8], total: usize) -> Vec<u8> {
    let mut glyf = vec![0, 0, 0, 0, 0, 1, 0, 0]; // reserved, optionFlags, numGlyphs 1, indexFormat
    // nContour, nPoints, flag, glyph, composite, bbox and instruction stream sizes.
    for size in [2, counts.len(), total, 0, 0, 4, 0] {
        glyf.extend_from_slice(&(size as u32).to_be_bytes());
    }
    glyf.extend_from_slice(&n.to_be_bytes());
    glyf.extend_from_slice(counts); // 255UInt16 point counts
    glyf.resize(glyf.len() + total, 1); // on-curve flags
    glyf.extend_from_slice(&[0; 4]); // no explicit bounding boxes
    let mut dir = vec![10]; // glyf, transform version 0
    base128(4096, &mut dir);
    base128(glyf.len() as u32, &mut dir);
    dir.push(11); // loca, transformed away
    base128(8, &mut dir);
    base128(0, &mut dir);
    woff2_file(&[0, 1, 0, 0], 2, &dir, &glyf, 12 + 32 + 4096 + 8)
}

/// A WOFF 1.0 file whose one table is a zlib stream of nothing that claims to inflate to `orig`
/// bytes.
fn woff1_claiming(orig: u32) -> Vec<u8> {
    let mut out = b"wOFF\0\x01\0\0".to_vec();
    out.extend_from_slice(&72u32.to_be_bytes()); // length
    out.extend_from_slice(&1u16.to_be_bytes()); // numTables
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&orig.saturating_add(28).to_be_bytes()); // totalSfntSize
    out.extend_from_slice(&[0, 1, 0, 0]);
    out.extend_from_slice(&[0; 20]);
    out.extend_from_slice(b"glyf");
    for v in [64, 8, orig, 0] {
        out.extend_from_slice(&u32::to_be_bytes(v)); // offset, compLength, origLength, checksum
    }
    out.extend_from_slice(&[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    out
}

#[test]
fn a_woff_claiming_a_huge_table_is_refused_before_inflating() {
    // #3703: the decoder sizes a table's buffer from the directory before inflating it.
    for orig in [200 << 20, u32::MAX] {
        assert!(matches!(decode_font_file(&woff1_claiming(orig)), Err(UserFontError::TooLarge { .. })), "{orig}");
    }
    // A small claim reaches the decoder, which finds the data doesn't inflate to it.
    assert!(matches!(decode_font_file(&woff1_claiming(64)), Err(UserFontError::BadWebFont(_))));
}

#[test]
fn a_woff2_glyph_with_too_many_points_is_refused_before_unpacking() {
    // #3703: the decoder allocates a glyph's points from these counts before checking them.
    assert_eq!(decode_font_file(&woff2_glyph_with(40_000)), Err(UserFontError::BadWebFont(TOO_MANY_POINTS.into())));
    // Within the limit the scan passes the file on, to the decoder.
    assert!(matches!(decode_font_file(&woff2_glyph_with(3)), Err(UserFontError::BadWebFont(e)) if e != TOO_MANY_POINTS));
}

#[test]
fn a_woff2_glyph_with_a_contour_count_past_i16_max_is_counted() {
    // #3703: the decoder reads the contour count as unsigned (only 0xFFFF is a composite), so
    // 0x8000..=0xFFFE are that many contours; read as negative they counted no points and the
    // decoder then allocated about 1 GB for a 1.3 MB font.
    for n in [0x7FFFu16, 0x8000, 0xFFFE] {
        let file = woff2_glyph_with_contours(n, &vec![3; usize::from(n)], 3 * usize::from(n));
        assert_eq!(decode_font_file(&file), Err(UserFontError::BadWebFont(TOO_MANY_POINTS.into())), "{n:#x}");
    }
}

#[test]
fn a_woff2_composite_glyph_is_not_counted_as_having_points() {
    // 0xFFFF marks a composite glyph: the point check skips it (the decoder reads its own
    // streams), so it is not refused for its points.
    let file = woff2_glyph_with_contours(0xFFFF, &[], 0);
    assert!(matches!(decode_font_file(&file), Err(UserFontError::BadWebFont(e)) if e != TOO_MANY_POINTS));
}

#[test]
fn an_added_font_is_listed_resolved_and_measured() {
    let db = db();
    let gen0 = db.generation();
    assert!(!db.has_family(FAMILY));
    let r = db.install_font("Userfont-Regular.ttf", &renamed("SourceSans3-Regular.ttf", FAMILY)).unwrap();
    assert_eq!(r.font.families(), vec![FAMILY.to_string()]);
    assert_eq!(r.font.file, "Userfont-Regular.ttf");
    assert!(r.font.path.is_none() && r.warning.is_none(), "no folder: kept for the session only");
    assert!(db.generation() > gen0);
    assert!(db.families().iter().any(|f| f == FAMILY));
    let face = db.face(FAMILY, "Regular");
    assert_eq!(face.family, FAMILY);
    // Measured with its own glyphs: the copy has Source Sans 3's metrics.
    let src = db.face("Source Sans 3", "Regular");
    let g = face.glyph_for('W');
    assert!(g != 0);
    assert_eq!(face.advance(g), src.advance(src.glyph_for('W')));
    assert_eq!(db.user_fonts().len(), 1);
}

#[test]
fn the_bundled_baseline_is_never_replaced() {
    let db = db();
    let before = db.face("Source Sans 3", "Regular").id();
    let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    assert!(matches!(db.install_font("x.ttf", &bytes), Err(UserFontError::AlreadyAvailable(_))));
    assert_eq!(db.face("Source Sans 3", "Regular").id(), before);
    assert!(db.user_fonts().is_empty());
    assert!(matches!(db.remove_user_font("Source Sans 3"), Err(UserFontError::NotUserFont(_))));
    assert!(db.has_family("Source Sans 3"));
}

#[test]
fn junk_is_rejected_without_adding_anything() {
    let db = db();
    let n = db.families().len();
    let regular = renamed("SourceSans3-Regular.ttf", FAMILY);
    let mut cases: Vec<Vec<u8>> = vec![
        vec![],
        b"not a font at all".to_vec(),
        b"\0\x01\0\0".to_vec(),
        b"\0\x01\0\0\xff\xff\0\0\0\0\0\0".to_vec(),
        b"ttcf\0\x01\0\0\xff\xff\xff\xff".to_vec(),
        b"OTTO\0\x01".to_vec(),
        b"wOFF\0\x01\0\0\0\0\0\x08".to_vec(),
        b"wOF2\0\x01\0\0\xff\xff\xff\xff\0\x05".to_vec(),
        regular[..regular.len() / 3].to_vec(),
    ];
    // Damaged web fonts: every prefix, and flipped bytes through the headers and directory.
    for packed in [woff1(&regular), woff2(&regular)] {
        for cut in [8, 20, 44, 47, 48, 60, 100, 300, packed.len() / 2, packed.len() - 1] {
            cases.push(packed[..cut.min(packed.len())].to_vec());
        }
        for at in 4..200 {
            let mut b = packed.clone();
            b[at] ^= 0xA5;
            cases.push(b);
        }
    }
    for (i, c) in cases.iter().enumerate() {
        let r = db.install_font("junk.ttf", c);
        // A flipped byte inside a table can leave a usable font; the cut-off files can't.
        assert!(i >= 9 || r.is_err(), "case {i}");
    }
    assert_eq!(db.families().len(), n + db.user_fonts().iter().map(|u| u.families().len()).sum::<usize>());
    let mut big = regular.clone();
    big.resize(MAX_FONT_FILE_BYTES + 1, 0);
    assert!(matches!(db.install_font("big.ttf", &big), Err(UserFontError::TooLarge { .. })));
}

#[test]
fn woff_and_woff2_are_unwrapped() {
    let regular = renamed("SourceSans3-Regular.ttf", FAMILY);
    for (name, packed) in [("a.woff", woff1(&regular)), ("a.woff2", woff2(&regular))] {
        assert!(is_font_file_name(name));
        let db = db();
        let r = db.install_font(name, &packed).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(r.font.file, "a.ttf", "stored unwrapped");
        let face = db.face(FAMILY, "Regular");
        assert_eq!(face.family, FAMILY);
        assert!(face.glyph_for('g') != 0);
    }
}

#[test]
fn user_fonts_are_stored_loaded_and_removed() {
    let dir = temp_dir("store");
    let db = db();
    let (loaded, skipped) = db.load_user_fonts(&dir);
    assert!(loaded.is_empty() && skipped.is_empty(), "a missing folder is empty");
    let r = db.install_font("../../evil/Userfont Bold?.woff2", &woff2(&renamed("SourceSans3-Bold.ttf", FAMILY))).unwrap();
    assert!(r.warning.is_none());
    let path = r.font.path.clone().unwrap();
    assert_eq!(path.parent(), Some(dir.as_path()), "a file name can't leave the folder");
    assert_eq!(r.font.file, "Userfont Bold_.ttf");
    assert!(path.exists());
    db.install_font("Userfont-Regular.ttf", &renamed("SourceSans3-Regular.ttf", FAMILY)).unwrap();
    // A damaged file in the folder is skipped, never fatal.
    std::fs::write(dir.join("damaged.ttf"), b"ttcf\0\x01\0\0\xff\xff\xff\xff").unwrap();
    std::fs::write(dir.join("notes.txt"), b"hello").unwrap();

    // The next start loads both from the folder.
    let next = super::super::FontDb::with_font_dirs(vec![]);
    let (loaded, skipped) = next.load_user_fonts(&dir);
    assert_eq!(loaded.len(), 2, "{skipped:?}");
    assert_eq!(skipped.len(), 1);
    assert_eq!(next.styles(FAMILY), vec!["Regular".to_string(), "Bold".to_string()]);
    assert_eq!(next.face(FAMILY, "Bold").style, "Bold");

    // Removing the family deletes both files; text in it falls back.
    let gone = next.remove_user_font(FAMILY).unwrap();
    assert_eq!(gone.len(), 2);
    assert!(!next.has_family(FAMILY));
    assert_eq!(next.face(FAMILY, "Regular").family, super::super::FALLBACK_FAMILY);
    assert!(!path.exists());
    assert!(next.user_fonts().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The messages logged while this test binary runs.
fn logged() -> &'static std::sync::Mutex<Vec<String>> {
    static LOG: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, r: &log::Record) {
            if let Some(l) = LOG.get() {
                l.lock().unwrap().push(r.args().to_string());
            }
        }
        fn flush(&self) {}
    }
    LOG.get_or_init(|| {
        let _ = log::set_logger(&Capture);
        log::set_max_level(log::LevelFilter::Warn);
        std::sync::Mutex::new(Vec::new())
    })
}

#[test]
fn a_supplied_name_is_logged_escaped() {
    // #3692: a line break in the name must not start a forged record in the log file.
    let log = logged();
    let not_a_folder = temp_dir("not-a-folder");
    std::fs::write(&not_a_folder, b"x").unwrap();
    let db = db();
    db.load_user_fonts(&not_a_folder);
    let r = db.install_font("a\nERROR forged record.ttf", &renamed("SourceSans3-Regular.ttf", FAMILY)).unwrap();
    assert!(r.warning.is_some(), "the font can't be stored");
    let lines = log.lock().unwrap();
    let line = lines.iter().find(|l| l.contains("forged record")).unwrap();
    assert!(!line.contains('\n'), "{line}");
    let _ = std::fs::remove_file(&not_a_folder);
}

#[test]
fn stored_names_are_safe_and_unique() {
    let ttf = renamed("SourceSans3-Regular.ttf", FAMILY);
    assert_eq!(stored_name("C:\\x\\..\\a.b.TTF", &ttf), "a_b.ttf");
    assert_eq!(stored_name("", &ttf), "font.ttf");
    assert_eq!(stored_name("-rf.otf", b"OTTO"), "font-rf.otf");
    assert_eq!(unique_name("a.ttf", |n| n == "a.ttf" || n == "a-2.ttf"), "a-3.ttf");
}

#[cfg(unix)]
#[test]
fn a_link_planted_at_the_temporary_name_is_not_written_through() {
    // #3703 review: the temporary file is created afresh, so a planted link can't carry the
    // font bytes to a file outside the folder.
    let root = std::env::temp_dir().join(format!("wordcraft-font-part-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("fonts");
    std::fs::create_dir_all(&dir).unwrap();
    let outside = root.join("outside.txt");
    std::fs::write(&outside, b"keep me").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join(".a.ttf.part")).unwrap();
    let path = write_unique(&dir, "a.ttf", b"font bytes").unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep me");
    assert_eq!(path, dir.join("a.ttf"));
    assert_eq!(std::fs::read(&path).unwrap(), b"font bytes");
    assert!(!std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert!(std::fs::symlink_metadata(dir.join(".a.ttf.part")).is_err(), "the temporary name is gone");
    // A leftover from a crash doesn't block the next add either.
    std::fs::write(dir.join(".b.ttf.part"), b"half a font").unwrap();
    assert_eq!(std::fs::read(write_unique(&dir, "b.ttf", b"whole").unwrap()).unwrap(), b"whole");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fonts_stored_at_once_under_one_name_each_get_their_own_file() {
    // #3703 review: concurrent adds of one name never share a temporary or a destination.
    let dir = std::env::temp_dir().join(format!("wordcraft-font-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths: Vec<_> = (0..8u8)
        .map(|i| {
            let dir = dir.clone();
            std::thread::spawn(move || (i, write_unique(&dir, "a.ttf", &[i; 64]).unwrap()))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    let distinct: std::collections::HashSet<_> = paths.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(distinct.len(), 8);
    for (i, p) in &paths {
        assert_eq!(std::fs::read(p).unwrap(), vec![*i; 64], "{}", p.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
