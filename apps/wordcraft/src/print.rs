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
// let process exit end an in-process Windows job or the CUPS hand-off before the spooler has it.
static JOBS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

pub fn finish_pending_jobs() {
    let workers = std::mem::take(&mut *JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for worker in workers {
        if worker.join().is_err() {
            log::error!("the print worker stopped unexpectedly during shutdown");
        }
    }
}

pub fn submit() -> Submit {
    Box::new(|bytes, settings: Settings, collate, options| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new().name("print-submit".into()).spawn(move || {
            let job = craft_print::Job {
                printer: Some(settings.printer),
                copies: settings.copies.clamp(1, 999),
                collate,
                duplex: settings.duplex.map(|value| match value {
                    Duplex::LongEdge => craft_print::Duplex::LongEdge,
                    Duplex::ShortEdge => craft_print::Duplex::ShortEdge,
                    Duplex::Off => craft_print::Duplex::Off,
                }),
                grayscale: settings.grayscale,
                scaling: craft_print::Scaling::ShrinkToFit,
                title: "WordCraft document".into(),
                options,
            };
            let _ = sender.send(craft_print::submit(&bytes, &job).map_err(|error| error.to_string()));
        });
        if let Ok(worker) = worker {
            let mut jobs = JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
