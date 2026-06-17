//! Built-in MCP tools the chat runner can dispatch locally without
//! going through an external server.
//!
//! Surfaced through [`crate::mcp::builtin_registry`] so both the chat
//! catalog (so the model sees them in its system prompt) and the
//! `mcp_list_builtins` IPC command (so the Tools page can render them)
//! pull from the same source of truth.

pub mod clipboard;
pub mod code;
pub mod discovery;
pub mod fs;
pub mod http;
pub mod memory;
pub mod shell;
pub mod task;
pub mod web;
