//! HTTPS fetch for the update check and the installer download (#15).
//!
//! An installer is around 100 MB, so a deadline on the whole transfer would fail on a slow
//! connection. Connecting and the first response get [`CONNECT`]; after that the transfer may take
//! as long as it needs while data keeps arriving, and fails once nothing has arrived for
//! [`STALL`]. [`BACKSTOP`] only ends a transfer that never finishes at all.

use std::io::Read;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use wordcraft_engine::update::HttpGet;

/// Connecting, sending the request and the response's headers.
const CONNECT: Duration = Duration::from_secs(30);
/// The longest wait for the next piece of a response body.
const STALL: Duration = Duration::from_secs(60);
/// The longest a whole body may take, however steadily it arrives.
const BACKSTOP: Duration = Duration::from_secs(4 * 60 * 60);
/// How much is read at a time.
const CHUNK: usize = 64 * 1024;

pub fn http_get() -> HttpGet {
    Arc::new(fetch_url)
}

fn fetch_url(url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    // HTTPS only, redirects included: release assets redirect to GitHub's download host.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(CONNECT))
        .timeout_send_request(Some(CONNECT))
        .timeout_recv_response(Some(CONNECT))
        .timeout_recv_body(Some(BACKSTOP))
        .user_agent(concat!("WordCraft/", env!("CARGO_PKG_VERSION"), " (+https://github.com/storytold/wordcraft)"))
        .build()
        .into();
    let response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| e.to_string())?;
    read_until_stalled(response.into_body().into_reader(), max_bytes, STALL)
}

/// Read `reader` to its end, failing when it holds more than `max_bytes` or when nothing arrives
/// for `stall`. The reads run on their own thread so a stalled read can't hold this one; that
/// thread ends with the connection.
fn read_until_stalled<R: Read + Send + 'static>(mut reader: R, max_bytes: usize, stall: Duration) -> Result<Vec<u8>, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<u8>, String>>();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; CHUNK];
        loop {
            let piece = match reader.read(&mut buf) {
                Ok(0) => Ok(Vec::new()),
                Ok(n) => Ok(buf.get(..n).unwrap_or_default().to_vec()),
                Err(e) => Err(e.to_string()),
            };
            let last = !matches!(&piece, Ok(p) if !p.is_empty());
            if tx.send(piece).is_err() || last {
                return;
            }
        }
    });
    let mut out = Vec::new();
    loop {
        match rx.recv_timeout(stall) {
            Ok(Ok(piece)) if piece.is_empty() => return Ok(out),
            Ok(Ok(piece)) => {
                out.extend_from_slice(&piece);
                if out.len() > max_bytes {
                    return Err(format!("the download is larger than {max_bytes} bytes"));
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(RecvTimeoutError::Timeout) => return Err(format!("the download stalled: nothing arrived for {} seconds", stall.as_secs())),
            Err(RecvTimeoutError::Disconnected) => return Err("the download ended unexpectedly".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body that arrives in pieces with a pause before each.
    struct Slow {
        pieces: Vec<Vec<u8>>,
        pause: Duration,
    }

    impl Read for Slow {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pieces.is_empty() {
                return Ok(0);
            }
            std::thread::sleep(self.pause);
            let p = self.pieces.remove(0);
            buf[..p.len()].copy_from_slice(&p);
            Ok(p.len())
        }
    }

    /// A long transfer that keeps moving finishes, however long it takes in total: the limit is
    /// on silence, not on the whole download.
    #[test]
    fn a_slow_steady_download_finishes() {
        let pieces: Vec<Vec<u8>> = (0..10u8).map(|i| vec![i; 100]).collect();
        let slow = Slow { pieces, pause: Duration::from_millis(30) };
        let got = read_until_stalled(slow, 10_000, Duration::from_millis(200)).unwrap();
        assert_eq!(got.len(), 1000, "ten pieces over ~300 ms, well past the 200 ms stall limit in total");
        assert_eq!(got[999], 9);
    }

    #[test]
    fn a_stalled_download_fails() {
        let slow = Slow { pieces: vec![vec![1; 10]], pause: Duration::from_millis(500) };
        let e = read_until_stalled(slow, 10_000, Duration::from_millis(100)).unwrap_err();
        assert!(e.contains("stalled"), "{e}");
    }

    #[test]
    fn a_download_over_the_limit_fails() {
        let slow = Slow { pieces: vec![vec![1; 600], vec![2; 600]], pause: Duration::ZERO };
        let e = read_until_stalled(slow, 1000, Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("larger than 1000"), "{e}");
    }
}
