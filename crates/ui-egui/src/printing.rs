//! Localized print-page wording around the shared, backend-independent controls.
pub use craft::print::ui::{Destination, Discover, Duplex, Feedback, Printer, Settings, State};
use serde_json::{Value, json};

use crate::print_page::{NO_SCALING, ORIENTATIONS, PAGES_PER_SHEET, PARITIES, PrintPage, RANGES};

/// Metadata for the app-owned commands. Inspecting it never discovers printers or submits.
pub fn commands(app: &crate::WordApp) -> Vec<Value> {
    let enabled = match &app.services.print {
        Some(spool) if spool.chooses_printer() => app.printing.ready(),
        Some(_) => true,
        None => false,
    };
    let submit_params = "pagesPerSheet?: 1|2|4|6|9|16|booklet|bookletReverse; range?: all|current|selection|custom; pages?: string (1-3,5); parity?: all|odd|even; copies?: integer 1..999; collate?: bool; orientation?: auto|portrait|landscape; paper?: none|Letter|Legal|A4|… (Layout › Size names: Scale to paper size, every page scaled onto that paper and centred; the document is unchanged). Native uses the selected printer, duplex and grayscale from ui.printSettings; omitted copies/collation use current settings. Browser printing opens its dialog. Submits only when explicitly executed.";
    vec![
        json!({"id":"ui.printSettings", "label":"Print settings", "location":"File › Print", "shortcut":null,
            "params":"printer?: discovered name; copies?: integer 1..999; duplex?: printerDefault|off|longEdge|shortEdge|null; grayscale?: bool; refresh?: bool. Empty params inspect printer/copies/duplex/grayscale, printers, busy/discovering/ready and status/detail. Updates are atomic; refresh discovers only on native hosts. Never submits or opens a dialog.",
            "enabled":true}),
        json!({"id":"ui.printPage", "label":"Print page settings", "location":"File › Print", "shortcut":null,
            "params":"range?: all|current|selection|custom; pages?: string (1-3,5); parity?: all|odd|even; orientation?: auto|portrait|landscape; pagesPerSheet?: 1|2|4|6|9|16|booklet|bookletReverse; paper?: none|Layout › Size name; preview?: sheet index (0-based); zoom?: number 0.1..5 or null (fit). Empty params inspect. Updates are atomic. Never prints or opens a dialog.",
            "enabled":true}),
        json!({"id":"ui.printSubmit", "label":"Submit print job", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
        json!({"id":"ui.print", "label":"Print (alias of ui.printSubmit)", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
        json!({"id":"ui.printCancel", "label":"Cancel printing", "location":"File › Print", "shortcut":null,
            "params":"none. Stops the job being submitted if it hasn't started reaching the spooler (it may wait on a sleeping printer's driver); returns cancelled: true when nothing will be sent, false when the job was already being sent (cancel it from the system's print queue).",
            "enabled": app.services.print.is_some() && app.printing.busy()}),
    ]
}

/// Read print settings/status, or validate an explicit update before changing anything.
/// `refresh: true` starts discovery only; this command never submits a job or opens a dialog.
pub fn settings(state: &mut State, params: &Value, discover: Option<&Discover>) -> Result<Value, String> {
    let object = params.as_object().ok_or("print settings must be an object")?;
    if object.keys().any(|key| !["printer", "copies", "duplex", "grayscale", "refresh"].contains(&key.as_str())) {
        return Err("unknown print setting; use printer, copies, duplex, grayscale or refresh".into());
    }
    if state.busy() && !object.is_empty() {
        return Err("wait for the active print submission before changing settings".into());
    }
    let mut next = state.settings.clone();
    if let Some(value) = object.get("printer") {
        let printer = value.as_str().ok_or("printer must be a name from the discovered list")?;
        if !state.printers.iter().any(|available| available.name == printer) {
            return Err("printer must be a name from the discovered list".into());
        }
        next.destination = Destination::Printer(printer.to_owned());
    }
    if let Some(value) = object.get("copies") {
        let copies = value.as_u64().filter(|copies| (1..=999).contains(copies)).ok_or("copies must be an integer from 1 to 999")?;
        next.copies = copies as u32;
    }
    if let Some(value) = object.get("duplex") {
        next.duplex = match value.as_str() {
            Some("printerDefault") => None,
            Some("off") => Some(Duplex::Off),
            Some("longEdge") => Some(Duplex::LongEdge),
            Some("shortEdge") => Some(Duplex::ShortEdge),
            None if value.is_null() => None,
            _ => return Err("duplex must be printerDefault, off, longEdge or shortEdge".into()),
        };
    }
    if let Some(value) = object.get("grayscale") {
        next.grayscale = value.as_bool().ok_or("grayscale must be a boolean")?;
    }
    let refresh = object.get("refresh").map(|value| value.as_bool().ok_or("refresh must be a boolean")).transpose()?.unwrap_or(false);
    if refresh {
        let discover = discover.ok_or("native printer discovery is unavailable here")?;
        if !state.refresh(discover) {
            return Err("printer discovery or submission is already active".into());
        }
    }
    state.settings = next;
    let (status, detail) = match state.message.as_ref() {
        None => ("idle", ""),
        Some(Feedback::FindingPrinters) => ("findingPrinters", ""),
        Some(Feedback::NoPrinters) => ("noPrinters", ""),
        Some(Feedback::DiscoveryFailed) => ("discoveryFailed", ""),
        Some(Feedback::Submitting) => ("submitting", ""),
        Some(Feedback::Submitted(receipt)) => ("submitted", receipt.as_str()),
        Some(Feedback::Failed(error)) => ("failed", error.as_str()),
        Some(Feedback::WorkerStopped) => ("workerStopped", ""),
        Some(_) => ("idle", ""),
    };
    Ok(json!({
        "printer": printer_name(&state.settings), "copies": state.settings.copies,
        "duplex": match state.settings.duplex { None => "printerDefault", Some(Duplex::LongEdge) => "longEdge", Some(Duplex::ShortEdge) => "shortEdge", Some(_) => "off" },
        "grayscale": state.settings.grayscale,
        "printers": state.printers.iter().map(|printer| json!({"name": printer.name, "default": printer.default})).collect::<Vec<Value>>(),
        "busy": state.busy(), "discovering": state.discovering(), "ready": state.ready(), "status": status, "detail": detail,
    }))
}

/// The value of `key` when it names one of `ids`; an error naming the key otherwise.
fn one_of<'a>(object: &'a serde_json::Map<String, Value>, key: &str, ids: impl Iterator<Item = &'a str>) -> Result<Option<String>, String> {
    let Some(value) = object.get(key) else { return Ok(None) };
    let ids: Vec<&str> = ids.collect();
    match value.as_str() {
        Some(text) if ids.contains(&text) => Ok(Some(text.to_owned())),
        _ => Err(format!("{key} must be one of {}", ids.join(", "))),
    }
}

// PLAN(shell-w6): ui.printPage arm in app/commands.rs beside ui.printSettings: `"ui.printPage" => printing::print_page(self, &params)`.
/// `ui.printPage`: read the Print page's settings, or validate an explicit update before changing any.
/// Never prints, opens a dialog or starts discovery.
pub(crate) fn print_page(app: &mut crate::WordApp, params: &Value) -> Result<Value, String> {
    let object = params.as_object().ok_or("print page settings must be an object")?;
    const KEYS: [&str; 8] = ["range", "pages", "parity", "orientation", "pagesPerSheet", "paper", "preview", "zoom"];
    if object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err("unknown print page setting; use range, pages, parity, orientation, pagesPerSheet, paper, preview or zoom".into());
    }
    let mut next = app.print.settings.clone();
    let mut view = app.print.view.clone();
    if let Some(range) = one_of(object, "range", RANGES.iter().map(|(id, _)| *id))? {
        next.range = range;
    }
    if let Some(parity) = one_of(object, "parity", PARITIES.iter().map(|(id, _)| *id))? {
        next.parity = parity;
    }
    if let Some(orientation) = one_of(object, "orientation", ORIENTATIONS.iter().map(|(id, _)| *id))? {
        next.orientation = orientation;
    }
    if let Some(value) = object.get("pagesPerSheet") {
        let text = value.as_u64().map_or_else(|| value.as_str().map(str::to_owned), |n| Some(n.to_string()));
        match text {
            Some(text) if PAGES_PER_SHEET.iter().any(|(id, _)| *id == text) => next.pages_per_sheet = text,
            _ => return Err("pagesPerSheet must be 1, 2, 4, 6, 9, 16, booklet or bookletReverse".into()),
        }
    }
    if let Some(value) = object.get("paper") {
        match value.as_str() {
            Some(paper) if paper == NO_SCALING || wordcraft_geom::paper_by_name(paper).is_some() => next.paper = paper.to_owned(),
            _ => return Err("paper must be none or a Layout › Size name".into()),
        }
    }
    if let Some(value) = object.get("pages") {
        let pages = value.as_str().filter(|pages| pages.chars().count() <= wordcraft_engine::print::MAX_SPEC_CHARS);
        next.pages = pages.ok_or("pages must be a page list of at most 500 characters")?.to_owned();
    }
    let restarts = next.range != app.print.settings.range || next.pages != app.print.settings.pages || next.parity != app.print.settings.parity;
    if restarts {
        // As on the page: a new range starts at its first sheet.
        next.preview = 0;
    }
    if let Some(value) = object.get("preview") {
        next.preview = value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("preview must be a sheet index from 0")?;
    }
    if let Some(value) = object.get("zoom") {
        view = match value.as_f64() {
            None if value.is_null() => PrintPage::fit_view(),
            Some(zoom) if zoom.is_finite() && (0.1..=5.0).contains(&zoom) => view.zoomed_to(zoom as f32),
            _ => return Err("zoom must be a number from 0.1 to 5, or null to fit".into()),
        };
    }
    app.print.settings = next;
    app.print.view = view;
    let settings = &app.print.settings;
    Ok(json!({
        "range": settings.range, "pages": settings.pages, "parity": settings.parity, "orientation": settings.orientation,
        "pagesPerSheet": settings.pages_per_sheet, "paper": settings.paper, "preview": settings.preview,
        "zoom": if app.print.view.fit { Value::Null } else { json!(app.print.view.transform.scale) },
    }))
}

const SPOOLED: &str = "Print job submitted: spooled {count} sheet(s)";

/// The sheet count of the Windows backend's English receipt `spooled N sheet(s)`.
fn spooled_sheets(receipt: &str) -> Option<usize> {
    receipt.trim().strip_prefix("spooled ")?.strip_suffix(" sheet(s)")?.parse().ok()
}

pub fn feedback(message: &Feedback, translate: impl Fn(&str) -> String) -> String {
    match message {
        Feedback::FindingPrinters => translate("Finding printers…"),
        Feedback::NoPrinters => translate("No printers found. Configure a system printer or export a PDF."),
        Feedback::DiscoveryFailed => translate("Printer discovery failed."),
        Feedback::Submitting => translate("Submitting print job…"),
        Feedback::Submitted(receipt) if receipt.trim().is_empty() => translate("Print job submitted."),
        Feedback::Submitted(receipt) if spooled_sheets(receipt).is_some_and(|_| translate(SPOOLED) != SPOOLED) => {
            let count = spooled_sheets(receipt).unwrap_or_default();
            translate(SPOOLED).replace("{count}", &count.to_string())
        }
        Feedback::Submitted(receipt) => translate("Print job submitted: {receipt}").replace("{receipt}", receipt),
        Feedback::Failed(error) => translate("Print failed: {error}").replace("{error}", error),
        Feedback::WorkerStopped => translate("Print failed: the printing worker stopped."),
        _ => String::new(),
    }
}

/// `ui.printCancel`: stop the job being submitted before it reaches the spooler. A job can wait
/// a long time on a sleeping network printer's driver (#5710) with the print settings locked;
/// once cancelled it is never sent, so the panel is free again at once: printer list and
/// settings kept, the printers listed again.
pub fn cancel(app: &mut crate::WordApp) -> Result<Value, String> {
    if !app.printing.busy() {
        return Err("no print job is being submitted".into());
    }
    let spool = app.services.print.clone().ok_or("cancelling a print job isn't available here")?;
    if !spool.cancel_latest() {
        app.status(tl!("Cancellation could not be confirmed here. The printer driver may still be processing the job. If it appears in the system's print queue, cancel it there."));
        return Ok(json!({"cancelled": false}));
    }
    // The shared state keeps the job's receiver private: a fresh one drops it (the worker's
    // answer, if it ever comes, goes nowhere) and keeps the user's settings.
    let old = std::mem::take(&mut app.printing);
    app.printing.settings = old.settings;
    app.printing.printers = old.printers;
    app.status(tl!("Printing cancelled. Nothing was sent to the printer."));
    Ok(json!({"cancelled": true}))
}

/// The IPP/PWG 5101.1 media name of a Layout › Size paper (the spooler's `media` option).
pub fn pwg_media(paper: &str) -> Option<&'static str> {
    Some(match paper {
        "Letter" => "na_letter_8.5x11in",
        "Legal" => "na_legal_8.5x14in",
        "Executive" => "na_executive_7.25x10.5in",
        "A3" => "iso_a3_297x420mm",
        "A4" => "iso_a4_210x297mm",
        "A5" => "iso_a5_148x210mm",
        "B4 (JIS)" => "jis_b4_257x364mm",
        "B5 (JIS)" => "jis_b5_182x257mm",
        "Tabloid" => "na_ledger_11x17in",
        "Statement" => "na_invoice_5.5x8.5in",
        "Envelope #10" => "na_number-10_4.125x9.5in",
        "Envelope DL" => "iso_dl_110x220mm",
        _ => return None,
    })
}

/// The chosen printer's name; empty when none is chosen or the destination is not a printer.
pub(crate) fn printer_name(settings: &Settings) -> &str {
    match &settings.destination {
        Destination::Printer(name) => name,
        _ => "",
    }
}

/// The spooler's job for `settings`: the app's choices, shrink-to-fit paper and the options list.
// PLAN: assumed `craft::print` is reachable from ui-egui (the Job is built here, as apps/wordcraft/src/print.rs did).
fn job_of(settings: &Settings, collate: bool, options: Vec<(String, String)>) -> craft::print::Job {
    craft::print::Job {
        printer: Some(printer_name(settings)).filter(|name| !name.is_empty()).map(str::to_owned),
        copies: settings.copies.clamp(1, 999),
        collate,
        duplex: settings.duplex.map(|value| match value {
            Duplex::LongEdge => craft::print::Duplex::LongEdge,
            Duplex::ShortEdge => craft::print::Duplex::ShortEdge,
            _ => craft::print::Duplex::Off,
        }),
        grayscale: settings.grayscale,
        grey_pages: false,
        scaling: craft::print::Scaling::ShrinkToFit,
        title: "WordCraft document".into(),
        options,
        driver_settings: None,
    }
}

/// Build the upstream page-range/markup PDF and hand it to the spool.
pub fn submit(app: &mut crate::WordApp, params: &Value) -> Result<Value, String> {
    let Some(spool) = app.services.print.clone() else {
        return Err("printing to the system print dialog isn't available here; export a PDF instead".into());
    };
    // Save as PDF is WordCraft's own route, not the spooler's.
    if app.printing.settings.destination == Destination::File {
        app.print_pdf_dialog();
        return Ok(json!({"saveAsPdf": true}));
    }
    let chooses = spool.chooses_printer();
    if chooses && !app.printing.ready() {
        return Err("Select an available printer in File > Print before submitting.".into());
    }
    if app.printing.busy() {
        return Err("A print job is already being submitted.".into());
    }
    let mut pdf_params = params.clone();
    let mut settings = app.printing.settings.clone();
    let collate =
        params.get("collate").map(|value| value.as_bool().ok_or("collate must be a boolean")).transpose()?.unwrap_or(app.print.settings.collate);
    if chooses {
        let object = pdf_params.as_object_mut().ok_or("print settings must be an object")?;
        if let Some(copies) = object.get("copies") {
            settings.copies = copies.as_u64().filter(|copies| (1..=999).contains(copies)).ok_or("copies must be an integer from 1 to 999")? as u32;
        }
        // Copies/collation belong to the spooler. Baking them into the PDF would multiply
        // the copies twice, and duplex could put the end of one copy behind the next.
        object.insert("copies".into(), json!(1));
    }
    if app.ui.update_fields_before_print
        && let Err(error) = app.session.run("references.updateFields", &json!({"all": true}))
    {
        log::warn!("updating fields before printing: {error}");
    }
    let colour = if app.ui.print_backgrounds { None } else { app.session.doc.settings.page_color.take() };
    let picture = if app.ui.print_backgrounds { None } else { app.session.doc.settings.page_picture.take() };
    let pdf = app.session.print_job_pdf(&pdf_params).map_err(|error| error.to_string());
    if colour.is_some() {
        app.session.doc.settings.page_color = colour;
    }
    if picture.is_some() {
        app.session.doc.settings.page_picture = picture;
    }
    let result = pdf.and_then(|(bytes, plan)| {
        if chooses && plan.chosen.checked_mul(settings.copies as usize).is_none_or(|pages| pages > wordcraft_engine::print::MAX_PRINT_PAGES) {
            return Err("the selected pages and copies exceed the print-job page limit".into());
        }
        let len = bytes.len();
        let mut print_options = Vec::new();
        let orientation = params.get("orientation").and_then(|v| v.as_str()).unwrap_or(&app.print.settings.orientation);
        let mode = wordcraft_engine::print::imposition(params).map_err(|e| e.to_string())?;
        let is_landscape = if mode != wordcraft_engine::print::Imposition::Single {
            let markup = wordcraft_engine::cmd::file::export_options(&app.session, &pdf_params).markup;
            let lay = app.session.export_layout_with(markup);
            wordcraft_engine::print::sheets(params, &lay, app.session.current_pages(&lay))
                .map_err(|e| e.to_string())?
                .first()
                .is_some_and(|s| s.width > s.height)
        } else if orientation == "landscape" {
            true
        } else if orientation == "portrait" {
            false
        } else {
            let lay = app.session.export_layout();
            lay.pages.first().is_some_and(|p| p.w > p.h)
        };
        // Scale to paper size: ask the printer for that paper too, so it takes it from the right tray.
        if let Some(media) = wordcraft_engine::print::paper(params).ok().flatten().and_then(|(name, _, _)| pwg_media(name)) {
            print_options.push(("media".into(), media.into()));
        }
        if is_landscape {
            print_options.push(("orientation-requested".into(), "4".into()));
            print_options.push(("landscape".into(), "true".into()));
        } else {
            print_options.push(("orientation-requested".into(), "3".into()));
        }
        let receiver = spool.submit(bytes, job_of(&settings, collate, print_options));
        app.printing.begin(receiver).map_err(|_| "A print job is already being submitted.".to_owned())?;
        app.print_feedback_seen = None;
        app.print_gen = app.print_gen.wrapping_add(1);
        Ok(len)
    });
    match result {
        Ok(len) => {
            app.status(tl!("Submitting print job…"));
            Ok(json!({"printing": true, "bytes": len, "pending": true}))
        }
        Err(error) => {
            log::error!("print failed: {error}");
            let msg = crate::i18n::fmt(tl!("Print failed: {error}"), &[("error", &error)]);
            app.status(msg.clone());
            Err(msg)
        }
    }
}

#[cfg(test)]
mod tests {
    /// Every Layout › Size paper has a printer media name (#2108) the spooler accepts as a keyword.
    #[test]
    fn every_paper_size_has_a_media_name() {
        for (name, _, _) in wordcraft_geom::PAPER_SIZES {
            let media = super::pwg_media(name).unwrap_or_else(|| panic!("{name} has no media name"));
            assert!(media.len() <= 64 && media.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'/' | b':' | b'=' | b'"')), "{media}");
        }
        assert_eq!(super::pwg_media("Quarto"), None);
    }
}
