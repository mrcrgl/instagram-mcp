//! What each sealed value contains. These types are the server's state: they
//! travel with the client or the browser instead of living in a store.

use serde::{Deserialize, Serialize};

use crate::domain::tenant::{Credential, Tenant};

/// `client_id`: the registration itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientClaims {
    pub redirect_uris: Vec<String>,
    pub client_name: Option<String>,
}

/// A validated authorization request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationRequest {
    /// Short stand-in for the client id, which would otherwise bloat every
    /// URL that carries this request.
    pub client_fingerprint: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub state: Option<String>,
}

/// Issued with the consent page; valid only in the browser holding `nonce`.
#[derive(Serialize, Deserialize)]
pub struct ConsentClaims {
    pub request: AuthorizationRequest,
    pub nonce: String,
}

/// `state` sent to Facebook for the sign-in.
#[derive(Serialize, Deserialize)]
pub struct FlowClaims {
    pub request: AuthorizationRequest,
    pub nonce: String,
}

#[derive(Serialize, Deserialize)]
pub struct CodeClaims {
    pub client_fingerprint: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub tenant: Tenant,
    pub credential: Credential,
    pub credential_expires_at: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct AccessClaims {
    pub tenant: Tenant,
    pub credential: Credential,
    /// RFC 8707 audience: the MCP endpoint this token is valid for.
    pub audience: String,
}

#[derive(Serialize, Deserialize)]
pub struct RefreshClaims {
    pub client_fingerprint: String,
    pub user_id: String,
    pub credential: Credential,
    pub credential_expires_at: Option<u64>,
}
