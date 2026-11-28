//! Native print onboarding preserves engine PDF generation and submits the selected settings.
use craft::automation::control::{ControlRequest, Reply};
use craft::host::InlineJobs;
use craft::print;
use craft::print::spool::{Pending, Spool, SpoolBackend};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use wordcraft_engine::Session;
use wordcraft_ui_egui::control::handle;
use wordcraft_ui_egui::printing::{Feedback, Printer};
use wordcraft_ui_egui::{Services, WordApp};

/// What the status bar says, as `ui.inspect` reports it; empty once the message has expired.
fn status_text(app: &mut WordApp) -> String {
    let (request, _) = ControlRequest::new("ui.inspect", json!({}));
    let Reply::Done(reply) = handle(app, &egui::Context::default(), &request) else { panic!("inspect replies immediately") };
    reply["result"]["status"]["text"].as_str().unwrap_or_default().to_owned()
}

/// The Print page's settings as `ui.printPage` reports them.
fn page_of(app: &mut WordApp) -> serde_json::Value {
    app.run("ui.printPage", json!({})).unwrap()
}

/// A spooler that keeps every job it is given and answers each with `outcome`.
struct Fake {
    printers: Vec<print::Printer>,
    outcome: Result<String, String>,
    sent: Mutex<Vec<(Vec<u8>, print::Job)>>,
    discoveries: AtomicUsize,
}

impl Fake {
    /// A spooler offering one printer, `Queue`, that accepts every job with a receipt.
    fn accepting() -> Arc<Fake> {
        Fake::answering(Ok("queued".into()))
    }

    /// A spooler offering one printer, `Queue`, that answers every job with `outcome`.
    fn answering(outcome: Result<String, String>) -> Arc<Fake> {
        Arc::new(Fake {
            printers: vec![print::Printer { name: "Queue".into(), description: String::new(), default: true, two_sided: None }],
            outcome,
            sent: Mutex::new(Vec::new()),
            discoveries: AtomicUsize::new(0),
        })
    }

    /// How many jobs reached the spooler.
    fn jobs(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    /// The PDF of the latest job.
    fn last_pdf(&self) -> Vec<u8> {
        self.sent.lock().unwrap().last().unwrap().0.clone()
    }

    /// The options of the latest job.
    fn last_options(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().last().unwrap().1.options.clone()
    }
}

impl SpoolBackend for Fake {
    fn printers(&self) -> Option<Vec<print::Printer>> {
        self.discoveries.fetch_add(1, Ordering::SeqCst);
        Some(self.printers.clone())
    }

    fn prepare(&self, pdf: Vec<u8>, job: print::Job) -> Pending {
        self.sent.lock().unwrap().push((pdf, job));
        let outcome = self.outcome.clone();
        Pending { cancel: Box::new(|| true), run: Box::new(move || outcome.map_err(print::Error::Spool)), waits_after_cancel: false }
    }
}

/// An app whose spool is `fake`, with a blank document.
fn app_over(fake: &Arc<Fake>) -> WordApp {
    app_with(Session::new(wordcraft_doc::Document::new()), fake)
}

/// An app on `session` whose spool is `fake`.
fn app_with(session: Session, fake: &Arc<Fake>) -> WordApp {
    let mut app = WordApp::new(session, Services::default());
    app.services.print = Some(Spool::with_backend(Arc::new(InlineJobs), fake.clone()));
    app
}

/// Discovery has found `Queue` and the app has chosen it.
fn choose_queue(app: &mut WordApp, params: serde_json::Value) {
    app.printing.printers.push(Printer::new("Queue").is_default(true));
    let mut settings = json!({"printer": "Queue"});
    settings.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    app.run("ui.printSettings", settings).unwrap();
}

/// Runs the app's frame hook before each step, as the host does before every frame.
fn before_frames(h: &mut egui_kittest::Harness<'_, WordApp>, frames: usize) {
    for _ in 0..frames {
        let ctx = h.ctx.clone();
        craft::ui::shell::embed::before_input(&ctx, |output| h.state_mut().frame_input(output));
        h.step();
    }
}

/// Print command metadata lists engine and app commands, exposes the settings and status fields,
/// and keeps submission disabled until a printer is selected. It never discovers printers,
/// submits, opens a dialog or Print page, or dirties the document.
#[test]
fn control_print_discovery_reports_metadata_without_host_side_effects() {
    let fake = Fake::accepting();
    let mut app = app_over(&fake);
    let ctx = egui::Context::default();
    let (request, _) = ControlRequest::new("engine.commands", json!({"search":"print"}));
    let Reply::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("metadata replies immediately") };
    assert_eq!(reply["ok"], true);
    let commands = reply["result"].as_array().unwrap();
    assert!(commands.iter().any(|command| command["id"] == "file.print"), "engine print commands remain visible");
    let settings = commands.iter().find(|command| command["id"] == "ui.printSettings").unwrap();
    assert_eq!(settings["enabled"], true, "settings/status inspection is available");
    for field in ["printer", "copies", "duplex", "grayscale", "refresh", "status", "detail"] {
        assert!(settings["params"].as_str().unwrap().contains(field), "documented setting: {field}");
    }
    for id in ["ui.printSubmit", "ui.print"] {
        let command = commands.iter().find(|command| command["id"] == id).unwrap();
        assert_eq!(command["enabled"], false, "a printer must be selected before submission");
        for field in ["range", "pages", "parity", "copies", "collate"] {
            assert!(command["params"].as_str().unwrap().contains(field), "{id}: {field}");
        }
    }
    app.printing.printers.push(Printer::new("Queue").is_default(true));
    app.run("ui.printSettings", json!({"printer":"Queue"})).unwrap();
    let Reply::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("metadata replies immediately") };
    for id in ["ui.printSubmit", "ui.print"] {
        let command = reply["result"].as_array().unwrap().iter().find(|command| command["id"] == id).unwrap();
        assert_eq!(command["enabled"], true, "the selected queue enables {id}");
    }
    assert_eq!(fake.discoveries.load(Ordering::SeqCst), 0, "metadata never starts printer discovery");
    assert_eq!(fake.jobs(), 0, "metadata never submits");
    assert!(app.dialog_host.is_empty() && !app.ui.backstage, "metadata never opens a dialog or Print page");
    assert!(!app.printing.busy() && !app.session.dirty);
}

/// Submission passes the current grayscale setting and collates by default; a spool failure
/// becomes Failed feedback and a status message.
#[test]
fn native_print_uses_selected_settings_and_reports_spool_errors() {
    let fake = Fake::answering(Err("printer is offline".into()));
    let mut app = app_over(&fake);
    app.run("text.insert", json!({"text": "A document to print"})).unwrap();
    assert!(app.run("ui.print", json!({})).is_err());
    assert_eq!(fake.jobs(), 0, "opening settings/exporting does not spool");
    app.printing.printers.push(Printer::new("Chosen queue"));
    let configured = app.run("ui.printSettings", json!({"printer":"Chosen queue", "copies":3,"duplex":"shortEdge","grayscale":true})).unwrap();
    assert_eq!(configured["ready"], true);
    assert_eq!(configured["printers"][0]["name"], "Chosen queue");
    app.run("ui.print", json!({})).unwrap();
    assert!(app.printing.busy());
    assert!(!app.close_requested(), "a pending print job cannot be interrupted by normal close");
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].0.starts_with(b"%PDF"));
    assert_eq!(sent[0].1.printer.as_deref(), Some("Chosen queue"));
    assert_eq!(sent[0].1.copies, 3);
    assert_eq!(sent[0].1.duplex, Some(print::Duplex::ShortEdge));
    assert!(sent[0].1.grayscale);
    assert!(sent[0].1.collate, "copies are collated by default");
    app.poll_printing(&egui::Context::default());
    assert_eq!(app.printing.message, Some(Feedback::Failed("printer is offline".into())));
    assert!(status_text(&mut app).contains("printer is offline"));
}

/// At 800 by 700 the print page keeps its settings inside the page, beside the preview and not over it;
/// Refresh and Grayscale take a click, and Grayscale updates the settings and readiness.
#[test]
fn print_settings_are_visible_in_a_narrow_window() {
    use egui_kittest::kittest::{By, Queryable};
    let fake = Fake::accepting();
    let mut app = app_over(&fake);
    // The labels below are English; an integration test otherwise follows the machine's language.
    app.ui.language = "en".into();
    app.run("text.insert", json!({"text": "A document to print"})).unwrap();
    app.ui.backstage = true;
    app.ui.backstage_page = "print".into();
    let mut harness = crate::harness::builder().with_size(egui::vec2(800.0, 700.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    before_frames(&mut harness, 5);
    let page = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0));
    // The panel and the Print button share the label "Print"; the panel is the larger of the two.
    let panel = harness
        .query_all(By::new().label("Print"))
        .map(|node| node.rect())
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("the settings panel is shown");
    // The framework's preview pane is a group named for the sheet (the page of a picture's own role is not used).
    let preview = harness.get(By::new().role(egui::accesskit::Role::Group).label("Sheet 1 of 1")).rect();
    assert!(page.contains_rect(panel), "{panel:?} inside {page:?}");
    assert!(!panel.intersects(preview), "{panel:?} beside the preview {preview:?}");
    let discoveries = fake.discoveries.load(Ordering::SeqCst);
    harness.get_by_label("Refresh").click();
    harness.run_steps(3);
    assert!(fake.discoveries.load(Ordering::SeqCst) > discoveries, "Refresh takes a click");
    harness.get_by_label("Grayscale").click();
    harness.run_steps(2);
    assert!(harness.state().printing.settings.grayscale);
    assert!(harness.state().printing.ready());
}

/// Every invalid settings payload is rejected and leaves the previous settings unchanged.
#[test]
fn invalid_print_settings_are_rejected_without_partial_changes() {
    let mut app = app_over(&Fake::accepting());
    app.printing.printers.push(Printer::new("Queue").is_default(true));
    let before = app.run("ui.printSettings", json!({})).unwrap();
    for params in [
        json!({"printer":"Missing"}),
        json!({"copies":0}),
        json!({"copies":1000}),
        json!({"copies":1.5}),
        json!({"copies":-1}),
        json!({"copies":"3"}),
        json!({"duplex":"maybe"}),
        json!({"grayscale":"yes"}),
        json!({"refresh":"yes"}),
        json!({"copies":3,"duplex":"invalid"}),
        json!([]),
    ] {
        assert!(app.run("ui.printSettings", params.clone()).is_err(), "{params}");
        assert_eq!(app.run("ui.printSettings", json!({})).unwrap(), before, "no partial update: {params}");
    }
    let page_before = app.run("ui.printPage", json!({})).unwrap();
    for params in
        [json!({"range":"all","parity":"bad"}), json!({"zoom":9.0}), json!({"pagesPerSheet":3}), json!({"extra":1}), json!({"paper":"Quarto"})]
    {
        assert!(app.run("ui.printPage", params.clone()).is_err(), "{params}");
        assert_eq!(app.run("ui.printPage", json!({})).unwrap(), page_before, "no partial update: {params}");
    }
}

/// Two fast jobs each surface the same receipt, and consuming it clears the status.
#[test]
fn repeated_fast_jobs_each_surface_an_identical_receipt() {
    let fake = Fake::answering(Ok("same receipt".into()));
    let mut app = app_over(&fake);
    choose_queue(&mut app, json!({}));
    for _ in 0..2 {
        app.status("");
        app.run("ui.printSubmit", json!({})).unwrap();
        // Completion may be consumed by the shared widget before the app's logic pass.
        app.printing.poll(&egui::Context::default());
        app.poll_printing(&egui::Context::default());
        assert!(status_text(&mut app).contains("same receipt"));
        app.status("");
        app.poll_printing(&egui::Context::default());
        assert!(status_text(&mut app).is_empty());
    }
}

/// Invalid range, parity, copy and collate values send nothing; odd pages from 1-3 appear once in
/// one PDF, while three uncollated copies are delegated to the spooler.
#[test]
fn native_ranges_and_parity_do_not_duplicate_spooler_copies() {
    // A fixed creation date: without one each PDF is stamped with the current time, and the two
    // compared below differ when a clock second ticks over between them (#987).
    let mut doc = wordcraft_doc::Document::new();
    doc.core.created = "2026-01-01T00:00:00Z".into();
    let fake = Fake::accepting();
    let mut app = app_with(Session::new(doc), &fake);
    for page in 1..=3 {
        app.run("text.insert", json!({"text":format!("Distinct page {page}")})).unwrap();
        if page < 3 {
            app.run("insert.pageBreak", json!({})).unwrap();
        }
    }
    assert_eq!(app.session.export_layout().pages.len(), 3);
    choose_queue(&mut app, json!({"copies":3}));
    for bad in [json!({"pages":"8"}), json!({"parity":"bad"}), json!({"copies":0}), json!({"collate":"yes"})] {
        assert!(app.run("ui.printSubmit", bad).is_err());
    }
    assert_eq!(fake.jobs(), 0, "invalid settings do not send any job");
    let params = json!({"pages":"1-3","parity":"odd","collate":false});
    let (expected, plan) = app.session.print_job_pdf(&params).unwrap();
    assert_eq!(plan.pages, vec![0, 2]);
    app.run("ui.printSubmit", params).unwrap();
    let sent = fake.sent.lock().unwrap();
    let stamped = |pdf: &[u8]| pdf.windows(16).any(|w| w == b"D:20260101000000");
    assert!(stamped(&expected) && stamped(&sent[0].0), "both PDFs carry the document's own creation date");
    assert_eq!(sent[0].0, expected, "the PDF contains only the two selected pages, once each");
    assert_eq!(sent[0].1.copies, 3, "copies go to the spooler, not the PDF");
    assert!(!sent[0].1.collate, "uncollated copies reach the spooler");
}

/// #1211: with no CUPS destinations, discovery reports `noPrinters`, not silence.
#[test]
fn empty_discovery_reports_no_printers_instead_of_silence() {
    // #1211: a machine with no CUPS destinations must say "no printers", never detour silently.
    let fake = Arc::new(Fake { printers: Vec::new(), outcome: Ok(String::new()), sent: Mutex::new(Vec::new()), discoveries: AtomicUsize::new(0) });
    let mut app = app_over(&fake);
    app.run("ui.printSettings", json!({"refresh": true})).unwrap();
    app.printing.poll(&egui::Context::default());
    let st = app.run("ui.printSettings", json!({})).unwrap();
    assert!(st["printers"].as_array().is_some_and(|p| p.is_empty()));
    assert_eq!(st["status"], "noPrinters", "{st}");
}

/// Default or auto orientation sends `orientation-requested=3`, landscape sends 4 plus
/// `landscape=true`, and portrait sends 3.
#[test]
fn native_print_passes_orientation_options_to_spooler() {
    let fake = Fake::accepting();
    let mut app = app_over(&fake);
    app.run("text.insert", json!({"text": "Test orientation"})).unwrap();
    choose_queue(&mut app, json!({}));

    // 1. Default (auto for portrait document) sends orientation-requested=3
    app.run("ui.printSubmit", json!({})).unwrap();
    assert_eq!(fake.last_options(), vec![("orientation-requested".to_owned(), "3".to_owned())]);
    app.printing.poll(&egui::Context::default());
    app.poll_printing(&egui::Context::default());

    // 2. Explicit landscape sends orientation-requested=4 and landscape=true
    app.run("ui.printSubmit", json!({"orientation": "landscape"})).unwrap();
    assert_eq!(fake.last_options(), vec![("orientation-requested".to_owned(), "4".to_owned()), ("landscape".to_owned(), "true".to_owned())]);
    app.printing.poll(&egui::Context::default());
    app.poll_printing(&egui::Context::default());

    // 3. Explicit portrait sends orientation-requested=3
    app.run("ui.printSubmit", json!({"orientation": "portrait"})).unwrap();
    assert_eq!(fake.last_options(), vec![("orientation-requested".to_owned(), "3".to_owned())]);
}

/// Scale to paper size (#2108): the job's PDF goes on the chosen paper, the spooler is asked
/// for that media, and the document keeps its own page size.
#[test]
fn native_print_scales_to_the_chosen_paper() {
    let fake = Fake::accepting();
    let mut app = app_over(&fake);
    app.run("text.insert", json!({"text": "Scale me"})).unwrap();
    choose_queue(&mut app, json!({}));
    let page = (app.session.doc.last_section.page_w, app.session.doc.last_section.page_h);
    let media_box = |bytes: &[u8]| {
        let raw = String::from_utf8_lossy(bytes).into_owned();
        let n: Vec<f32> =
            raw.split("/MediaBox[").nth(1).and_then(|r| r.split(']').next()).unwrap().split_whitespace().map(|n| n.parse().unwrap()).collect();
        (n[2].round(), n[3].round())
    };

    // The Print page's choice reaches the job.
    assert_eq!(page_of(&mut app)["paper"], "none", "No Scaling by default");
    app.run("ui.printPage", json!({"paper": "Legal"})).unwrap();
    let chosen = page_of(&mut app);
    assert_eq!(chosen["paper"], "Legal");
    app.run("ui.printSubmit", json!({"paper": chosen["paper"]})).unwrap();
    assert_eq!(media_box(&fake.last_pdf()), (612.0, 1008.0));
    assert!(fake.last_options().contains(&("media".to_owned(), "na_legal_8.5x14in".to_owned())), "{:?}", fake.last_options());
    app.printing.poll(&egui::Context::default());
    app.poll_printing(&egui::Context::default());

    app.run("ui.printSubmit", json!({"paper": "A4"})).unwrap();
    assert_eq!(media_box(&fake.last_pdf()), (595.0, 842.0));
    app.printing.poll(&egui::Context::default());
    app.poll_printing(&egui::Context::default());

    // An unknown paper submits nothing.
    let before = fake.jobs();
    assert!(app.run("ui.printSubmit", json!({"paper": "Quarto"})).is_err());
    assert_eq!(fake.jobs(), before);
    assert_eq!((app.session.doc.last_section.page_w, app.session.doc.last_section.page_h), page, "the document is unchanged");
}

/// Native copies stay in the spooler; booklet pairing and padding happen once in its PDF.
#[test]
fn native_booklet_uses_landscape_sheets_without_multiplying_copies() {
    let fake = Fake::answering(Ok("test spooler".into()));
    let mut app = app_with(Session::new(wordcraft_doc::Document::from_text("1\u{c}2\u{c}3\u{c}4\u{c}5")), &fake);
    choose_queue(&mut app, json!({"copies":2,"duplex":"shortEdge"}));
    app.run("ui.printSubmit", json!({"pagesPerSheet":"booklet"})).unwrap();
    let sent = fake.sent.lock().unwrap();
    let (bytes, job) = &sent[0];
    assert_eq!(job.copies, 2);
    assert_eq!(job.duplex, Some(print::Duplex::ShortEdge));
    assert!(job.collate);
    assert_eq!(String::from_utf8_lossy(bytes).matches("/MediaBox[").count(), 4);
    assert!(job.options.contains(&("orientation-requested".into(), "4".into())));
}

/// The booklet choice changes both the submitted parameters and the physical preview's sheet count.
#[test]
fn print_booklet_choice_updates_sheet_preview() {
    use egui_kittest::kittest::{By, Queryable};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("1\u{c}2\u{c}3\u{c}4\u{c}5")), Services::default());
    app.ui.language = "en".into();
    app.run("file.print", json!({})).unwrap();
    let mut h = crate::harness::builder().with_size(egui::vec2(1440.0, 1000.0)).with_pixels_per_point(1.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    before_frames(&mut h, 4);
    h.state_mut().run("ui.printPage", json!({"pagesPerSheet": "booklet"})).unwrap();
    h.run_steps(4);
    assert_eq!(page_of(h.state_mut())["pagesPerSheet"], "booklet");
    assert!(h.query(By::new().role(egui::accesskit::Role::Group).label("Sheet 1 of 4")).is_some());
    h.get_by_label("Next sheet").click();
    h.run_steps(3);
    assert!(h.query(By::new().role(egui::accesskit::Role::Group).label("Sheet 2 of 4")).is_some());
    h.state_mut().run("ui.printPage", json!({"pagesPerSheet": 4})).unwrap();
    h.run_steps(4);
    assert_eq!(page_of(h.state_mut())["pagesPerSheet"], "4");
    assert!(h.query(By::new().role(egui::accesskit::Role::Group).label("Sheet 2 of 2")).is_some());
}

/// Changing a late-page range opens its first physical sheet in single-page and N-up previews.
#[test]
fn print_range_changes_start_at_the_first_planned_sheet() {
    use egui_kittest::kittest::{By, Queryable};
    for (range, selection, per_sheet, sheets) in [("selection", "3\u{c}4\u{c}5", "1", 3), ("custom", "9", "2", 5), ("custom", "9", "4", 3)] {
        let mut app = WordApp::new(
            Session::new(wordcraft_doc::Document::from_text("1\u{c}2\u{c}3\u{c}4\u{c}5\u{c}6\u{c}7\u{c}8\u{c}9\u{c}10")),
            Services::default(),
        );
        app.ui.language = "en".into();
        app.run("select.text", json!({"text": selection})).unwrap();
        assert!(app.session.current_pages(&app.session.export_layout()).0 >= 2);
        app.run("file.print", json!({})).unwrap();
        app.run("ui.printPage", json!({"pages": "2-10", "pagesPerSheet": per_sheet, "preview": 8})).unwrap();
        let mut h = crate::harness::builder().with_size(egui::vec2(1440.0, 1000.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        before_frames(&mut h, 4);
        h.state_mut().run("ui.printPage", json!({"range": range})).unwrap();
        h.run_steps(4);
        assert_eq!(page_of(h.state_mut())["range"], range);
        assert_eq!(page_of(h.state_mut())["preview"], 0, "{range}, {per_sheet} pages per sheet");
        let preview_label = format!("Sheet 1 of {sheets}");
        assert!(h.query(By::new().role(egui::accesskit::Role::Group).label(&preview_label)).is_some());
        if range == "custom" {
            h.state_mut().run("ui.printPage", json!({"preview": sheets - 1})).unwrap();
            h.run_steps(2);
            h.state_mut().run("ui.printPage", json!({"pages": "2-9"})).unwrap();
            h.run_steps(4);
            assert_eq!(page_of(h.state_mut())["pages"], "2-9");
            assert_eq!(page_of(h.state_mut())["preview"], 0, "editing the custom range starts at its first sheet");
            if per_sheet == "2" {
                h.state_mut().run("ui.printPage", json!({"preview": 3})).unwrap();
                h.run_steps(2);
                h.state_mut().run("ui.printPage", json!({"parity": "odd"})).unwrap();
                h.run_steps(4);
                assert_eq!(page_of(h.state_mut())["parity"], "odd");
                assert_eq!(page_of(h.state_mut())["preview"], 0, "filtering the selected pages starts at its first sheet");
            }
        }
    }
}

/// Native N-up accepts comment balloons and custom ranges from preferences or an explicit markup override.
#[test]
fn native_imposed_print_accepts_comment_markup_and_explicit_overrides() {
    use wordcraft_doc::{Comment, Document, InlineObject, Paragraph, PartKind, Pos, para_block};
    let mut doc = Document::from_text("First page\u{c}Second page\u{c}Third page");
    let part = doc.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Check these totals", Default::default()))]);
    doc.comments.insert(0, Comment { author: "Synthetic reviewer".into(), part, ..Default::default() });
    doc.insert_object(&Pos::body(0, 0), InlineObject::CommentStart { id: 0 }, &Default::default()).unwrap();
    for explicit_override in [false, true] {
        let fake = Fake::answering(Ok("test spooler".into()));
        let mut app = app_with(Session::new(doc.clone()), &fake);
        app.session.prefs.print_markup = !explicit_override;
        app.session.prefs.markup.comments = true;
        let marked = app.session.export_layout_with(true);
        let plain = app.session.export_layout_with(false);
        assert!(marked.pages[0].w > plain.pages[0].w && marked.pages[0].markup > 0.0, "the fixture includes actual balloon geometry");
        let pages = marked.pages.len();
        assert!(pages >= 3);
        choose_queue(&mut app, json!({}));
        let mut params = json!({"range":"custom","pages":format!("1-{pages}"),"pagesPerSheet":2});
        if explicit_override {
            params["markup"] = json!(true);
        }
        app.run("ui.printSubmit", params).unwrap();
        assert_eq!(fake.jobs(), 1);
        assert_eq!(String::from_utf8_lossy(&fake.last_pdf()).matches("/MediaBox[").count(), pages.div_ceil(2));
        assert!(fake.last_options().contains(&("orientation-requested".into(), "4".into())));
    }
}

/// #4734: the job sent to the printer has no link annotations, which a printer driver can't fit
/// to the paper ("flatten page annotations before using shrink-to-fit"); `file.printPdf` (Save
/// these pages as PDF) keeps them.
#[test]
fn native_print_job_leaves_out_link_annotations() {
    for mode in [json!(1), json!(2), json!("booklet"), json!("bookletReverse")] {
        let fake = Fake::accepting();
        let mut app = app_over(&fake);
        app.run("insert.link", json!({"url": "https://example.com", "text": "a link"})).unwrap();
        choose_queue(&mut app, json!({}));
        let annots = |bytes: &[u8]| String::from_utf8_lossy(bytes).contains("/Annots");
        app.run("ui.printSubmit", json!({"paper": "A4", "pagesPerSheet": mode})).unwrap();
        assert!(!annots(&fake.last_pdf()), "the printer gets no link annotations");
        let (saved, _) = app.session.print_pdf(&json!({"paper": "A4", "pagesPerSheet": mode})).unwrap();
        assert!(annots(&saved), "a saved print PDF keeps its links");
    }
}
