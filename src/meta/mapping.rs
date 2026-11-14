//! Graph API rows → domain types.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::domain::insights::{BreakdownRow, BreakdownTable, Metric, SeriesPoint};
use crate::domain::media::{
    Comment, CommentPage, Cursor, Hashtag, Media, MediaPage, MediaType, ProductType, Profile,
};
use crate::domain::tenant::IgAccount;
use crate::meta::models::{
    CommentRow, HashtagRow, InsightRow, MediaRow, PageRow, Paged, Paging, ProfileRow,
};

/// Meta writes `2026-11-02T09:00:00+0000`, which is not RFC 3339.
pub fn timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    let value = value?;
    DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%z")
        .or_else(|_| DateTime::parse_from_rfc3339(value))
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// A cursor only when Meta says there is a next page. Malformed ones are
/// dropped rather than handed to the model.
pub fn next_cursor(paging: Option<&Paging>) -> Option<Cursor> {
    let paging = paging?;
    paging.next.as_ref()?;
    let after = paging.cursors.as_ref()?.after.as_deref()?;
    Cursor::parse(after).ok()
}

pub fn media_type(value: Option<&str>) -> MediaType {
    match value {
        Some("IMAGE") => MediaType::Image,
        Some("VIDEO") => MediaType::Video,
        Some("CAROUSEL_ALBUM") => MediaType::CarouselAlbum,
        Some(other) => MediaType::Other(other.to_string()),
        None => MediaType::Other("UNKNOWN".into()),
    }
}

pub fn product_type(value: Option<&str>) -> Option<ProductType> {
    Some(match value? {
        "FEED" => ProductType::Feed,
        "REELS" => ProductType::Reels,
        "STORY" => ProductType::Story,
        "AD" => ProductType::Ad,
        other => ProductType::Other(other.to_string()),
    })
}

pub fn media(row: MediaRow) -> Media {
    Media {
        media_type: media_type(row.media_type.as_deref()),
        product_type: product_type(row.media_product_type.as_deref()),
        timestamp: timestamp(row.timestamp.as_deref()),
        owner_id: row.owner.map(|owner| owner.id),
        id: row.id,
        caption: row.caption,
        permalink: row.permalink,
        media_url: row.media_url,
        thumbnail_url: row.thumbnail_url,
        like_count: row.like_count,
        comments_count: row.comments_count,
        view_count: row.view_count,
        username: row.username,
    }
}

pub fn media_page(page: Paged<MediaRow>) -> MediaPage {
    MediaPage {
        next: next_cursor(page.paging.as_ref()),
        media: page.data.into_iter().map(media).collect(),
    }
}

pub fn profile(row: ProfileRow) -> Profile {
    Profile {
        username: row.username.unwrap_or_default(),
        id: row.id,
        name: row.name,
        biography: row.biography,
        website: row.website,
        followers_count: row.followers_count,
        follows_count: row.follows_count,
        media_count: row.media_count,
        profile_picture_url: row.profile_picture_url,
    }
}

pub fn comment_page(page: Paged<CommentRow>) -> CommentPage {
    CommentPage {
        next: next_cursor(page.paging.as_ref()),
        comments: page
            .data
            .into_iter()
            .map(|row| Comment {
                timestamp: timestamp(row.timestamp.as_deref()),
                id: row.id,
                text: row.text.unwrap_or_default(),
                username: row.username,
                like_count: row.like_count,
            })
            .collect(),
    }
}

pub fn hashtag(row: HashtagRow, fallback_name: &str) -> Hashtag {
    Hashtag {
        id: row.id,
        name: row.name.unwrap_or_else(|| fallback_name.to_string()),
    }
}

/// Pages without a linked Instagram professional account are skipped.
pub fn accounts(pages: Vec<PageRow>) -> Vec<IgAccount> {
    let mut accounts: Vec<IgAccount> = pages
        .into_iter()
        .filter_map(|page| {
            let linked = page.instagram_business_account?;
            Some(IgAccount {
                username: linked.username.unwrap_or_else(|| linked.id.clone()),
                id: linked.id,
                page_name: page.name.unwrap_or_default(),
            })
        })
        .collect();
    accounts.sort_by(|a, b| a.username.cmp(&b.username));
    accounts.dedup_by(|a, b| a.id == b.id);
    accounts
}

/// Numbers stay numbers. Object values, such as a breakdown keyed by hour or
/// action, become a one-dimensional breakdown table.
pub fn metric(row: InsightRow) -> Metric {
    let mut breakdowns = Vec::new();
    let mut values = Vec::new();
    for point in row.values {
        match number(&point.value) {
            Some(value) => values.push(SeriesPoint {
                end_time: point.end_time,
                value,
            }),
            None => breakdowns.extend(object_table(&point.value)),
        }
    }
    let mut total = None;
    if let Some(total_value) = row.total_value {
        if let Some(value) = total_value.value.as_ref() {
            total = number(value);
            if total.is_none() {
                breakdowns.extend(object_table(value));
            }
        }
        for breakdown in total_value.breakdowns {
            breakdowns.push(BreakdownTable {
                dimension_keys: breakdown.dimension_keys,
                rows: breakdown
                    .results
                    .into_iter()
                    .filter_map(|result| {
                        Some(BreakdownRow {
                            value: number(&result.value)?,
                            dimensions: result.dimension_values,
                        })
                    })
                    .collect(),
            });
        }
    }
    // Lifetime metrics report a single value as a one-point series.
    if total.is_none() && values.len() == 1 && values[0].end_time.is_none() {
        total = values.pop().map(|point| point.value);
    }
    for table in &mut breakdowns {
        table.rows.sort_by(|a, b| {
            b.value
                .total_cmp(&a.value)
                .then_with(|| a.dimensions.cmp(&b.dimensions))
        });
    }
    Metric {
        name: row.name,
        period: row.period,
        title: row.title,
        description: row.description,
        total,
        values,
        breakdowns,
    }
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64()
}

fn object_table(value: &Value) -> Option<BreakdownTable> {
    let object = value.as_object()?;
    Some(BreakdownTable {
        dimension_keys: vec!["key".into()],
        rows: object
            .iter()
            .filter_map(|(key, value)| {
                Some(BreakdownRow {
                    dimensions: vec![key.clone()],
                    value: number(value)?,
                })
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::meta::models::{DiscoveryEnvelope, InsightRow};

    #[test]
    fn meta_timestamps_parse() {
        let t = timestamp(Some("2026-11-02T09:00:00+0000")).unwrap();
        assert_eq!(t.to_rfc3339(), "2026-11-02T09:00:00+00:00");
        assert!(timestamp(Some("yesterday")).is_none());
    }

    #[test]
    fn a_cursor_needs_a_next_page() {
        let last: Paged<MediaRow> =
            serde_json::from_value(json!({"data": [], "paging": {"cursors": {"after": "abc"}}}))
                .unwrap();
        assert!(next_cursor(last.paging.as_ref()).is_none());
        let more: Paged<MediaRow> = serde_json::from_value(
            json!({"data": [], "paging": {"cursors": {"after": "abc"}, "next": "https://x"}}),
        )
        .unwrap();
        assert_eq!(next_cursor(more.paging.as_ref()).unwrap().as_str(), "abc");
    }

    #[test]
    fn media_rows_map_with_owner_and_kind() {
        let row: MediaRow = serde_json::from_value(json!({
            "id": "1", "media_type": "VIDEO", "media_product_type": "REELS",
            "like_count": 5, "owner": {"id": "17841"}
        }))
        .unwrap();
        let m = media(row);
        assert_eq!(m.media_type, MediaType::Video);
        assert_eq!(m.product_type, Some(ProductType::Reels));
        assert_eq!(m.owner_id.as_deref(), Some("17841"));
    }

    #[test]
    fn pages_without_instagram_are_skipped() {
        let pages: Vec<PageRow> = serde_json::from_value(json!([
            {"name": "Brand", "instagram_business_account": {"id": "2", "username": "brand"}},
            {"name": "No IG"},
            {"name": "Brand again", "instagram_business_account": {"id": "2", "username": "brand"}},
        ]))
        .unwrap();
        let accounts = accounts(pages);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].page_name, "Brand");
    }

    #[test]
    fn total_values_and_breakdowns_map() {
        let row: InsightRow = serde_json::from_value(json!({
            "name": "follower_demographics", "period": "lifetime",
            "total_value": {"breakdowns": [{
                "dimension_keys": ["country"],
                "results": [
                    {"dimension_values": ["DE"], "value": 10},
                    {"dimension_values": ["US"], "value": 30}
                ]
            }]}
        }))
        .unwrap();
        let m = metric(row);
        assert_eq!(m.total, None);
        assert_eq!(m.breakdowns[0].rows[0].dimensions, ["US"]);
    }

    #[test]
    fn series_and_object_values_map() {
        let series: InsightRow = serde_json::from_value(json!({
            "name": "reach", "period": "day",
            "values": [{"value": 3, "end_time": "a"}, {"value": 4, "end_time": "b"}]
        }))
        .unwrap();
        assert_eq!(metric(series).values.len(), 2);

        let lifetime: InsightRow = serde_json::from_value(json!({
            "name": "views", "period": "lifetime", "values": [{"value": 42}]
        }))
        .unwrap();
        assert_eq!(metric(lifetime).total, Some(42.0));

        let keyed: InsightRow = serde_json::from_value(json!({
            "name": "online_followers", "period": "lifetime",
            "values": [{"value": {"0": 5, "1": 9}, "end_time": "a"}]
        }))
        .unwrap();
        let m = metric(keyed);
        assert_eq!(m.breakdowns[0].rows[0].dimensions, ["1"]);
    }

    #[test]
    fn business_discovery_flattens_the_profile() {
        let env: DiscoveryEnvelope = serde_json::from_value(json!({
            "business_discovery": {
                "id": "9", "username": "rival", "followers_count": 100,
                "media": {"data": [{"id": "5", "like_count": 1}]}
            },
            "id": "1"
        }))
        .unwrap();
        assert_eq!(
            profile(env.business_discovery.profile).followers_count,
            Some(100)
        );
    }
}
