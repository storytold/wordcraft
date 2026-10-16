//! Native print onboarding preserves engine PDF generation and submits the selected settings.
use serde_json::json;
use std::{cell::RefCell, rc::Rc};
use wordcraft_engine::Session;
use wordcraft_ui_egui::printing::{Duplex, Feedback, Printer};
use wordcraft_ui_egui::{Services, WordApp};

#[test]
fn control_print_discovery_reports_metadata_without_host_side_effects() {
    use std::cell::Cell;
    use wordcraft_ui_egui::control::{ControlRequest, Outcome, handle};
    let discoveries = Rc::new(Cell::new(0));
    let submissions = Rc::new(Cell::new(0));
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    let observed = discoveries.clone();
    app.services.printers = Some(Box::new(move || {
        observed.set(observed.get() + 1);
        let (_, receiver) = std::sync::mpsc::channel();
        receiver
    }));
    let observed = submissions.clone();
    app.services.print_native = Some(Box::new(move |_, _, _, _| {
        observed.set(observed.get() + 1);
        let (_, receiver) = std::sync::mpsc::channel();
        receiver
    }));
    let ctx = egui::Context::default();
    let (request, _) = ControlRequest::new("engine.commands", json!({"search":"print"}));
    let Outcome::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("metadata replies immediately") };
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
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    app.run("ui.printSettings", json!({"printer":"Queue"})).unwrap();
    let Outcome::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("metadata replies immediately") };
    for id in ["ui.printSubmit", "ui.print"] {
        let command = reply["result"].as_array().unwrap().iter().find(|command| command["id"] == id).unwrap();
        assert_eq!(command["enabled"], true, "the selected queue enables {id}");
    }
    assert_eq!(discoveries.get(), 0, "metadata never starts printer discovery");
    assert_eq!(submissions.get(), 0, "metadata never submits");
    assert!(app.dialog.is_none() && !app.ui.backstage, "metadata never opens a dialog or Print page");
    assert!(!app.printing.busy() && !app.session.dirty);
}

#[test]
fn native_print_uses_selected_settings_and_reports_spool_errors() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.run("text.insert", json!({"text": "A document to print"})).unwrap();
    let jobs = Rc::new(RefCell::new(Vec::new()));
    let observed = jobs.clone();
    app.services.print_native = Some(Box::new(move |bytes, settings, collate, _| {
        observed.borrow_mut().push((bytes, settings, collate));
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Err("printer is offline".into())).unwrap();
        receiver
    }));
    assert!(app.run("ui.print", json!({})).is_err());
    assert!(jobs.borrow().is_empty(), "opening settings/exporting does not spool");
    app.printing.printers.push(Printer { name: "Chosen queue".into(), default: false });
    let configured = app.run("ui.printSettings", json!({"printer":"Chosen queue", "copies":3,"duplex":"shortEdge","grayscale":true})).unwrap();
    assert_eq!(configured["ready"], true);
    assert_eq!(configured["printers"][0]["name"], "Chosen queue");
    app.run("ui.print", json!({})).unwrap();
    assert!(app.printing.busy());
    assert!(!app.close_requested(), "a pending print job cannot be interrupted by normal close");
    let jobs = jobs.borrow();
    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].0.starts_with(b"%PDF"));
    assert_eq!(jobs[0].1.printer, "Chosen queue");
    assert_eq!(jobs[0].1.copies, 3);
    assert_eq!(jobs[0].1.duplex, Some(Duplex::ShortEdge));
    assert!(jobs[0].1.grayscale);
    assert!(jobs[0].2, "copies are collated by default");
    app.poll_printing(&egui::Context::default());
    assert_eq!(app.printing.message, Some(Feedback::Failed("printer is offline".into())));
    assert!(app.status_msg.as_ref().unwrap().0.contains("printer is offline"));
}

#[test]
fn print_settings_are_visible_in_a_narrow_window() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    // The labels below are English; an integration test otherwise follows the machine's language.
    app.ui.language = "en".into();
    app.run("text.insert", json!({"text": "A document to print"})).unwrap();
    app.services.printers = Some(Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(vec![Printer { name: "Test queue".into(), default: true }]).unwrap();
        receiver
    }));
    app.services.print_native = Some(Box::new(|_, _, _, _| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok("queued".into())).unwrap();
        receiver
    }));
    app.ui.backstage = true;
    app.ui.backstage_page = "print".into();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 700.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(5);
    assert!(!harness.get_by_label("Refresh printers").accesskit_node().is_disabled());
    assert!(!harness.get_by_label("Grayscale").accesskit_node().is_disabled());
    harness.get_by_label("Grayscale").click();
    harness.run_steps(2);
    assert!(harness.state().printing.settings.grayscale);
    assert!(harness.state().printing.ready());
}

#[test]
fn invalid_print_settings_are_rejected_without_partial_changes() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
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
}

#[test]
fn repeated_fast_jobs_each_surface_an_identical_receipt() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    app.run("ui.printSettings", json!({"printer":"Queue"})).unwrap();
    app.services.print_native = Some(Box::new(|_, _, _, _| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok("same receipt".into())).unwrap();
        receiver
    }));
    for _ in 0..2 {
        app.status_msg = None;
        app.run("ui.printSubmit", json!({})).unwrap();
        // Completion may be consumed by the shared widget before the app's logic pass.
        app.printing.poll(&egui::Context::default());
        app.poll_printing(&egui::Context::default());
        assert!(app.status_msg.as_ref().unwrap().0.contains("same receipt"));
        app.status_msg = None;
        app.poll_printing(&egui::Context::default());
        assert!(app.status_msg.is_none());
    }
}

#[test]
fn native_ranges_and_parity_do_not_duplicate_spooler_copies() {
    // A fixed creation date: without one each PDF is stamped with the current time, and the two
    // compared below differ when a clock second ticks over between them (#987).
    let mut doc = wordcraft_doc::Document::new();
    doc.core.created = "2026-01-01T00:00:00Z".into();
    let mut app = WordApp::new(Session::new(doc), Services::default());
    for page in 1..=3 {
        app.run("text.insert", json!({"text":format!("Distinct page {page}")})).unwrap();
        if page < 3 {
            app.run("insert.pageBreak", json!({})).unwrap();
        }
    }
    assert_eq!(app.session.export_layout().pages.len(), 3);
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    app.run("ui.printSettings", json!({"printer":"Queue","copies":3})).unwrap();
    let jobs = Rc::new(RefCell::new(Vec::new()));
    let observed = jobs.clone();
    app.services.print_native = Some(Box::new(move |bytes, settings, collate, _| {
        observed.borrow_mut().push((bytes, settings, collate));
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok("queued".into())).unwrap();
        receiver
    }));
    for bad in [json!({"pages":"8"}), json!({"parity":"bad"}), json!({"copies":0}), json!({"collate":"yes"})] {
        assert!(app.run("ui.printSubmit", bad).is_err());
    }
    assert!(jobs.borrow().is_empty(), "invalid settings do not send any job");
    let params = json!({"pages":"1-3","parity":"odd","collate":false});
    let (expected, plan) = app.session.print_pdf(&params).unwrap();
    assert_eq!(plan.pages, vec![0, 2]);
    app.run("ui.printSubmit", params).unwrap();
    let jobs = jobs.borrow();
    let stamped = |pdf: &[u8]| pdf.windows(16).any(|w| w == b"D:20260101000000");
    assert!(stamped(&expected) && stamped(&jobs[0].0), "both PDFs carry the document's own creation date");
    assert_eq!(jobs[0].0, expected, "the PDF contains only the two selected pages, once each");
    assert_eq!(jobs[0].1.copies, 3, "copies go to the spooler, not the PDF");
    assert!(!jobs[0].2, "uncollated copies reach the spooler");
}

#[test]
fn empty_discovery_reports_no_printers_instead_of_silence() {
    // #1211: a machine with no CUPS destinations must say "no printers", never detour silently.
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.services.printers = Some(Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Vec::new()).unwrap();
        receiver
    }));
    app.run("ui.printSettings", json!({"refresh": true})).unwrap();
    app.printing.poll(&egui::Context::default());
    let st = app.run("ui.printSettings", json!({})).unwrap();
    assert!(st["printers"].as_array().is_some_and(|p| p.is_empty()));
    assert_eq!(st["status"], "noPrinters", "{st}");
}

#[test]
fn native_print_passes_orientation_options_to_spooler() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.run("text.insert", json!({"text": "Test orientation"})).unwrap();
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    app.run("ui.printSettings", json!({"printer":"Queue"})).unwrap();
    let jobs = Rc::new(RefCell::new(Vec::new()));
    let observed = jobs.clone();
    app.services.print_native = Some(Box::new(move |bytes, settings, collate, options| {
        observed.borrow_mut().push((bytes, settings, collate, options));
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok("queued".into())).unwrap();
        receiver
    }));

    // 1. Default (auto for portrait document) sends orientation-requested=3
    app.run("ui.printSubmit", json!({})).unwrap();
    assert_eq!(jobs.borrow().last().unwrap().3, vec![("orientation-requested".to_owned(), "3".to_owned())]);
    app.printing.poll(&egui::Context::default());
    app.poll_printing(&egui::Context::default());

    // 2. Explicit landscape sends orientation-requested=4 and landscape=true
    app.run("ui.printSubmit", json!({"orientation": "landscape"})).unwrap();
    assert_eq!(
        jobs.borrow().last().unwrap().3,
        vec![("orientation-requested".to_owned(), "4".to_owned()), ("landscape".to_owned(), "true".to_owned())]
    );
    app.printing.poll(&egui::Context::default());
    app.poll_printing(&egui::Context::default());

    // 3. Explicit portrait sends orientation-requested=3
    app.run("ui.printSubmit", json!({"orientation": "portrait"})).unwrap();
    assert_eq!(jobs.borrow().last().unwrap().3, vec![("orientation-requested".to_owned(), "3".to_owned())]);
}
