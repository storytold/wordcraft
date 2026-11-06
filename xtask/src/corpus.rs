//! `cargo xtask corpus [--download]`: where the test corpora live, and fetching PngSuite.
//!
//! The PngSuite archive is fetched over HTTP (the host serves no TLS) and extracted only after
//! its SHA-256 matches the digest pinned below, so a tampered or truncated download is refused
//! rather than unpacked (#1357). Extraction uses a hand-rolled reader, not `tar`: `tar -xzf`
//! honors `../`, absolute paths and links in a hostile archive, while this one checks every
//! member name and writes only regular files inside the corpus directory.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;

/// The canonical 2017jul19 PngSuite release. The host serves no HTTPS, so the archive is
/// authenticated by its digest instead of by the transport.
const PNGSUITE_URL: &str = "http://www.schaik.com/pngsuite/PngSuite-2017jul19.tgz";
/// SHA-256 of `PngSuite-2017jul19.tgz` as published (68,521 bytes).
const PNGSUITE_SHA256: &str = "0294b244c95a8342c01b00010cf34abdcabc7c6a34fd0fe1bd963917537bfdc8";
/// PngSuite unpacks to a few hundred KB; anything near this is not the archive we asked for.
const MAX_UNPACKED: u64 = 64 * 1024 * 1024;

pub fn run(root: &Path, download: bool) -> Result<(), String> {
    let corpus = root.join("corpus");
    println!(
        "Test corpora live under {} (git-ignored, never committed).
Tests that use a corpus skip cleanly when it is absent.

  corpus/pngsuite/   PngSuite (public domain) — wordcraft-codecs compares every file
                     against the `image` crate. Fetch: cargo xtask corpus --download
                     (checked against the archive's published SHA-256 before unpacking)
  corpus/psd/        PSD samples from MIT/BSD projects (ag-psd, psd-tools test data).
                     Copy files in manually; licences must be MIT/BSD/CC0.
  corpus/tiff/       libtiff pics (optional)
  corpus/exr/        OpenEXR sample images (optional)
  corpus/raw/        raw.pixls.us samples, CC0 (optional)
",
        corpus.display()
    );
    if !download {
        return Ok(());
    }
    std::fs::create_dir_all(&corpus).map_err(|e| format!("create {}: {e}", corpus.display()))?;
    let tgz = corpus.join("PngSuite-2017jul19.tgz");
    let bytes = download_and_verify(&tgz)?;
    let _ = std::fs::remove_file(&tgz); // verified; the archive itself is not kept
    let tar = gunzip(&bytes)?;
    let dest = corpus.join("pngsuite");
    let pngs = extract(&tar, &dest)?;
    if pngs == 0 {
        return Err(format!("no PNG files extracted into {}", dest.display()));
    }
    println!("PngSuite: {pngs} PNG files in {}", dest.display());
    Ok(())
}

/// Fetch PngSuite to `path` and return it only when its SHA-256 matches [`PNGSUITE_SHA256`].
fn download_and_verify(path: &Path) -> Result<Vec<u8>, String> {
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "-o"]).arg(path).arg(PNGSUITE_URL);
    crate::run(curl, &format!("curl {PNGSUITE_URL}"))?;
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let got = hex(&sha256(&bytes));
    if got != PNGSUITE_SHA256 {
        let _ = std::fs::remove_file(path);
        return Err(format!("PngSuite archive digest mismatch: expected {PNGSUITE_SHA256}, got {got}; refused, nothing extracted"));
    }
    Ok(bytes)
}

/// Write the regular files of the tar stream `tar` into `dir`; how many of them are PNG files.
/// A member that would escape `dir`, or a non-regular member, is refused.
fn extract(tar: &[u8], dir: &Path) -> Result<usize, String> {
    let entries = parse_tar(tar)?;
    if entries.is_empty() {
        return Err("the archive held no regular files".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let mut pngs = 0;
    for e in &entries {
        let out = dir.join(&e.name);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
        let data = tar.get(e.start..e.end()).ok_or("tar member out of range")?;
        std::fs::write(&out, data).map_err(|err| format!("write {}: {err}", out.display()))?;
        if e.name.extension().is_some_and(|x| x == "png") {
            pngs += 1;
        }
    }
    Ok(pngs)
}

/// One regular-file member of the archive: its safe relative path and where its bytes are in the
/// decompressed stream.
struct Entry {
    name: PathBuf,
    start: usize,
    len: usize,
}

impl Entry {
    fn end(&self) -> usize {
        self.start + self.len
    }
}

/// The regular-file members of a tar stream, in order. Only regular files are returned —
/// directories, links and GNU long-name headers are skipped — and each name is checked to be a
/// relative path that stays inside the archive.
fn parse_tar(tar: &[u8]) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut long_name: Option<String> = None;
    while let Some(header) = tar.get(at..at + 512) {
        if header.iter().all(|b| *b == 0) {
            break; // end-of-archive marker, and any zero padding after it
        }
        let mut name = tar_str(&header[0..100]);
        let prefix = tar_str(&header[345..500]);
        if !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
        let size = tar_octal(&header[124..136])?;
        let typeflag = header[156];
        let start = at + 512;
        let end = start.checked_add(size).ok_or("bad tar member size")?;
        let data = tar.get(start..end).ok_or("truncated tar member")?;
        at = start.checked_add(round_up_512(size).ok_or("bad tar member size")?).ok_or("bad tar member size")?;
        if typeflag == b'L' {
            // A GNU long name: this member's data is the next member's name.
            long_name = Some(tar_str(data));
            continue;
        }
        if let Some(n) = long_name.take() {
            name = n;
        }
        if typeflag != b'0' && typeflag != 0 {
            continue;
        }
        out.push(Entry { name: safe_member_path(&name)?, start, len: size });
    }
    Ok(out)
}

/// A tar member name turned into a path that stays inside the archive: relative, no `..`, no
/// backslash, no colon, nothing empty.
fn safe_member_path(name: &str) -> Result<PathBuf, String> {
    let bad = || Err(format!("refusing unsafe archive member {name:?}"));
    if name.is_empty() || name.starts_with('/') || name.contains('\\') || name.contains('\0') {
        return bad();
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return bad();
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        return bad();
    }
    Ok(out)
}

/// The text in a tar header field, up to its NUL or space padding.
fn tar_str(field: &[u8]) -> String {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).trim().to_string()
}

/// A tar numeric field (`size`), octal ASCII, or the GNU base-256 form when the top bit of the
/// first byte is set.
fn tar_octal(field: &[u8]) -> Result<usize, String> {
    if field.first().is_some_and(|b| *b & 0x80 != 0) {
        let mut v = (field.first().copied().unwrap_or(0) & 0x7f) as usize;
        for b in field.iter().skip(1) {
            v = v.wrapping_mul(256).wrapping_add(*b as usize);
        }
        return Ok(v);
    }
    let text = String::from_utf8_lossy(field);
    let text = text.trim_matches(|c: char| c == '\0' || c == ' ');
    if text.is_empty() {
        return Ok(0);
    }
    usize::from_str_radix(text, 8).map_err(|_| format!("bad tar size field {text:?}"))
}

/// `n` rounded up to the next 512-byte tar block boundary.
fn round_up_512(n: usize) -> Option<usize> {
    n.div_ceil(512).checked_mul(512)
}

/// Decompress a gzip stream, refusing one that unpacks far larger than PngSuite ever could.
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    GzDecoder::new(bytes).take(MAX_UNPACKED + 1).read_to_end(&mut out).map_err(|e| format!("gunzip: {e}"))?;
    if out.len() as u64 > MAX_UNPACKED {
        return Err("the archive unpacks far larger than expected".into());
    }
    Ok(out)
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write as _;

    #[test]
    fn hashes_match_the_known_sha256_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn member_paths_stay_inside_the_archive() {
        assert_eq!(safe_member_path("basi0g01.png").unwrap(), PathBuf::from("basi0g01.png"));
        assert_eq!(safe_member_path("./a/b.png").unwrap(), PathBuf::from("a/b.png"));
        for bad in ["", "/etc/passwd", "../escape.png", "a/../../b.png", "..", "C:evil.png", "a\\b.png", "..\\x", "a/./../b", "/"] {
            assert!(safe_member_path(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn reads_the_regular_files_of_a_tar() {
        let mut tar = tar_header("basi0g01.png", b"png-bytes", b'0');
        tar.extend_from_slice(&tar_header("PngSuite.README", b"readme", b'0'));
        tar.extend_from_slice(&[0u8; 1024]);
        let entries = parse_tar(&tar).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, PathBuf::from("basi0g01.png"));
        assert_eq!(&tar[entries[0].start..entries[0].end()], b"png-bytes");
        assert_eq!(entries[1].name, PathBuf::from("PngSuite.README"));
        assert_eq!(&tar[entries[1].start..entries[1].end()], b"readme");
    }

    #[test]
    fn refuses_a_traversing_member() {
        let mut tar = tar_header("../escape.png", b"x", b'0');
        tar.extend_from_slice(&[0u8; 1024]);
        assert!(parse_tar(&tar).is_err());
    }

    #[test]
    fn skips_links_and_directories() {
        let mut tar = tar_header("basi0g01.png", b"real", b'0');
        tar.extend_from_slice(&tar_header("evil", b"", b'2')); // symlink
        tar.extend_from_slice(&tar_header("sub/", b"", b'5')); // directory
        tar.extend_from_slice(&[0u8; 1024]);
        let entries = parse_tar(&tar).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, PathBuf::from("basi0g01.png"));
    }

    #[test]
    fn takes_a_gnu_long_name() {
        let long = "a".repeat(120) + ".png";
        let mut tar = tar_header("././@LongLink", long.as_bytes(), b'L');
        tar.extend_from_slice(&tar_header("truncated", b"body", b'0'));
        tar.extend_from_slice(&[0u8; 1024]);
        let entries = parse_tar(&tar).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, PathBuf::from(&long));
        assert_eq!(&tar[entries[0].start..entries[0].end()], b"body");
    }

    #[test]
    fn gunzips_a_gzip_stream() {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"hello corpus").unwrap();
        let gz = enc.finish().unwrap();
        assert_eq!(gunzip(&gz).unwrap(), b"hello corpus");
    }

    #[test]
    fn extracts_the_regular_files_into_a_directory() {
        let mut tar = tar_header("basi0g01.png", b"1", b'0');
        tar.extend_from_slice(&tar_header("sub/basn0g01.png", b"22", b'0'));
        tar.extend_from_slice(&tar_header("evil", b"", b'2')); // symlink, skipped
        tar.extend_from_slice(&[0u8; 1024]);
        let dir = std::env::temp_dir().join(format!("wordcraft-corpus-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let pngs = extract(&tar, &dir).unwrap();
        assert_eq!(pngs, 2);
        assert_eq!(std::fs::read(dir.join("basi0g01.png")).unwrap(), b"1");
        assert_eq!(std::fs::read(dir.join("sub/basn0g01.png")).unwrap(), b"22");
        assert!(!dir.join("evil").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_traversing_archive_writes_nothing() {
        let mut tar = tar_header("../../escape.png", b"x", b'0');
        tar.extend_from_slice(&[0u8; 1024]);
        let dir = std::env::temp_dir().join(format!("wordcraft-corpus-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(extract(&tar, &dir).is_err());
        assert!(!dir.exists(), "nothing is written before a member is refused");
    }

    /// A 512-byte tar header for `name`, followed by `data` padded to a 512-byte block.
    fn tar_header(name: &str, data: &[u8], typeflag: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000644\0");
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[148..156].copy_from_slice(b"        "); // checksum field, spaces while summing
        h[156] = typeflag;
        h[257..263].copy_from_slice(b"ustar\0");
        let sum: u32 = h.iter().map(|b| *b as u32).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        h.extend_from_slice(data);
        h.resize(h.len().div_ceil(512) * 512, 0);
        h
    }
}
