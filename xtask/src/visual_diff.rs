//! `cargo xtask visual-diff REF.pdf TEST.pdf OUTDIR [--private] [--dpi N]`: page-by-page
//! comparison of two PDFs of the same document — a reference the user exported themselves (never
//! committed: files produced by other word processors stay out of the repo) and WordCraft's export.
//!
//! Per page: SSIM of the greyscale raster (8×8 windows), share of pixels differing by more than 64
//! levels, lines in each, share of the reference's lines found with the same text in WordCraft
//! (same line break), median |Δy| and max |Δx| of the matched lines, whether the page starts with
//! the same line. Writes `OUTDIR/result.json` and, unless `--private`, `diff_pNNN.ppm` maps
//! (reference red, WordCraft blue, shared ink black). `--private` writes numbers and line hashes
//! only, for confidential documents.
//!
//! Needs poppler's `pdftoppm` and `pdftotext` on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

struct Gray {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

/// One text line: page (0-based), top y, left x, text.
struct Line {
    page: usize,
    y: f64,
    x: f64,
    text: String,
}

/// Reject unknown options before creating output; a typo in --private must not expose text.
fn parse_args<'a>(args: &[&'a str]) -> Result<([&'a str; 3], u32, bool), String> {
    let mut private = false;
    let mut dpi = 100;
    let mut pos = Vec::new();
    let mut args = args.iter().copied();
    while let Some(arg) = args.next() {
        match arg {
            "--private" => private = true,
            "--dpi" => {
                dpi = args.next().and_then(|v| v.parse::<u32>().ok()).filter(|d| (36..=300).contains(d)).ok_or("--dpi takes 36..=300")?;
            }
            "--" => {
                pos.extend(args);
                break;
            }
            flag if flag.starts_with('-') => return Err(format!("unknown visual-diff option: {flag}")),
            path => pos.push(path),
        }
    }
    let [reference, test, out] = pos.as_slice() else {
        return Err("usage: cargo xtask visual-diff REF.pdf TEST.pdf OUTDIR [--private] [--dpi N]".into());
    };
    Ok(([*reference, *test, *out], dpi, private))
}

pub fn run(args: &[&str]) -> Result<(), String> {
    let ([reference, test, out], dpi, private) = parse_args(args)?;
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let tmp = out.join(".visual-diff-tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let result = compare(Path::new(reference), Path::new(test), &out, &tmp, dpi, private);
    let _ = std::fs::remove_dir_all(&tmp);
    let report = result?;
    std::fs::write(out.join("result.json"), serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    println!("{}", report["summary"]);
    Ok(())
}

fn compare(reference: &Path, test: &Path, out: &Path, tmp: &Path, dpi: u32, private: bool) -> Result<Value, String> {
    let rp = rasterize(reference, dpi, tmp, "r")?;
    let tp = rasterize(test, dpi, tmp, "t")?;
    let l1 = lines_of(reference)?;
    let l2 = lines_of(test)?;
    let t1: Vec<String> = l1.iter().map(|l| norm(&l.text)).collect();
    let t2: Vec<String> = l2.iter().map(|l| norm(&l.text)).collect();
    let pairs = lcs(&t1, &t2);
    let mut matched = vec![None; l1.len()];
    for &(a, b) in &pairs {
        matched[a] = Some(b);
    }
    let label = |t: &str| if private { hash(t) } else { t.chars().take(70).collect() };
    let mut pages = Vec::new();
    let mut dx_diffs = Vec::new();
    let mut ssims = Vec::new();
    for p in 0..rp.len().max(tp.len()) {
        let mut row = json!({"page": p + 1});
        if let (Some(a), Some(b)) = (rp.get(p), tp.get(p)) {
            let (ga, gb) = (read_pgm(a)?, read_pgm(b)?);
            let s = ssim(&ga, &gb);
            ssims.push(s);
            row["ssim"] = json!(round(s, 4));
            row["diff_px"] = json!(round(diff_share(&ga, &gb) * 100.0, 2));
            if !private {
                write_diff_map(&ga, &gb, &out.join(format!("diff_p{:03}.ppm", p + 1)))?;
            }
        }
        let ref_ix: Vec<usize> = (0..l1.len()).filter(|&i| l1[i].page == p).collect();
        row["lines"] = json!([ref_ix.len(), l2.iter().filter(|l| l.page == p).count()]);
        let same: Vec<(usize, usize)> = ref_ix.iter().filter_map(|&i| matched[i].map(|j| (i, j))).collect();
        if !ref_ix.is_empty() {
            row["same_break"] = json!(round(100.0 * same.len() as f64 / ref_ix.len() as f64, 1));
        }
        let mut dys: Vec<f64> = same.iter().filter(|&&(_, j)| l2[j].page == p).map(|&(i, j)| (l1[i].y - l2[j].y).abs()).collect();
        dys.sort_by(f64::total_cmp);
        if let Some(m) = dys.get(dys.len() / 2) {
            row["dy"] = json!(round(*m, 1));
        }
        let mut max_dx = 0.0f64;
        for &(i, j) in &same {
            let dx = l2[j].x - l1[i].x;
            max_dx = max_dx.max(dx.abs());
            if dx.abs() > 6.0 {
                dx_diffs.push(json!({"page": p + 1, "kind": "dx", "value": round(dx, 1), "line": label(&t1[i])}));
            }
        }
        row["dx"] = json!(round(max_dx, 1));
        if let Some(&first) = ref_ix.first() {
            let starts = matched[first].is_some_and(|j| l2[j].page == p && (j == 0 || l2[j - 1].page != p));
            row["page_start_same"] = json!(starts);
        }
        pages.push(row);
    }
    // Runs of lines that differ between matched lines: different line breaks (first 40).
    let mut breaks = Vec::new();
    let (mut i0, mut j0) = (0usize, 0usize);
    for &(a, b) in pairs.iter().chain(std::iter::once(&(l1.len(), l2.len()))) {
        if (a > i0 || b > j0) && breaks.len() < 40 {
            breaks.push(json!({
                "ref_page": l1.get(i0).map(|l| l.page + 1),
                "ref_lines": a - i0,
                "test_lines": b - j0,
                "ref": t1[i0..a].iter().take(3).map(|t| label(t)).collect::<Vec<_>>(),
                "test": t2[j0..b].iter().take(3).map(|t| label(t)).collect::<Vec<_>>(),
            }));
        }
        (i0, j0) = (a + 1, b + 1);
    }
    dx_diffs.sort_by(|a, b| b["value"].as_f64().unwrap_or(0.0).abs().total_cmp(&a["value"].as_f64().unwrap_or(0.0).abs()));
    dx_diffs.truncate(50);
    let summary = json!({
        "pages": [rp.len(), tp.len()],
        "ssim_mean": round(if ssims.is_empty() { 0.0 } else { ssims.iter().sum::<f64>() / ssims.len() as f64 }, 4),
        "same_break_pct": round(100.0 * pairs.len() as f64 / l1.len().max(1) as f64, 1),
        "page_starts_same": pages.iter().filter(|r| r["page_start_same"] == true).count(),
        "lines": [l1.len(), l2.len()],
    });
    Ok(json!({"summary": summary, "pages": pages, "dx_diffs": dx_diffs, "break_diffs": breaks}))
}

fn tool(name: &str, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    let o = Command::new(name).args(args).output().map_err(|e| format!("{name} (poppler) not found: {e}"))?;
    if !o.status.success() {
        return Err(format!("{name} failed: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(o.stdout)
}

/// Page images as greyscale PGM files, in page order.
fn rasterize(pdf: &Path, dpi: u32, tmp: &Path, tag: &str) -> Result<Vec<PathBuf>, String> {
    let prefix = tmp.join(tag);
    let dpi = dpi.to_string();
    tool("pdftoppm", &["-r".as_ref(), dpi.as_ref(), "-gray".as_ref(), pdf.as_os_str(), prefix.as_os_str()])?;
    let mut files: Vec<(usize, PathBuf)> = std::fs::read_dir(tmp)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix(&format!("{tag}-"))?.strip_suffix(".pgm")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    files.sort();
    Ok(files.into_iter().map(|(_, p)| p).collect())
}

/// A binary (P5) 8-bit PGM.
fn read_pgm(path: &Path) -> Result<Gray, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 && i < data.len() {
        if data[i] == b'#' {
            while i < data.len() && data[i] != b'\n' {
                i += 1;
            }
        } else if data[i].is_ascii_whitespace() {
            i += 1;
        } else {
            let start = i;
            while i < data.len() && !data[i].is_ascii_whitespace() {
                i += 1;
            }
            fields.push(String::from_utf8_lossy(&data[start..i]).into_owned());
        }
    }
    let bad = || format!("{}: not an 8-bit P5 PGM", path.display());
    let [magic, w, h, max] = fields.as_slice() else { return Err(bad()) };
    let (w, h): (usize, usize) = (w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?);
    if magic != "P5" || max != "255" {
        return Err(bad());
    }
    let px = data.get(i + 1..i + 1 + w * h).ok_or_else(bad)?.to_vec();
    Ok(Gray { w, h, px })
}

/// Mean SSIM over non-overlapping 8×8 windows of the common area.
fn ssim(a: &Gray, b: &Gray) -> f64 {
    const WIN: usize = 8;
    let (w, h) = ((a.w.min(b.w) / WIN) * WIN, (a.h.min(b.h) / WIN) * WIN);
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let (mut sum, mut n) = (0.0, 0usize);
    for by in (0..h).step_by(WIN) {
        for bx in (0..w).step_by(WIN) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for y in by..by + WIN {
                for x in bx..bx + WIN {
                    let (p, q) = (f64::from(a.px[y * a.w + x]), f64::from(b.px[y * b.w + x]));
                    (sa, sb, saa, sbb, sab) = (sa + p, sb + q, saa + p * p, sbb + q * q, sab + p * q);
                }
            }
            let k = (WIN * WIN) as f64;
            let (ma, mb) = (sa / k, sb / k);
            let (va, vb, cov) = (saa / k - ma * ma, sbb / k - mb * mb, sab / k - ma * mb);
            sum += ((2.0 * ma * mb + c1) * (2.0 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

fn diff_share(a: &Gray, b: &Gray) -> f64 {
    let (w, h) = (a.w.min(b.w), a.h.min(b.h));
    let mut n = 0usize;
    for y in 0..h {
        for x in 0..w {
            if a.px[y * a.w + x].abs_diff(b.px[y * b.w + x]) > 64 {
                n += 1;
            }
        }
    }
    if w * h == 0 { 0.0 } else { n as f64 / (w * h) as f64 }
}

/// Reference ink red, WordCraft ink blue, both black (binary PPM).
fn write_diff_map(a: &Gray, b: &Gray, path: &Path) -> Result<(), String> {
    let (w, h) = (a.w.min(b.w), a.h.min(b.h));
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for y in 0..h {
        for x in 0..w {
            let (p, q) = (a.px[y * a.w + x], b.px[y * b.w + x]);
            out.extend_from_slice(&[q, p.min(q), p]);
        }
    }
    std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
}

/// Lines of the text layer, from `pdftotext -bbox-layout`.
fn lines_of(pdf: &Path) -> Result<Vec<Line>, String> {
    let xml = String::from_utf8_lossy(&tool("pdftotext", &["-bbox-layout".as_ref(), pdf.as_os_str(), "-".as_ref()])?).into_owned();
    Ok(parse_bbox(&xml))
}

fn attr(tag: &str, name: &str) -> Option<f64> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    tag.get(at..)?.split('"').next()?.parse().ok()
}

fn parse_bbox(xml: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for (page, body) in xml.split("<page ").skip(1).enumerate() {
        for line in body.split("<line").skip(1) {
            let line = line.split("</line>").next().unwrap_or("");
            let mut words = Vec::new();
            let mut first: Option<(f64, f64)> = None;
            for w in line.split("<word").skip(1) {
                let Some((tag, rest)) = w.split_once('>') else { continue };
                let text = rest.split("</word>").next().unwrap_or("");
                if first.is_none() {
                    first = attr(tag, "xMin").zip(attr(tag, "yMin"));
                }
                words.push(unescape(text));
            }
            if let (Some((x, y)), false) = (first, words.is_empty()) {
                lines.push(Line { page, y, x, text: words.join(" ") });
            }
        }
    }
    lines
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Longest common subsequence of two line lists, as matched index pairs in order.
fn lcs(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    // Equal lines at the start and end first: keeps the table small for near-identical documents.
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let (n, m) = (ma.len(), mb.len());
    let mut out: Vec<(usize, usize)> = (0..pre).map(|i| (i, i)).collect();
    // The table is (n+1)(m+1) cells: past ~100 M (400 MB) keep only the equal start and end.
    if (n + 1).saturating_mul(m + 1) > 100_000_000 {
        out.extend((0..suf).map(|k| (a.len() - suf + k, b.len() - suf + k)));
        return out;
    }
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * (m + 1) + j] = if ma[i] == mb[j] { t[(i + 1) * (m + 1) + j + 1] + 1 } else { t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if ma[i] == mb[j] {
            out.push((pre + i, pre + j));
            (i, j) = (i + 1, j + 1);
        } else if t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out.extend((0..suf).map(|k| (a.len() - suf + k, b.len() - suf + k)));
    out
}

/// A short stable hash of a line, for `--private` (FNV-1a, 64 bit).
fn hash(s: &str) -> String {
    let h = s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{h:016x}")[..10].to_string()
}

fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (v * k).round() / k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_flags_and_dpi_are_validated_before_output() {
        assert_eq!(parse_args(&["r.pdf", "--private", "t.pdf", "--dpi", "144", "out"]).unwrap(), (["r.pdf", "t.pdf", "out"], 144, true));
        assert_eq!(parse_args(&["r", "t", "out"]).unwrap(), (["r", "t", "out"], 100, false));
        for flag in ["--privte", "--unknown", "-private"] {
            assert!(parse_args(&["r", "t", "out", flag]).is_err());
        }
        for args in
            [vec!["r", "t", "out", "--dpi"], vec!["r", "t", "out", "--dpi", "301"], vec!["r", "t", "out", "--dpi", "--private"], vec!["r", "t"]]
        {
            assert!(parse_args(&args).is_err());
        }
        assert_eq!(parse_args(&["--", "-r", "t", "out"]).unwrap().0, ["-r", "t", "out"]);
    }

    fn gray(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Gray {
        Gray { w, h, px: (0..w * h).map(|i| f(i % w, i / w)).collect() }
    }

    #[test]
    fn ssim_is_one_for_the_same_image_and_lower_for_a_shifted_one() {
        let a = gray(64, 64, |x, y| if (x / 4 + y / 4) % 2 == 0 { 0 } else { 255 });
        let b = gray(64, 64, |x, y| if ((x + 2) / 4 + y / 4) % 2 == 0 { 0 } else { 255 });
        assert!((ssim(&a, &a) - 1.0).abs() < 1e-9);
        assert!(ssim(&a, &b) < 0.9);
        assert_eq!(diff_share(&a, &a), 0.0);
    }

    #[test]
    fn lcs_matches_lines_in_order() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let a = v(&["intro", "one two", "three", "end"]);
        let b = v(&["intro", "one", "two three", "end"]);
        assert_eq!(lcs(&a, &b), vec![(0, 0), (3, 3)]);
        assert_eq!(lcs(&a, &a).len(), 4);
    }

    #[test]
    fn bbox_layout_lines_keep_page_position_and_text() {
        let xml = r#"<doc><page width="595" height="842"><flow><block><line xMin="72" yMin="80" xMax="200" yMax="92">
<word xMin="72.0" yMin="80.5" xMax="100" yMax="92">Tom</word><word xMin="104" yMin="80.5" xMax="200" yMax="92">&amp; Jerry</word>
</line></block></flow></page><page width="595" height="842"><flow><block><line><word xMin="90" yMin="100" xMax="120" yMax="110">Two</word></line></block></flow></page></doc>"#;
        let l = parse_bbox(xml);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].page, l[0].x, l[0].y, l[0].text.as_str()), (0, 72.0, 80.5, "Tom & Jerry"));
        assert_eq!((l[1].page, l[1].text.as_str()), (1, "Two"));
    }

    #[test]
    fn pgm_header_with_a_comment_is_read() {
        let dir = std::env::temp_dir().join(format!("wc-vd-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("a.pgm");
        let mut data = b"P5\n# made by test\n2 2\n255\n".to_vec();
        data.extend_from_slice(&[0, 64, 128, 255]);
        std::fs::write(&p, data).unwrap();
        let g = read_pgm(&p).unwrap();
        assert_eq!((g.w, g.h, g.px.clone()), (2, 2, vec![0, 64, 128, 255]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
