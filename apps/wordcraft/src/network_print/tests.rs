//! The network print workers against craft-print's in-memory WS-Print printer. No sockets, no real
//! printer: every address here is a documentation or loopback one answered by the fake.
use super::*;
use craft_print::wsd::testing::{Ending, FakeConfig, FakePrinter, MemTransport};
use std::time::Duration;

const ADDRESS: &str = "http://127.0.0.1:5357/print";
const WAIT: Duration = Duration::from_secs(20);

fn env_for(printer: &Arc<FakePrinter>) -> Env {
    Env { transport: Some(Arc::new(MemTransport(Arc::clone(printer)))) }
}

fn request() -> net::Request {
    net::Request { copies: Some(2), sides: Some(net::Sides::OneSided), expected_pages: Some(1), ..Default::default() }
}

#[test]
fn the_address_picks_the_protocol() {
    for (address, proto) in [
        ("http://192.168.1.50:5357/service", Proto::Wsd),
        ("http://192.168.1.50/", Proto::Wsd),
        ("ipp://192.168.1.50/ipp/print", Proto::Ipp),
        ("IPP://192.168.1.50", Proto::Ipp),
        ("http://192.168.1.50:631/printers/office", Proto::Ipp),
        ("http://192.168.1.50:8080/ipp/print", Proto::Ipp),
        ("http://192.168.1.50:5357/ippy", Proto::Wsd),
    ] {
        assert_eq!(proto_of(address), proto, "{address}");
    }
}

#[test]
fn only_numeric_local_addresses_pass_the_policy() {
    for good in ["http://127.0.0.1:5357/print", "http://192.168.1.50:5357/service", "ipp://10.0.0.7/ipp/print", "http://[::1]:5357/x"] {
        assert!(validate(good).is_ok(), "{good}");
    }
    for bad in [
        "http://printer.local:5357/",
        "http://8.8.8.8:5357/",
        "ipp://printer.example/ipp/print",
        "ipps://192.168.1.2/ipp/print",
        "ftp://192.168.1.2/",
        "",
    ] {
        let error = validate(bad).expect_err(bad);
        assert_eq!(error.kind, net::FailureKind::Policy, "{bad}");
    }
}

#[test]
fn a_query_reads_the_printer_and_sends_no_job() {
    let fake = FakePrinter::new(FakeConfig { sides: vec!["OneSided"], max_copies: 5, color: false, state: "Idle", ..FakeConfig::default() });
    let service = service_with(env_for(&fake));
    let summary = (service.query)(ADDRESS).recv_timeout(WAIT).expect("an answer").expect("the printer answers");
    assert!(summary.caps.formats.iter().any(|f| f == "application/pdf"), "{:?}", summary.caps.formats);
    assert_eq!(summary.state, "Idle");
    assert_eq!(summary.color, Some(false));
    assert!(matches!(summary.caps.copies, Capability::Supported(_)), "copies come from the printer's maximum");
    assert!(
        matches!(summary.caps.orientations, Capability::Unknown | Capability::Supported(_)),
        "no field for orientation: unknown, not unsupported"
    );
    assert_eq!(fake.jobs(), 0, "a query creates no job");
    assert!(!fake.actions().iter().any(|a| a.contains("CreatePrintJob") || a.contains("SendDocument")), "{:?}", fake.actions());
}

#[test]
fn a_confirmed_job_is_sent_once_accepted_then_followed_to_the_end() {
    let fake = FakePrinter::new(FakeConfig::default());
    let service = service_with(env_for(&fake));
    (service.query)(ADDRESS).recv_timeout(WAIT).expect("an answer").expect("queried");
    let pdf = Arc::new(b"%PDF-1.7\n%wordcraft test\n".to_vec());
    let worker = (service.send)("My letter".into(), Arc::clone(&pdf), true).expect("a confirmed job gets a worker");
    let handle = worker(ADDRESS, request());
    let report = handle.result.recv_timeout(WAIT).expect("the worker ends").expect("the job was accepted");
    assert_eq!(report.outcome, net::Outcome::Completed, "{report:?}");
    let updates: Vec<_> = handle.updates.expect("updates").try_iter().collect();
    assert!(matches!(updates.first(), Some(net::JobUpdate::Accepted(_))), "accepted comes first: {updates:?}");
    assert_eq!(fake.jobs(), 1, "one document, sent once");
    let job = fake.job(1).expect("the printer has the job");
    assert_eq!(job.document.as_deref(), Some(pdf.as_slice()), "the print-ready PDF is what was sent");
}

#[test]
fn an_unconfirmed_job_gets_no_worker() {
    let fake = FakePrinter::new(FakeConfig::default());
    let service = service_with(env_for(&fake));
    assert!((service.send)("x".into(), Arc::new(Vec::new()), false).is_none());
    assert!(fake.log().is_empty(), "nothing reached the printer");
}

#[test]
fn a_printer_that_aborts_the_job_is_reported_as_stopped_by_the_printer() {
    let fake = FakePrinter::new(FakeConfig { ending: Ending::Aborts("PrintJobAborted"), ..FakeConfig::default() });
    let service = service_with(env_for(&fake));
    let worker = (service.send)("x".into(), Arc::new(b"%PDF-1.7\n".to_vec()), true).expect("worker");
    let report = worker(ADDRESS, request()).result.recv_timeout(WAIT).expect("ends").expect("accepted");
    assert_eq!(report.outcome, net::Outcome::Aborted);
}

#[test]
fn a_refused_address_never_reaches_a_transport() {
    let fake = FakePrinter::new(FakeConfig::default());
    let service = service_with(env_for(&fake));
    let failure = (service.query)("http://printer.example:5357/").recv_timeout(WAIT).expect("an answer").expect_err("refused");
    assert_eq!(failure.kind, net::FailureKind::Policy);
    assert!(fake.log().is_empty());
}

#[test]
fn a_printer_that_lists_no_pdf_gets_no_pdf() {
    let fake = FakePrinter::new(FakeConfig { formats: vec!["image/pwg-raster"], ..FakeConfig::default() });
    let service = service_with(env_for(&fake));
    let summary = (service.query)(ADDRESS).recv_timeout(WAIT).expect("an answer").expect("queried");
    assert!(!summary.takes_any(wordcraft_ui_egui::network_print::FORMATS), "the dialog blocks Print for it");
    let worker = (service.send)("x".into(), Arc::new(b"%PDF-1.7\n".to_vec()), true).expect("worker");
    let failure = worker(ADDRESS, request()).result.recv_timeout(WAIT).expect("ends").expect_err("no converter, no common format");
    assert_eq!(failure.kind, net::FailureKind::NoCommonFormat);
    assert_eq!(fake.jobs(), 0);
}
