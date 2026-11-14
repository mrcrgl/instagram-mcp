//! Stateless OAuth 2.1 authorization server for MCP clients, in front of
//! Facebook Login.
//!
//! Nothing is stored. Client ids, authorization codes, and tokens are values
//! sealed by `seal`, so any replica can validate them. Access and refresh
//! tokens carry the user's long-lived Meta token, encrypted, because Instagram
//! has no app-level credential that could read a user's data on their behalf.

pub mod bearer;
pub mod claims;
pub mod dto;
pub mod identity;
pub mod pages;
pub mod pkce;
pub mod redirect;
pub mod routes;
pub mod seal;
pub mod service;
