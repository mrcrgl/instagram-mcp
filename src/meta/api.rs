//! Thin Graph API client shared by the adapter's components.
//!
//! The user token goes in the `Authorization` header, never the URL, so it
//! cannot end up in proxy or client logs. Every call carries an
//! `appsecret_proof`, which makes a leaked token useless to any other app.

use hmac::{Hmac, Mac};
use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::Sha256;

use crate::domain::tenant::Credential;
use crate::instagram::store::StoreError;

const USER_AGENT: &str = "instagram-mcp";
const MAX_MESSAGE_CHARS: usize = 300;

/// Holds the app secret, hence no `Debug`.
#[derive(Clone)]
pub struct GraphApi {
    http: reqwest::Client,
    /// `https://graph.facebook.com/v26.0`, without a trailing slash.
    versioned_url: String,
    app_secret: String,
}

impl GraphApi {
    pub fn new(graph_url: &str, version: &str, app_secret: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("static HTTP client configuration"),
            versioned_url: format!("{}/{version}", graph_url.trim_end_matches('/')),
            app_secret,
        }
    }

    pub fn url(&self, segments: &[&str]) -> Result<url::Url, StoreError> {
        let mut url = url::Url::parse(&self.versioned_url)
            .map_err(|e| StoreError::Unavailable(format!("invalid Graph API URL: {e}")))?;
        url.path_segments_mut()
            .map_err(|()| StoreError::Unavailable("invalid Graph API URL".into()))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        credential: &Credential,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<T, StoreError> {
        let mut url = self.url(segments)?;
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())))
            .append_pair("appsecret_proof", &self.proof(credential));
        let response = self
            .http
            .get(url)
            .bearer_auth(credential.expose())
            .send()
            .await
            .map_err(transport_error)?;
        decode(check(response).await?).await
    }

    /// Unauthenticated GET for the OAuth endpoints, whose query carries the
    /// app secret and codes. Hence errors drop the URL.
    pub async fn get_public<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, StoreError> {
        let mut url = self.url(segments)?;
        url.query_pairs_mut().extend_pairs(query);
        let response = self.http.get(url).send().await.map_err(transport_error)?;
        decode(check(response).await?).await
    }

    fn proof(&self, credential: &Credential) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.app_secret.as_bytes())
            .expect("HMAC accepts keys of any length");
        mac.update(credential.expose().as_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// `without_url` because a request URL can carry a code or the app secret.
fn transport_error(error: reqwest::Error) -> StoreError {
    StoreError::Unavailable(error.without_url().to_string())
}

async fn check(response: Response) -> Result<Response, StoreError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body: Value = response.json().await.unwrap_or(Value::Null);
    Err(classify(status, &body["error"]))
}

/// Maps a Graph API error object. Codes from Meta's error reference.
pub fn classify(status: StatusCode, error: &Value) -> StoreError {
    let code = error["code"].as_i64().unwrap_or(0);
    let subcode = error["error_subcode"].as_i64().unwrap_or(0);
    let message: String = error["error_user_msg"]
        .as_str()
        .or(error["message"].as_str())
        .unwrap_or("no details given")
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .collect();
    match code {
        102 | 190 => StoreError::SessionExpired,
        4 | 17 | 32 | 613 | 80001..=80014 => StoreError::RateLimited(message),
        3 | 10 | 200..=299 => StoreError::PermissionDenied(message),
        803 | 110 => StoreError::NotFound,
        100 if subcode == 33 => StoreError::NotFound,
        _ if status == StatusCode::TOO_MANY_REQUESTS => StoreError::RateLimited(message),
        _ if status == StatusCode::NOT_FOUND => StoreError::NotFound,
        _ if status.is_server_error() => StoreError::Unavailable(message),
        _ => StoreError::Rejected(message),
    }
}

async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, StoreError> {
    response
        .json()
        .await
        .map_err(|e| StoreError::Unavailable(format!("unreadable Graph API response: {e}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn errors_map_to_what_the_user_can_do() {
        let cases = [
            (
                json!({"code": 190, "message": "expired"}),
                "session_expired",
            ),
            (json!({"code": 4, "message": "slow down"}), "rate_limited"),
            (json!({"code": 80002, "message": "buc"}), "rate_limited"),
            (
                json!({"code": 10, "message": "no permission"}),
                "permission_denied",
            ),
            (
                json!({"code": 100, "error_subcode": 33, "message": "x"}),
                "not_found",
            ),
            (json!({"code": 100, "message": "bad metric"}), "rejected"),
        ];
        for (error, kind) in cases {
            assert_eq!(
                classify(StatusCode::BAD_REQUEST, &error).kind(),
                kind,
                "{error}"
            );
        }
        assert_eq!(
            classify(StatusCode::BAD_GATEWAY, &Value::Null).kind(),
            "unavailable"
        );
    }

    #[test]
    fn the_user_facing_message_is_preferred_and_bounded() {
        let error = json!({"code": 100, "message": "tech", "error_user_msg": "x".repeat(1000)});
        let StoreError::Rejected(message) = classify(StatusCode::BAD_REQUEST, &error) else {
            panic!("expected rejected");
        };
        assert_eq!(message.len(), MAX_MESSAGE_CHARS);
    }

    #[test]
    fn the_proof_is_an_hmac_of_the_token() {
        let api = GraphApi::new("https://graph.example", "v26.0", "secret".into());
        let proof = api.proof(&Credential::new("token".into()));
        assert_eq!(proof.len(), 64);
        assert_ne!(proof, api.proof(&Credential::new("other".into())));
    }
}
