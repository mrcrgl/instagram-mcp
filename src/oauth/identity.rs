//! Identity port: what the authorization server needs from Meta.

use std::future::Future;
use std::pin::Pin;

use crate::domain::tenant::{Credential, IgAccount};

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = Result<T, IdentityError>> + Send + 'a>>;

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The code or token is invalid, expired, or was revoked.
    #[error("the grant was rejected")]
    Rejected,
    #[error("identity provider unavailable: {0}")]
    Unavailable(String),
}

/// A Meta user token and when it stops working, if Meta said.
pub struct GrantedToken {
    pub credential: Credential,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityUser {
    pub id: String,
}

pub trait IdentityProvider: Send + Sync {
    fn authorize_url(&self, state: &str, redirect_uri: &str) -> String;
    /// Exchanges the sign-in code for a long-lived user token.
    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        redirect_uri: &'a str,
    ) -> BoxFut<'a, GrantedToken>;
    /// Extends a long-lived token. Meta may return the same token.
    fn extend<'a>(&'a self, credential: &'a Credential) -> BoxFut<'a, GrantedToken>;
    fn user<'a>(&'a self, credential: &'a Credential) -> BoxFut<'a, IdentityUser>;
    /// Instagram professional accounts linked to the Pages the user granted.
    fn accounts<'a>(&'a self, credential: &'a Credential) -> BoxFut<'a, Vec<IgAccount>>;
}
