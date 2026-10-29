//! Desktop spooler adapter. Discovery and submission never run on the UI thread.
use wordcraft_ui_egui::printing::{Discover, Settings, Submit};
#[cfg(not(target_vendor = "win7"))]
use wordcraft_ui_egui::printing::{Duplex, Printer};

/// The experimental Windows 7 build's spooler: the PDF viewer's print verb (docs/windows7.md).
#[cfg(any(test, target_vendor = "win7"))]
mod windows7;

/// Returns a discovery callback that enumerates spooler printers on a named worker thread and
/// sends the list through a channel, keeping spooler calls off the UI thread.
pub fn discover() -> Discover {
    Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("print-discovery".into()).spawn(move || {
            let _ = sender.send(printers());
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

/// The spooler's printers.
#[cfg(not(target_vendor = "win7"))]
fn printers() -> Vec<Printer> {
    craft_print::printers().into_iter().map(|printer| Printer { name: printer.name, label: printer.description, default: printer.default }).collect()
}

#[cfg(target_vendor = "win7")]
use windows7::printers;

/// Hands `bytes` to the spooler; the result is its receipt or the error.
#[cfg(not(target_vendor = "win7"))]
fn send(bytes: &[u8], settings: Settings, collate: bool, options: Vec<(String, String)>) -> Result<String, String> {
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
    craft_print::submit(bytes, &job).map_err(|error| error.to_string())
}

/// Hands `bytes` to the PDF viewer, which prints with its own settings (docs/windows7.md).
#[cfg(target_vendor = "win7")]
fn send(bytes: &[u8], settings: Settings, _collate: bool, _options: Vec<(String, String)>) -> Result<String, String> {
    windows7::submit(bytes, &settings.printer, settings.copies)
}

pub fn submit() -> Submit {
    Box::new(|bytes, settings: Settings, collate, options| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new().name("print-submit".into()).spawn(move || {
            let _ = sender.send(send(&bytes, settings, collate, options));
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
