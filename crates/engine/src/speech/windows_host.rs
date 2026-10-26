//! The persistent Windows speech host: one PowerShell process per reading, so System.Speech and
//! the synthesizer load once instead of once per sentence. The current `speak` path remains the
//! fallback when the host cannot start or fails at any point; a failure after a sentence starts
//! repeats it on that path, so no text is dropped (the part already spoken may be heard twice).
//! The Windows default voice is captured when the host starts and restored before every sentence,
//! so changing it mid-reading takes effect on the next reading.

#[cfg(target_os = "windows")]
use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "windows")]
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(target_os = "windows")]
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};

use serde_json::json;

#[cfg(target_os = "windows")]
use super::Spoken;

/// How long the host has to create its synthesizer and say it is ready.
#[cfg(target_os = "windows")]
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
/// A sentence can be long and the speed can be slow; this only catches a host that stopped
/// answering.
#[cfg(target_os = "windows")]
const SPEAK_TIMEOUT: Duration = Duration::from_secs(300);

/// The PowerShell program. It reads one JSON command per line and writes one reply per line.
const HOST_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
# Captured once per reading: a default-voice change mid-reading takes effect next time.
$defaultVoice = $synth.Voice.Name
$defaultLanguage = $synth.Voice.Culture.TwoLetterISOLanguageName
$utf8 = New-Object System.Text.UTF8Encoding($false)
$reader = New-Object System.IO.StreamReader([Console]::OpenStandardInput(), $utf8)
$out = New-Object System.IO.StreamWriter([Console]::OpenStandardOutput(), $utf8)
$out.AutoFlush = $true
$out.WriteLine('ready')
$task = $reader.ReadLineAsync()
$speaking = $null
$prompt = $null
$stopId = $null
while ($true) {
    if ($task.IsCompleted) {
        try { $line = $task.Result } catch { break }
        if ($null -eq $line) { break }
        $m = $null
        # 0 is never a sentence id; it marks a line the host could not parse.
        $id = '0'
        try {
            $m = $line | ConvertFrom-Json
            $id = [string]$m.id
            if ($id -eq '') { $id = '0' }
            switch ([string]$m.cmd) {
                'speak' {
                    $synth.SpeakAsyncCancelAll()
                    $speaking = $null
                    $prompt = $null
                    $synth.Rate = [int]$m.rate
                    $voice = [string]$m.voice
                    $lang = [string]$m.lang
                    $voiceError = $false
                    try { $synth.SelectVoice($defaultVoice) } catch { $voiceError = $true }
                    if (-not $voiceError) {
                        if ($voice -ne '') {
                            try { $synth.SelectVoice($voice) } catch { $voiceError = $true }
                        } elseif ($lang -ne '' -and $defaultLanguage -ne $lang) {
                            try {
                                $v = $synth.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq $lang } | Select-Object -First 1
                                if ($v) { $synth.SelectVoice($v.VoiceInfo.Name) }
                            } catch { $voiceError = $true }
                        }
                    }
                    if ($voiceError) {
                        $out.WriteLine('error ' + $id + ' voice')
                    } else {
                        $prompt = $synth.SpeakAsync([string]$m.text)
                        $out.WriteLine('started ' + $id)
                        $speaking = $id
                    }
                }
                'stop' {
                    $synth.SpeakAsyncCancelAll()
                    $speaking = $null
                    $prompt = $null
                    if ($synth.State -eq [System.Speech.Synthesis.SynthesizerState]::Ready) {
                        $out.WriteLine('stopped ' + $id)
                    } else {
                        $stopId = $id
                    }
                }
                'quit' {
                    if ($null -ne $speaking) { $synth.SpeakAsyncCancelAll() }
                    $synth.Dispose()
                    exit 0
                }
            }
        } catch {
            $out.WriteLine('error ' + $id + ' other')
        }
        $task = $reader.ReadLineAsync()
    }
    if ($null -ne $stopId -and $synth.State -eq [System.Speech.Synthesis.SynthesizerState]::Ready) {
        $out.WriteLine('stopped ' + $stopId)
        $stopId = $null
    }
    if ($null -ne $speaking -and $null -ne $prompt -and $prompt.IsCompleted) {
        $out.WriteLine('done ' + $speaking)
        $speaking = $null
        $prompt = $null
    }
    if ($null -eq $speaking -and $null -eq $stopId) {
        # Nothing is speaking: wait for the next command instead of polling.
        try { [void]$task.Wait() } catch { break }
    } else {
        Start-Sleep -Milliseconds 15
    }
}
$synth.Dispose()
"#;

/// One line of the host protocol.
#[derive(Debug, PartialEq)]
enum Reply {
    Started(u64),
    Done(u64),
    Stopped(u64),
    Error(u64, String),
}

fn parse_reply(line: &str) -> Option<Reply> {
    let (kind, rest) = line.trim().split_once(' ')?;
    let (id, code) = match rest.split_once(' ') {
        Some((id, code)) => (id, Some(code)),
        None => (rest, None),
    };
    let id = id.parse().ok()?;
    match kind {
        "started" if code.is_none() => Some(Reply::Started(id)),
        "done" if code.is_none() => Some(Reply::Done(id)),
        "stopped" if code.is_none() => Some(Reply::Stopped(id)),
        "error" => Some(Reply::Error(id, code?.to_string())),
        _ => None,
    }
}

fn speak_message(id: u64, text: &str, lang: Option<&str>, rate: f32) -> String {
    let lang = lang.and_then(super::primary_tag).unwrap_or_default();
    json!({
        "cmd": "speak",
        "id": id,
        "rate": super::sapi_rate(rate),
        "lang": lang,
        "voice": null,
        "text": text,
    })
    .to_string()
}

#[cfg(target_os = "windows")]
pub(super) enum HostError {
    /// The requested voice could not be selected; the caller retries with the default voice.
    Voice,
    /// The host is gone or did not answer; the caller falls back. If the sentence had started,
    /// the caller repeats it from the beginning on the fallback path.
    Unavailable,
}

#[cfg(target_os = "windows")]
pub(super) struct Host {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    reader: Option<std::thread::JoinHandle<()>>,
    next_id: u64,
    broken: bool,
}

#[cfg(target_os = "windows")]
impl Host {
    pub(super) fn start(changed: &dyn Fn() -> bool) -> Result<Self, String> {
        Self::start_with(super::windows_command(std::env::var_os("SystemRoot"), HOST_SCRIPT).command(), changed)
    }

    fn start_with(mut command: Command, changed: &dyn Fn() -> bool) -> Result<Self, String> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Read Aloud host has no input".into());
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Read Aloud host has no output".into());
        };
        let (tx, lines) = channel();
        let reader = std::thread::Builder::new()
            .name("read-aloud-host".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| {
                let _ = child.kill();
                let _ = child.wait();
                e.to_string()
            })?;
        // Sentence ids start at 1: id 0 is the host's "could not parse the line" sentinel.
        let host = Self { child, stdin, lines, reader: Some(reader), next_id: 1, broken: false };
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if changed() {
                return Err("Read Aloud host start cancelled".into());
            }
            match host.lines.recv_timeout(Duration::from_millis(30)) {
                Ok(line) if line.trim() == "ready" => return Ok(host),
                Ok(line) => return Err(format!("unexpected Read Aloud host greeting: {line}")),
                Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    return Err("Read Aloud host did not start".into());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err("Read Aloud host stopped".into()),
            }
        }
    }

    pub(super) fn speak(&mut self, text: &str, lang: Option<&str>, rate: f32, changed: &dyn Fn() -> bool) -> Result<Spoken, HostError> {
        if self.broken {
            return Err(HostError::Unavailable);
        }
        if changed() {
            return Ok(Spoken::Interrupted);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let line = speak_message(id, text, lang, rate);
        if writeln!(self.stdin, "{line}").and_then(|_| self.stdin.flush()).is_err() {
            self.broken = true;
            return Err(HostError::Unavailable);
        }
        // Any failure falls back to the per-sentence path. If the sentence had started, that path
        // repeats it from the beginning, so the part already spoken may be heard twice.
        let mut started = false;
        let deadline = Instant::now() + SPEAK_TIMEOUT;
        loop {
            if changed() {
                self.stop(id);
                return Ok(Spoken::Interrupted);
            }
            match self.lines.recv_timeout(Duration::from_millis(30)) {
                Ok(line) => match parse_reply(&line) {
                    Some(Reply::Started(got)) if got == id => started = true,
                    Some(Reply::Done(got)) if got == id => return Ok(Spoken::Done),
                    Some(Reply::Stopped(got)) if got == id => return Ok(Spoken::Interrupted),
                    Some(Reply::Error(0, _)) => {
                        // Id 0 is the host's parser sentinel, never a sentence id.
                        return Err(HostError::Unavailable);
                    }
                    Some(Reply::Error(got, code)) if got == id => {
                        if !started && code == "voice" {
                            return Err(HostError::Voice);
                        }
                        return Err(HostError::Unavailable);
                    }
                    _ => {}
                },
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        self.broken = true;
                        return Err(HostError::Unavailable);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.broken = true;
                    return Err(HostError::Unavailable);
                }
            }
        }
    }

    fn stop(&mut self, id: u64) {
        let _ = writeln!(self.stdin, "{{\"cmd\":\"stop\",\"id\":{id}}}");
        let _ = self.stdin.flush();
        let stopped = format!("stopped {id}");
        let done = format!("done {id}");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match self.lines.recv_timeout(Duration::from_millis(30)) {
                Ok(line) => {
                    let line = line.trim();
                    if line == stopped.as_str() || line == done.as_str() {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.broken = true;
                    return;
                }
            }
        }
        self.broken = true;
    }
}

#[cfg(target_os = "windows")]
impl Drop for Host {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{{\"cmd\":\"quit\"}}");
        let _ = self.stdin.flush();
        let mut exited = wait_for_exit(&mut self.child, Duration::from_millis(500));
        if !exited {
            let _ = self.child.kill();
            exited = wait_for_exit(&mut self.child, Duration::from_secs(1));
        }
        if exited {
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        } else {
            // The child outlived `kill`; joining its reader could block forever on its stdout.
            let _ = self.reader.take();
        }
    }
}

/// Wait up to `timeout` for `child` to exit; `false` if it is still running or reaping failed.
#[cfg(target_os = "windows")]
fn wait_for_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the real host: no System.Speech, so these tests run without audio and can
    /// drive the failure paths the real host cannot be made to hit on demand.
    #[cfg(target_os = "windows")]
    const FAKE_HOST_SCRIPT: &str = r#"$utf8 = New-Object System.Text.UTF8Encoding($false)
$reader = New-Object System.IO.StreamReader([Console]::OpenStandardInput(), $utf8)
$out = New-Object System.IO.StreamWriter([Console]::OpenStandardOutput(), $utf8)
$out.AutoFlush = $true
$out.WriteLine('ready')
while ($null -ne ($line = $reader.ReadLine())) {
    $m = $line | ConvertFrom-Json
    $id = [string]$m.id
    switch ([string]$m.cmd) {
        'speak' {
            $text = [string]$m.text
            if ($text -eq 'voice') {
                $out.WriteLine('error ' + $id + ' voice')
            } elseif ($text -eq 'parse') {
                $out.WriteLine('error 0 other')
            } elseif ($text -eq 'crash') {
                $out.WriteLine('started ' + $id)
                exit 1
            } elseif ($text -eq 'late') {
                $out.WriteLine('done ' + $id)
            } elseif ($text -eq 'wait') {
                $out.WriteLine('started ' + $id)
                Start-Sleep -Milliseconds 200
                $out.WriteLine('done ' + $id)
            } else {
                $out.WriteLine('started ' + $id)
                $out.WriteLine('done ' + $id)
            }
        }
        'stop' { $out.WriteLine('stopped ' + $id) }
        'quit' { exit 0 }
    }
}
"#;

    #[cfg(target_os = "windows")]
    fn fake_host() -> Host {
        Host::start_with(super::super::windows_command(None, FAKE_HOST_SCRIPT).command(), &|| false).unwrap()
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn fake_host_start_fails_if_it_exits_before_ready() {
        let err = Host::start_with(super::super::windows_command(None, "exit 1").command(), &|| false);
        assert!(err.is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn fake_host_reports_done_including_before_started() {
        let mut host = fake_host();
        assert!(matches!(host.speak("ok", None, 1.0, &|| false), Ok(Spoken::Done)));
        assert!(matches!(host.speak("late", None, 1.0, &|| false), Ok(Spoken::Done)));
        assert!(matches!(host.speak("wait", None, 1.0, &|| false), Ok(Spoken::Done)));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn fake_host_reports_voice_and_protocol_errors() {
        let mut host = fake_host();
        assert!(matches!(host.speak("voice", None, 1.0, &|| false), Err(HostError::Voice)));
        assert!(matches!(host.speak("parse", None, 1.0, &|| false), Err(HostError::Unavailable)));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn fake_host_falls_back_when_it_dies_after_started() {
        let mut host = fake_host();
        assert!(matches!(host.speak("crash", None, 1.0, &|| false), Err(HostError::Unavailable)));
        assert!(host.broken);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn fake_host_stops_when_idle_and_quits() {
        let mut host = fake_host();
        host.stop(99);
        assert!(!host.broken);
        drop(host);
    }

    #[test]
    fn host_script_keeps_one_synthesizer_alive() {
        for needle in [
            "Add-Type -AssemblyName System.Speech",
            "SpeechSynthesizer",
            "SpeakAsync",
            "SpeakAsyncCancelAll",
            "IsCompleted",
            "ReadLineAsync",
            "ConvertFrom-Json",
            "AutoFlush",
        ] {
            assert!(HOST_SCRIPT.contains(needle), "{needle}");
        }
    }

    #[test]
    fn speak_message_is_one_json_line() {
        let line = speak_message(7, "Привет, \"мир\".", Some("ru-RU"), 1.0);
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["cmd"], "speak");
        assert_eq!(v["id"], 7);
        assert_eq!(v["rate"], 0);
        assert_eq!(v["lang"], "ru");
        assert_eq!(v["text"], "Привет, \"мир\".");
        assert!(v["voice"].is_null());
    }

    #[test]
    fn speak_message_escapes_control_characters() {
        let text = "line\nbreak\ttab\\slash\"quote\u{1}";
        let line = speak_message(1, text, None, 1.0);
        assert!(!line.contains('\n') && !line.contains('\t'), "{line}");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["text"], text);
        assert_eq!(v["lang"], "");
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "starts a real PowerShell speech host"]
    fn host_round_trips_a_speak_command() {
        let mut host = Host::start(&|| false).unwrap();
        assert!(matches!(host.speak("", None, 1.0, &|| false), Ok(Spoken::Done)));
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "starts a real PowerShell speech host"]
    fn host_reuses_one_process_for_two_sentences() {
        let mut host = Host::start(&|| false).unwrap();
        assert_eq!(host.next_id, 1, "sentence ids start at 1; 0 is the parser sentinel");
        assert!(matches!(host.speak("", None, 1.0, &|| false), Ok(Spoken::Done)));
        assert!(matches!(host.speak("", None, 1.0, &|| false), Ok(Spoken::Done)));
        assert_eq!(host.next_id, 3);
        host.stop(99);
        assert!(!host.broken, "an idle stop must not break the host");
    }

    #[test]
    fn replies_are_parsed() {
        assert_eq!(parse_reply("started 3"), Some(Reply::Started(3)));
        assert_eq!(parse_reply("done 3"), Some(Reply::Done(3)));
        assert_eq!(parse_reply("stopped 3"), Some(Reply::Stopped(3)));
        assert_eq!(parse_reply("error 3 voice"), Some(Reply::Error(3, "voice".into())));
        assert_eq!(parse_reply("error 0 other"), Some(Reply::Error(0, "other".into())));
        assert_eq!(parse_reply("done 3 "), Some(Reply::Done(3)));
        assert_eq!(parse_reply(" error 3 voice "), Some(Reply::Error(3, "voice".into())));
        assert_eq!(parse_reply("error 3"), None);
        assert_eq!(parse_reply("error"), None);
        assert_eq!(parse_reply("what 3"), None);
        assert_eq!(parse_reply("started"), None);
    }
}
