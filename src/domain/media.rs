//! Instagram content as this server understands it, and the validated values
//! that may reach the Meta adapter.
//!
//! Every identifier that ends up in a Graph API path or field expression is a
//! type here. Their constructors are the control that keeps tool input from
//! changing which object or field a request addresses.

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("{0}")]
    Invalid(String),
}

fn invalid(message: impl Into<String>) -> ValidationError {
    ValidationError::Invalid(message.into())
}

/// An Instagram username, without the @. Interpolated into Business Discovery
/// field expressions, so the character set is strict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Username(String);

impl Username {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let name = value.trim().trim_start_matches('@');
        let valid = (1..=30).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_');
        if !valid {
            return Err(invalid(format!(
                "{value:?} is not an Instagram username (letters, digits, '.' and '_', at most 30)"
            )));
        }
        Ok(Self(name.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A hashtag, without the #, lowercased.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HashtagName(String);

impl HashtagName {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let name = value.trim().trim_start_matches('#');
        let valid = (1..=100).contains(&name.chars().count())
            && name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !valid {
            return Err(invalid(format!(
                "{value:?} is not a hashtag (letters, digits and '_', no spaces)"
            )));
        }
        Ok(Self(name.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Graph API object id (media, comment, hashtag). Digits only, so it cannot
/// address another path such as `me` or `../`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectId(String);

impl ObjectId {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let id = value.trim();
        if !(1..=40).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid(format!(
                "{value:?} is not an Instagram id (digits)"
            )));
        }
        Ok(Self(id.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A pagination cursor as Meta returns it. Business Discovery interpolates it
/// into a field expression, so only the base64url-like alphabet is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor(String);

impl Cursor {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let valid = (1..=1024).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'='));
        if !valid {
            return Err(invalid(
                "the cursor is malformed: pass `next` back unchanged",
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
    CarouselAlbum,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductType {
    Feed,
    Reels,
    Story,
    Ad,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub id: String,
    pub caption: Option<String>,
    pub media_type: MediaType,
    pub product_type: Option<ProductType>,
    pub timestamp: Option<DateTime<Utc>>,
    pub permalink: Option<String>,
    pub media_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub like_count: Option<u64>,
    pub comments_count: Option<u64>,
    pub view_count: Option<u64>,
    pub username: Option<String>,
    pub owner_id: Option<String>,
}

impl Media {
    /// Likes plus comments: the only engagement Meta exposes for content the
    /// caller does not own.
    pub fn interactions(&self) -> u64 {
        self.like_count.unwrap_or(0) + self.comments_count.unwrap_or(0)
    }

    /// Every whitespace-separated term occurs in the caption, ignoring case.
    pub fn caption_matches(&self, query: &str) -> bool {
        let caption = self.caption.as_deref().unwrap_or_default().to_lowercase();
        query
            .split_whitespace()
            .all(|term| caption.contains(&term.to_lowercase()))
    }

    pub fn hashtags(&self) -> Vec<String> {
        hashtags_in(self.caption.as_deref().unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaPage {
    pub media: Vec<Media>,
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: String,
    pub username: String,
    pub name: Option<String>,
    pub biography: Option<String>,
    pub website: Option<String>,
    pub followers_count: Option<u64>,
    pub follows_count: Option<u64>,
    pub media_count: Option<u64>,
    pub profile_picture_url: Option<String>,
}

/// Another professional account as Business Discovery shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredAccount {
    pub profile: Profile,
    pub media: MediaPage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub id: String,
    pub text: String,
    pub username: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub like_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommentPage {
    pub comments: Vec<Comment>,
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hashtag {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashtagSort {
    /// Most popular media, as ranked by Instagram.
    Top,
    /// Media from the last 24 hours, newest first.
    Recent,
}

/// Lowercased hashtags in a caption, in order of appearance, without the #.
pub fn hashtags_in(text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('#') {
        let after = &rest[start + 1..];
        let end = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        if end > 0 {
            tags.push(after[..end].to_lowercase());
        }
        rest = &after[end..];
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_normalized_and_strict() {
        assert_eq!(Username::parse(" @Nat.Geo_ ").unwrap().as_str(), "nat.geo_");
        for bad in ["", "a b", "x){id}", "a,b", &"a".repeat(31), "name}"] {
            assert!(Username::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hashtags_allow_unicode_letters_only() {
        assert_eq!(
            HashtagName::parse("#Café_2026").unwrap().as_str(),
            "café_2026"
        );
        for bad in ["", "#", "two words", "a#b", "x&y"] {
            assert!(HashtagName::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn object_ids_cannot_name_another_path() {
        assert!(ObjectId::parse("17895695668004550").is_ok());
        for bad in ["me", "../me", "1/insights", "", "12a"] {
            assert!(ObjectId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cursors_cannot_break_out_of_a_field_expression() {
        assert!(Cursor::parse("QVFIUmVa-_x=").is_ok());
        for bad in ["", "a)}", "a{b", "a,b", "a b"] {
            assert!(Cursor::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn finds_hashtags_in_captions() {
        assert_eq!(
            hashtags_in("Sunset #Travel, #travel_tips!#x #"),
            ["travel", "travel_tips", "x"]
        );
    }

    fn media(caption: &str) -> Media {
        Media {
            id: "1".into(),
            caption: Some(caption.into()),
            media_type: MediaType::Image,
            product_type: None,
            timestamp: None,
            permalink: None,
            media_url: None,
            thumbnail_url: None,
            like_count: Some(3),
            comments_count: None,
            view_count: None,
            username: None,
            owner_id: None,
        }
    }

    #[test]
    fn caption_search_needs_every_term() {
        let m = media("Vegan Pasta recipe");
        assert!(m.caption_matches("pasta VEGAN"));
        assert!(!m.caption_matches("pasta pizza"));
        assert_eq!(m.interactions(), 3);
    }
}
