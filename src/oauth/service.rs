//! The authorization flow. Handlers stay thin; every protocol decision is here.
//!
//! Browser path: `/authorize` → consent → Facebook Login → back to the client
//! with a code. The code and every token carry the user's long-lived Meta
//! token, sealed, and the Instagram accounts it could reach at sign-in.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::domain::tenant::{Credential, Session, Tenant};
use crate::oauth::claims::{
    AccessClaims, AuthorizationRequest, ClientClaims, CodeClaims, ConsentClaims, FlowClaims,
    RefreshClaims,
};
use crate::oauth::dto::{
    AuthorizationServerMetadata, AuthorizeParams, MetaCallbackParams, RegistrationRequest,
    RegistrationResponse, ResourceMetadata, TokenRequest, TokenResponse,
};
use crate::oauth::identity::{GrantedToken, IdentityError, IdentityProvider};
use crate::oauth::seal::{Purpose, SealError, Sealer, random_nonce};
use crate::oauth::{pkce, redirect};

pub const SCOPE: &str = "instagram.read";
pub const FLOW_TTL: u64 = 30 * 60;
const CONSENT_TTL: u64 = 10 * 60;
const CODE_TTL: u64 = 60;
const ACCESS_TTL: u64 = 60 * 60;
/// Meta's long-lived user tokens last 60 days. Used when Meta does not say.
const DEFAULT_CREDENTIAL_TTL: u64 = 60 * 24 * 60 * 60;

const MAX_REDIRECT_URIS: usize = 5;
const MAX_REDIRECT_URI_CHARS: usize = 512;
const MAX_CLIENT_NAME_CHARS: usize = 100;
const MAX_STATE_CHARS: usize = 1024;

pub const CALLBACK_PATH: &str = "/oauth/meta/callback";

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("{0}")]
    InvalidRedirectUri(String),
    #[error("{0}")]
    InvalidMetadata(String),
    #[error("registration could not be completed")]
    Internal,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthorizeError {
    /// The client is unverified, so the error must not leave this server.
    #[error("{0}")]
    Page(String),
    #[error("redirecting an error to the client")]
    Redirect(String),
}

#[derive(Debug, thiserror::Error)]
pub enum FlowError {
    #[error("{0}")]
    Rejected(String),
    #[error("Meta could not be reached: try again in a moment")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    InvalidGrant(String),
    #[error("{0}")]
    InvalidTarget(String),
    #[error("unsupported grant type")]
    UnsupportedGrantType,
    #[error("try again later")]
    TemporarilyUnavailable,
    #[error("the server could not issue a token")]
    Internal,
}

pub struct ConsentView {
    pub client_name: Option<String>,
    pub target: String,
    pub consent_token: String,
    pub nonce: String,
}

pub struct OAuthService {
    sealer: Sealer,
    identity: Arc<dyn IdentityProvider>,
    base_url: String,
    clock: fn() -> u64,
}

impl OAuthService {
    pub fn new(
        sealer: Sealer,
        identity: Arc<dyn IdentityProvider>,
        base_url: String,
        clock: fn() -> u64,
    ) -> Self {
        Self {
            sealer,
            identity,
            base_url,
            clock,
        }
    }

    /// The RFC 8707 resource identifier of the MCP endpoint.
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    pub fn resource_metadata_url(&self) -> String {
        format!("{}/.well-known/oauth-protected-resource/mcp", self.base_url)
    }

    pub fn resource_metadata(&self) -> ResourceMetadata {
        ResourceMetadata {
            resource: self.resource(),
            authorization_servers: vec![self.base_url.clone()],
            scopes_supported: [SCOPE],
            bearer_methods_supported: ["header"],
        }
    }

    pub fn server_metadata(&self) -> AuthorizationServerMetadata {
        AuthorizationServerMetadata {
            issuer: self.base_url.clone(),
            authorization_endpoint: format!("{}/authorize", self.base_url),
            token_endpoint: format!("{}/token", self.base_url),
            registration_endpoint: format!("{}/register", self.base_url),
            scopes_supported: [SCOPE],
            response_types_supported: ["code"],
            grant_types_supported: ["authorization_code", "refresh_token"],
            code_challenge_methods_supported: ["S256"],
            token_endpoint_auth_methods_supported: ["none"],
            authorization_response_iss_parameter_supported: true,
        }
    }

    /// Dynamic client registration (RFC 7591). The `client_id` is the sealed
    /// registration, so an open endpoint has nothing to fill up.
    pub fn register(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationResponse, RegistrationError> {
        validate_redirect_uris(&request.redirect_uris)?;
        if let Some(grants) = &request.grant_types
            && !grants.iter().any(|grant| grant == "authorization_code")
        {
            return Err(RegistrationError::InvalidMetadata(
                "grant_types must include authorization_code".into(),
            ));
        }
        if let Some(responses) = &request.response_types
            && !responses.iter().any(|response| response == "code")
        {
            return Err(RegistrationError::InvalidMetadata(
                "response_types must include code".into(),
            ));
        }

        let client_name = request.client_name.as_deref().map(sanitize_client_name);
        let claims = ClientClaims {
            redirect_uris: request.redirect_uris,
            client_name,
        };
        let client_id = self
            .sealer
            .seal(Purpose::Client, &claims, None)
            .map_err(|_| RegistrationError::Internal)?;
        Ok(RegistrationResponse {
            client_id,
            client_id_issued_at: (self.clock)(),
            client_name: claims.client_name,
            redirect_uris: claims.redirect_uris,
            grant_types: ["authorization_code", "refresh_token"],
            response_types: ["code"],
            token_endpoint_auth_method: "none",
        })
    }

    /// Validates an authorization request and prepares the consent page.
    /// `browser_nonce` is the nonce the browser already holds, if any: reusing
    /// it lets two flows run side by side in one browser.
    pub fn begin_authorization(
        &self,
        params: &AuthorizeParams,
        browser_nonce: Option<&str>,
    ) -> Result<ConsentView, AuthorizeError> {
        let client_id = params
            .client_id
            .as_deref()
            .ok_or_else(|| AuthorizeError::Page("client_id is missing".into()))?;
        let client: ClientClaims = self
            .sealer
            .open(Purpose::Client, client_id, self.now())
            .map_err(|_| AuthorizeError::Page("This client is not registered here.".into()))?;
        let redirect_uri = params
            .redirect_uri
            .as_deref()
            .ok_or_else(|| AuthorizeError::Page("redirect_uri is missing".into()))?;
        if !client
            .redirect_uris
            .iter()
            .any(|registered| redirect::matches_registered(registered, redirect_uri))
        {
            return Err(AuthorizeError::Page(
                "This redirect URI was not registered by the client.".into(),
            ));
        }

        // From here on the redirect URI is trusted, so errors go to the client.
        let state = params.state.as_deref();
        let fail = |code: &str, description: &str| {
            AuthorizeError::Redirect(self.client_error(redirect_uri, state, code, description))
        };

        if params.response_type.as_deref() != Some("code") {
            return Err(fail(
                "unsupported_response_type",
                "response_type must be code",
            ));
        }
        let code_challenge = params
            .code_challenge
            .as_deref()
            .filter(|challenge| pkce::is_valid_challenge(challenge))
            .ok_or_else(|| fail("invalid_request", "a valid PKCE code_challenge is required"))?;
        if params.code_challenge_method.as_deref() != Some("S256") {
            return Err(fail(
                "invalid_request",
                "code_challenge_method must be S256",
            ));
        }
        if state.is_some_and(|value| value.chars().count() > MAX_STATE_CHARS) {
            return Err(fail("invalid_request", "state is too long"));
        }
        if !self.resource_matches(params.resource.as_deref()) {
            return Err(fail(
                "invalid_target",
                "this server only serves its own resource",
            ));
        }
        if let Some(scope) = &params.scope
            && scope.split_whitespace().any(|requested| requested != SCOPE)
        {
            return Err(fail(
                "invalid_scope",
                "only the instagram.read scope exists",
            ));
        }

        let nonce = match browser_nonce {
            Some(existing) => existing.to_string(),
            None => random_nonce().map_err(|_| internal_page())?,
        };
        let request = AuthorizationRequest {
            client_fingerprint: fingerprint(client_id),
            redirect_uri: redirect_uri.to_string(),
            code_challenge: code_challenge.to_string(),
            state: state.map(String::from),
        };
        let consent_token = self
            .sealer
            .seal(
                Purpose::Consent,
                &ConsentClaims {
                    request,
                    nonce: nonce.clone(),
                },
                Some(self.now() + CONSENT_TTL),
            )
            .map_err(|_| internal_page())?;

        Ok(ConsentView {
            client_name: client.client_name,
            target: redirect::display_target(redirect_uri),
            consent_token,
            nonce,
        })
    }

    /// Returns the next URL for the browser. This consent is what stops a
    /// dynamically registered client from riding on a Facebook authorization
    /// the user gave earlier, which Facebook would not ask about again.
    pub fn decide_consent(
        &self,
        consent_token: &str,
        browser_nonce: Option<&str>,
        allow: bool,
    ) -> Result<String, FlowError> {
        let consent: ConsentClaims = self
            .sealer
            .open(Purpose::Consent, consent_token, self.now())
            .map_err(|_| expired_flow())?;
        require_same_browser(&consent.nonce, browser_nonce)?;

        if !allow {
            return Ok(self.client_error(
                &consent.request.redirect_uri,
                consent.request.state.as_deref(),
                "access_denied",
                "the user declined",
            ));
        }
        let state = self
            .sealer
            .seal(
                Purpose::Flow,
                &FlowClaims {
                    request: consent.request,
                    nonce: consent.nonce,
                },
                Some(self.now() + FLOW_TTL),
            )
            .map_err(|_| internal_flow())?;
        Ok(self.identity.authorize_url(&state, &self.callback_url()))
    }

    /// Facebook Login's redirect back to this server.
    pub async fn meta_callback(
        &self,
        params: MetaCallbackParams,
        browser_nonce: Option<&str>,
    ) -> Result<String, FlowError> {
        let state = params.state.as_deref().ok_or_else(expired_flow)?;
        let flow: FlowClaims = self
            .sealer
            .open(Purpose::Flow, state, self.now())
            .map_err(|_| expired_flow())?;
        require_same_browser(&flow.nonce, browser_nonce)?;

        if params.error.is_some() {
            return Ok(self.client_error(
                &flow.request.redirect_uri,
                flow.request.state.as_deref(),
                "access_denied",
                "Facebook sign-in was not completed",
            ));
        }
        let code = params.code.as_deref().ok_or_else(expired_flow)?;
        let granted = self
            .identity
            .exchange_code(code, &self.callback_url())
            .await
            .map_err(flow_error)?;
        let tenant = self
            .tenant_for(&granted.credential)
            .await
            .map_err(flow_error)?;
        if tenant.accounts.is_empty() {
            return Err(FlowError::Rejected(
                "No Instagram professional account was shared. Link your Instagram business or \
                 creator account to a Facebook Page, then connect again and select that Page \
                 and its Instagram account when Facebook asks."
                    .into(),
            ));
        }

        let code = self
            .sealer
            .seal(
                Purpose::Code,
                &CodeClaims {
                    client_fingerprint: flow.request.client_fingerprint,
                    redirect_uri: flow.request.redirect_uri.clone(),
                    code_challenge: flow.request.code_challenge,
                    tenant,
                    credential: granted.credential,
                    credential_expires_at: granted.expires_at,
                },
                Some(self.now() + CODE_TTL),
            )
            .map_err(|_| internal_flow())?;
        let mut params = vec![("code", code.as_str()), ("iss", self.base_url.as_str())];
        if let Some(state) = &flow.request.state {
            params.push(("state", state));
        }
        redirect::with_params(&flow.request.redirect_uri, &params).map_err(|_| internal_flow())
    }

    async fn tenant_for(&self, credential: &Credential) -> Result<Tenant, IdentityError> {
        let user = self.identity.user(credential).await?;
        let accounts = self.identity.accounts(credential).await?;
        Ok(Tenant {
            user_id: user.id,
            accounts,
        })
    }

    /// The token endpoint.
    pub async fn token(&self, request: TokenRequest) -> Result<TokenResponse, TokenError> {
        if !self.resource_matches(request.resource.as_deref()) {
            return Err(TokenError::InvalidTarget(
                "this server only serves its own resource".into(),
            ));
        }
        match request.grant_type.as_deref() {
            Some("authorization_code") => self.exchange_code(request),
            Some("refresh_token") => self.refresh(request).await,
            _ => Err(TokenError::UnsupportedGrantType),
        }
    }

    fn exchange_code(&self, request: TokenRequest) -> Result<TokenResponse, TokenError> {
        let code = required(request.code.as_deref(), "code")?;
        let client_id = required(request.client_id.as_deref(), "client_id")?;
        let redirect_uri = required(request.redirect_uri.as_deref(), "redirect_uri")?;
        let verifier = required(request.code_verifier.as_deref(), "code_verifier")?;

        let claims: CodeClaims = self
            .sealer
            .open(Purpose::Code, code, self.now())
            .map_err(|_| invalid_grant("the code is invalid or expired"))?;
        if claims.client_fingerprint != fingerprint(client_id) {
            return Err(invalid_grant("the code was issued to another client"));
        }
        if claims.redirect_uri != redirect_uri {
            return Err(invalid_grant("redirect_uri does not match the code"));
        }
        if !pkce::verify(verifier, &claims.code_challenge) {
            return Err(invalid_grant("PKCE verification failed"));
        }
        tracing::info!(
            user_id = %claims.tenant.user_id,
            accounts = claims.tenant.accounts.len(),
            "authorization completed"
        );
        self.issue_tokens(
            claims.client_fingerprint,
            claims.tenant,
            GrantedToken {
                credential: claims.credential,
                expires_at: claims.credential_expires_at,
            },
        )
    }

    /// Extends the embedded Meta token where Meta allows it, re-checks that it
    /// still works for the same user, and re-reads the accounts it reaches, so
    /// a Page added or removed in Meta shows up at the next refresh.
    async fn refresh(&self, request: TokenRequest) -> Result<TokenResponse, TokenError> {
        let refresh_token = required(request.refresh_token.as_deref(), "refresh_token")?;
        let client_id = required(request.client_id.as_deref(), "client_id")?;
        let claims: RefreshClaims = self
            .sealer
            .open(Purpose::Refresh, refresh_token, self.now())
            .map_err(|_| invalid_grant("the refresh token is invalid or expired"))?;
        if claims.client_fingerprint != fingerprint(client_id) {
            return Err(invalid_grant("the refresh token belongs to another client"));
        }

        // Meta does not always extend a long-lived token. Keeping the current
        // one is fine: the refresh token expires with it.
        let granted = match self.identity.extend(&claims.credential).await {
            Ok(extended) => extended,
            Err(IdentityError::Rejected) => GrantedToken {
                credential: claims.credential,
                expires_at: claims.credential_expires_at,
            },
            Err(other) => return Err(refresh_error(other)),
        };
        let tenant = self
            .tenant_for(&granted.credential)
            .await
            .map_err(refresh_error)?;
        if tenant.user_id != claims.user_id {
            return Err(invalid_grant("the refresh token belongs to another user"));
        }
        self.issue_tokens(claims.client_fingerprint, tenant, granted)
    }

    fn issue_tokens(
        &self,
        client_fingerprint: String,
        tenant: Tenant,
        granted: GrantedToken,
    ) -> Result<TokenResponse, TokenError> {
        let now = self.now();
        let credential_expires_at = granted.expires_at.unwrap_or(now + DEFAULT_CREDENTIAL_TTL);
        if credential_expires_at <= now {
            return Err(invalid_grant("the Meta session has expired: sign in again"));
        }
        // Neither token may outlive the Meta token inside it.
        let access_expires_at = (now + ACCESS_TTL).min(credential_expires_at);
        let access_token = self
            .sealer
            .seal(
                Purpose::Access,
                &AccessClaims {
                    tenant: tenant.clone(),
                    credential: granted.credential.clone(),
                    audience: self.resource(),
                },
                Some(access_expires_at),
            )
            .map_err(|_| TokenError::Internal)?;
        let refresh_token = self
            .sealer
            .seal(
                Purpose::Refresh,
                &RefreshClaims {
                    client_fingerprint,
                    user_id: tenant.user_id,
                    credential: granted.credential,
                    credential_expires_at: Some(credential_expires_at),
                },
                Some(credential_expires_at),
            )
            .map_err(|_| TokenError::Internal)?;
        Ok(TokenResponse {
            access_token,
            token_type: "Bearer",
            expires_in: access_expires_at - now,
            refresh_token,
            scope: SCOPE,
        })
    }

    /// The only way a `Session` comes into existence on the MCP path.
    pub fn authenticate(&self, access_token: &str) -> Result<Session, SealError> {
        let claims: AccessClaims = self
            .sealer
            .open(Purpose::Access, access_token, self.now())?;
        if claims.audience != self.resource() {
            return Err(SealError::Invalid);
        }
        Ok(Session {
            tenant: claims.tenant,
            credential: claims.credential,
        })
    }

    /// Missing is accepted because a client without RFC 8707 support can only
    /// mean this server; the token is audience-bound either way.
    fn resource_matches(&self, requested: Option<&str>) -> bool {
        requested.is_none_or(|resource| resource.trim_end_matches('/') == self.resource())
    }

    fn client_error(
        &self,
        redirect_uri: &str,
        state: Option<&str>,
        code: &str,
        description: &str,
    ) -> String {
        let mut params = vec![
            ("error", code),
            ("error_description", description),
            ("iss", self.base_url.as_str()),
        ];
        if let Some(state) = state {
            params.push(("state", state));
        }
        // The URI parsed when it was registered.
        redirect::with_params(redirect_uri, &params).unwrap_or_else(|_| redirect_uri.to_string())
    }

    fn callback_url(&self) -> String {
        format!("{}{CALLBACK_PATH}", self.base_url)
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }
}

fn validate_redirect_uris(uris: &[String]) -> Result<(), RegistrationError> {
    if uris.is_empty() || uris.len() > MAX_REDIRECT_URIS {
        return Err(RegistrationError::InvalidRedirectUri(format!(
            "between 1 and {MAX_REDIRECT_URIS} redirect_uris are required"
        )));
    }
    for uri in uris {
        if uri.chars().count() > MAX_REDIRECT_URI_CHARS || !redirect::is_registrable(uri) {
            return Err(RegistrationError::InvalidRedirectUri(format!(
                "{uri:?} must be https, loopback http, or a private app scheme"
            )));
        }
    }
    Ok(())
}

fn sanitize_client_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_CLIENT_NAME_CHARS)
        .collect()
}

fn fingerprint(client_id: &str) -> String {
    B64.encode(&Sha256::digest(client_id.as_bytes())[..16])
}

fn require_same_browser(expected: &str, presented: Option<&str>) -> Result<(), FlowError> {
    let same =
        presented.is_some_and(|nonce| bool::from(nonce.as_bytes().ct_eq(expected.as_bytes())));
    if same {
        Ok(())
    } else {
        Err(FlowError::Rejected(
            "This link was not started in this browser.".into(),
        ))
    }
}

fn required<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str, TokenError> {
    value.ok_or_else(|| TokenError::InvalidRequest(format!("{name} is missing")))
}

fn invalid_grant(description: &str) -> TokenError {
    TokenError::InvalidGrant(description.to_string())
}

fn refresh_error(error: IdentityError) -> TokenError {
    match error {
        IdentityError::Rejected => invalid_grant("the grant is no longer valid: sign in again"),
        IdentityError::Unavailable(reason) => {
            tracing::warn!(%reason, "token refresh could not reach Meta");
            TokenError::TemporarilyUnavailable
        }
    }
}

fn flow_error(error: IdentityError) -> FlowError {
    match error {
        IdentityError::Rejected => FlowError::Rejected("Facebook rejected the sign-in.".into()),
        IdentityError::Unavailable(reason) => {
            tracing::warn!(%reason, "authorization flow could not reach Meta");
            FlowError::Unavailable
        }
    }
}

fn expired_flow() -> FlowError {
    FlowError::Rejected("This sign-in link is invalid or has expired.".into())
}

fn internal_flow() -> FlowError {
    FlowError::Rejected("The sign-in could not be completed.".into())
}

fn internal_page() -> AuthorizeError {
    AuthorizeError::Page("The sign-in could not be started.".into())
}
