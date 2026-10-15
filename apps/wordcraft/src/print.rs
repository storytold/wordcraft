//! Desktop spooler adapter. Discovery and submission never run on the UI thread.
use wordcraft_ui_egui::printing::{Discover, Duplex, Printer, Settings, Submit};

pub fn discover() -> Discover {
    Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("print-discovery".into()).spawn(move || {
            let printers = craft_print::printers().into_iter().map(|printer| Printer { name: printer.name, default: printer.default }).collect();
            let _ = sender.send(printers);
        });
        receiver
    })
}

// Keep submission workers reachable until normal app shutdown; dropping a JoinHandle would
// let process exit kill PowerShell/CUPS before it hands the job to the spooler.
static JOBS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

pub fn finish_pending_jobs() {
    let workers = std::mem::take(&mut *JOBS.lock().unwrap_or_else(|poison| poison.into_inner()));
    for worker in workers {
        if worker.join().is_err() {
            log::error!("the print worker stopped unexpectedly during shutdown");
        }
    }
}

/// The spooler job for a local queue. The pages are the document's own sizes, so the job never
/// asks for a paper the queue lacks: it prints on the queue's default paper, shrunk to fit.
/// (Exact scaling is for a paper the person picked in the dialog; this dialog has none.)
fn job(settings: Settings, collate: bool, options: Vec<(String, String)>) -> craft_print::Job {
    craft_print::Job {
        printer: Some(settings.printer),
        copies: settings.copies.clamp(1, 999),
        collate,
        duplex: settings.duplex.map(|value| match value {
            Duplex::LongEdge => craft_print::Duplex::LongEdge,
            Duplex::ShortEdge => craft_print::Duplex::ShortEdge,
            Duplex::Off => craft_print::Duplex::Off,
        }),
        // The pages are already gray when this is set; the driver is asked for monochrome as well.
        grayscale: settings.grayscale,
        scaling: craft_print::Scaling::ShrinkToFit,
        title: "WordCraft document".into(),
        options,
    }
}

pub fn submit() -> Submit {
    Box::new(|bytes, settings: Settings, collate, options| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new().name("print-submit".into()).spawn(move || {
            let job = job(settings, collate, options);
            let _ = sender.send(craft_print::submit(&bytes, &job).map_err(|error| error.to_string()));
        });
        if let Ok(worker) = worker {
            let mut jobs = JOBS.lock().unwrap_or_else(|poison| poison.into_inner());
            jobs.retain(|job| !job.is_finished());
            jobs.push(worker);
        }
        receiver
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_job_uses_the_queues_default_paper_and_shrinks_to_fit() {
        let settings = Settings { printer: "Queue".into(), copies: 2, duplex: None, grayscale: true };
        let job = job(settings, true, Vec::new());
        assert!(matches!(job.scaling, craft_print::Scaling::ShrinkToFit), "no exact scaling: a 13.33 x 7.5 in page needs no such paper");
        assert!(job.grayscale);
        assert_eq!(job.printer.as_deref(), Some("Queue"));
    }

    #[test]
    fn shutdown_waits_for_pending_submission_workers() {
        let (release, wait) = std::sync::mpsc::channel();
        let (completed, completion) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
            completed.send(()).unwrap();
        });
        JOBS.lock().unwrap().push(worker);
        let shutdown = std::thread::spawn(finish_pending_jobs);
        assert!(completion.try_recv().is_err(), "the print worker is still pending");
        release.send(()).unwrap();
        shutdown.join().unwrap();
        completion.recv().unwrap();
        assert!(JOBS.lock().unwrap().is_empty(), "workers were joined before shutdown returned");
    }
}
