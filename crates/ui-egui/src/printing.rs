//! Localized print-page wording around the shared, backend-independent controls.
pub use craft_ui::printing::{Discover, Duplex, Feedback, Printer, Settings, State};

/// Native submission adds WordCraft's PDF-page collation setting and spooler options to the shared spooler settings.
pub type Submit = Box<dyn Fn(Vec<u8>, Settings, bool, Vec<(String, String)>) -> std::sync::mpsc::Receiver<Result<String, String>>>;

/// Metadata for the app-owned commands. Inspecting it never discovers printers or submits.
pub fn commands(app: &crate::WordApp) -> Vec<serde_json::Value> {
    use serde_json::json;
    let network = app.ui.print.network && app.services.print_network.is_some();
    let enabled = if network {
        !app.net_print.draft.running()
    } else if app.services.print_native.is_some() {
        app.printing.ready()
    } else {
        app.services.print.is_some()
    };
    let submit_params = "range?: all|current|selection|custom; pages?: string (1-3,5); parity?: all|odd|even; copies?: integer 1..999; collate?: bool; orientation?: auto|portrait|landscape. Native uses the selected printer, duplex and grayscale from ui.printSettings; with a network printer chosen on the Print page the job goes to it only after its confirmation box was ticked there (never by a parameter); omitted copies/collation use current settings. Browser printing opens its dialog. Submits only when explicitly executed.";
    vec![
        json!({"id":"ui.printSettings", "label":"Print settings", "location":"File › Print", "shortcut":null,
            "params":"printer?: discovered name; copies?: integer 1..999; duplex?: printerDefault|off|longEdge|shortEdge|null; grayscale?: bool; refresh?: bool. Empty params inspect printer/copies/duplex/grayscale, printers, busy/discovering/ready and status/detail. Updates are atomic; refresh discovers only on native hosts. Never submits or opens a dialog.",
            "enabled":true}),
        json!({"id":"ui.printSubmit", "label":"Submit print job", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
        json!({"id":"ui.print", "label":"Print (alias of ui.printSubmit)", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
    ]
}

/// The printers in the order the Printer list shows them: the default queue first, the rest as
/// discovered. Every real queue is listed.
pub fn listed(printers: &[Printer]) -> Vec<&Printer> {
    let mut out: Vec<&Printer> = printers.iter().filter(|p| p.default).collect();
    out.extend(printers.iter().filter(|p| !p.default));
    out
}

/// The one Printer list (File › Print): the system's queues, then "Network printer…" when the host
/// brings that service. `network` says whether that last entry is the chosen one. Also the Refresh
/// button. Discovery starts the first time this is drawn.
pub fn show_picker(
    state: &mut State,
    network: &mut bool,
    network_offered: bool,
    locked: bool,
    ui: &mut egui::Ui,
    discover: &Discover,
    translate: impl Fn(&str) -> String,
) {
    if state.message.is_none() && state.printers.is_empty() && !state.discovering() && !state.busy() {
        state.refresh(discover);
    }
    state.poll(ui.ctx());
    ui.push_id(egui::Id::new("wordcraft_print_settings"), |ui| {
        ui.add_enabled_ui(!state.busy() && !locked, |ui| {
            let width = ui.available_width().clamp(32.0, 230.0);
            let label = ui.label(translate("Printer"));
            let network_label = translate("Network printer…");
            let shown = if *network {
                network_label.clone()
            } else if state.settings.printer.is_empty() {
                translate("Select a printer")
            } else {
                state.settings.printer.clone()
            };
            egui::ComboBox::from_id_salt("printer")
                .width(width)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    let names: Vec<String> = listed(&state.printers).into_iter().map(|p| p.name.clone()).collect();
                    for name in names {
                        if ui.selectable_label(!*network && state.settings.printer == name, &name).clicked() {
                            state.settings.printer = name;
                            *network = false;
                        }
                    }
                    if network_offered && ui.selectable_label(*network, &network_label).clicked() {
                        *network = true;
                    }
                })
                .response
                .labelled_by(label.id);
            if ui.add_enabled(!state.discovering(), egui::Button::new(translate("Refresh printers"))).clicked() {
                state.refresh(discover);
            }
        });
    });
    if let Some(message) = &state.message {
        ui.label(feedback(message, translate));
    }
}

/// A local queue's copies, two-sided choice and Grayscale box.
pub fn show_options(state: &mut State, ui: &mut egui::Ui, translate: impl Fn(&str) -> String) {
    let width = ui.available_width().clamp(32.0, 230.0);
    ui.push_id(egui::Id::new("wordcraft_print_options"), |ui| {
        ui.add_enabled_ui(!state.busy(), |ui| {
            ui.horizontal(|ui| {
                let label = ui.label(translate("Copies"));
                ui.add(egui::DragValue::new(&mut state.settings.copies).range(1..=999)).labelled_by(label.id);
            });
            let choices = [
                (None, translate("Printer default")),
                (Some(Duplex::Off), translate("One-sided")),
                (Some(Duplex::LongEdge), translate("Two-sided: long edge")),
                (Some(Duplex::ShortEdge), translate("Two-sided: short edge")),
            ];
            let duplex_label = ui.label(translate("Two-sided printing"));
            let shown =
                choices.iter().find(|(value, _)| *value == state.settings.duplex).map_or_else(|| translate("Printer default"), |(_, t)| t.clone());
            egui::ComboBox::from_id_salt("duplex")
                .width(width)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (value, text) in &choices {
                        ui.selectable_value(&mut state.settings.duplex, *value, text);
                    }
                })
                .response
                .labelled_by(duplex_label.id);
            ui.checkbox(&mut state.settings.grayscale, translate("Grayscale"));
        });
    });
}

/// Whether the pages should print gray: the local queue's Grayscale box, or the network
/// printer's Color setting when that is the chosen destination.
pub fn grayscale(app: &crate::WordApp) -> bool {
    if app.ui.print.network && app.services.print_network.is_some() {
        app.net_print.draft.request(None).color == Some(craft_ui::printing::network::ColorMode::Monochrome)
    } else {
        app.printing.settings.grayscale
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

pub fn feedback(message: &Feedback, translate: impl Fn(&str) -> String) -> String {
    match message {
        Feedback::FindingPrinters => translate("Finding printers…"),
        Feedback::NoPrinters => translate("No printers found. Configure a system printer or export a PDF."),
        Feedback::DiscoveryFailed => translate("Printer discovery failed."),
        Feedback::Submitting => translate("Submitting print job…"),
        Feedback::Submitted(receipt) if receipt.trim().is_empty() => translate("Print job submitted."),
        Feedback::Submitted(receipt) => translate("Print job submitted: {receipt}").replace("{receipt}", receipt),
        Feedback::Failed(error) => translate("Print failed: {error}").replace("{error}", error),
        Feedback::WorkerStopped => translate("Print failed: the printing worker stopped."),
    }
}

/// Build the upstream page-range/markup PDF and hand it to the chosen host print flow.
pub fn submit(app: &mut crate::WordApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::json;
    // The destination chosen on the Print page: a network printer by address, or the spooler.
    let network = app.ui.print.network && app.services.print_network.is_some();
    if app.services.print.is_none() && app.services.print_native.is_none() && !network {
        return Err("printing to the system print dialog isn't available here; export a PDF instead".into());
    }
    let native = app.services.print_native.is_some() || network;
    if network {
        if app.net_print.draft.running() {
            return Err("A print job is already being sent.".into());
        }
    } else if native && !app.printing.ready() {
        return Err("Select an available printer in File > Print before submitting.".into());
    }
    let mut pdf_params = params.clone();
    // The pages themselves are gray when Grayscale is chosen; the driver is asked for monochrome too.
    let gray = grayscale(app);
    if let Some(object) = pdf_params.as_object_mut() {
        object.insert("grayscale".into(), json!(gray));
    }
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
        && let Err(error) = app.session.run("references.updateFields", &json!({}))
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
        // The network printer takes its copies from its own section; the PDF has one.
        let copies = if network { 1 } else { settings.copies as usize };
        if native && plan.chosen.checked_mul(copies).is_none_or(|pages| pages > wordcraft_engine::print::MAX_PRINT_PAGES) {
            return Err("the selected pages and copies exceed the print-job page limit".into());
        }
        let len = bytes.len();
        if network {
            // The sheet size picks the printer's media when it lists it; the page count is the job's total.
            let lay = app.session.export_layout();
            let media =
                plan.pages.first().and_then(|&index| lay.pages.get(index)).and_then(|page| crate::network_print::media_for_points(page.w, page.h));
            if !crate::network_print::send(app, bytes, plan.pages.len(), media) {
                return Err("Query the printer, then tick the confirmation box in File > Print before sending.".into());
            }
        } else if let Some(submit) = &app.services.print_native {
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
            app.status(if network {
                tl!("Sending to the network printer…")
            } else if native {
                tl!("Submitting print job…")
            } else {
                tl!("Opening print dialog…")
            });
            Ok(json!({"printing": true, "bytes": len, "pending": native, "network": network}))
        }
        Err(error) => {
            log::error!("print failed: {error}");
            let msg = crate::i18n::fmt(tl!("Print failed: {error}"), &[("error", &error)]);
            app.status(msg.clone());
            Err(msg)
        }
    }
}
