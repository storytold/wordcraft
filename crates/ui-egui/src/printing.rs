//! Localized print-page wording around the shared, backend-independent controls.
use craft_ui::printing::Labels;
pub use craft_ui::printing::{Discover, Duplex, Feedback, Printer, Settings, State};

/// Native submission adds WordCraft's PDF-page collation setting and spooler options to the shared spooler settings.
pub type Submit = Box<dyn Fn(Vec<u8>, Settings, bool, Vec<(String, String)>) -> std::sync::mpsc::Receiver<Result<String, String>>>;

/// Metadata for the app-owned commands. Inspecting it never discovers printers or submits.
pub fn commands(app: &crate::WordApp) -> Vec<serde_json::Value> {
    use serde_json::json;
    let enabled = if app.services.print_native.is_some() { app.printing.ready() } else { app.services.print.is_some() };
    let submit_params = "range?: all|current|selection|custom; pages?: string (1-3,5); parity?: all|odd|even; copies?: integer 1..999; collate?: bool; orientation?: auto|portrait|landscape; paper?: none|Letter|Legal|A4|… (Layout › Size names: Scale to paper size, every page scaled onto that paper and centred; the document is unchanged). Native uses the selected printer, duplex and grayscale from ui.printSettings; omitted copies/collation use current settings. Browser printing opens its dialog. Submits only when explicitly executed.";
    vec![
        json!({"id":"ui.printSettings", "label":"Print settings", "location":"File › Print", "shortcut":null,
            "params":"printer?: discovered name; copies?: integer 1..999; duplex?: printerDefault|off|longEdge|shortEdge|null; grayscale?: bool; refresh?: bool. Empty params inspect printer/copies/duplex/grayscale, printers, busy/discovering/ready and status/detail. Updates are atomic; refresh discovers only on native hosts. Never submits or opens a dialog.",
            "enabled":true}),
        json!({"id":"ui.printSubmit", "label":"Submit print job", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
        json!({"id":"ui.print", "label":"Print (alias of ui.printSubmit)", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
    ]
}

/// Builds translated print labels, draws the shared print-settings panel, and shows the current
/// feedback message.
pub fn show(state: &mut State, ui: &mut egui::Ui, discover: &Discover, translate: impl Fn(&str) -> String) {
    let labels = Labels {
        printer: &translate("Printer"),
        select_printer: &translate("Select a printer"),
        refresh: &translate("Refresh printers"),
        copies: &translate("Copies"),
        duplex: &translate("Two-sided printing"),
        printer_default: &translate("Printer default"),
        one_sided: &translate("One-sided"),
        long_edge: &translate("Two-sided: long edge"),
        short_edge: &translate("Two-sided: short edge"),
        grayscale: &translate("Grayscale"),
    };
    state.show(ui, egui::Id::unique("wordcraft_print_settings"), discover, &labels);
    if let Some(message) = &state.message {
        ui.label(feedback(message, translate));
    }
}

/// Read print settings/status, or validate an explicit update before changing anything.
/// `refresh: true` starts discovery only; this command never submits a job or opens a dialog.
pub fn settings(state: &mut State, params: &serde_json::Value, discover: Option<&Discover>) -> Result<serde_json::Value, String> {
    use serde_json::{Value, json};
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
        next.printer = printer.to_owned();
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
    };
    Ok(json!({
        "printer": state.settings.printer, "copies": state.settings.copies,
        "duplex": match state.settings.duplex { None => "printerDefault", Some(Duplex::Off) => "off", Some(Duplex::LongEdge) => "longEdge", Some(Duplex::ShortEdge) => "shortEdge" },
        "grayscale": state.settings.grayscale,
        "printers": state.printers.iter().map(|printer| json!({"name": printer.name, "default": printer.default})).collect::<Vec<Value>>(),
        "busy": state.busy(), "discovering": state.discovering(), "ready": state.ready(), "status": status, "detail": detail,
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
    }
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

/// Build the upstream page-range/markup PDF and hand it to the chosen host print flow.
pub fn submit(app: &mut crate::WordApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::json;
    if app.services.print.is_none() && app.services.print_native.is_none() {
        return Err("printing to the system print dialog isn't available here; export a PDF instead".into());
    }
    let native = app.services.print_native.is_some();
    if native && !app.printing.ready() {
        return Err("Select an available printer in File > Print before submitting.".into());
    }
    let mut pdf_params = params.clone();
    let mut settings = app.printing.settings.clone();
    let collate = params.get("collate").map(|value| value.as_bool().ok_or("collate must be a boolean")).transpose()?.unwrap_or(app.ui.print.collate);
    if native {
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
    let pdf = app.session.print_pdf(&pdf_params).map_err(|error| error.to_string());
    if colour.is_some() {
        app.session.doc.settings.page_color = colour;
    }
    if picture.is_some() {
        app.session.doc.settings.page_picture = picture;
    }
    let result = pdf.and_then(|(bytes, plan)| {
        if native && plan.chosen.checked_mul(settings.copies as usize).is_none_or(|pages| pages > wordcraft_engine::print::MAX_PRINT_PAGES) {
            return Err("the selected pages and copies exceed the print-job page limit".into());
        }
        let len = bytes.len();
        if let Some(submit) = &app.services.print_native {
            let mut print_options = Vec::new();
            let orientation = params.get("orientation").and_then(|v| v.as_str()).unwrap_or(&app.ui.print.orientation);
            let is_landscape = if orientation == "landscape" {
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
            app.printing.begin(submit(bytes, settings, collate, print_options)).map_err(|_| "A print job is already being submitted.".to_owned())?;
            app.print_feedback_seen = None;
        } else if let Some(print) = &app.services.print {
            print(&bytes)?;
        }
        Ok(len)
    });
    match result {
        Ok(len) => {
            app.status(if native { tl!("Submitting print job…") } else { tl!("Opening print dialog…") });
            Ok(json!({"printing": true, "bytes": len, "pending": native}))
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
