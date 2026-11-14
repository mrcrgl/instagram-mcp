//! Multi-tenancy and input controls over the real MCP endpoint. The fake Meta
//! answers an object only for its owner's token, so a request made with the
//! wrong credential shows up as a failure rather than as someone else's data.

mod support;

use serde_json::json;
use support::*;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, ResponseTemplate};

async fn alice_and_bob(h: &Harness) -> (Tokens, Tokens) {
    h.given_user(&ALICE).await;
    h.given_user(&BOB).await;
    (h.sign_in(&ALICE).await, h.sign_in(&BOB).await)
}

#[tokio::test]
async fn each_token_reads_its_own_media() {
    let h = Harness::start().await;
    let (alice, bob) = alice_and_bob(&h).await;
    for user in [&ALICE, &BOB] {
        h.given_get(
            user,
            &format!("/{}/media", user.ig_id),
            json!({"data": [media_json("1", user.ig_id, &format!("{} secret", user.username), 5)]}),
        )
        .await;
    }

    let seen_by_alice = h.call_tool(&alice.access, "media_list", json!({})).await;
    let seen_by_bob = h.call_tool(&bob.access, "media_list", json!({})).await;
    assert!(seen_by_alice.text.contains("alice.brand secret"));
    assert!(!seen_by_alice.text.contains("bob.shop"));
    assert!(seen_by_bob.text.contains("bob.shop secret"));
}

#[tokio::test]
async fn another_tenants_account_cannot_be_selected() {
    let h = Harness::start().await;
    let (alice, _) = alice_and_bob(&h).await;
    let outcome = h
        .call_tool(
            &alice.access,
            "media_list",
            json!({"account": BOB.username}),
        )
        .await;
    assert!(outcome.is_error);
    assert!(outcome.text.contains("not one of your connected accounts"));
}

#[tokio::test]
async fn foreign_media_ids_are_not_found() {
    let h = Harness::start().await;
    let (alice, _) = alice_and_bob(&h).await;
    // Even if Meta returned it, media owned by someone else is refused.
    h.given_get(
        &ALICE,
        "/555",
        media_json("555", BOB.ig_id, "bob's draft", 1),
    )
    .await;
    for tool in ["media_get", "media_insights", "comments_list"] {
        let outcome = h
            .call_tool(&alice.access, tool, json!({"media_id": "555"}))
            .await;
        assert!(outcome.is_error, "{tool}");
        assert!(!outcome.text.contains("draft"), "{tool}");
    }
}

#[tokio::test]
async fn own_media_insights_fall_back_per_metric() {
    let h = Harness::start().await;
    let (alice, _) = alice_and_bob(&h).await;
    h.given_get(&ALICE, "/77", media_json("77", ALICE.ig_id, "post", 3))
        .await;
    // Meta rejects the batch because one metric is not offered; single
    // metrics succeed except that one.
    Mock::given(method("GET"))
        .and(path_regex(r"^/v26\.0/77/insights$"))
        .and(wiremock::matchers::query_param("metric", "reach"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"name": "reach", "period": "lifetime", "values": [{"value": 99}]}]
        })))
        .mount(&h.meta)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v26\.0/77/insights$"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"code": 100, "message": "metric not supported"}
        })))
        .mount(&h.meta)
        .await;

    let outcome = h
        .call_tool(
            &alice.access,
            "media_insights",
            json!({"media_id": "77", "metrics": ["reach", "plays"]}),
        )
        .await
        .json();
    assert_eq!(outcome["metrics"][0]["name"], "reach");
    assert_eq!(outcome["metrics"][0]["total"], 99.0);
    assert_eq!(outcome["unavailable"][0]["metric"], "plays");
}

#[tokio::test]
async fn usernames_and_cursors_cannot_inject_fields() {
    let h = Harness::start().await;
    let (alice, _) = alice_and_bob(&h).await;
    let bad_username = h
        .call_tool(
            &alice.access,
            "business_profile",
            json!({"username": "rival){access_token}"}),
        )
        .await;
    assert!(bad_username.is_error);
    let bad_cursor = h
        .call_tool(
            &alice.access,
            "business_profile",
            json!({"username": "rival", "after": "x){id}"}),
        )
        .await;
    assert!(bad_cursor.is_error);
    let bad_id = h
        .call_tool(&alice.access, "media_get", json!({"media_id": "me"}))
        .await;
    assert!(bad_id.is_error);
    // None of these reached Meta beyond the sign-in.
    let requests = h.meta.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| !r.url.as_str().contains("business_discovery"))
    );
}

#[tokio::test]
async fn an_expired_meta_session_says_to_reconnect() {
    let h = Harness::start().await;
    let (alice, _) = alice_and_bob(&h).await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v26\.0/\d+$"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"code": 190, "message": "Error validating access token"}
        })))
        .mount(&h.meta)
        .await;
    let outcome = h
        .call_tool(&alice.access, "account_profile", json!({}))
        .await;
    assert!(outcome.is_error);
    assert!(outcome.text.contains("Reconnect"));
}

#[tokio::test]
async fn every_graph_call_carries_an_appsecret_proof_and_no_token_in_the_url() {
    let h = Harness::start().await;
    let (alice, _) = alice_and_bob(&h).await;
    h.given_get(
        &ALICE,
        &format!("/{}", ALICE.ig_id),
        json!({"id": ALICE.ig_id}),
    )
    .await;
    h.call_tool(&alice.access, "account_profile", json!({}))
        .await;
    let requests = h.meta.received_requests().await.unwrap();
    let api_calls: Vec<_> = requests
        .iter()
        .filter(|r| !r.url.path().contains("oauth"))
        .collect();
    assert!(!api_calls.is_empty());
    for request in api_calls {
        assert!(request.url.as_str().contains("appsecret_proof="));
        assert!(!request.url.as_str().contains("EAA_"));
    }
}
