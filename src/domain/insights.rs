//! Insights: metric names, the shapes of a query, and the shape of a result.
//!
//! Meta adds and retires metrics several times a year, so metric names are a
//! validated string rather than an enum. Defaults below track the current
//! Graph API and are the only place to update when Meta changes them.

use crate::domain::media::{MediaType, ProductType, ValidationError};

/// The longest window Meta accepts for one account insights request.
pub const MAX_RANGE_DAYS: i64 = 30;
pub const DEFAULT_RANGE_DAYS: i64 = 28;
const DAY: i64 = 24 * 60 * 60;

/// Account metrics that work with `metric_type=total_value` and `period=day`.
pub const DEFAULT_ACCOUNT_METRICS: &[&str] = &[
    "views",
    "reach",
    "accounts_engaged",
    "total_interactions",
    "likes",
    "comments",
    "shares",
    "saves",
    "reposts",
    "replies",
    "follows_and_unfollows",
    "profile_links_taps",
];

const FEED_METRICS: &[&str] = &[
    "views",
    "reach",
    "likes",
    "comments",
    "shares",
    "saved",
    "reposts",
    "total_interactions",
    "profile_visits",
    "follows",
];
const REELS_METRICS: &[&str] = &[
    "views",
    "reach",
    "likes",
    "comments",
    "shares",
    "saved",
    "reposts",
    "total_interactions",
    "ig_reels_avg_watch_time",
    "ig_reels_video_view_total_time",
    "reels_skip_rate",
];
const STORY_METRICS: &[&str] = &[
    "views",
    "reach",
    "replies",
    "shares",
    "total_interactions",
    "follows",
    "profile_visits",
    "navigation",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricName(String);

impl MetricName {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let name = value.trim();
        let valid = (1..=64).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !valid {
            return Err(ValidationError::Invalid(format!(
                "{value:?} is not a metric name (lowercase letters, digits, '_')"
            )));
        }
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn known(names: &[&str]) -> Vec<Self> {
        names.iter().map(|name| Self((*name).to_string())).collect()
    }
}

/// Validates a requested list, or falls back to `defaults` when it is empty.
pub fn metrics_or(
    requested: &[String],
    defaults: Vec<MetricName>,
) -> Result<Vec<MetricName>, ValidationError> {
    if requested.is_empty() {
        return Ok(defaults);
    }
    if requested.len() > 30 {
        return Err(ValidationError::Invalid(
            "at most 30 metrics at once".into(),
        ));
    }
    requested
        .iter()
        .map(|name| MetricName::parse(name))
        .collect()
}

pub fn default_account_metrics() -> Vec<MetricName> {
    MetricName::known(DEFAULT_ACCOUNT_METRICS)
}

/// The metrics that make sense for one piece of media. Meta rejects a whole
/// request for one unsupported metric, so this has to follow the media kind.
pub fn default_media_metrics(
    media_type: &MediaType,
    product_type: Option<&ProductType>,
) -> Vec<MetricName> {
    let names = match (product_type, media_type) {
        (Some(ProductType::Story), _) => STORY_METRICS,
        (Some(ProductType::Reels), _) | (None, MediaType::Video) => REELS_METRICS,
        _ => FEED_METRICS,
    };
    MetricName::known(names)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Days28,
    Lifetime,
}

impl Period {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Days28 => "days_28",
            Self::Lifetime => "lifetime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    /// One number for the whole range, optionally broken down.
    TotalValue,
    /// One value per period, for charts.
    TimeSeries,
}

impl MetricType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TotalValue => "total_value",
            Self::TimeSeries => "time_series",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakdown {
    MediaProductType,
    FollowType,
    ContactButtonType,
    Age,
    Gender,
    Country,
    City,
}

impl Breakdown {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MediaProductType => "media_product_type",
            Self::FollowType => "follow_type",
            Self::ContactButtonType => "contact_button_type",
            Self::Age => "age",
            Self::Gender => "gender",
            Self::Country => "country",
            Self::City => "city",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeframe {
    ThisWeek,
    ThisMonth,
}

impl Timeframe {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ThisWeek => "this_week",
            Self::ThisMonth => "this_month",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// Everyone following the account.
    Followers,
    /// Accounts that interacted with the account's content.
    Engaged,
}

impl Audience {
    pub fn metric(self) -> MetricName {
        MetricName(
            match self {
                Self::Followers => "follower_demographics",
                Self::Engaged => "engaged_audience_demographics",
            }
            .to_string(),
        )
    }
}

/// A half-open time window in Unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub since: i64,
    pub until: i64,
}

impl Range {
    /// Missing ends default to the last `DEFAULT_RANGE_DAYS` up to `now`.
    pub fn resolve(
        since: Option<i64>,
        until: Option<i64>,
        now: i64,
    ) -> Result<Self, ValidationError> {
        let until = until.unwrap_or(now).min(now);
        let since = since.unwrap_or(until - DEFAULT_RANGE_DAYS * DAY);
        if since >= until {
            return Err(ValidationError::Invalid(
                "since must be before until, and not in the future".into(),
            ));
        }
        if until - since > MAX_RANGE_DAYS * DAY {
            return Err(ValidationError::Invalid(format!(
                "Instagram serves at most {MAX_RANGE_DAYS} days per request: split the range"
            )));
        }
        Ok(Self { since, until })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountInsightQuery {
    pub metrics: Vec<MetricName>,
    pub period: Period,
    pub metric_type: MetricType,
    pub breakdown: Option<Breakdown>,
    pub range: Option<Range>,
    pub timeframe: Option<Timeframe>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesPoint {
    pub end_time: Option<String>,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BreakdownRow {
    pub dimensions: Vec<String>,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BreakdownTable {
    pub dimension_keys: Vec<String>,
    pub rows: Vec<BreakdownRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: String,
    pub period: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub total: Option<f64>,
    pub values: Vec<SeriesPoint>,
    pub breakdowns: Vec<BreakdownTable>,
}

/// Metrics that could be read, and the ones Instagram refused for this object.
#[derive(Debug, Clone, PartialEq)]
pub struct InsightReport {
    pub metrics: Vec<Metric>,
    pub unavailable: Vec<UnavailableMetric>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnavailableMetric {
    pub name: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_names_are_plain_identifiers() {
        assert!(MetricName::parse("follows_and_unfollows").is_ok());
        for bad in ["", "Views", "views,reach", "a b", "x&y=1"] {
            assert!(MetricName::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn empty_request_means_defaults() {
        let metrics = metrics_or(&[], default_account_metrics()).unwrap();
        assert_eq!(metrics.len(), DEFAULT_ACCOUNT_METRICS.len());
        let chosen = metrics_or(&["reach".into()], default_account_metrics()).unwrap();
        assert_eq!(chosen, [MetricName::parse("reach").unwrap()]);
    }

    #[test]
    fn media_defaults_follow_the_kind() {
        let story = default_media_metrics(&MediaType::Image, Some(&ProductType::Story));
        assert!(story.iter().any(|m| m.as_str() == "navigation"));
        let reel = default_media_metrics(&MediaType::Video, Some(&ProductType::Reels));
        assert!(reel.iter().any(|m| m.as_str() == "ig_reels_avg_watch_time"));
        let post = default_media_metrics(&MediaType::CarouselAlbum, Some(&ProductType::Feed));
        assert!(post.iter().all(|m| m.as_str() != "navigation"));
    }

    #[test]
    fn ranges_default_and_are_bounded() {
        let now = 1_800_000_000;
        let r = Range::resolve(None, None, now).unwrap();
        assert_eq!(r.until, now);
        assert_eq!(r.until - r.since, DEFAULT_RANGE_DAYS * DAY);
        assert!(Range::resolve(Some(now - 31 * DAY), None, now).is_err());
        assert!(Range::resolve(Some(now), Some(now - DAY), now).is_err());
        // The future is clamped to now.
        assert_eq!(
            Range::resolve(Some(now - DAY), Some(now + DAY), now)
                .unwrap()
                .until,
            now
        );
    }
}
