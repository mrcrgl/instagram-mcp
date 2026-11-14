//! `InstagramStore` over the Graph API (Instagram API with Facebook Login).

use std::time::Duration;

use moka::future::Cache;

use crate::domain::insights::{AccountInsightQuery, Metric, MetricName};
use crate::domain::media::{
    CommentPage, DiscoveredAccount, Hashtag, HashtagName, HashtagSort, Media, MediaPage, ObjectId,
    Profile, Username,
};
use crate::domain::tenant::Credential;
use crate::instagram::store::{BoxFut, InstagramStore, PageRequest};
use crate::meta::api::GraphApi;
use crate::meta::mapping;
use crate::meta::models::{
    CommentRow, DiscoveryEnvelope, HashtagRow, IdRow, InsightRow, MediaRow, Paged, ProfileRow,
};

const PROFILE_FIELDS: &str = "id,username,name,biography,website,followers_count,follows_count,media_count,profile_picture_url";
const OWN_MEDIA_FIELDS: &str = "id,caption,media_type,media_product_type,timestamp,permalink,media_url,thumbnail_url,like_count,comments_count,username,owner";
const STORY_FIELDS: &str =
    "id,caption,media_type,media_product_type,timestamp,permalink,media_url,thumbnail_url,username";
const TAGGED_FIELDS: &str =
    "id,caption,media_type,timestamp,permalink,media_url,like_count,comments_count,username";
/// Hashtag media expose no owner, username, or product type.
const HASHTAG_MEDIA_FIELDS: &str =
    "id,caption,media_type,timestamp,permalink,media_url,like_count,comments_count";
const DISCOVERED_MEDIA_FIELDS: &str = "id,caption,media_type,media_product_type,timestamp,permalink,media_url,thumbnail_url,like_count,comments_count,view_count";
const COMMENT_FIELDS: &str = "id,text,username,timestamp,like_count";

/// Hashtag ids never change, and looking one up spends one of the account's
/// 30 weekly hashtag searches. Cached per account, because Meta counts them
/// per account.
const HASHTAG_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub struct MetaInstagramStore {
    api: GraphApi,
    hashtags: Cache<(String, String), Hashtag>,
}

impl MetaInstagramStore {
    pub fn new(api: GraphApi) -> Self {
        Self {
            api,
            hashtags: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(HASHTAG_CACHE_TTL)
                .build(),
        }
    }
}

fn page_query(fields: &str, page: &PageRequest) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("fields", fields.to_string()),
        ("limit", page.limit.to_string()),
    ];
    if let Some(after) = &page.after {
        query.push(("after", after.as_str().to_string()));
    }
    if let Some(since) = page.since {
        query.push(("since", since.to_string()));
    }
    if let Some(until) = page.until {
        query.push(("until", until.to_string()));
    }
    query
}

fn join(metrics: &[MetricName]) -> String {
    metrics
        .iter()
        .map(MetricName::as_str)
        .collect::<Vec<_>>()
        .join(",")
}

/// Business Discovery takes paging inside the field expression. `Username`
/// and `Cursor` only admit characters that cannot close it.
fn discovery_fields(username: &Username, page: &PageRequest) -> String {
    let after = page
        .after
        .as_ref()
        .map(|cursor| format!(".after({})", cursor.as_str()))
        .unwrap_or_default();
    format!(
        "business_discovery.username({}){{{PROFILE_FIELDS},media.limit({}){after}{{{DISCOVERED_MEDIA_FIELDS}}}}}",
        username.as_str(),
        page.limit,
    )
}

impl InstagramStore for MetaInstagramStore {
    fn profile<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
    ) -> BoxFut<'a, Profile> {
        Box::pin(async move {
            let row: ProfileRow = self
                .api
                .get(
                    credential,
                    &[account_id],
                    &[("fields", PROFILE_FIELDS.into())],
                )
                .await?;
            Ok(mapping::profile(row))
        })
    }

    fn media_page<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        page: &'a PageRequest,
    ) -> BoxFut<'a, MediaPage> {
        Box::pin(async move {
            let rows: Paged<MediaRow> = self
                .api
                .get(
                    credential,
                    &[account_id, "media"],
                    &page_query(OWN_MEDIA_FIELDS, page),
                )
                .await?;
            Ok(mapping::media_page(rows))
        })
    }

    fn media<'a>(&'a self, credential: &'a Credential, id: &'a ObjectId) -> BoxFut<'a, Media> {
        Box::pin(async move {
            let row: MediaRow = self
                .api
                .get(
                    credential,
                    &[id.as_str()],
                    &[("fields", OWN_MEDIA_FIELDS.into())],
                )
                .await?;
            Ok(mapping::media(row))
        })
    }

    fn media_insights<'a>(
        &'a self,
        credential: &'a Credential,
        id: &'a ObjectId,
        metrics: &'a [MetricName],
    ) -> BoxFut<'a, Vec<Metric>> {
        Box::pin(async move {
            let rows: Paged<InsightRow> = self
                .api
                .get(
                    credential,
                    &[id.as_str(), "insights"],
                    &[("metric", join(metrics))],
                )
                .await?;
            Ok(rows.data.into_iter().map(mapping::metric).collect())
        })
    }

    fn account_insights<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        query: &'a AccountInsightQuery,
    ) -> BoxFut<'a, Vec<Metric>> {
        Box::pin(async move {
            let mut params = vec![
                ("metric", join(&query.metrics)),
                ("period", query.period.as_str().to_string()),
                ("metric_type", query.metric_type.as_str().to_string()),
            ];
            if let Some(breakdown) = query.breakdown {
                params.push(("breakdown", breakdown.as_str().to_string()));
            }
            if let Some(range) = query.range {
                params.push(("since", range.since.to_string()));
                params.push(("until", range.until.to_string()));
            }
            if let Some(timeframe) = query.timeframe {
                params.push(("timeframe", timeframe.as_str().to_string()));
            }
            let rows: Paged<InsightRow> = self
                .api
                .get(credential, &[account_id, "insights"], &params)
                .await?;
            Ok(rows.data.into_iter().map(mapping::metric).collect())
        })
    }

    fn stories<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
    ) -> BoxFut<'a, Vec<Media>> {
        Box::pin(async move {
            let rows: Paged<MediaRow> = self
                .api
                .get(
                    credential,
                    &[account_id, "stories"],
                    &[("fields", STORY_FIELDS.into()), ("limit", "100".into())],
                )
                .await?;
            Ok(rows.data.into_iter().map(mapping::media).collect())
        })
    }

    fn tagged_media<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        page: &'a PageRequest,
    ) -> BoxFut<'a, MediaPage> {
        Box::pin(async move {
            let rows: Paged<MediaRow> = self
                .api
                .get(
                    credential,
                    &[account_id, "tags"],
                    &page_query(TAGGED_FIELDS, page),
                )
                .await?;
            Ok(mapping::media_page(rows))
        })
    }

    fn comments<'a>(
        &'a self,
        credential: &'a Credential,
        media_id: &'a ObjectId,
        page: &'a PageRequest,
    ) -> BoxFut<'a, CommentPage> {
        Box::pin(async move {
            let rows: Paged<CommentRow> = self
                .api
                .get(
                    credential,
                    &[media_id.as_str(), "comments"],
                    &page_query(COMMENT_FIELDS, page),
                )
                .await?;
            Ok(mapping::comment_page(rows))
        })
    }

    fn hashtag<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        name: &'a HashtagName,
    ) -> BoxFut<'a, Option<Hashtag>> {
        Box::pin(async move {
            let key = (account_id.to_string(), name.as_str().to_string());
            if let Some(hit) = self.hashtags.get(&key).await {
                return Ok(Some(hit));
            }
            let rows: Paged<IdRow> = self
                .api
                .get(
                    credential,
                    &["ig_hashtag_search"],
                    &[
                        ("user_id", account_id.to_string()),
                        ("q", name.as_str().to_string()),
                    ],
                )
                .await?;
            let Some(row) = rows.data.into_iter().next() else {
                return Ok(None);
            };
            let hashtag = Hashtag {
                id: row.id,
                name: name.as_str().to_string(),
            };
            self.hashtags.insert(key, hashtag.clone()).await;
            Ok(Some(hashtag))
        })
    }

    fn hashtag_media<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        hashtag_id: &'a str,
        sort: HashtagSort,
        page: &'a PageRequest,
    ) -> BoxFut<'a, MediaPage> {
        Box::pin(async move {
            let edge = match sort {
                HashtagSort::Top => "top_media",
                HashtagSort::Recent => "recent_media",
            };
            let mut query = page_query(HASHTAG_MEDIA_FIELDS, page);
            query.push(("user_id", account_id.to_string()));
            let rows: Paged<MediaRow> = self
                .api
                .get(credential, &[hashtag_id, edge], &query)
                .await?;
            Ok(mapping::media_page(rows))
        })
    }

    fn recent_hashtags<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
    ) -> BoxFut<'a, Vec<Hashtag>> {
        Box::pin(async move {
            let rows: Paged<HashtagRow> = self
                .api
                .get(
                    credential,
                    &[account_id, "recently_searched_hashtags"],
                    &[("limit", "30".into())],
                )
                .await?;
            Ok(rows
                .data
                .into_iter()
                .map(|row| mapping::hashtag(row, ""))
                .collect())
        })
    }

    fn discover<'a>(
        &'a self,
        credential: &'a Credential,
        account_id: &'a str,
        username: &'a Username,
        page: &'a PageRequest,
    ) -> BoxFut<'a, DiscoveredAccount> {
        Box::pin(async move {
            let envelope: DiscoveryEnvelope = self
                .api
                .get(
                    credential,
                    &[account_id],
                    &[("fields", discovery_fields(username, page))],
                )
                .await?;
            let row = envelope.business_discovery;
            Ok(DiscoveredAccount {
                profile: mapping::profile(row.profile),
                media: row.media.map(mapping::media_page).unwrap_or(MediaPage {
                    media: vec![],
                    next: None,
                }),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::media::Cursor;

    #[test]
    fn discovery_expression_nests_paging() {
        let page = PageRequest {
            after: Some(Cursor::parse("QVFI").unwrap()),
            ..PageRequest::first(25)
        };
        let fields = discovery_fields(&Username::parse("Rival").unwrap(), &page);
        assert!(fields.starts_with("business_discovery.username(rival){id,username,"));
        assert!(fields.contains(",media.limit(25).after(QVFI){id,caption,"));
        assert!(fields.ends_with("view_count}}"));
    }
}
