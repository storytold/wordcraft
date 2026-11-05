//! WordCraft assistant.
//!
//! A conversational agent that carries out writing requests ("draft an introduction", "rewrite
//! this paragraph", "turn these lines into a table") with WordCraft's own commands. The model
//! comes from the host as a [`Provider`] that takes chat-completions requests; the agent's tools
//! are the MCP tools of `wordcraft-mcp`, run against any [`wordcraft_mcp::Backend`] (a headless
//! session or the app). Model output is treated as hostile input: bounded steps, tool calls and
//! result sizes, and tools that replace the document or write files run only when the user
//! confirms.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod agent;
mod provider;

pub use agent::{Agent, Config, Error, Event, MAX_STEPS, Observer, Turn};
pub use provider::{Delta, Provider, Scripted, text_reply, tool_reply};

#[cfg(test)]
mod tests;
