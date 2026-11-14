//! Hashtag search and Business Discovery against a fake Graph API.

mod support;

use serde_json::json;
use support::*;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

async fn hashtag(h: &Harness, name: &str, id: &str) {
    Mock::given(method("GET"))
        .and(path(v("/ig_hashtag_search")))
        .and(query_param("user_id", ALICE.ig_id))
        .and(query_param("q", name))
        .and(header("authorization", ALICE.bearer().as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": id}]})))
        .expect(1)
        .mount(&h.meta)
        .await;
}

#[tokio::test]
async fn hashtag_media_is_looked_up_once_and_filtered_by_caption() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let alice = h.sign_in(&ALICE).await;
    hashtag(&h, "veganfood", "178400").await;
    Mock::given(method("GET"))
        .and(path(v("/178400/top_media")))
        .and(query_param("user_id", ALICE.ig_id))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
            {"id": "1", "caption": "Vegan pasta #veganfood", "media_type": "IMAGE", "like_count": 10, "comments_count": 2},
            {"id": "2", "caption": "Tofu bowl #veganfood", "media_type": "VIDEO", "like_count": 50, "comments_count": 5},
        ]})))
        .mount(&h.meta)
        .await;

    let all = h
        .call_tool(
            &alice.access,
            "hashtag_media",
            json!({"hashtag": "#VeganFood"}),
        )
        .await
        .json();
    assert_eq!(all["hashtag"]["id"], "178400");
    assert_eq!(all["media"].as_array().unwrap().len(), 2);

    // Second call reuses the cached id: `expect(1)` on the search fails otherwise.
    let pasta = h
        .call_tool(
            &alice.access,
            "hashtag_media",
            json!({"hashtag": "veganfood", "caption_contains": "PASTA"}),
        )
        .await
        .json();
    assert_eq!(pasta["media"][0]["id"], "1");
    assert_eq!(pasta["media"].as_array().unwrap().len(), 1);
    assert_eq!(pasta["scanned"], 2);
}

#[tokio::test]
async fn a_hashtag_report_ranks_media_and_related_tags() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let alice = h.sign_in(&ALICE).await;
    hashtag(&h, "coffee", "99").await;
    Mock::given(method("GET"))
        .and(path(v("/99/recent_media")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
            {"id": "1", "caption": "#coffee #latteart", "media_type": "IMAGE", "like_count": 10, "comments_count": 0, "timestamp": "2026-11-13T07:00:00+0000"},
            {"id": "2", "caption": "#coffee #latteart #barista", "media_type": "CAROUSEL_ALBUM", "like_count": 40, "comments_count": 4, "timestamp": "2026-11-13T08:00:00+0000"},
        ]})))
        .mount(&h.meta)
        .await;

    let report = h
        .call_tool(
            &alice.access,
            "hashtag_report",
            json!({"hashtag": "coffee", "sort": "recent", "sample_size": 10}),
        )
        .await
        .json();
    assert_eq!(report["stats"]["sample_size"], 2);
    assert_eq!(report["stats"]["top_hashtags"][0]["tag"], "latteart");
    assert_eq!(report["top_media"][0]["id"], "2");
}

#[tokio::test]
async fn an_unknown_hashtag_is_reported_not_crashed() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let alice = h.sign_in(&ALICE).await;
    h.given_get(&ALICE, "/ig_hashtag_search", json!({"data": []}))
        .await;
    let outcome = h
        .call_tool(
            &alice.access,
            "hashtag_media",
            json!({"hashtag": "nosuchtag"}),
        )
        .await;
    assert!(outcome.is_error);
    assert!(outcome.text.contains("does not exist"));
}

#[tokio::test]
async fn business_discovery_reads_a_competitor_and_benchmarks_it() {
    let h = Harness::start().await;
    h.given_user(&ALICE).await;
    let alice = h.sign_in(&ALICE).await;
    Mock::given(method("GET"))
        .and(path(v(&format!("/{}", ALICE.ig_id))))
        .and(header("authorization", ALICE.bearer().as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ALICE.ig_id,
            "business_discovery": {
                "id": "555", "username": "rival", "followers_count": 1000, "media_count": 2,
                "media": {"data": [
                    {"id": "a", "caption": "#launch", "media_type": "VIDEO", "media_product_type": "REELS", "like_count": 80, "comments_count": 20, "view_count": 5000},
                    {"id": "b", "caption": "hello", "media_type": "IMAGE", "like_count": 20, "comments_count": 0},
                ]}
            }
        })))
        .mount(&h.meta)
        .await;

    let profile = h
        .call_tool(
            &alice.access,
            "business_profile",
            json!({"username": "@Rival"}),
        )
        .await
        .json();
    assert_eq!(profile["profile"]["followers"], 1000);
    assert_eq!(profile["media"][0]["kind"], "reel");
    assert_eq!(profile["media"][0]["views"], 5000);

    let report = h
        .call_tool(
            &alice.access,
            "business_report",
            json!({"username": "rival"}),
        )
        .await
        .json();
    // (100 + 20) / 2 = 60 interactions per post on 1000 followers.
    assert_eq!(report["stats"]["engagement_rate_percent"], 6.0);
    let requests = h.meta.received_requests().await.unwrap();
    assert!(requests.iter().any(|r| {
        r.url
            .query_pairs()
            .any(|(k, v)| k == "fields" && v.starts_with("business_discovery.username(rival){"))
    }));
}
