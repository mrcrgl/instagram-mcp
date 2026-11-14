//! Tool arguments and results, with explicit mapping to and from the domain.

use chrono::{Days, NaiveDate};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::domain::analytics::{Bucket, Distribution, MediaStats, TagCount};
use crate::domain::insights::{
    Audience, Breakdown, InsightReport, Metric, MetricType, Period, Timeframe,
};
use crate::domain::media::{
    Comment, CommentPage, Cursor, DiscoveredAccount, Hashtag, HashtagSort, Media, MediaPage,
    MediaType, ProductType, Profile, ValidationError,
};
use crate::domain::tenant::IgAccount;
use crate::instagram::service::{AccountOverview, MediaReport, MediaSearch};
use crate::instagram::store::PageRequest;

const DEFAULT_LIMIT: u32 = 25;
const MAX_LIMIT: u32 = 100;
const DISCOVERY_MAX_LIMIT: u32 = 50;
const DEFAULT_SAMPLE: u32 = 50;

// ---------------------------------------------------------------- arguments

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct AccountArgs {
    /// Username of one of your connected accounts. Omit when only one is connected.
    pub account: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct MediaListArgs {
    pub account: Option<String>,
    /// Page size, 1-100. Defaults to 25.
    pub limit: Option<u32>,
    /// `next` from a previous page.
    pub after: Option<String>,
    /// Only media posted on or after this date (YYYY-MM-DD, UTC).
    pub since: Option<String>,
    /// Only media posted on or before this date (YYYY-MM-DD, UTC).
    pub until: Option<String>,
    /// Keyword search: only media whose caption contains every word, ignoring
    /// case. Scans up to 500 media.
    pub caption_contains: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MediaIdArgs {
    /// Id of a media item of one of your connected accounts.
    pub media_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MediaInsightsArgs {
    pub media_id: String,
    /// Metric names. Omit for the defaults that fit the media kind, e.g.
    /// views, reach, likes, comments, shares, saved, total_interactions;
    /// ig_reels_avg_watch_time for reels; navigation and replies for stories.
    pub metrics: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CommentsArgs {
    pub media_id: String,
    /// Page size, 1-100. Defaults to 25.
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PeriodArg {
    Day,
    Week,
    Days28,
    Lifetime,
}

impl From<PeriodArg> for Period {
    fn from(value: PeriodArg) -> Self {
        match value {
            PeriodArg::Day => Self::Day,
            PeriodArg::Week => Self::Week,
            PeriodArg::Days28 => Self::Days28,
            PeriodArg::Lifetime => Self::Lifetime,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricTypeArg {
    TotalValue,
    TimeSeries,
}

impl From<MetricTypeArg> for MetricType {
    fn from(value: MetricTypeArg) -> Self {
        match value {
            MetricTypeArg::TotalValue => Self::TotalValue,
            MetricTypeArg::TimeSeries => Self::TimeSeries,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BreakdownArg {
    MediaProductType,
    FollowType,
    ContactButtonType,
}

impl From<BreakdownArg> for Breakdown {
    fn from(value: BreakdownArg) -> Self {
        match value {
            BreakdownArg::MediaProductType => Self::MediaProductType,
            BreakdownArg::FollowType => Self::FollowType,
            BreakdownArg::ContactButtonType => Self::ContactButtonType,
        }
    }
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct AccountInsightsArgs {
    pub account: Option<String>,
    /// Metric names. Omit for: views, reach, accounts_engaged,
    /// total_interactions, likes, comments, shares, saves, reposts, replies,
    /// follows_and_unfollows, profile_links_taps. Also available:
    /// follower_count (time_series only), website_clicks, profile_views.
    pub metrics: Option<Vec<String>>,
    /// First day, YYYY-MM-DD (UTC). Defaults to 28 days before `until`.
    pub since: Option<String>,
    /// Last day, YYYY-MM-DD (UTC). Defaults to today. At most 30 days in total.
    pub until: Option<String>,
    /// Defaults to day.
    pub period: Option<PeriodArg>,
    /// total_value (default): one number for the range. time_series: one per
    /// day, supported by reach and follower_count.
    pub metric_type: Option<MetricTypeArg>,
    /// Split totals, e.g. by media_product_type (post, reel, story).
    pub breakdown: Option<BreakdownArg>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceArg {
    #[default]
    Followers,
    Engaged,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DemographicArg {
    Age,
    Gender,
    Country,
    City,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeframeArg {
    ThisWeek,
    #[default]
    ThisMonth,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DemographicsArgs {
    pub account: Option<String>,
    /// followers (default) or engaged: accounts that interacted recently.
    pub audience: Option<AudienceArg>,
    pub breakdown: DemographicArg,
    /// For the engaged audience. Defaults to this_month.
    pub timeframe: Option<TimeframeArg>,
}

impl DemographicsArgs {
    pub fn into_parts(self) -> (Option<String>, Audience, Breakdown, Timeframe) {
        let audience = match self.audience.unwrap_or_default() {
            AudienceArg::Followers => Audience::Followers,
            AudienceArg::Engaged => Audience::Engaged,
        };
        let breakdown = match self.breakdown {
            DemographicArg::Age => Breakdown::Age,
            DemographicArg::Gender => Breakdown::Gender,
            DemographicArg::Country => Breakdown::Country,
            DemographicArg::City => Breakdown::City,
        };
        let timeframe = match self.timeframe.unwrap_or_default() {
            TimeframeArg::ThisWeek => Timeframe::ThisWeek,
            TimeframeArg::ThisMonth => Timeframe::ThisMonth,
        };
        (self.account, audience, breakdown, timeframe)
    }
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct PageArgs {
    pub account: Option<String>,
    /// Page size, 1-100. Defaults to 25.
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortArg {
    /// The most popular media, as ranked by Instagram.
    #[default]
    Top,
    /// Media from the last 24 hours only, newest first.
    Recent,
}

impl From<SortArg> for HashtagSort {
    fn from(value: SortArg) -> Self {
        match value {
            SortArg::Top => Self::Top,
            SortArg::Recent => Self::Recent,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HashtagMediaArgs {
    /// The hashtag, with or without #.
    pub hashtag: String,
    pub sort: Option<SortArg>,
    /// Page size, 1-50. Defaults to 25.
    pub limit: Option<u32>,
    pub after: Option<String>,
    /// Only media whose caption contains every word, ignoring case.
    pub caption_contains: Option<String>,
    /// Your account to search as. Each account may look up 30 distinct
    /// hashtags per 7 days.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HashtagReportArgs {
    pub hashtag: String,
    pub sort: Option<SortArg>,
    /// How many media to analyze, 1-200. Defaults to 50.
    pub sample_size: Option<u32>,
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BusinessArgs {
    /// Username of another Instagram business or creator account.
    pub username: String,
    /// Recent media to include, 1-50. Defaults to 25.
    pub media_limit: Option<u32>,
    pub after: Option<String>,
    /// Your account to look up as.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BusinessReportArgs {
    pub username: String,
    /// How many recent media to analyze, 1-200. Defaults to 50.
    pub sample_size: Option<u32>,
    pub account: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct AccountReportArgs {
    pub account: Option<String>,
    /// How many recent media to analyze, 1-200. Defaults to 50.
    pub sample_size: Option<u32>,
}

pub fn page_request(
    limit: Option<u32>,
    after: Option<&str>,
    max: u32,
) -> Result<PageRequest, ValidationError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT.min(max));
    if !(1..=max).contains(&limit) {
        return Err(ValidationError::Invalid(format!(
            "limit must be between 1 and {max}"
        )));
    }
    Ok(PageRequest {
        limit,
        after: after.map(Cursor::parse).transpose()?,
        since: None,
        until: None,
    })
}

pub fn max_page() -> u32 {
    MAX_LIMIT
}

pub fn max_discovery_page() -> u32 {
    DISCOVERY_MAX_LIMIT
}

pub fn sample(size: Option<u32>) -> usize {
    size.unwrap_or(DEFAULT_SAMPLE) as usize
}

/// Start of the day, UTC.
pub fn day_start(value: &str) -> Result<i64, ValidationError> {
    Ok(date(value)?
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists")
        .and_utc()
        .timestamp())
}

/// End of the day, UTC: the start of the next one.
pub fn day_end(value: &str) -> Result<i64, ValidationError> {
    Ok(date(value)?
        .checked_add_days(Days::new(1))
        .ok_or_else(|| ValidationError::Invalid(format!("{value} is out of range")))?
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists")
        .and_utc()
        .timestamp())
}

fn date(value: &str) -> Result<NaiveDate, ValidationError> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map_err(|_| ValidationError::Invalid(format!("{value:?} is not a YYYY-MM-DD date")))
}

impl MediaListArgs {
    pub fn page(&self) -> Result<PageRequest, ValidationError> {
        let mut page = page_request(self.limit, self.after.as_deref(), MAX_LIMIT)?;
        page.since = self.since.as_deref().map(day_start).transpose()?;
        page.until = self.until.as_deref().map(day_end).transpose()?;
        Ok(page)
    }
}

// ------------------------------------------------------------------ results

#[derive(Debug, Serialize)]
pub struct MediaDto {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

pub fn media_kind(media_type: &MediaType, product_type: Option<&ProductType>) -> String {
    match (product_type, media_type) {
        (Some(ProductType::Reels), _) => "reel".into(),
        (Some(ProductType::Story), _) => "story".into(),
        (Some(ProductType::Ad), _) => "ad".into(),
        (_, MediaType::CarouselAlbum) => "carousel".into(),
        (_, MediaType::Video) => "video".into(),
        (_, MediaType::Image) => "image".into(),
        (_, MediaType::Other(other)) => other.to_lowercase(),
    }
}

impl From<Media> for MediaDto {
    fn from(m: Media) -> Self {
        Self {
            kind: media_kind(&m.media_type, m.product_type.as_ref()),
            posted_at: m.timestamp.map(|t| t.to_rfc3339()),
            id: m.id,
            username: m.username,
            caption: m.caption,
            likes: m.like_count,
            comments: m.comments_count,
            views: m.view_count,
            permalink: m.permalink,
            media_url: m.media_url,
            thumbnail_url: m.thumbnail_url,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MediaPageDto {
    pub media: Vec<MediaDto>,
    /// Pass as `after` for the next page. Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// Media read to find these, when a caption filter was applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanned: Option<usize>,
}

impl From<MediaPage> for MediaPageDto {
    fn from(page: MediaPage) -> Self {
        Self {
            media: page.media.into_iter().map(Into::into).collect(),
            next: page.next.map(|c| c.as_str().to_string()),
            scanned: None,
        }
    }
}

impl MediaPageDto {
    pub fn from_search(found: MediaSearch, filtered: bool) -> Self {
        Self {
            media: found.media.into_iter().map(Into::into).collect(),
            next: found.next.map(|c| c.as_str().to_string()),
            scanned: filtered.then_some(found.scanned),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HashtagMediaDto {
    pub hashtag: HashtagDto,
    #[serde(flatten)]
    pub page: MediaPageDto,
}

#[derive(Debug, Serialize)]
pub struct HashtagDto {
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl From<Hashtag> for HashtagDto {
    fn from(h: Hashtag) -> Self {
        Self {
            id: h.id,
            name: h.name,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProfileDto {
    pub id: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub biography: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_picture_url: Option<String>,
}

impl From<Profile> for ProfileDto {
    fn from(p: Profile) -> Self {
        Self {
            id: p.id,
            username: p.username,
            name: p.name,
            biography: p.biography,
            website: p.website,
            followers: p.followers_count,
            following: p.follows_count,
            media_count: p.media_count,
            profile_picture_url: p.profile_picture_url,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AccountDto {
    pub id: String,
    pub username: String,
    pub facebook_page: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<AccountOverview> for AccountDto {
    fn from(overview: AccountOverview) -> Self {
        let IgAccount {
            id,
            username,
            page_name,
        } = overview.account;
        let (followers, media_count, error) = match overview.profile {
            Ok(p) => (p.followers_count, p.media_count, None),
            Err(e) => (None, None, Some(e.to_string())),
        };
        Self {
            id,
            username,
            facebook_page: page_name,
            followers,
            media_count,
            error,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DiscoveredDto {
    pub profile: ProfileDto,
    #[serde(flatten)]
    pub media: MediaPageDto,
}

impl From<DiscoveredAccount> for DiscoveredDto {
    fn from(d: DiscoveredAccount) -> Self {
        Self {
            profile: d.profile.into(),
            media: d.media.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CommentDto {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
}

impl From<Comment> for CommentDto {
    fn from(c: Comment) -> Self {
        Self {
            posted_at: c.timestamp.map(|t| t.to_rfc3339()),
            id: c.id,
            username: c.username,
            text: c.text,
            likes: c.like_count,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CommentPageDto {
    pub comments: Vec<CommentDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

impl From<CommentPage> for CommentPageDto {
    fn from(page: CommentPage) -> Self {
        Self {
            comments: page.comments.into_iter().map(Into::into).collect(),
            next: page.next.map(|c| c.as_str().to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MetricDto {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<PointDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breakdowns: Vec<BreakdownDto>,
}

#[derive(Debug, Serialize)]
pub struct PointDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    pub value: f64,
}

#[derive(Debug, Serialize)]
pub struct BreakdownDto {
    pub by: Vec<String>,
    pub rows: Vec<BreakdownRowDto>,
}

#[derive(Debug, Serialize)]
pub struct BreakdownRowDto {
    pub key: Vec<String>,
    pub value: f64,
}

impl From<Metric> for MetricDto {
    fn from(m: Metric) -> Self {
        Self {
            name: m.name,
            title: m.title,
            description: m.description,
            period: m.period,
            total: m.total,
            series: m
                .values
                .into_iter()
                .map(|p| PointDto {
                    end_time: p.end_time,
                    value: p.value,
                })
                .collect(),
            breakdowns: m
                .breakdowns
                .into_iter()
                .map(|b| BreakdownDto {
                    by: b.dimension_keys,
                    rows: b
                        .rows
                        .into_iter()
                        .map(|r| BreakdownRowDto {
                            key: r.dimensions,
                            value: r.value,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InsightsDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaDto>,
    pub metrics: Vec<MetricDto>,
    /// Requested metrics Instagram does not provide here, with its reason.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unavailable: Vec<UnavailableDto>,
}

#[derive(Debug, Serialize)]
pub struct UnavailableDto {
    pub metric: String,
    pub reason: String,
}

impl InsightsDto {
    pub fn new(media: Option<Media>, report: InsightReport) -> Self {
        Self {
            media: media.map(Into::into),
            metrics: report.metrics.into_iter().map(Into::into).collect(),
            unavailable: report
                .unavailable
                .into_iter()
                .map(|u| UnavailableDto {
                    metric: u.name,
                    reason: u.reason,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DistributionDto {
    pub mean: f64,
    pub median: f64,
    pub max: u64,
    pub samples: usize,
}

impl From<Distribution> for DistributionDto {
    fn from(d: Distribution) -> Self {
        Self {
            mean: d.mean,
            median: d.median,
            max: d.max,
            samples: d.samples,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BucketDto {
    pub key: String,
    pub media: usize,
    pub mean_interactions: f64,
}

impl From<Bucket> for BucketDto {
    fn from(b: Bucket) -> Self {
        Self {
            key: b.key,
            media: b.media,
            mean_interactions: b.mean_interactions,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TagCountDto {
    pub tag: String,
    pub media: usize,
}

impl From<TagCount> for TagCountDto {
    fn from(t: TagCount) -> Self {
        Self {
            tag: t.tag,
            media: t.media,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StatsDto {
    pub sample_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_post: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts_per_week: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<DistributionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<DistributionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<DistributionDto>,
    /// Likes plus comments per post.
    pub mean_interactions: f64,
    /// Mean interactions per post as % of followers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engagement_rate_percent: Option<f64>,
    pub mean_hashtags_per_post: f64,
    pub mean_caption_chars: f64,
    pub by_format: Vec<BucketDto>,
    /// UTC.
    pub by_weekday: Vec<BucketDto>,
    /// UTC hour.
    pub by_hour: Vec<BucketDto>,
    pub top_hashtags: Vec<TagCountDto>,
}

impl From<MediaStats> for StatsDto {
    fn from(s: MediaStats) -> Self {
        let all = |b: Vec<Bucket>| b.into_iter().map(Into::into).collect();
        Self {
            sample_size: s.sample_size,
            first_post: s.first_post,
            last_post: s.last_post,
            posts_per_week: s.posts_per_week,
            likes: s.likes.map(Into::into),
            comments: s.comments.map(Into::into),
            views: s.views.map(Into::into),
            mean_interactions: s.mean_interactions,
            engagement_rate_percent: s.engagement_rate_percent,
            mean_hashtags_per_post: s.mean_hashtags_per_post,
            mean_caption_chars: s.mean_caption_chars,
            by_format: all(s.by_format),
            by_weekday: all(s.by_weekday),
            by_hour: all(s.by_hour),
            top_hashtags: s.top_hashtags.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReportDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hashtag: Option<HashtagDto>,
    pub stats: StatsDto,
    /// Media with the most likes plus comments, best first.
    pub top_media: Vec<MediaDto>,
}

impl From<MediaReport> for ReportDto {
    fn from(r: MediaReport) -> Self {
        Self {
            profile: r.profile.map(Into::into),
            hashtag: r.hashtag.map(Into::into),
            stats: r.stats.into(),
            top_media: r.top.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_cover_whole_days() {
        assert_eq!(day_start("2026-11-14").unwrap(), 1_794_614_400);
        assert_eq!(day_end("2026-11-14").unwrap(), 1_794_614_400 + 86_400);
        assert!(day_start("14.11.2026").is_err());
    }

    #[test]
    fn page_limits_are_bounded() {
        assert_eq!(page_request(None, None, 100).unwrap().limit, 25);
        assert!(page_request(Some(0), None, 100).is_err());
        assert!(page_request(Some(51), None, 50).is_err());
        assert!(page_request(None, Some("a}b"), 100).is_err());
    }

    #[test]
    fn media_kinds_are_readable() {
        assert_eq!(
            media_kind(&MediaType::Video, Some(&ProductType::Reels)),
            "reel"
        );
        assert_eq!(
            media_kind(&MediaType::CarouselAlbum, Some(&ProductType::Feed)),
            "carousel"
        );
    }
}
