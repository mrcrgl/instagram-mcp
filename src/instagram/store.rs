//! The storage port: what the use cases need from Instagram. Every call takes
//! the caller's `Credential`; the adapter has no credential of its own, so it
//! cannot read anything the signed-in user could not.

use std::future::Future;
use std::pin::Pin;

use crate::domain::insights::{AccountInsightQuery, Metric, MetricName};
use crate::domain::media::{
    CommentPage, Cursor, DiscoveredAccount, Hashtag, HashtagName, HashtagSort, Media, MediaPage,
    ObjectId, Profile, Username,
};
use crate::domain::tenant::Credential;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error(
        "The Meta session is no longer valid (expired, password changed, or access removed). Reconnect this server in your MCP client."
    )]
    SessionExpired,
    #[error("Instagram refused access: {0}")]
    PermissionDenied(String),
    #[error("Instagram rate limit reached: {0}. Try again later.")]
    RateLimited(String),
    #[error("Instagram rejected the request: {0}")]
    Rejected(String),
    #[error("Instagram could not be reached: {0}")]
    Unavailable(String),
}

impl StoreError {
    /// A stable label for logs, which must not carry Meta's message text.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::SessionExpired => "session_expired",
            Self::PermissionDenied(_) => "permission_denied",
            Self::RateLimited(_) => "rate_limited",
            Self::Rejected(_) => "rejected",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub limit: u32,
    pub after: Option<Cursor>,
    /// Unix seconds, for endpoints that support time bounds.
    pub since: Option<i64>,
    pub until: Option<i64>,
}

impl PageRequest {
    pub fn first(limit: u32) -> Self {
        Self {
            limit,
            after: None,
            since: None,
            until: None,
        }
    }
}

pub trait InstagramStore: Send + Sync {
    fn profile<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
    ) -> BoxFut<'a, Profile>;

    fn media_page<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        page: &'a PageRequest,
    ) -> BoxFut<'a, MediaPage>;

    fn media<'a>(&'a self, credential: &'a Credential, id: &'a ObjectId) -> BoxFut<'a, Media>;

    /// Fails as a whole when one metric is not supported for this media.
    fn media_insights<'a>(
        &'a self,
        credential: &'a Credential,
        id: &'a ObjectId,
        metrics: &'a [MetricName],
    ) -> BoxFut<'a, Vec<Metric>>;

    fn account_insights<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        query: &'a AccountInsightQuery,
    ) -> BoxFut<'a, Vec<Metric>>;

    fn stories<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
    ) -> BoxFut<'a, Vec<Media>>;

    fn tagged_media<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        page: &'a PageRequest,
    ) -> BoxFut<'a, MediaPage>;

    fn comments<'a>(
        &'a self,
        credential: &'a Credential,
        media_id: &'a ObjectId,
        page: &'a PageRequest,
    ) -> BoxFut<'a, CommentPage>;

    /// Counts against the account's 30 unique hashtags per 7 days.
    fn hashtag<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        name: &'a HashtagName,
    ) -> BoxFut<'a, Option<Hashtag>>;

    fn hashtag_media<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        hashtag_id: &'a str,
        sort: HashtagSort,
        page: &'a PageRequest,
    ) -> BoxFut<'a, MediaPage>;

    fn recent_hashtags<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
    ) -> BoxFut<'a, Vec<Hashtag>>;

    /// Another professional account's public profile and media.
    fn discover<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        username: &'a Username,
        page: &'a PageRequest,
    ) -> BoxFut<'a, DiscoveredAccount>;
}
