//! Print > Network printer: the desktop's part. The model, rules and widgets are craft-ui's
//! (`printing::network`, drawn by `wordcraft_ui_egui::network_print`); the printing is craft-print's
//! WS-Print and IPP backends behind one private [`Registry`] per printer. This file supplies what a
//! host supplies: the address check, the worker that asks a printer what it lists, the worker that
//! sends a job and follows it, and the mapping of craft-print's answers onto craft-ui's light types.
//!
//! Nothing here discovers anything or connects on its own: a printer is contacted when the person
//! presses Query printer, and a document is sent only after the confirmation box was ticked (the
//! registry refuses a network job without a [`UserConfirmed`]). Addresses are numeric private,
//! link-local or loopback IPs, as craft-wsd's policy decides.
//!
//! The protocol follows the address: `ipp://...`, or a `/ipp/` path, or port 631 is IPP; anything
//! else is WS-Print. All network work runs on its own threads; the UI thread only starts it and
//! drains receivers.
//!
//! A job has two phases and so does its worker: it sends the document and reports **accepted** as
//! soon as the printer has it, then follows the job with craft-print's `JobWatch`, reporting each
//! new status until it ends. A document is sent once: a send whose outcome is unknown is shown with
//! the request to check the printer before sending again, and nothing retries it.

use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use craft_print::ipp::IppBackend;
use craft_print::ipp::policy::PrinterUri;
use craft_print::wsd::{PrinterUrl, WsdBackend};
use craft_print::{
    CancelToken, ColourMode, Document, ErrorKind, Job, JobHint, JobOptions, JobState, JobStatus, JobWatch, Orientation, Phase, PrintError,
    PrinterDetails, PrinterState, Quality, Registry, Resolution, SubmitRequest, UserConfirmed, WaitPolicy, WatchOutcome, WatchStep, namespaced_id,
};
use craft_ui::capability::Capability;
use craft_ui::printing::network as net;
use wordcraft_ui_egui::network_print::NetworkPrint;

/// Any transport that can be shared with the worker threads (tests give an in-memory printer).
pub type Transport = Arc<dyn craft_print::wsd::adapter::Transport + Send + Sync>;

const MAX_JOB_NAME: usize = 100;
/// How long a job is followed after the printer accepted it; it carries on at the printer after that.
const FOLLOW_FOR: Duration = Duration::from_secs(300);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Proto {
    Wsd,
    Ipp,
}

/// Which protocol an address means. A wrong guess is a printer that does not answer, never a job
/// sent anywhere else.
fn proto_of(address: &str) -> Proto {
    let lower = address.trim().to_ascii_lowercase();
    let rest = lower.split_once("://").map_or(lower.as_str(), |(_, rest)| rest);
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    let port_631 = authority.rsplit_once(':').is_some_and(|(_, port)| port == "631");
    if lower.starts_with("ipp://") || port_631 || path.split('/').any(|segment| segment == "ipp") { Proto::Ipp } else { Proto::Wsd }
}

/// The address checked against the network policy. The error is what the dialog shows under the field.
pub fn validate(text: &str) -> Result<(), net::Failure> {
    let refused = |e: String| net::Failure::new(net::FailureKind::Policy, e);
    match proto_of(text) {
        Proto::Ipp => PrinterUri::parse(text).map(|_| ()).map_err(|e| refused(e.to_string())),
        Proto::Wsd => PrinterUrl::parse(text).map(|_| ()).map_err(|e| refused(e.to_string())),
    }
}

/// What the workers are made with: the transport (`None`: plain HTTP). Tests give an in-memory printer.
#[derive(Clone, Default)]
pub struct Env {
    pub transport: Option<Transport>,
}

/// One printer: its checked address behind a registry that holds only that printer's backend.
struct Printer {
    id: String,
    registry: Registry,
}

impl Printer {
    /// Opens no connection.
    fn open(address: &str, env: &Env) -> Result<Self, PrintError> {
        let registry = Registry::new();
        let transport = env.transport.clone().unwrap_or_else(craft_print::wsd::adapter::default_transport);
        let (backend, id): (Box<dyn craft_print::backends::Backend>, String) = match proto_of(address) {
            Proto::Wsd => {
                let backend = WsdBackend::new().with_transport(Arc::clone(&transport));
                let id = backend.add(address)?;
                (Box::new(backend), namespaced_id("wsd", &id))
            }
            Proto::Ipp => {
                let backend = IppBackend::new().with_transport(Arc::clone(&transport));
                let id = backend.add(address)?;
                (Box::new(backend), namespaced_id("ipp", &id))
            }
        };
        registry.register(backend).map_err(|e| PrintError::new(ErrorKind::Failed, e.to_string()))?;
        Ok(Self { id, registry })
    }

    /// What the printer is and lists: one read-only request.
    fn details(&self) -> Result<PrinterDetails, PrintError> {
        self.registry.printer_details(&self.id)
    }
}

/// What the last query of each address read, so a job to that printer does not read it again.
type Queried = Arc<Mutex<Option<(String, PrinterDetails)>>>;

/// What went wrong, as craft-ui words it. `open`: the failure is the printer's address being refused.
fn failure(e: &PrintError, open: bool) -> net::Failure {
    use net::FailureKind as K;
    let kind = if e.native().is_some_and(|n| n.domain == "wsd-fault") {
        K::Fault
    } else if e.phase() == Some(Phase::Prepare) && e.kind() == ErrorKind::Failed {
        K::Encode
    } else {
        match e.kind() {
            ErrorKind::InvalidRequest if open => K::Policy,
            ErrorKind::InvalidRequest | ErrorKind::ConsentRequired => K::Invalid,
            ErrorKind::UnsupportedFormat => K::NoCommonFormat,
            ErrorKind::Unsupported => K::Unsupported,
            ErrorKind::Unavailable | ErrorKind::Timeout | ErrorKind::Busy | ErrorKind::Io => K::Transport,
            ErrorKind::Failed => K::Protocol,
            ErrorKind::Rejected | ErrorKind::NotFound => K::Fault,
            ErrorKind::Cancelled => K::Stopped,
            _ => K::Other,
        }
    };
    net::Failure::new(kind, e.message()).with_outcome_unknown(e.outcome_unknown())
}

fn state_name(state: PrinterState) -> &'static str {
    match state {
        PrinterState::Idle => "Idle",
        PrinterState::Processing => "Processing",
        PrinterState::Stopped => "Stopped",
        PrinterState::Offline => "Offline",
        _ => "Unknown",
    }
}

/// What the printer published, as craft-ui's three-state `Caps` (the mapping table of craft-ui's
/// README). A list the printer left out is empty here, which reads as unknown, not unsupported: the
/// control stays enabled with a note and the value is sent as a hint.
fn caps_of(details: &PrinterDetails) -> net::Caps {
    let honours = |hint: JobHint| details.honoured.as_ref().is_none_or(|list| list.contains(&hint));
    let hinted = |hint: JobHint| if honours(hint) { Capability::Unknown } else { Capability::Unsupported };
    let tokens = |list: &[String]| if list.is_empty() { Capability::Unknown } else { Capability::from_option_list(Some(list)) };
    let resolutions: Vec<net::Resolution> = details.resolutions.iter().map(|r| net::Resolution::new(r.x_dpi, r.y_dpi)).collect();
    net::Caps {
        formats: details.formats.iter().map(|f| f.mime().to_owned()).collect(),
        sides: match details.two_sided {
            None => Capability::Unknown,
            Some(true) => Capability::from_option_list(Some(&[
                net::Sides::OneSided.token().to_owned(),
                net::Sides::LongEdge.token().to_owned(),
                net::Sides::ShortEdge.token().to_owned(),
            ])),
            Some(false) => Capability::from_option_list(Some(&[net::Sides::OneSided.token().to_owned()])),
        },
        orientations: hinted(JobHint::Orientation),
        qualities: hinted(JobHint::Quality),
        color_modes: match details.colour {
            None => Capability::Unknown,
            Some(true) => {
                Capability::from_option_list(Some(&[net::ColorMode::Color.token().to_owned(), net::ColorMode::Monochrome.token().to_owned()]))
            }
            Some(false) => Capability::from_option_list(Some(&[net::ColorMode::Monochrome.token().to_owned()])),
        },
        media_sizes: tokens(&details.paper),
        resolutions: if resolutions.is_empty() { Capability::Unknown } else { Capability::from_option_list(Some(&resolutions)) },
        copies: Capability::from_option_range(details.max_copies.map(|max| (1, max))),
    }
}

fn summary(details: &PrinterDetails) -> net::PrinterSummary {
    let status = details.status.as_ref();
    net::PrinterSummary {
        name: details.name.clone(),
        color: details.colour,
        state: status.map_or_else(String::new, |s| state_name(s.state).to_owned()),
        queued_jobs: None,
        reasons: status.map(|s| s.reasons.clone()).unwrap_or_default(),
        caps: caps_of(details),
    }
}

/// The worker that asks the printer at the (normalised) address what it is and what it lists. It
/// sends no job.
fn query_printer(env: Env, queried: Queried) -> net::QueryPrinter {
    Box::new(move |address: &str| {
        let (tx, rx) = mpsc::channel();
        let (address, env, queried) = (address.to_owned(), env.clone(), Arc::clone(&queried));
        std::thread::spawn(move || {
            let result = match Printer::open(&address, &env) {
                Ok(printer) => printer
                    .details()
                    .map(|details| {
                        let shown = summary(&details);
                        *lock(&queried) = Some((printer.id.clone(), details));
                        shown
                    })
                    .map_err(|e| failure(&e, false)),
                Err(e) => Err(failure(&e, true)),
            };
            let _ = tx.send(result);
        });
        rx
    })
}

/// The job and its per-job options for the registry.
fn options(request: &net::Request, job_name: &str) -> (Job, JobOptions) {
    let name: String = job_name.chars().filter(|c| !c.is_control()).take(MAX_JOB_NAME).collect();
    let mut job = Job::default();
    if !name.trim().is_empty() {
        job.title = name;
    }
    job.copies = request.copies.unwrap_or(1);
    job.duplex = request.sides.map(|s| match s {
        net::Sides::OneSided => craft_print::Duplex::Off,
        net::Sides::LongEdge => craft_print::Duplex::LongEdge,
        net::Sides::ShortEdge => craft_print::Duplex::ShortEdge,
    });
    let mut options = JobOptions::new();
    if let Some(o) = request.orientation {
        options = options.with_orientation(match o {
            net::Orientation::Portrait => Orientation::Portrait,
            net::Orientation::Landscape => Orientation::Landscape,
            net::Orientation::ReversePortrait => Orientation::ReversePortrait,
            net::Orientation::ReverseLandscape => Orientation::ReverseLandscape,
        });
    }
    match request.quality {
        Some(net::Quality::Draft) => options = options.with_quality(Quality::Draft),
        Some(net::Quality::Normal) => options = options.with_quality(Quality::Normal),
        Some(net::Quality::High) => options = options.with_quality(Quality::High),
        Some(net::Quality::Photo) => options = options.with_quality(Quality::Photo),
        None => {}
    }
    if let Some(color) = request.color {
        options = options.with_colour(match color {
            net::ColorMode::Color => ColourMode::Colour,
            net::ColorMode::Monochrome => ColourMode::Mono,
        });
    }
    if let Some(r) = request.resolution {
        options = options.with_resolution(Resolution::new(r.x_dpi, r.y_dpi));
    }
    if let Some(media) = &request.media_size {
        options = options.with_media(media.as_str());
    }
    if let Some(pages) = request.expected_pages {
        options = options.with_expected_pages(pages);
    }
    (job, options)
}

/// What the printer says about an accepted job, as craft-ui shows it.
fn progress(status: &JobStatus) -> net::Progress {
    net::Progress {
        stage: net::Stage::from_name(status.state.name()),
        sheets_done: status.sheets_done,
        sheets_total: status.sheets_total,
        notes: status.reasons.clone(),
    }
}

/// How following a job ended, as craft-ui's report. The document was accepted in every case, so a
/// watch that could not finish is "sent, outcome unknown", never a failure.
fn report(outcome: &WatchOutcome, printer: &str, warnings: &[String]) -> net::Report {
    use net::Outcome as O;
    let (kind, status) = match outcome {
        WatchOutcome::Terminal(s) => (
            match s.state {
                // craft-print models "completed with warnings" as Completed plus warnings on the status.
                JobState::Completed if !s.warnings.is_empty() => O::CompletedWithWarnings,
                JobState::Completed => O::Completed,
                JobState::CompletedWithErrors => O::CompletedWithErrors,
                JobState::Cancelled => O::Canceled,
                JobState::Aborted => O::Aborted,
                _ => O::AcceptedUnknown,
            },
            Some(s),
        ),
        WatchOutcome::Forgotten { last } => (O::Forgotten, last.as_ref()),
        WatchOutcome::TimedOut { last } => (O::StillRunning, last.as_ref()),
        _ => (O::AcceptedUnknown, None),
    };
    let mut notes: Vec<String> = status.map(|s| s.reasons.clone()).unwrap_or_default();
    notes.extend(warnings.iter().cloned());
    if let WatchOutcome::Failed(e) = outcome {
        notes.push(e.message().to_owned());
    }
    net::Report { outcome: kind, printer: printer.to_owned(), sheets: status.and_then(|s| s.sheets_done).map_or(0, u64::from), notes }
}

/// The worker that sends the print-ready `pdf` and follows the job. `queried` is what the last
/// query read of the printer, when it is the one addressed. Each call makes a new job, with the
/// person's `confirmed` (the attestation of the ticked box, taken when Print was pressed).
fn send_job(env: Env, name: String, pdf: Arc<Vec<u8>>, queried: Option<(String, PrinterDetails)>, confirmed: UserConfirmed) -> net::SendJob {
    Box::new(move |address: &str, request: net::Request| {
        let (result_tx, result_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        let cancel = CancelToken::new();
        let (address, env) = (address.to_owned(), env.clone());
        let (name, pdf, queried, worker_cancel) = (name.clone(), Arc::clone(&pdf), queried.clone(), cancel.clone());
        std::thread::spawn(move || {
            let (job, options) = options(&request, &name);
            let result = run_job(&address, &env, (job, options), &pdf, queried, confirmed, &worker_cancel, &update_tx);
            let _ = result_tx.send(result);
        });
        net::JobHandle::new(result_rx, Box::new(move || cancel.cancel())).with_updates(update_rx)
    })
}

#[allow(clippy::too_many_arguments)]
fn run_job(
    address: &str,
    env: &Env,
    (job, options): (Job, JobOptions),
    pdf: &Arc<Vec<u8>>,
    queried: Option<(String, PrinterDetails)>,
    confirmed: UserConfirmed,
    cancel: &CancelToken,
    updates: &mpsc::Sender<net::JobUpdate>,
) -> Result<net::Report, net::Failure> {
    let printer = Printer::open(address, env).map_err(|e| failure(&e, true))?;
    // What the person was shown is what the registry decides on: no second read of the printer,
    // unless nothing was queried.
    let details = match queried.filter(|(id, _)| *id == printer.id) {
        Some((_, details)) => details,
        None => printer.details().map_err(|e| failure(&e, false))?,
    };
    let printer_name = if details.name.is_empty() { printer.id.clone() } else { details.name.clone() };
    let job = Job { printer: Some(printer.id.clone()), ..job };
    // Phase one: the printer accepts the document. A cancel before that sends nothing, or cancels the job just made.
    let request = SubmitRequest::new(Document::pdf(pdf.as_slice()), job).with_options(options).with_consent(confirmed).with_cancel(cancel.clone());
    let submitted = match printer.registry.submit_job_with_details(request, Some(details)) {
        Ok(submitted) => submitted,
        Err(e) if e.kind() == ErrorKind::Cancelled => {
            return Ok(net::Report { outcome: net::Outcome::Canceled, printer: String::new(), sheets: 0, notes: Vec::new() });
        }
        Err(e) => return Err(failure(&e, false)),
    };
    let notes = submitted.warnings.clone();
    let _ = updates.send(net::JobUpdate::Accepted(net::Progress { notes: notes.clone(), ..net::Progress::default() }));
    // Phase two: follow it. One cancel is sent if asked; the watch then carries on to the end state.
    let policy = WaitPolicy { interval: Some(Duration::from_millis(500)), ..WaitPolicy::default() }.with_max_wait(FOLLOW_FOR);
    let outcome = match JobWatch::for_submitted(&submitted, policy) {
        Some(mut watch) => {
            watch = watch.with_cancel(cancel.clone());
            loop {
                match watch.tick(&printer.registry) {
                    WatchStep::Pending { next_in } => std::thread::sleep(next_in.min(Duration::from_millis(100))),
                    WatchStep::Update(status) => {
                        let _ = updates.send(net::JobUpdate::Status(progress(&status)));
                    }
                    WatchStep::Done(outcome) => break outcome,
                    _ => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
        None => WatchOutcome::NoStatus,
    };
    Ok(report(&outcome, &printer_name, &notes))
}

/// The service the Print page uses.
pub fn service_with(env: Env) -> NetworkPrint {
    let queried = Queried::default();
    let query = query_printer(env.clone(), Arc::clone(&queried));
    NetworkPrint {
        validate,
        query,
        send: Box::new(move |name, pdf, confirmed| {
            // The attestation comes from the ticked box and nothing else.
            let confirmed = UserConfirmed::attest(Some(confirmed)).ok()?;
            let address = lock(&queried).clone();
            Some(send_job(env.clone(), name, pdf, address, confirmed))
        }),
    }
}

/// The service for the running app: the plain-HTTP transport.
pub fn service() -> NetworkPrint {
    service_with(Env::default())
}

#[cfg(test)]
mod tests;
