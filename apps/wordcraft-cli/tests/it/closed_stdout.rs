//! Readers closing stdout early must not panic (#508). Listing and MCP stop quietly;
//! validation retains its verdict exit status.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
}

#[test]
fn commands_json_into_a_closed_pipe_exits_quietly() {
    let mut child = cli().args(["commands", "--json"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // The listing is far larger than a pipe buffer: read a little, then hang up like `head -c`.
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0u8; 16];
    stdout.read_exact(&mut first).unwrap();
    drop(stdout);
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "status {:?}, stderr: {stderr}", out.status);
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
}

#[test]
fn mcp_server_ends_cleanly_when_its_client_closes_stdout() {
    let mut child = cli().arg("mcp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    drop(child.stdout.take());
    let mut stdin = child.stdin.take().unwrap();
    // The reply to this request has nowhere to go.
    let _ = stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "status {:?}, stderr: {stderr}", out.status);
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
}

/// A closed reader preserves validation's verdict for both report formats, including INCOMPLETE.
#[test]
fn validate_closed_stdout_preserves_verdict() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/validate/corpus/calibration");
    let sparse = std::env::temp_dir().join(format!("wordcraft-validate-closed-stdout-{}.docx", std::process::id()));
    // The validator checks metadata before reading inputs over its 512 MiB limit.
    std::fs::File::create(&sparse).unwrap().set_len((512 << 20) + 1).unwrap();
    let pass = sparse.with_file_name(format!("wordcraft-validate-closed-stdout-pass-{}.docx", std::process::id()));
    let bytes = wordcraft_engine::io::save_bytes("pass.docx", &wordcraft_doc::Document::from_text("Control")).unwrap();
    std::fs::write(&pass, bytes).unwrap();
    let mut results = Vec::new();
    for (fixture, expected) in [(pass.clone(), 0), (corpus.join("14-comment-dup.docx"), 1), (sparse.clone(), 2)] {
        for json in [false, true] {
            let mut open = cli();
            open.arg("validate").arg(&fixture);
            if json {
                open.arg("--json");
            }
            let open = open.output().unwrap();
            // Close the only reader before spawning, so even a short report reliably gets BrokenPipe.
            let (reader, writer) = std::io::pipe().unwrap();
            drop(reader);
            let mut closed = cli();
            closed.arg("validate").arg(&fixture).stdout(writer);
            if json {
                closed.arg("--json");
            }
            results.push((fixture.clone(), json, expected, open, closed.output().unwrap()));
        }
    }
    std::fs::remove_file(sparse).unwrap();
    std::fs::remove_file(pass).unwrap();
    for (fixture, json, expected, open, closed) in results {
        assert_eq!(open.status.code(), Some(expected), "{}, json={json}: {open:?}", fixture.display());
        assert_eq!(closed.status.code(), Some(expected), "{}, json={json}: {closed:?}", fixture.display());
        assert!(!String::from_utf8_lossy(&closed.stderr).contains("panicked"), "{closed:?}");
    }
}

/// Other report-write failures are I/O status 3; a child-only file-size limit forces EFBIG on stdout.
#[test]
fn validate_stdout_write_error_is_io_status() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/validate/corpus/calibration/14-comment-dup.docx");
    let stdout = std::env::temp_dir().join(format!("wordcraft-validate-stdout-error-{}", std::process::id()));
    let mut results = Vec::new();
    for json in [false, true] {
        // Ignore SIGXFSZ so the write returns an error rather than terminating the CLI.
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "trap '' XFSZ; ulimit -f 0 || exit 99; exec \"$@\"", "sh", env!("CARGO_BIN_EXE_wordcraft-cli"), "validate"]);
        command.arg(&fixture).stdout(std::fs::File::create(&stdout).unwrap());
        if json {
            command.arg("--json");
        }
        results.push((json, command.output().unwrap()));
    }
    std::fs::remove_file(stdout).unwrap();
    for (json, output) in results {
        assert_eq!(output.status.code(), Some(3), "json={json}: {output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"), "{output:?}");
    }
}
