//! Graph API wire formats. Only `mapping` turns these into domain types.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Paged<T> {
    #[serde(default = "Vec::new")]
    pub data: Vec<T>,
    #[serde(default)]
    pub paging: Option<Paging>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Paging {
    #[serde(default)]
    pub cursors: Option<Cursors>,
    /// Present only when there is a next page.
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Cursors {
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IdRow {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct MediaRow {
    pub id: String,
    pub caption: Option<String>,
    pub media_type: Option<String>,
    pub media_product_type: Option<String>,
    pub timestamp: Option<String>,
    pub permalink: Option<String>,
    pub media_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub like_count: Option<u64>,
    pub comments_count: Option<u64>,
    pub view_count: Option<u64>,
    pub username: Option<String>,
    pub owner: Option<IdRow>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileRow {
    pub id: String,
    pub username: Option<String>,
    pub name: Option<String>,
    pub biography: Option<String>,
    pub website: Option<String>,
    pub followers_count: Option<u64>,
    pub follows_count: Option<u64>,
    pub media_count: Option<u64>,
    pub profile_picture_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryRow {
    #[serde(flatten)]
    pub profile: ProfileRow,
    pub media: Option<Paged<MediaRow>>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryEnvelope {
    pub business_discovery: DiscoveryRow,
}

#[derive(Debug, Deserialize)]
pub struct CommentRow {
    pub id: String,
    pub text: Option<String>,
    pub username: Option<String>,
    pub timestamp: Option<String>,
    pub like_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct HashtagRow {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InsightRow {
    pub name: String,
    pub period: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub values: Vec<InsightValue>,
    pub total_value: Option<TotalValue>,
}

#[derive(Debug, Deserialize)]
pub struct InsightValue {
    pub value: Value,
    pub end_time: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TotalValue {
    pub value: Option<Value>,
    #[serde(default)]
    pub breakdowns: Vec<BreakdownRow>,
}

#[derive(Debug, Deserialize)]
pub struct BreakdownRow {
    #[serde(default)]
    pub dimension_keys: Vec<String>,
    #[serde(default)]
    pub results: Vec<BreakdownResult>,
}

#[derive(Debug, Deserialize)]
pub struct BreakdownResult {
    #[serde(default)]
    pub dimension_values: Vec<String>,
    pub value: Value,
}

/// `/me/accounts`: a Page and the Instagram account linked to it.
#[derive(Debug, Deserialize)]
pub struct PageRow {
    pub name: Option<String>,
    pub instagram_business_account: Option<LinkedAccountRow>,
}

#[derive(Debug, Deserialize)]
pub struct LinkedAccountRow {
    pub id: String,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserRow {
    pub id: String,
}

/// Token endpoint response. Holds a token, hence no `Debug`.
#[derive(Deserialize)]
pub struct TokenRow {
    pub access_token: String,
    pub expires_in: Option<u64>,
}
