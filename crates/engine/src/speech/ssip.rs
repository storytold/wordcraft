//! Speech Dispatcher through its own protocol (SSIP) on the user's socket, for systems that have
//! Speech Dispatcher but not the `espeak-ng` command.
//!
//! Talking SSIP directly, instead of starting `spd-say`, keeps two promises `spd-say` can't: the
//! sentence is always data (pipe mode runs a line starting with `!-!` as an SSIP command, and a
//! document must never send commands), and stopping cancels only WordCraft's own speech
//! (`CANCEL self`), where `spd-say -C` would silence every client, a screen reader included.
//!
//! One connection speaks one sentence: it names the client, asks for the end and cancel events,
//! sets the rate, language and voice, sends the text, and waits for the sentence's end event.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// How often a wait for the end event looks at whether reading moved on.
const POLL: Duration = Duration::from_millis(30);
/// Longest wait for a reply to a command (the server answers at once; this only bounds a stuck one).
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest reply line kept (a server sends short lines; this bounds a broken one).
const MAX_LINE: usize = 4096;

/// What to say and how.
pub(super) struct Request<'a> {
    pub text: &'a str,
    /// Speech Dispatcher's rate, -100 to 100.
    pub rate: i32,
    /// A primary language subtag (`hu`), already checked to be letters only.
    pub lang: Option<&'a str>,
    /// A synthesis voice name from Speech Dispatcher's own list.
    pub voice: Option<&'a str>,
}

/// How a sentence ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Spoken to the end (or discarded by the server, which ends the wait the same way).
    Done,
    /// Reading moved on: the sentence was cancelled.
    Interrupted,
}

/// The server's socket: `SPEECHD_ADDRESS` (`unix_socket:/path`) when it names one, else
/// `speech-dispatcher/speechd.sock` in the runtime folder, else in `~/.cache` (older servers).
pub(super) fn socket_path(address: Option<&str>, runtime_dir: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(a) = address.map(str::trim).filter(|a| !a.is_empty()) {
        return a.strip_prefix("unix_socket:").filter(|p| !p.is_empty()).map(PathBuf::from);
    }
    runtime_dir
        .map(|d| d.join("speech-dispatcher").join("speechd.sock"))
        .or_else(|| home.map(|h| h.join(".cache").join("speech-dispatcher").join("speechd.sock")))
}

/// Connect to the user's Speech Dispatcher, starting it once if it isn't running (as `spd-say`
/// does). `None` when there is none.
pub(super) fn connect() -> Option<UnixStream> {
    let path = socket_path(
        std::env::var("SPEECHD_ADDRESS").ok().as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )?;
    if let Ok(s) = UnixStream::connect(&path) {
        return Some(s);
    }
    let started = std::process::Command::new("speech-dispatcher")
        .arg("--spawn")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !started {
        return None;
    }
    // The server creates its socket a moment after it starts.
    for _ in 0..20 {
        if let Ok(s) = UnixStream::connect(&path) {
            return Some(s);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// The sentence as SSIP message data: one line (a line break is a space when read aloud anyway),
/// a leading `.` doubled so it can't end the message, and the terminating `.` line.
pub(super) fn message_data(text: &str) -> String {
    let line: String = text.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect();
    let dot = if line.starts_with('.') { "." } else { "" };
    format!("{dot}{line}\r\n.\r\n")
}

/// A voice name that is safe on an SSIP command line (no line breaks; the name comes from the
/// server's list, but a saved choice is hostile input).
fn plain_voice(name: &str) -> Option<&str> {
    let n = name.trim();
    (!n.is_empty() && n.len() <= 200 && !n.chars().any(char::is_control)).then_some(n)
}

/// Reads reply lines, keeping a partial line across read timeouts.
struct Lines<R> {
    reader: BufReader<R>,
    pending: String,
}

impl<R: std::io::Read> Lines<R> {
    /// The next complete line without its line ending, `Ok(None)` when the read timed out.
    fn next(&mut self) -> std::io::Result<Option<String>> {
        match self.reader.read_line(&mut self.pending) {
            Ok(0) => Err(ErrorKind::UnexpectedEof.into()),
            Ok(_) if self.pending.ends_with('\n') => {
                let line = self.pending.trim_end_matches(['\r', '\n']).to_string();
                self.pending.clear();
                Ok(Some(line))
            }
            Ok(_) => Ok(None),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {
                if self.pending.len() > MAX_LINE {
                    return Err(ErrorKind::InvalidData.into());
                }
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

/// The three-digit code of a final reply line (`225 OK MESSAGE QUEUED`); `None` for a
/// continuation line (`225-12`).
fn final_code(line: &str) -> Option<u16> {
    let code = line.get(..3)?.parse().ok()?;
    (line.as_bytes().get(3) == Some(&b' ') || line.len() == 3).then_some(code)
}

/// The reply to the command just sent: its final code. An end or cancel event (702, 703) that
/// arrives meanwhile sets `ended`.
fn reply<R: std::io::Read>(lines: &mut Lines<R>, ended: &mut bool) -> std::io::Result<u16> {
    let start = std::time::Instant::now();
    loop {
        match lines.next()? {
            Some(line) => match final_code(&line) {
                Some(702 | 703) => *ended = true,
                Some(code) if (700..800).contains(&code) => {}
                Some(code) => return Ok(code),
                None => {}
            },
            None if start.elapsed() > REPLY_TIMEOUT => return Err(ErrorKind::TimedOut.into()),
            None => {}
        }
    }
}

/// Send one command line and read its reply code.
fn command<R: std::io::Read>(w: &mut impl Write, lines: &mut Lines<R>, cmd: &str, ended: &mut bool) -> std::io::Result<u16> {
    w.write_all(cmd.as_bytes())?;
    w.write_all(b"\r\n")?;
    reply(lines, ended)
}

/// A `2xx` reply, or an error naming the code.
fn ok(code: u16) -> std::io::Result<()> {
    if (200..300).contains(&code) { Ok(()) } else { Err(std::io::Error::other(format!("Speech Dispatcher refused a command ({code})"))) }
}

/// Most voices kept from the server's list (it lists every variant of every voice: thousands).
const MAX_VOICES: usize = 2000;

/// The plain voices in a `LIST SYNTHESIS_VOICES` reply line (`249-name<TAB>lang<TAB>variant`), as
/// (name, language); variants are left out, since each voice has dozens.
fn voice_line(line: &str) -> Option<(String, String)> {
    let mut f = line.strip_prefix("249-")?.split('\t');
    let (name, lang, variant) = (f.next()?, f.next()?, f.next().unwrap_or("none"));
    (variant == "none" && plain_voice(name).is_some() && !lang.is_empty()).then(|| (name.to_string(), lang.to_string()))
}

/// The server's synthesis voices, as (name, language).
pub(super) fn list_voices(stream: &UnixStream) -> std::io::Result<Vec<(String, String)>> {
    stream.set_read_timeout(Some(POLL))?;
    stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
    let mut w = stream;
    let mut lines = Lines { reader: BufReader::new(stream.try_clone()?), pending: String::new() };
    let mut ended = false;
    ok(command(&mut w, &mut lines, "SET SELF CLIENT_NAME user:wordcraft:read-aloud", &mut ended)?)?;
    w.write_all(b"LIST SYNTHESIS_VOICES\r\n")?;
    let mut out = Vec::new();
    let start = std::time::Instant::now();
    loop {
        match lines.next()? {
            Some(line) => match final_code(&line) {
                Some(code) if (700..800).contains(&code) => {}
                Some(code) => {
                    ok(code)?;
                    break;
                }
                None => {
                    if out.len() < MAX_VOICES
                        && let Some(v) = voice_line(&line)
                    {
                        out.push(v);
                    }
                }
            },
            None if start.elapsed() > REPLY_TIMEOUT => return Err(ErrorKind::TimedOut.into()),
            None => {}
        }
    }
    let _ = command(&mut w, &mut lines, "QUIT", &mut ended);
    Ok(out)
}

/// Speak `req` over `stream`, cancelling it when `changed()`.
pub(super) fn converse(stream: &UnixStream, req: &Request<'_>, changed: &dyn Fn() -> bool) -> std::io::Result<Outcome> {
    stream.set_read_timeout(Some(POLL))?;
    stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
    let mut w = stream;
    let mut lines = Lines { reader: BufReader::new(stream.try_clone()?), pending: String::new() };
    let mut ended = false;
    let e = &mut ended;
    ok(command(&mut w, &mut lines, "SET SELF CLIENT_NAME user:wordcraft:read-aloud", e)?)?;
    ok(command(&mut w, &mut lines, "SET SELF NOTIFICATION END on", e)?)?;
    ok(command(&mut w, &mut lines, "SET SELF NOTIFICATION CANCEL on", e)?)?;
    ok(command(&mut w, &mut lines, &format!("SET SELF RATE {}", req.rate.clamp(-100, 100)), e)?)?;
    // A language or voice the server doesn't know leaves its default: no error.
    if let Some(lang) = req.lang.filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphabetic())) {
        command(&mut w, &mut lines, &format!("SET SELF LANGUAGE {lang}"), e)?;
    }
    if let Some(voice) = req.voice.and_then(plain_voice) {
        command(&mut w, &mut lines, &format!("SET SELF SYNTHESIS_VOICE {voice}"), e)?;
    }
    ok(command(&mut w, &mut lines, "SPEAK", e)?)?;
    w.write_all(message_data(req.text).as_bytes())?;
    ok(reply(&mut lines, e)?)?;
    loop {
        if ended {
            let _ = command(&mut w, &mut lines, "QUIT", &mut ended);
            return Ok(Outcome::Done);
        }
        if changed() {
            let mut ignored = false;
            let _ = command(&mut w, &mut lines, "CANCEL self", &mut ignored);
            let _ = command(&mut w, &mut lines, "QUIT", &mut ignored);
            return Ok(Outcome::Interrupted);
        }
        if let Some(line) = lines.next()? {
            ended |= matches!(final_code(&line), Some(702 | 703));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn the_socket_follows_the_environment() {
        assert_eq!(socket_path(Some("unix_socket:/tmp/s.sock"), None, None), Some(PathBuf::from("/tmp/s.sock")));
        assert_eq!(socket_path(Some("inet_socket:localhost:6560"), Some("/run/user/1".into()), None), None, "only Unix sockets");
        assert_eq!(socket_path(None, Some("/run/user/1".into()), None), Some(PathBuf::from("/run/user/1/speech-dispatcher/speechd.sock")));
        assert_eq!(socket_path(Some(" "), None, Some("/home/u".into())), Some(PathBuf::from("/home/u/.cache/speech-dispatcher/speechd.sock")));
        assert_eq!(socket_path(None, None, None), None);
    }

    #[test]
    fn document_text_is_always_message_data() {
        // `spd-say -e` would run this line as an SSIP command; here it's spoken text.
        assert_eq!(message_data("!-!SET SELF RATE 50"), "!-!SET SELF RATE 50\r\n.\r\n");
        // A line of a single `.` would end the message early, and a line break would start a
        // new line that could be one.
        assert_eq!(message_data(".\r\nQUIT"), "..  QUIT\r\n.\r\n");
        assert_eq!(message_data("a\n.\nb"), "a . b\r\n.\r\n");
        assert_eq!(plain_voice("Hungarian"), Some("Hungarian"));
        assert_eq!(plain_voice("x\r\nCANCEL all"), None);
        assert_eq!(final_code("225 OK MESSAGE QUEUED"), Some(225));
        assert_eq!(final_code("225-17"), None);
        assert_eq!(final_code("x"), None);
    }

    /// A Speech Dispatcher stand-in: answers every command with `2xx`, and after the message
    /// either sends the end event or waits for `CANCEL self`. Returns the lines it received.
    fn server(mut s: UnixStream, end: bool) -> std::thread::JoinHandle<Vec<String>> {
        std::thread::spawn(move || {
            let mut got = Vec::new();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut in_data = false;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    return got;
                }
                let line = line.trim_end_matches(['\r', '\n']).to_string();
                got.push(line.clone());
                let reply: &[u8] = if in_data {
                    if line != "." {
                        continue;
                    }
                    in_data = false;
                    if end {
                        b"225-1\r\n225 OK MESSAGE QUEUED\r\n702-1\r\n702-7\r\n702 END\r\n"
                    } else {
                        b"225-1\r\n225 OK MESSAGE QUEUED\r\n701 BEGIN\r\n"
                    }
                } else if line == "SPEAK" {
                    in_data = true;
                    b"230 OK RECEIVING DATA\r\n"
                } else if line == "CANCEL self" {
                    b"703-1\r\n703-7\r\n703 CANCELLED\r\n210 OK CANCELLED\r\n"
                } else if line == "QUIT" {
                    let _ = s.write_all(b"231 HAPPY HACKING\r\n");
                    return got;
                } else {
                    b"200 OK\r\n"
                };
                s.write_all(reply).unwrap();
            }
        })
    }

    #[test]
    fn a_sentence_is_spoken_to_its_end() {
        let (client, srv) = UnixStream::pair().unwrap();
        let h = server(srv, true);
        let req = Request { text: "!-!QUIT", rate: 50, lang: Some("hu"), voice: Some("Hungarian") };
        assert_eq!(converse(&client, &req, &|| false).unwrap(), Outcome::Done);
        drop(client);
        let got = h.join().unwrap();
        assert!(got.contains(&"SET SELF RATE 50".to_string()) && got.contains(&"SET SELF LANGUAGE hu".to_string()), "{got:?}");
        assert!(got.contains(&"SET SELF SYNTHESIS_VOICE Hungarian".to_string()));
        let at = got.iter().position(|l| l == "SPEAK").unwrap();
        assert_eq!(got[at + 1..at + 3], ["!-!QUIT".to_string(), ".".to_string()], "the text went as data");
        assert!(!got.iter().any(|l| l.starts_with("CANCEL")), "nothing cancelled");
    }

    #[test]
    fn moving_on_cancels_only_our_own_speech() {
        let (client, srv) = UnixStream::pair().unwrap();
        let h = server(srv, false);
        let req = Request { text: "Long sentence.", rate: 0, lang: None, voice: None };
        let calls = std::cell::Cell::new(0);
        let changed = || {
            calls.set(calls.get() + 1);
            calls.get() > 3
        };
        assert_eq!(converse(&client, &req, &changed).unwrap(), Outcome::Interrupted);
        drop(client);
        let got = h.join().unwrap();
        assert!(got.contains(&"CANCEL self".to_string()), "{got:?}");
        assert!(!got.iter().any(|l| l.eq_ignore_ascii_case("CANCEL all")));
        assert!(!got.iter().any(|l| l.starts_with("SET SELF LANGUAGE")), "no language, no command");
    }

    #[test]
    fn the_voice_list_keeps_the_plain_voices() {
        assert_eq!(voice_line("249-Hungarian\thu\tnone"), Some(("Hungarian".into(), "hu".into())));
        assert_eq!(voice_line("249-Hungarian+Adam\thu\tAdam"), None, "a variant");
        assert_eq!(voice_line("249 OK VOICE LIST SENT"), None);
        assert_eq!(voice_line("249-\thu\tnone"), None);
        let (client, mut srv) = UnixStream::pair().unwrap();
        let h = std::thread::spawn(move || {
            let mut r = BufReader::new(srv.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            srv.write_all(b"208 OK CLIENT NAME SET\r\n").unwrap();
            line.clear();
            r.read_line(&mut line).unwrap();
            assert_eq!(line, "LIST SYNTHESIS_VOICES\r\n");
            srv.write_all(b"249-Afrikaans\taf\tnone\r\n249-Afrikaans+Adam\taf\tAdam\r\n249-Hungarian\thu\tnone\r\n249 OK VOICE LIST SENT\r\n")
                .unwrap();
            line.clear();
            let _ = r.read_line(&mut line);
            let _ = srv.write_all(b"231 HAPPY HACKING\r\n");
        });
        let v = list_voices(&client).unwrap();
        assert_eq!(v, [("Afrikaans".to_string(), "af".to_string()), ("Hungarian".to_string(), "hu".to_string())]);
        h.join().unwrap();
    }

    #[test]
    fn a_server_that_hangs_up_is_an_error_not_a_hang() {
        let (client, mut srv) = UnixStream::pair().unwrap();
        let h = std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            let _ = srv.read(&mut buf);
        });
        let req = Request { text: "x", rate: 0, lang: None, voice: None };
        assert!(converse(&client, &req, &|| false).is_err());
        h.join().unwrap();
    }
}
