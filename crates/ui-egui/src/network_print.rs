//! File > Print > Network printer: the app's part of `craft_ui::printing::network`.
//!
//! The model (address, query, three-state options, confirmation, job view) and the widgets are
//! craft-ui's. The network itself is the host's: the desktop app hands in [`NetworkPrint`] (the
//! address check, the printer query and the job worker, built on craft-print's WS-Print and IPP
//! backends), so this crate stays free of sockets and builds for the web unchanged. Without the
//! service the destination does not exist.
//!
//! The document is the print-ready PDF the engine already makes for the system spooler. Copies,
//! two-sided printing and the other options are the printer's, chosen in the section after the
//! printer has answered; the PDF is made with one copy.

use std::sync::Arc;

use craft_ui::printing::network as net;

/// The media types the Print page produces. A printer that lists none of them blocks Print.
pub const FORMATS: &[&str] = &["application/pdf"];

/// What the host supplies for printing to a network printer by address.
pub struct NetworkPrint {
    /// Checks the normalised address against the host's network policy (numeric private,
    /// link-local or loopback addresses only). The error is shown under the address field.
    pub validate: fn(&str) -> Result<(), net::Failure>,
    /// Asks the printer at an address what it is and lists. Sends no job.
    pub query: net::QueryPrinter,
    /// A job worker for `(document name, PDF, confirmed)`, or `None` when the person has not
    /// confirmed. Called once per job; the worker sends the document and follows the job.
    #[allow(clippy::type_complexity)]
    pub send: Box<dyn Fn(String, Arc<Vec<u8>>, bool) -> Option<net::SendJob>>,
}

/// The wording of the destination, in the interface language. Templates keep their `{...}` holes.
pub fn labels() -> net::Labels<'static> {
    net::Labels {
        address: tl!("Printer address:"),
        address_hint: "http://192.168.1.50:5357/service",
        address_note: tl!(
            "Only IP addresses on this network are accepted (private, link-local or loopback), not host names. ipp:// or an /ipp/ path means IPP; anything else is WS-Print."
        ),
        query_printer: tl!("Query printer"),
        querying: tl!("Asking the printer…"),
        query_first: tl!("Query the printer first to see what it supports."),
        printer_status: tl!("Printer status: {state}"),
        reasons: tl!("Reasons: {reasons}"),
        no_common_format: tl!("The printer accepts none of the formats this document can be sent in"),
        copies: tl!("Copies:"),
        sides: tl!("Two-sided:"),
        sides_off: tl!("Off"),
        sides_long_edge: tl!("Flip on long edge"),
        sides_short_edge: tl!("Flip on short edge"),
        orientation: tl!("Printer orientation:"),
        portrait: tl!("Portrait"),
        landscape: tl!("Landscape"),
        reverse_portrait: tl!("Reverse portrait"),
        reverse_landscape: tl!("Reverse landscape"),
        quality: tl!("Quality:"),
        draft: tl!("Draft"),
        normal: tl!("Normal"),
        high: tl!("High"),
        photo: tl!("Photo"),
        color: tl!("Color:"),
        color_color: tl!("Colour"),
        color_monochrome: tl!("Grayscale"),
        resolution: tl!("Resolution:"),
        resolution_dpi: "{dpi} dpi",
        resolution_dpi_xy: "{x} x {y} dpi",
        auto: tl!("Auto"),
        unsupported_copies: tl!("This printer does not take a copies setting."),
        unsupported_sides: tl!("This printer only prints one-sided."),
        unsupported_orientation: tl!("This printer does not take an orientation setting."),
        unsupported_quality: tl!("This printer does not take a quality setting."),
        unsupported_color: tl!("This printer only prints in grayscale."),
        unsupported_resolution: tl!("This printer does not take a resolution setting."),
        unconfirmed_note: tl!("Not confirmed by the printer"),
        confirm: tl!("I confirm sending this document to this printer"),
        sending: tl!("Sending to {printer}…"),
        canceling: tl!("Canceling…"),
        cancel: tl!("Cancel"),
        sent: tl!("Sent to {printer}"),
        accepted: tl!("Accepted by {printer}"),
        stage_waiting: tl!("Waiting in the printer's queue"),
        stage_held: tl!("Held by the printer"),
        stage_printing: tl!("The printer is printing it"),
        stage_stopped: tl!("The printer paused the job; it needs attention"),
        sheets: tl!("Sheets printed: {n}"),
        sheets_progress: tl!("Sheets printed: {done} of {total}"),
        outcome_warnings: tl!("Printed with warnings"),
        outcome_errors: tl!("Printed with errors"),
        outcome_canceled: tl!("The job was canceled"),
        outcome_aborted: tl!("The printer stopped the job"),
        outcome_forgotten: tl!("The printer no longer lists the job; it most likely finished"),
        outcome_still_running: tl!("Still printing; the job continues on the printer"),
        outcome_accepted_unknown: tl!("Sent. The printer can't say whether it printed."),
        outcome_unknown_warning: tl!("The printer may have received the document. Check the printer before sending again."),
        failure_policy: tl!("The printer address was refused"),
        failure_transport: tl!("Couldn't reach the printer"),
        failure_protocol: tl!("The printer's reply couldn't be understood"),
        failure_fault: tl!("The printer refused the request"),
        failure_no_common_format: tl!("The printer accepts none of the formats this document can be sent in"),
        failure_unsupported: tl!("The printer doesn't support this setting"),
        failure_encode: tl!("The document couldn't be prepared"),
        failure_invalid: tl!("A setting isn't valid"),
        failure_stopped: tl!("The print job stopped unexpectedly"),
        failure_other: tl!("The print job stopped unexpectedly"),
    }
}

/// The `MediaSizeName` of a sheet of `width` x `height` points, for the sizes printers list.
pub fn media_for_points(width: f32, height: f32) -> Option<&'static str> {
    const SIZES: [(f32, f32, &str); 6] = [
        (612.0, 792.0, "na_letter_8.5x11in"),
        (612.0, 1008.0, "na_legal_8.5x14in"),
        (792.0, 1224.0, "na_ledger_11x17in"),
        (841.9, 1190.6, "iso_a3_297x420mm"),
        (595.3, 841.9, "iso_a4_210x297mm"),
        (419.5, 595.3, "iso_a5_148x210mm"),
    ];
    let (short, long) = if width <= height { (width, height) } else { (height, width) };
    SIZES.iter().find(|(w, h, _)| (short - w).abs() <= 3.0 && (long - h).abs() <= 3.0).map(|(_, _, name)| *name)
}

/// Draw the destination's section; the response says whether Print may start.
pub fn show(app: &mut crate::WordApp, ui: &mut egui::Ui, sheets_ok: bool) -> Option<net::Block> {
    let Some(service) = &app.services.print_network else { return Some(net::Block::Address) };
    let host = net::Host { validate: &service.validate, query: &service.query, sheets_ok, formats: FORMATS };
    app.net_print.show(ui, egui::Id::new("wordcraft-network-printer"), &host, &labels()).block
}

/// Start the job for the print-ready `pdf` of `pages` sheets (`media`: the sheet size, when the
/// printers' list has it). It needs the person's confirmation, which it spends. `true` when a job
/// started.
pub fn send(app: &mut crate::WordApp, pdf: Vec<u8>, pages: usize, media: Option<&str>) -> bool {
    let Some(service) = &app.services.print_network else { return false };
    let name = app.title_stem();
    let address = app.net_print.draft.normalized_url();
    let Some(worker) = (service.send)(name, Arc::new(pdf), app.net_print.draft.confirmed) else { return false };
    let address_ok = (service.validate)(&address).is_ok();
    // The sheets are the pages of the document sent: the total shown until the printer reports one.
    app.net_print.draft.expected_pages = u32::try_from(pages).ok();
    app.net_print.start_job(&worker, address_ok, true, FORMATS, media)
}

/// Collect finished work (each frame), also while the Print page is closed: with the page gone the
/// person still hears how a job went, in the status bar, naming the printer.
pub fn poll(app: &mut crate::WordApp, ctx: &egui::Context) {
    let was_running = app.net_print.draft.running();
    app.net_print.poll(ctx);
    if !was_running || app.net_print.draft.running() || (app.ui.backstage && app.ui.backstage_page == "print") {
        return;
    }
    let labels = labels();
    let message = match &app.net_print.draft.job {
        net::JobView::Finished(report) => {
            Some(labels.outcome_text(report.outcome).map_or_else(|| crate::i18n::fmt(labels.sent, &[("printer", &report.printer)]), str::to_owned))
        }
        net::JobView::Failed(failure) => {
            let mut text = format!("{}: {}", labels.failure_heading(failure.kind), failure.detail);
            if failure.outcome_unknown {
                text = format!("{text} {}", labels.outcome_unknown_warning);
            }
            Some(text)
        }
        _ => None,
    };
    if let Some(message) = message {
        app.status(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_sizes_map_to_media_names_in_either_orientation() {
        assert_eq!(media_for_points(612.0, 792.0), Some("na_letter_8.5x11in"));
        assert_eq!(media_for_points(842.0, 595.0), Some("iso_a4_210x297mm"));
        assert_eq!(media_for_points(500.0, 500.0), None);
    }
}
