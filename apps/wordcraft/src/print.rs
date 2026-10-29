//! Desktop spooler adapter. Discovery and submission never run on the UI thread.
use std::time::{Duration, Instant};
use wordcraft_ui_egui::printing::{Discover, Duplex, Printer, Settings, Submit};

/// Returns a discovery callback that enumerates spooler printers on a named worker thread and
/// sends the list through a channel, keeping spooler calls off the UI thread.
pub fn discover() -> Discover {
    Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("print-discovery".into()).spawn(move || {
            let printers = craft_print::printers()
                .into_iter()
                .map(|printer| Printer { name: printer.name, label: printer.description, default: printer.default })
                .collect();
            let _ = sender.send(printers);
        });
        receiver
    })
}

// Keep submission workers reachable until normal app shutdown; dropping a JoinHandle would
// let process exit end an in-process Windows job or the CUPS hand-off before the spooler has it.
static JOBS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

/// How long shutdown waits for submission workers: long enough for a normal handover to the
/// spooler, short enough that a stalled driver cannot keep a closed window's process alive.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(25);

pub fn finish_pending_jobs() {
    finish_pending_jobs_within(SHUTDOWN_DEADLINE);
}

/// Wait up to `deadline` in total for the submission workers, joining each one that finishes.
/// Never blocks past the deadline.
pub fn finish_pending_jobs_within(deadline: Duration) {
    let mut workers = std::mem::take(&mut *JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    join_within(&mut workers, deadline);
}

/// Join the finished workers in `workers`, polling until `deadline` has passed. Workers still
/// running at the deadline are left in the list (and dropped by the caller, which detaches them).
fn join_within(workers: &mut Vec<std::thread::JoinHandle<()>>, deadline: Duration) {
    let end = Instant::now() + deadline;
    loop {
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                if workers.swap_remove(index).join().is_err() {
                    log::error!("the print worker stopped unexpectedly during shutdown");
                }
            } else {
                index += 1;
            }
        }
        if workers.is_empty() || Instant::now() >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !workers.is_empty() {
        // Not a failure and not known to be safe to retry: the job may or may not have reached the
        // queue, so the print queue should be checked.
        log::warn!(
            "{} print job(s) were still being handed to the spooler after {deadline:?};              shutdown continued without them. Check the print queue before printing again",
            workers.len()
        );
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

    #[test]
    fn a_stalled_worker_cannot_hold_shutdown_past_the_deadline() {
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let stalled = std::thread::spawn(move || {
            let _ = wait.recv();
        });
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let finished = std::thread::spawn(move || done_tx.send(()).unwrap());
        done_rx.recv().unwrap();
        let mut workers = vec![stalled, finished];
        let started = Instant::now();
        join_within(&mut workers, Duration::from_millis(200));
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(5), "the wait returned at its deadline, took {waited:?}");
        assert!(waited >= Duration::from_millis(200), "a normal handover is given the whole deadline");
        assert_eq!(workers.len(), 1, "the completed handover was joined; only the stalled one is left");
        assert!(!workers[0].is_finished());
        release.send(()).unwrap();
        for worker in workers {
            worker.join().unwrap();
        }
    }
}
