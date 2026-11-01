//! `wordcraft-cli mcp --allow-commands` at startup, through the real binary: a flag without its
//! list, an oversized `@FILE` or one that is a stream, and `--connect` all stop the server before
//! it serves; a good list starts it held to that list.

use std::process::{Command, Output};

/// Run the CLI with `args`. `output()` gives the child no stdin, so a server that does start
/// ends at once instead of waiting for a client.
fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(args).output().unwrap()
}

/// The run must fail with `needle` on stderr, without ever having served.
fn refused(args: &[&str], needle: &str) {
    let out = cli(args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{args:?} should not start: {stderr}");
    assert!(stderr.contains(needle), "{args:?}: expected `{needle}` in: {stderr}");
    assert!(!stderr.contains("MCP server on stdio") && !stderr.contains("commands allowed"), "{args:?} served: {stderr}");
}

#[test]
fn a_flag_without_its_list_never_starts_the_server() {
    refused(&["mcp", "--allow-commands"], "--allow-commands requires a list");
    refused(&["mcp", "--author", "Claude", "--allow-commands"], "--allow-commands requires a list");
    refused(&["mcp", "--allow-commands", "--author", "Claude"], "--allow-commands requires a list");
    refused(&["mcp", "--allow-commands", "format.*", "--allow-commands", "text.*"], "duplicate --allow-commands");
}

#[test]
fn connect_is_not_a_way_around_the_check() {
    refused(&["mcp", "--connect", "127.0.0.1:9", "--allow-commands"], "--allow-commands requires a list");
    refused(&["mcp", "--allow-commands", "--connect", "127.0.0.1:9"], "--allow-commands requires a list");
    refused(&["mcp", "--connect", "127.0.0.1:9", "--allow-commands", "format.*"], "running app");
}

#[test]
fn an_oversized_list_file_is_refused() {
    let dir = std::env::temp_dir().join(format!("wordcraft-it-allowlist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.txt");
    // Valid ids up front, so only the size can be the reason.
    let mut text = String::from("format.bold\n");
    text.push_str(&"#".repeat(256 * 1024));
    std::fs::write(&big, text).unwrap();
    refused(&["mcp", "--allow-commands", &format!("@{}", big.display())], "too long");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn streams_are_refused_at_once() {
    refused(&["mcp", "--allow-commands", "@/dev/zero"], "not a regular file");
    // A FIFO that nobody writes to: startup must not block on it.
    let dir = std::env::temp_dir().join(format!("wordcraft-it-fifo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fifo = dir.join("list");
    assert!(Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()), "mkfifo");
    refused(&["mcp", "--allow-commands", &format!("@{}", fifo.display())], "not a regular file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_good_list_starts_the_server_held_to_it() {
    let out = cli(&["mcp", "--allow-commands", "format.bold, document.text"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("commands allowed: document.text, format.bold"), "{stderr}");
}
