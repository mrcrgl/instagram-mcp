//! Read-only MCP server for Instagram professional accounts.
//!
//! Dependencies point one way: `mcp` and `oauth` (transport) call into
//! `instagram` (application), which depends on `domain` and its storage port.
//! `meta` implements the ports; nothing outside `app` and `main` knows which
//! adapter is wired in.

pub mod app;
pub mod config;
pub mod domain;
pub mod instagram;
pub mod mcp;
pub mod meta;
pub mod oauth;
