//! WordCraft in the browser: craft-live mode ([`live`]: the web build as a live editor inside
//! Nextcloud Office, through craft-live), whose protocol is plain Rust, built and tested natively
//! too. The eframe web shell that runs it is the `wordcraft-web` binary (`web`, wasm only).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod live;
