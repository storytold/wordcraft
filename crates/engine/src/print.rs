//! Print settings (#641): which pages go out, how many copies, as a PDF.
//!
//! WordCraft prints through a PDF (the host opens it in the system's print flow), so the settings
//! that can be delivered are the ones that shape the PDF: the page range, odd/even pages, copies
//! and collation. Printer choice, duplex and pages per sheet belong to the OS print dialog.
//!
//! Page ranges and copies come from users and control channels, so they are bounded: a range
//! text is at most [`MAX_SPEC_CHARS`] long, copies are clamped to [`MAX_COPIES`], and the
//! resulting page list never exceeds [`MAX_PRINT_PAGES`].

use serde_json::{Value, json};
use wordcraft_layout::DocLayout;

use crate::{CmdError, Session, p};

/// Longest page-range text accepted.
pub const MAX_SPEC_CHARS: usize = 500;
/// Most copies one print job asks for.
pub const MAX_COPIES: u32 = 999;
/// Most pages (counting copies) one print job holds.
pub const MAX_PRINT_PAGES: usize = 5000;

/// Parses a custom page range like `1-3, 5, 8-` into 0-based page indexes, in the order typed
/// (Word prints them in document order; so does this: sorted, without repeats). Pages are
/// 1-based in the text; `8-` means "to the last page" and `-3` "from the first". An empty text,
/// page 0, a page past `total`, a reversed range or anything else is an error.
pub fn parse_page_range(spec: &str, total: usize) -> Result<Vec<usize>, String> {
    if spec.chars().count() > MAX_SPEC_CHARS {
        return Err("the page range is too long".into());
    }
    if total == 0 {
        return Err("the document has no pages".into());
    }
    let mut keep = vec![false; total];
    let mut any = false;
    for item in spec.split([',', ';']) {
        let item: String = item.chars().filter(|c| !c.is_whitespace()).collect();
        if item.is_empty() {
            continue;
        }
        let page = |t: &str| -> Result<usize, String> {
            let n: usize = t.parse().map_err(|_| format!("`{item}` isn't a page or a range like 2-4"))?;
            if n == 0 || n > total {
                return Err(format!("page {n} doesn't exist; the document has {total} page{}", if total == 1 { "" } else { "s" }));
            }
            Ok(n)
        };
        let (from, to) = match item.split_once('-') {
            None => {
                let n = page(&item)?;
                (n, n)
            }
            Some(("", "")) => return Err("`-` isn't a page or a range like 2-4".into()),
            Some((a, b)) => {
                let from = if a.is_empty() { 1 } else { page(a)? };
                let to = if b.is_empty() { total } else { page(b)? };
                if from > to {
                    return Err(format!("`{item}` runs backwards"));
                }
                (from, to)
            }
        };
        for flag in keep.iter_mut().take(to).skip(from - 1) {
            *flag = true;
        }
        any = true;
    }
    if !any {
        return Err("type the pages to print, like 1-3, 5".into());
    }
    Ok(keep.iter().enumerate().filter(|(_, k)| **k).map(|(i, _)| i).collect())
}

/// What a print job holds: the pages (0-based, copies included, in print order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintPlan {
    /// Page indexes in print order, repeated for copies.
    pub pages: Vec<usize>,
    /// Distinct pages chosen (before copies).
    pub chosen: usize,
    pub copies: u32,
}

/// Works out the print plan for `v` against a layout of `total` pages whose caret and selection
/// sit on pages `current` (first, last). Params:
/// `range` (`all` | `current` | `selection` | `custom`; `custom` is implied by `pages`),
/// `pages` (text like `1-3,5`), `parity` (`all` | `odd` | `even`), `copies` (1–999),
/// `collate` (default true: 1,2,3 1,2,3; false: 1,1,2,2,3,3).
pub fn plan(v: &Value, total: usize, current: (usize, usize)) -> Result<PrintPlan, CmdError> {
    let bad = |m: String| CmdError::Params(m);
    let pages_text = p::str(v, "pages").filter(|t| !t.trim().is_empty());
    let range = p::str(v, "range").unwrap_or(if pages_text.is_some() { "custom" } else { "all" });
    let last = total.saturating_sub(1);
    let mut pages: Vec<usize> = match range {
        "all" => (0..total).collect(),
        "current" => vec![current.0.min(last)],
        "selection" => (current.0.min(last)..=current.1.min(last).max(current.0.min(last))).collect(),
        "custom" => parse_page_range(pages_text.unwrap_or(""), total).map_err(bad)?,
        other => return Err(bad(format!("unknown range `{other}`; use all, current, selection or custom"))),
    };
    match p::str(v, "parity").unwrap_or("all") {
        "all" => {}
        // Page numbers are 1-based: the odd ones are indexes 0, 2, 4…
        "odd" => pages.retain(|i| i % 2 == 0),
        "even" => pages.retain(|i| i % 2 == 1),
        other => return Err(bad(format!("unknown parity `{other}`; use all, odd or even"))),
    }
    if pages.is_empty() {
        return Err(bad("no pages to print with these settings".into()));
    }
    let copies = p::u64(v, "copies").unwrap_or(1).clamp(1, u64::from(MAX_COPIES)) as u32;
    let chosen = pages.len();
    let total_out = chosen.checked_mul(copies as usize).filter(|n| *n <= MAX_PRINT_PAGES);
    let Some(total_out) = total_out else {
        return Err(bad(format!("{chosen} pages × {copies} copies is more than the {MAX_PRINT_PAGES} pages one print job can hold")));
    };
    let mut out = Vec::with_capacity(total_out);
    if p::bool(v, "collate").unwrap_or(true) {
        for _ in 0..copies {
            out.extend_from_slice(&pages);
        }
    } else {
        for &pg in &pages {
            out.extend(std::iter::repeat_n(pg, copies as usize));
        }
    }
    Ok(PrintPlan { pages: out, chosen, copies })
}

impl Session {
    /// The pages the caret (or selection) is on, in the output layout: (first, last).
    pub fn current_pages(&self, lay: &DocLayout) -> (usize, usize) {
        let (a, b) = self.sel.ordered();
        let first = lay.caret_on(&a, self.page_hint).map_or(0, |c| c.page);
        let last = lay.caret_on(&b, self.page_hint).map_or(first, |c| c.page);
        (first, last.max(first))
    }

    /// The document as the PDF of a print job `v` describes (see [`plan`]), with the number of
    /// distinct pages chosen and copies. Does not touch the document.
    pub fn print_pdf(&self, v: &Value) -> Result<(Vec<u8>, PrintPlan), CmdError> {
        // Print Markup (File › Print) decides whether tracked changes and comments print.
        let markup = crate::cmd::file::export_options(self, v);
        let lay = self.export_layout_with(markup.markup);
        let plan = plan(v, lay.pages.len(), self.current_pages(&lay))?;
        let opts = wordcraft_pdf::PdfOptions {
            pages: Some(plan.pages.clone()),
            // Repeated pages would repeat the structure tree's entries too.
            tagged: plan.copies <= 1,
            include_markup: markup.markup,
            comments: markup.comments,
            ..Default::default()
        };
        let bytes = wordcraft_pdf::export_layout(&self.doc, &lay, &opts).map_err(|e| CmdError::Failed(e.to_string()))?;
        Ok((bytes, plan))
    }
}

/// `file.printPdf`: write the print job's PDF to `path` (no dialogs).
pub(crate) fn print_pdf_to_file(s: &mut Session, v: &Value) -> Result<Value, CmdError> {
    let path = p::req_str(v, "path")?;
    let path = if path.to_ascii_lowercase().ends_with(".pdf") { path.to_string() } else { format!("{path}.pdf") };
    // Printing to PDF is an export too: the applied profile's checks run before anything is
    // written (PDF verification included, since the output is a PDF).
    let opts = crate::cmd::file::export_options(s, v);
    let findings = s.run_export_checks(&opts, "pdf");
    let (bytes, plan) = s.print_pdf(v)?;
    // Through `io`, so an agent confined to an automation write root (#769) can't print elsewhere.
    #[cfg(not(target_arch = "wasm32"))]
    crate::io::write_file(&path, &bytes).map_err(CmdError::Failed)?;
    Ok(json!({"path": path, "pages": plan.pages.len(), "chosen": plan.chosen, "copies": plan.copies, "bytes": bytes.len(), "findings": findings}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_parse_and_hostile_ones_are_errors() {
        assert_eq!(parse_page_range("1-3, 5", 6), Ok(vec![0, 1, 2, 4]));
        assert_eq!(parse_page_range("5,1-2,2", 6), Ok(vec![0, 1, 4]), "document order, no repeats");
        assert_eq!(parse_page_range("4-", 6), Ok(vec![3, 4, 5]));
        assert_eq!(parse_page_range("-2", 6), Ok(vec![0, 1]));
        for bad in ["", " , ", "0", "7", "3-1", "a", "1-2-3", "99999999999999999999999", "-", "1..3", &"1,".repeat(400)] {
            assert!(parse_page_range(bad, 6).is_err(), "`{bad}` is refused");
        }
        assert!(parse_page_range("1", 0).is_err());
    }

    #[test]
    fn plans_apply_range_parity_copies_and_caps() {
        let p = |v: Value| plan(&v, 6, (2, 3)).map(|p| p.pages);
        assert_eq!(p(json!({})), Ok(vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(p(json!({"range": "current"})), Ok(vec![2]));
        assert_eq!(p(json!({"range": "selection"})), Ok(vec![2, 3]));
        assert_eq!(p(json!({"pages": "2-4", "copies": 2})), Ok(vec![1, 2, 3, 1, 2, 3]));
        assert_eq!(p(json!({"pages": "2-3", "copies": 2, "collate": false})), Ok(vec![1, 1, 2, 2]));
        assert_eq!(p(json!({"parity": "even"})), Ok(vec![1, 3, 5]));
        assert!(p(json!({"range": "custom"})).is_err(), "custom needs pages");
        assert!(p(json!({"range": "everything"})).is_err());
        assert!(p(json!({"range": "current", "parity": "even"})).is_err(), "page 3 is odd: nothing left");
        // Hostile copies are clamped; a job that would be too big is refused, not allocated.
        assert_eq!(p(json!({"range": "current", "copies": 1e12})).map(|v| v.len()), Ok(MAX_COPIES as usize));
        assert!(plan(&json!({"copies": 999}), 600, (0, 0)).is_err());
        assert_eq!(p(json!({"copies": -5, "range": "current"})), Ok(vec![2]));
    }

    #[test]
    fn print_pdf_writes_only_the_chosen_pages() {
        let mut s = Session::new(crate::sample::sample_document());
        let pages = s.export_layout().pages.len();
        let (all, _) = s.print_pdf(&json!({})).unwrap();
        let (one, plan) = s.print_pdf(&json!({"range": "current", "copies": 2})).unwrap();
        assert!(all.starts_with(b"%PDF") && one.starts_with(b"%PDF"));
        assert_eq!(plan.pages.len(), 2);
        if pages > 2 {
            assert!(one.len() < all.len());
        }
        assert!(s.print_pdf(&json!({"pages": "9999"})).is_err());
        let r = s.run("file.printPdf", &json!({})).is_err();
        assert!(r, "printPdf needs a path");
    }

    /// `file.printPdf` writes only inside the automation write root when there is one (#769).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn print_pdf_to_file_is_held_to_the_write_root() {
        use crate::file_access::{AutomationRoots, confine};
        let base = std::env::temp_dir().join(format!("wc-print-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["out", "outside"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        let roots = AutomationRoots::new(None, Some(&base.join("out"))).unwrap();
        let mut s = Session::new(crate::sample::sample_document());
        let inside = base.join("out/job").to_string_lossy().into_owned();
        let outside = base.join("outside/job").to_string_lossy().into_owned();
        confine(roots.as_ref(), || {
            let e = s.run("file.printPdf", &json!({"path": outside})).unwrap_err().to_string();
            assert!(e.contains("outside the write root"), "{e}");
            s.run("file.printPdf", &json!({"path": inside})).unwrap();
        });
        assert!(!base.join("outside/job.pdf").exists(), "nothing printed outside the root");
        assert!(std::fs::read(base.join("out/job.pdf")).unwrap().starts_with(b"%PDF"));
        // Without roots nothing is checked: the behaviour is unchanged.
        s.run("file.printPdf", &json!({"path": outside})).unwrap();
        assert!(base.join("outside/job.pdf").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }
}
