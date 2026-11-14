//! The authorization server, end to end: discovery, registration, consent,
//! Facebook Login, token exchange, refresh, and the bearer guard.

mod support;

use serde_json::{Value, json};
use support::*;

#[tokio::test]
async fn an_unauthenticated_call_points_to_the_metadata() {
    let h = Harness::start().await;
    let response = h
        .http
        .post(h.url("/mcp"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let challenge = response.headers()["www-authenticate"].to_str().unwrap();
    assert!(challenge.contains("/.well-known/oauth-protected-resource/mcp"));

    let metadata: Value = h
        .http
        .get(h.url("/.well-known/oauth-protected-resource/mcp"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(metadata["resource"], h.url("/mcp"));
    let server: Value = h
        .http
        .get(h.url("/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(server["registration_endpoint"], h.url("/register"));
    assert_eq!(server["code_challenge_methods_supported"], json!(["S256"]));
}

#[tokio::test]
async fn a_full_sign_in_lists_the_linked_account() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    h.given_get(
        &ALICE,
        &format!("/{}", ALICE.ig_id),
        json!({"id": ALICE.ig_id, "username": ALICE.username, "followers_count": 1234}),
    )
    .await;
    let tokens = h.sign_in(&ALICE).await;

    let accounts = h
        .call_tool(&tokens.access, "accounts_list", json!({}))
        .await
        .json();
    assert_eq!(accounts[0]["username"], ALICE.username);
    assert_eq!(accounts[0]["followers"], 1234);
}

#[tokio::test]
async fn the_dialog_names_this_app_and_the_callback() {
    let h = Harness::start().await;
    let client_id = h.register().await;
    let browser = h.browser_to_facebook(&client_id).await;
    assert!(browser.dialog.contains(&format!("/{VERSION}/dialog/oauth")));
    assert_eq!(query(&browser.dialog, "client_id").unwrap(), "424242");
    assert_eq!(
        query(&browser.dialog, "redirect_uri").unwrap(),
        h.url("/oauth/meta/callback")
    );
    assert!(
        query(&browser.dialog, "scope")
            .unwrap()
            .contains("instagram_manage_insights")
    );
}

#[tokio::test]
async fn sign_in_without_an_instagram_account_is_explained() {
    let h = Harness::start().await;
    h.given_login(&ALICE).await;
    h.given_pages(&ALICE, json!([{"name": "Page without Instagram"}]))
        .await;
    let client_id = h.register().await;
    let browser = h.browser_to_facebook(&client_id).await;
    let page = h.facebook_redirect(&browser, ALICE.code).await;
    assert_eq!(page.status(), 400);
    assert!(page.text().await.unwrap().contains("professional account"));
}

#[tokio::test]
async fn the_callback_must_come_back_to_the_same_browser() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let client_id = h.register().await;
    let mut browser = h.browser_to_facebook(&client_id).await;
    browser.flow_cookie = "__Host-ig_flow=someone-else".into();
    let page = h.facebook_redirect(&browser, ALICE.code).await;
    assert_eq!(page.status(), 400);
}

#[tokio::test]
async fn a_denied_dialog_returns_access_denied_to_the_client() {
    let h = Harness::start().await;
    let client_id = h.register().await;
    let browser = h.browser_to_facebook(&client_id).await;
    let mut url = url::Url::parse(&h.url("/oauth/meta/callback")).unwrap();
    url.query_pairs_mut()
        .append_pair("error", "access_denied")
        .append_pair("state", &browser.state);
    let response = h
        .http
        .get(url)
        .header("cookie", &browser.flow_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303);
    let target = location(&response);
    assert!(target.starts_with(CLIENT_REDIRECT));
    assert_eq!(query(&target, "error").unwrap(), "access_denied");
    assert_eq!(query(&target, "state").unwrap(), "client-state");
}

#[tokio::test]
async fn the_code_needs_the_pkce_verifier_and_the_same_client() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let client_id = h.register().await;
    let browser = h.browser_to_facebook(&client_id).await;
    let done = h.facebook_redirect(&browser, ALICE.code).await;
    let code = query(&location(&done), "code").unwrap();

    let wrong_verifier = h.exchange_code(&client_id, &code, &"x".repeat(43)).await;
    assert_eq!(wrong_verifier.status(), 400);
    let other_client = h.register_other().await;
    let wrong_client = h.exchange_code(&other_client, &code, VERIFIER).await;
    assert_eq!(wrong_client.status(), 400);
    let ok = h.exchange_code(&client_id, &code, VERIFIER).await;
    assert_eq!(ok.status(), 200);
}

#[tokio::test]
async fn refresh_issues_new_tokens_for_the_same_client_only() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let tokens = h.sign_in(&ALICE).await;

    let other_client = h.register_other().await;
    assert_eq!(
        h.refresh(&other_client, &tokens.refresh).await.status(),
        400
    );

    let refreshed = h.refresh(&tokens.client_id, &tokens.refresh).await;
    assert_eq!(refreshed.status(), 200);
    let body: Value = refreshed.json().await.unwrap();
    assert!(body["access_token"].as_str().unwrap() != tokens.access);
    assert!(body["expires_in"].as_u64().unwrap() <= 3600);
}

#[tokio::test]
async fn tokens_are_not_interchangeable() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let tokens = h.sign_in(&ALICE).await;
    // A refresh token is not an access token.
    let response = h
        .http
        .post(h.url("/mcp"))
        .bearer_auth(&tokens.refresh)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn the_meta_token_never_reaches_the_client_in_clear() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let tokens = h.sign_in(&ALICE).await;
    for value in [&tokens.access, &tokens.refresh] {
        assert!(!value.contains("EAA_"));
    }
}

#[tokio::test]
async fn all_tools_are_listed_and_read_only() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let tokens = h.sign_in(&ALICE).await;
    let body = h
        .rpc(
            &tokens.access,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .await;
    let tools = body["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 16);
    for tool in tools {
        assert_eq!(
            tool["annotations"]["readOnlyHint"], true,
            "{}",
            tool["name"]
        );
    }
}

trait RegisterOther {
    async fn register_other(&self) -> String;
}

impl RegisterOther for Harness {
    async fn register_other(&self) -> String {
        let response: Value = self
            .http
            .post(self.url("/register"))
            .json(&json!({"client_name": "Other", "redirect_uris": ["https://other.example/cb"]}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        response["client_id"].as_str().unwrap().to_string()
    }
}
