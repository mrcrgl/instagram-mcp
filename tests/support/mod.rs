//! Harness: the real application against a fake Meta Graph API, driven over
//! HTTP the way an MCP client and a browser would.

// Each test binary compiles this module and uses a different part of it.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use instagram_mcp::app;
use instagram_mcp::config::Config;
use reqwest::redirect::Policy;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use url::Url;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const CLIENT_REDIRECT: &str = "https://client.example/callback";
pub const PUBLIC_HOST: &str = "ig.example.test";
pub const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
pub const VERSION: &str = "v26.0";

/// One person as Meta sees them, with one Instagram account.
#[derive(Clone)]
pub struct FakeUser {
    pub user_id: &'static str,
    pub ig_id: &'static str,
    pub username: &'static str,
    pub code: &'static str,
}

pub const ALICE: FakeUser = FakeUser {
    user_id: "1001",
    ig_id: "17841000000000001",
    username: "alice.brand",
    code: "code-alice",
};
pub const BOB: FakeUser = FakeUser {
    user_id: "2002",
    ig_id: "17841000000000002",
    username: "bob.shop",
    code: "code-bob",
};

impl FakeUser {
    pub fn short_token(&self) -> String {
        format!("EAA_short_{}", self.user_id)
    }
    pub fn long_token(&self) -> String {
        format!("EAA_long_{}", self.user_id)
    }
    pub fn bearer(&self) -> String {
        format!("Bearer {}", self.long_token())
    }
}

pub fn v(path: &str) -> String {
    format!("/{VERSION}{path}")
}

pub struct Harness {
    pub base_url: String,
    pub meta: MockServer,
    pub http: reqwest::Client,
    _shutdown: CancellationToken,
}

impl Harness {
    pub async fn start() -> Self {
        let meta = MockServer::start().await;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let base_url = format!("http://localhost:{}", addr.port());

        let settings = HashMap::from([
            ("PUBLIC_BASE_URL", base_url.clone()),
            ("META_APP_ID", "424242".to_string()),
            ("META_APP_SECRET", "app-secret".to_string()),
            ("META_GRAPH_URL", meta.uri()),
            ("META_WEB_URL", meta.uri()),
            ("META_GRAPH_VERSION", VERSION.to_string()),
            ("TOKEN_KEYS", B64.encode([7u8; 32])),
        ]);
        let mut cfg = Config::from_lookup(&|k| settings.get(k).cloned()).unwrap();
        cfg.allowed_hosts.push(PUBLIC_HOST.to_string());
        let services = app::build_services(&cfg).unwrap();

        let shutdown = CancellationToken::new();
        let router = app::router(&cfg, services, shutdown.child_token());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        Self {
            base_url,
            meta,
            http: reqwest::Client::builder()
                .redirect(Policy::none())
                .build()
                .unwrap(),
            _shutdown: shutdown,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Facebook Login and identity for a user with one linked Instagram account.
    pub async fn given_user(&self, user: &FakeUser) {
        self.given_login(user).await;
        self.given_pages(
            user,
            json!([{
                "name": format!("{} page", user.username),
                "instagram_business_account": {"id": user.ig_id, "username": user.username},
            }]),
        )
        .await;
    }

    pub async fn given_login(&self, user: &FakeUser) {
        Mock::given(method("GET"))
            .and(path(v("/oauth/access_token")))
            .and(query_param("code", user.code))
            .and(query_param("client_secret", "app-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": user.short_token(), "token_type": "bearer", "expires_in": 3600
            })))
            .mount(&self.meta)
            .await;
        for from in [user.short_token(), user.long_token()] {
            Mock::given(method("GET"))
                .and(path(v("/oauth/access_token")))
                .and(query_param("grant_type", "fb_exchange_token"))
                .and(query_param("fb_exchange_token", from.as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": user.long_token(), "token_type": "bearer",
                    "expires_in": 5_184_000
                })))
                .mount(&self.meta)
                .await;
        }
        self.given_get(user, "/me", json!({"id": user.user_id}))
            .await;
    }

    pub async fn given_pages(&self, user: &FakeUser, pages: Value) {
        self.given_get(user, "/me/accounts", json!({"data": pages}))
            .await;
    }

    /// A Graph API object that answers only to this user's token.
    pub async fn given_get(&self, user: &FakeUser, object_path: &str, body: Value) {
        Mock::given(method("GET"))
            .and(path(v(object_path)))
            .and(header("authorization", user.bearer().as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.meta)
            .await;
    }

    pub async fn register(&self) -> String {
        let response: Value = self
            .http
            .post(self.url("/register"))
            .json(&json!({"client_name": "Test client", "redirect_uris": [CLIENT_REDIRECT]}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        response["client_id"].as_str().unwrap().to_string()
    }

    pub fn authorize_url(&self, client_id: &str) -> String {
        let mut url = Url::parse(&self.url("/authorize")).unwrap();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", CLIENT_REDIRECT)
            .append_pair("code_challenge", &challenge(VERIFIER))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", "client-state")
            .append_pair("resource", &self.url("/mcp"));
        url.into()
    }

    /// Runs the browser side up to Facebook's dialog.
    pub async fn browser_to_facebook(&self, client_id: &str) -> BrowserAtFacebook {
        let page = self
            .http
            .get(self.authorize_url(client_id))
            .send()
            .await
            .unwrap();
        assert_eq!(page.status(), 200);
        let flow_cookie = response_cookie(&page, "__Host-ig_flow=").expect("flow cookie is set");
        let html = page.text().await.unwrap();

        let consent = self
            .http
            .post(self.url("/authorize/consent"))
            .header("cookie", &flow_cookie)
            .form(&[
                ("consent", hidden_consent(&html).as_str()),
                ("decision", "allow"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(consent.status(), 303, "consent should redirect to Facebook");
        let dialog = location(&consent);
        let state = query(&dialog, "state").unwrap();
        BrowserAtFacebook {
            flow_cookie,
            state,
            dialog,
        }
    }

    /// Facebook redirecting the signed-in user back.
    pub async fn facebook_redirect(
        &self,
        browser: &BrowserAtFacebook,
        code: &str,
    ) -> reqwest::Response {
        let mut url = Url::parse(&self.url("/oauth/meta/callback")).unwrap();
        url.query_pairs_mut()
            .append_pair("code", code)
            .append_pair("state", &browser.state);
        self.http
            .get(url)
            .header("cookie", &browser.flow_cookie)
            .send()
            .await
            .unwrap()
    }

    pub async fn exchange_code(
        &self,
        client_id: &str,
        code: &str,
        verifier: &str,
    ) -> reqwest::Response {
        self.http
            .post(self.url("/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("code", code),
                ("redirect_uri", CLIENT_REDIRECT),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .unwrap()
    }

    pub async fn refresh(&self, client_id: &str, refresh_token: &str) -> reqwest::Response {
        self.http
            .post(self.url("/token"))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .unwrap()
    }

    /// Full happy path.
    pub async fn sign_in(&self, user: &FakeUser) -> Tokens {
        let client_id = self.register().await;
        let browser = self.browser_to_facebook(&client_id).await;
        let done = self.facebook_redirect(&browser, user.code).await;
        assert_eq!(done.status(), 303, "callback should return to the client");
        let code = query(&location(&done), "code").expect("redirect carries a code");
        let tokens: Value = self
            .exchange_code(&client_id, &code, VERIFIER)
            .await
            .json()
            .await
            .unwrap();
        Tokens {
            client_id,
            access: tokens["access_token"].as_str().unwrap().to_string(),
            refresh: tokens["refresh_token"].as_str().unwrap().to_string(),
        }
    }

    /// Calls a tool over the real MCP endpoint.
    pub async fn call_tool(&self, access_token: &str, tool: &str, arguments: Value) -> ToolOutcome {
        let body = self
            .rpc(
                access_token,
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": tool, "arguments": arguments},
                }),
            )
            .await;
        let result = &body["result"];
        assert!(result.is_object(), "expected a tool result, got {body}");
        ToolOutcome {
            is_error: result["isError"].as_bool().unwrap_or(false),
            text: result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        }
    }

    pub async fn rpc(&self, access_token: &str, request: Value) -> Value {
        let response = self
            .http
            .post(self.url("/mcp"))
            .bearer_auth(access_token)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }
}

pub struct BrowserAtFacebook {
    pub flow_cookie: String,
    pub state: String,
    pub dialog: String,
}

pub struct Tokens {
    pub client_id: String,
    pub access: String,
    pub refresh: String,
}

#[derive(Debug)]
pub struct ToolOutcome {
    pub is_error: bool,
    pub text: String,
}

impl ToolOutcome {
    pub fn json(&self) -> Value {
        assert!(!self.is_error, "tool failed: {}", self.text);
        serde_json::from_str(&self.text).unwrap()
    }
}

pub fn media_json(id: &str, owner: &str, caption: &str, likes: u64) -> Value {
    json!({
        "id": id, "caption": caption, "media_type": "IMAGE", "media_product_type": "FEED",
        "timestamp": "2026-11-02T09:00:00+0000", "permalink": format!("https://instagram.test/p/{id}"),
        "like_count": likes, "comments_count": 1, "owner": {"id": owner}
    })
}

pub fn challenge(verifier: &str) -> String {
    B64.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn location(response: &reqwest::Response) -> String {
    response.headers()["location"].to_str().unwrap().to_string()
}

pub fn query(url: &str, name: &str) -> Option<String> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The cookie named `prefix`, in `name=value` form, from a response.
pub fn response_cookie(response: &reqwest::Response, prefix: &str) -> Option<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(prefix))
        .map(|value| value.split(';').next().unwrap().to_string())
}

pub fn hidden_consent(html: &str) -> String {
    let marker = "name=\"consent\" value=\"";
    let start = html.find(marker).expect("consent token in page") + marker.len();
    let end = html[start..].find('"').unwrap();
    html[start..start + end].to_string()
}
