//! Desktop spooler adapter. Discovery and submission never run on the UI thread.
#[cfg(any(windows, test))]
pub(crate) mod isolated;

use wordcraft_ui_egui::printing::{Discover, Duplex, Printer, Settings, Submit};

/// Returns a discovery callback that enumerates spooler printers on a named worker thread and
/// sends the list through a channel, keeping spooler calls off the UI thread.
///
/// The Print panel needs only names and the default, so the list comes from
/// `craft_print::printer_queues`, which asks no printer driver anything: on Windows a networked
/// printer in power save can hold a driver's capability query for a long time, and the list must
/// not wait on it (#5710). Submission still checks the job's options with the driver.
pub fn discover() -> Discover {
    Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("print-discovery".into()).spawn(move || {
            let printers = craft_print::printer_queues()
                .into_iter()
                .map(|printer| Printer { name: printer.name, label: printer.description, default: printer.default })
                .collect();
            let _ = sender.send(printers);
        });
        receiver
    })
}

/// A submission worker and how to stop its job: `true` when it will never be sent, `false` when
/// it was already being sent.
struct Job {
    worker: std::thread::JoinHandle<()>,
    cancel: Box<dyn Fn() -> bool + Send>,
    // Acknowledged helper cancellation permits joining it to reap the process and its input PDF.
    isolated: bool,
}

// Keep submission workers reachable until normal app shutdown; dropping a JoinHandle would
// let process exit end an in-process Windows job or the CUPS hand-off before the spooler has it.
static JOBS: std::sync::Mutex<Vec<Job>> = std::sync::Mutex::new(Vec::new());

fn jobs() -> std::sync::MutexGuard<'static, Vec<Job>> {
    JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Waits for the jobs already being sent, so the spooler has them before the process exits. A
/// job still waiting (on a sleeping printer's driver, say: #5710) is cancelled instead, so it is
/// never sent and quitting doesn't wait on the driver.
pub fn finish_pending_jobs() {
    let pending = std::mem::take(&mut *jobs());
    // Cancel them all first: a job left waiting while another is joined could start sending.
    let finishing: Vec<_> = pending
        .into_iter()
        .filter(|job| {
            let cancelled = (job.cancel)();
            job.isolated || !cancelled
        })
        .collect();
    for job in finishing {
        if job.worker.join().is_err() {
            log::error!("the print worker stopped unexpectedly during shutdown");
        }
    }
}

/// The Print page's Cancel: stops the job being submitted (the Print page submits one at a
/// time, the latest) unless it has started reaching the spooler. `true` when it will never be
/// sent. A job that has finished sending answers `false`, so its receipt is still shown.
pub fn cancel() -> Box<dyn Fn() -> bool> {
    Box::new(|| jobs().last().is_none_or(|job| (job.cancel)()))
}

/// Submit off the UI thread, isolating Windows drivers while retaining CUPS token cancellation.
pub fn submit() -> Submit {
    Box::new(|bytes, settings: Settings, collate, options| {
        let (sender, receiver) = std::sync::mpsc::channel();
        #[cfg(not(windows))]
        let cancel = craft_print::Cancel::new();
        #[cfg(windows)]
        let cancel = isolated::Submission::new();
        let token = cancel.clone();
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
            #[cfg(not(windows))]
            let sent = craft_print::submit_cancellable(&bytes, &job, &mut |_, _| {}, &token).map_err(|error| error.to_string());
            #[cfg(windows)]
            let sent = token.submit(&bytes, &job);
            let _ = sender.send(sent);
        });
        if let Ok(worker) = worker {
            let mut jobs = jobs();
            jobs.retain(|job| !job.worker.is_finished());
            jobs.push(Job { worker, cancel: Box::new(move || cancel.cancel()), isolated: cfg!(windows) });
        }
        receiver
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A worker that waits until released, then reports that it finished.
    fn worker() -> (std::thread::JoinHandle<()>, std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>) {
        let (release, wait) = std::sync::mpsc::channel();
        let (completed, completion) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = wait.recv();
            let _ = completed.send(());
        });
        (worker, release, completion)
    }

    /// A cancel action that records when it was asked and answers `stops`.
    fn asked(stops: bool) -> (Box<dyn Fn() -> bool + Send>, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let set = flag.clone();
        let cancel = Box::new(move || {
            set.store(true, std::sync::atomic::Ordering::SeqCst);
            stops
        });
        (cancel, flag)
    }

    /// Shutdown waits for a job already being sent, so the spooler has it before the process
    /// exits, but not for one still waiting on a driver (a sleeping printer, #5710): that one is
    /// cancelled, so it is never sent, and quitting doesn't hang on it. Every job is cancelled
    /// before any is waited for, so a waiting job can't start sending meanwhile. The Print page's
    /// Cancel asks only the latest job. One test, because JOBS is global.
    #[test]
    fn shutdown_waits_for_jobs_being_sent_and_cancels_the_rest() {
        let (sending, release_sending, sent) = worker();
        let (stuck, release_stuck, _) = worker();
        let (stop_sending, sending_asked) = asked(false);
        let (stop_stuck, cancelled) = asked(true);
        // The job being sent comes first, so a one-at-a-time shutdown would join it before
        // cancelling the waiting one.
        *JOBS.lock().unwrap() =
            vec![Job { worker: sending, cancel: stop_sending, isolated: false }, Job { worker: stuck, cancel: stop_stuck, isolated: false }];
        // Cancel from the Print page asks the latest job only.
        assert!(cancel()(), "the latest job was still waiting: never sent");
        assert!(!sending_asked.load(std::sync::atomic::Ordering::SeqCst), "an earlier job isn't the one being submitted");
        cancelled.store(false, std::sync::atomic::Ordering::SeqCst);
        let (finished, shutdown) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            finish_pending_jobs();
            let _ = finished.send(());
        });
        let wait = std::time::Duration::from_millis(200);
        assert!(shutdown.recv_timeout(wait).is_err(), "shutdown waits for the job being sent");
        assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst), "the waiting job was cancelled before the wait");
        release_sending.send(()).unwrap();
        assert!(shutdown.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "shutdown doesn't wait for the stuck job");
        sent.recv().unwrap();
        assert!(JOBS.lock().unwrap().is_empty(), "shutdown took every job");
        // A job that has already been sent answers false, so the Print page keeps its receipt.
        let (done, release_done, finished) = worker();
        release_done.send(()).unwrap();
        finished.recv().unwrap();
        *JOBS.lock().unwrap() = vec![Job { worker: done, cancel: Box::new(|| false), isolated: false }];
        assert!(!cancel()(), "a sent job isn't reported as cancelled");
        JOBS.lock().unwrap().clear();
        // The stuck worker was left behind without blocking shutdown; let it end.
        release_stuck.send(()).unwrap();
    }
}
