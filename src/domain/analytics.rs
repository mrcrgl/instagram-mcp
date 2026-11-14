//! Aggregates over a sample of media. Pure: the same sample always gives the
//! same report, which is what makes hashtag and competitor reports testable.
//!
//! Engagement here is likes plus comments, because that is all Meta exposes
//! for content the caller does not own. Hidden like counts are skipped rather
//! than counted as zero.

use std::collections::HashMap;

use chrono::{Datelike, Timelike};

use crate::domain::media::{Media, MediaType, ProductType};

const TOP_HASHTAGS: usize = 20;
const TOP_MEDIA: usize = 5;
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[derive(Debug, Clone, PartialEq)]
pub struct Distribution {
    pub mean: f64,
    pub median: f64,
    pub max: u64,
    /// How many media had this value at all.
    pub samples: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    pub key: String,
    pub media: usize,
    pub mean_interactions: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TagCount {
    pub tag: String,
    pub media: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaStats {
    pub sample_size: usize,
    pub first_post: Option<String>,
    pub last_post: Option<String>,
    pub posts_per_week: Option<f64>,
    pub likes: Option<Distribution>,
    pub comments: Option<Distribution>,
    pub views: Option<Distribution>,
    pub mean_interactions: f64,
    /// Mean interactions per post as a percentage of followers, when known.
    pub engagement_rate_percent: Option<f64>,
    pub mean_hashtags_per_post: f64,
    pub mean_caption_chars: f64,
    pub by_format: Vec<Bucket>,
    /// UTC.
    pub by_weekday: Vec<Bucket>,
    /// UTC hour of day.
    pub by_hour: Vec<Bucket>,
    pub top_hashtags: Vec<TagCount>,
    /// Ids of the media with the most interactions, best first.
    pub top_media: Vec<String>,
}

/// `exclude_tag` leaves the hashtag a sample was drawn from out of the
/// co-occurrence list, where it would always rank first.
pub fn media_stats(
    media: &[Media],
    followers: Option<u64>,
    exclude_tag: Option<&str>,
) -> MediaStats {
    let n = media.len();
    let interactions: Vec<u64> = media.iter().map(Media::interactions).collect();
    let mean_interactions = mean(&interactions);

    let mut timestamps: Vec<_> = media.iter().filter_map(|m| m.timestamp).collect();
    timestamps.sort();
    let posts_per_week = match (timestamps.first(), timestamps.last()) {
        (Some(first), Some(last)) if timestamps.len() > 1 && last > first => {
            let weeks = (*last - *first).num_seconds() as f64 / (7.0 * 86_400.0);
            Some(round2(timestamps.len() as f64 / weeks.max(1.0 / 7.0)))
        }
        _ => None,
    };

    let mut tags: HashMap<String, usize> = HashMap::new();
    let mut tag_total = 0usize;
    for m in media {
        let mut seen = m.hashtags();
        tag_total += seen.len();
        seen.sort();
        seen.dedup();
        for tag in seen {
            if Some(tag.as_str()) != exclude_tag {
                *tags.entry(tag).or_default() += 1;
            }
        }
    }
    let mut top_hashtags: Vec<TagCount> = tags
        .into_iter()
        .map(|(tag, media)| TagCount { tag, media })
        .collect();
    top_hashtags.sort_by(|a, b| b.media.cmp(&a.media).then_with(|| a.tag.cmp(&b.tag)));
    top_hashtags.truncate(TOP_HASHTAGS);

    let mut ranked: Vec<&Media> = media.iter().collect();
    ranked.sort_by(|a, b| {
        b.interactions()
            .cmp(&a.interactions())
            .then_with(|| a.id.cmp(&b.id))
    });

    let caption_chars: Vec<u64> = media
        .iter()
        .map(|m| m.caption.as_deref().map_or(0, |c| c.chars().count() as u64))
        .collect();

    MediaStats {
        sample_size: n,
        first_post: timestamps.first().map(|t| t.to_rfc3339()),
        last_post: timestamps.last().map(|t| t.to_rfc3339()),
        posts_per_week,
        likes: distribution(media.iter().filter_map(|m| m.like_count)),
        comments: distribution(media.iter().filter_map(|m| m.comments_count)),
        views: distribution(media.iter().filter_map(|m| m.view_count)),
        mean_interactions: round2(mean_interactions),
        engagement_rate_percent: followers
            .filter(|f| *f > 0 && n > 0)
            .map(|f| round2(mean_interactions / f as f64 * 100.0)),
        mean_hashtags_per_post: if n == 0 {
            0.0
        } else {
            round2(tag_total as f64 / n as f64)
        },
        mean_caption_chars: round2(mean(&caption_chars)),
        by_format: buckets(media, |m| Some(format_label(m))),
        by_weekday: ordered(
            buckets(media, |m| {
                m.timestamp
                    .map(|t| WEEKDAYS[t.weekday().num_days_from_monday() as usize].to_string())
            }),
            |key| WEEKDAYS.iter().position(|d| *d == key).unwrap_or(7),
        ),
        by_hour: ordered(
            buckets(media, |m| m.timestamp.map(|t| format!("{:02}", t.hour()))),
            |key| key.parse::<usize>().unwrap_or(24),
        ),
        top_hashtags,
        top_media: ranked
            .iter()
            .take(TOP_MEDIA)
            .map(|m| m.id.clone())
            .collect(),
    }
}

fn format_label(media: &Media) -> String {
    match (&media.product_type, &media.media_type) {
        (Some(ProductType::Reels), _) => "reel".into(),
        (Some(ProductType::Story), _) => "story".into(),
        (_, MediaType::CarouselAlbum) => "carousel".into(),
        (_, MediaType::Video) => "video".into(),
        (_, MediaType::Image) => "image".into(),
        (_, MediaType::Other(other)) => other.to_lowercase(),
    }
}

fn buckets(media: &[Media], key: impl Fn(&Media) -> Option<String>) -> Vec<Bucket> {
    let mut groups: HashMap<String, Vec<u64>> = HashMap::new();
    for m in media {
        if let Some(k) = key(m) {
            groups.entry(k).or_default().push(m.interactions());
        }
    }
    let mut out: Vec<Bucket> = groups
        .into_iter()
        .map(|(key, values)| Bucket {
            key,
            media: values.len(),
            mean_interactions: round2(mean(&values)),
        })
        .collect();
    out.sort_by(|a, b| b.media.cmp(&a.media).then_with(|| a.key.cmp(&b.key)));
    out
}

fn ordered(mut buckets: Vec<Bucket>, rank: impl Fn(&str) -> usize) -> Vec<Bucket> {
    buckets.sort_by_key(|b| rank(&b.key));
    buckets
}

fn distribution(values: impl Iterator<Item = u64>) -> Option<Distribution> {
    let mut values: Vec<u64> = values.collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let len = values.len();
    let median = if len % 2 == 1 {
        values[len / 2] as f64
    } else {
        (values[len / 2 - 1] + values[len / 2]) as f64 / 2.0
    };
    Some(Distribution {
        mean: round2(mean(&values)),
        median,
        max: values[len - 1],
        samples: len,
    })
}

fn mean(values: &[u64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<u64>() as f64 / values.len() as f64
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;

    fn media(id: &str, at: &str, likes: Option<u64>, comments: u64, caption: &str) -> Media {
        Media {
            id: id.into(),
            caption: Some(caption.into()),
            media_type: MediaType::Image,
            product_type: Some(ProductType::Feed),
            timestamp: Some(at.parse::<DateTime<Utc>>().unwrap()),
            permalink: None,
            media_url: None,
            thumbnail_url: None,
            like_count: likes,
            comments_count: Some(comments),
            view_count: None,
            username: None,
            owner_id: None,
        }
    }

    fn sample() -> Vec<Media> {
        vec![
            // Monday 09:00
            media("a", "2026-11-02T09:00:00Z", Some(100), 10, "#food #vegan"),
            // Monday 18:00
            media("b", "2026-11-02T18:00:00Z", Some(300), 30, "#food #pasta"),
            // Wednesday 09:00, likes hidden
            media("c", "2026-11-04T09:00:00Z", None, 5, "#Food"),
        ]
    }

    #[test]
    fn engagement_is_averaged_over_known_values() {
        let stats = media_stats(&sample(), Some(1000), None);
        assert_eq!(stats.sample_size, 3);
        let likes = stats.likes.unwrap();
        assert_eq!((likes.mean, likes.median, likes.samples), (200.0, 200.0, 2));
        // (110 + 330 + 5) / 3
        assert_eq!(stats.mean_interactions, 148.33);
        assert_eq!(stats.engagement_rate_percent, Some(14.83));
        assert_eq!(stats.top_media, ["b", "a", "c"]);
    }

    #[test]
    fn hashtags_co_occur_without_the_sampled_tag() {
        let stats = media_stats(&sample(), None, Some("food"));
        let tags: Vec<_> = stats.top_hashtags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(tags, ["pasta", "vegan"]);
        assert_eq!(stats.mean_hashtags_per_post, 1.67);
    }

    #[test]
    fn timing_buckets_are_in_calendar_order() {
        let stats = media_stats(&sample(), None, None);
        let days: Vec<_> = stats
            .by_weekday
            .iter()
            .map(|b| (b.key.as_str(), b.media))
            .collect();
        assert_eq!(days, [("Mon", 2), ("Wed", 1)]);
        let hours: Vec<_> = stats.by_hour.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(hours, ["09", "18"]);
        assert_eq!(stats.posts_per_week, Some(10.5));
    }

    #[test]
    fn an_empty_sample_is_not_an_error() {
        let stats = media_stats(&[], Some(10), None);
        assert_eq!(stats.sample_size, 0);
        assert_eq!(stats.engagement_rate_percent, None);
        assert!(stats.likes.is_none());
    }
}
