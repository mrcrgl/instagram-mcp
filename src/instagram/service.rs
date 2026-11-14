//! Use cases. Resolves which of the tenant's accounts a call acts as, keeps
//! object reads inside the tenant, and turns media samples into reports.

use std::sync::Arc;

use crate::domain::analytics::{MediaStats, media_stats};
use crate::domain::insights::{
    AccountInsightQuery, Audience, Breakdown, InsightReport, MetricType, Period, Range, Timeframe,
    UnavailableMetric, default_media_metrics, metrics_or,
};
use crate::domain::media::{
    CommentPage, Cursor, DiscoveredAccount, Hashtag, HashtagName, HashtagSort, Media, MediaPage,
    ObjectId, Profile, Username, ValidationError,
};
use crate::domain::tenant::{AccountError, IgAccount, Session};
use crate::instagram::store::{InstagramStore, PageRequest, StoreError};

/// Upper bound on media read for one keyword search or report. Keeps a single
/// tool call within a few Graph API requests.
pub const MAX_SCAN: usize = 500;
pub const MAX_SAMPLE: usize = 200;
const SCAN_PAGE: u32 = 100;
/// Hashtag and Business Discovery pages are smaller on Meta's side.
const DISCOVERY_PAGE: u32 = 50;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Account(#[from] AccountError),
    #[error("{0}")]
    Invalid(#[from] ValidationError),
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    NotFound(String),
}

impl ServiceError {
    /// A stable label for logs.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Account(_) => "account",
            Self::Invalid(_) => "invalid",
            Self::Store(error) => error.kind(),
            Self::NotFound(_) => "not_found",
        }
    }
}

type Result<T> = std::result::Result<T, ServiceError>;

pub struct AccountOverview {
    pub account: IgAccount,
    pub profile: std::result::Result<Profile, StoreError>,
}

/// A page of media after an optional caption filter.
pub struct MediaSearch {
    pub media: Vec<Media>,
    pub next: Option<Cursor>,
    pub scanned: usize,
}

pub struct MediaReport {
    pub profile: Option<Profile>,
    pub hashtag: Option<Hashtag>,
    pub stats: MediaStats,
    pub top: Vec<Media>,
}

pub struct AccountInsightRequest {
    pub metrics: Vec<String>,
    pub period: Period,
    pub metric_type: MetricType,
    pub breakdown: Option<Breakdown>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

pub struct InstagramService {
    store: Arc<dyn InstagramStore>,
    clock: fn() -> i64,
}

impl InstagramService {
    pub fn new(store: Arc<dyn InstagramStore>, clock: fn() -> i64) -> Self {
        Self { store, clock }
    }

    pub async fn accounts(&self, session: &Session) -> Vec<AccountOverview> {
        let mut out = Vec::with_capacity(session.tenant.accounts.len());
        for account in &session.tenant.accounts {
            let profile = self.store.profile(&session.credential, &account.id).await;
            out.push(AccountOverview {
                account: account.clone(),
                profile,
            });
        }
        out
    }

    pub async fn profile(&self, session: &Session, account: Option<&str>) -> Result<Profile> {
        let account = session.tenant.account(account)?;
        Ok(self.store.profile(&session.credential, &account.id).await?)
    }

    pub async fn media_list(
        &self,
        session: &Session,
        account: Option<&str>,
        page: PageRequest,
        caption: Option<&str>,
    ) -> Result<MediaSearch> {
        let account = session.tenant.account(account)?;
        let credential = &session.credential;
        let store = &self.store;
        search(page, caption, |request| async move {
            store.media_page(credential, &account.id, &request).await
        })
        .await
    }

    /// Only media of the tenant's own accounts. Meta would also refuse most
    /// foreign media, but isolation should not depend on that.
    pub async fn media(&self, session: &Session, id: &str) -> Result<Media> {
        let id = ObjectId::parse(id)?;
        let media = match self.store.media(&session.credential, &id).await {
            Err(StoreError::NotFound | StoreError::Rejected(_)) => None,
            other => Some(other?),
        };
        media
            .filter(|m| {
                m.owner_id
                    .as_deref()
                    .is_some_and(|owner| session.tenant.owns(owner))
            })
            .ok_or_else(|| {
                ServiceError::NotFound(format!(
                    "Media {} was not found among your connected accounts' media.",
                    id.as_str()
                ))
            })
    }

    /// Tries all metrics in one request, and on rejection one by one, so a
    /// single metric Meta does not offer for this media does not hide the rest.
    pub async fn media_insights(
        &self,
        session: &Session,
        id: &str,
        metrics: &[String],
    ) -> Result<(Media, InsightReport)> {
        let media = self.media(session, id).await?;
        let id = ObjectId::parse(&media.id)?;
        let metrics = metrics_or(
            metrics,
            default_media_metrics(&media.media_type, media.product_type.as_ref()),
        )?;
        let credential = &session.credential;
        match self.store.media_insights(credential, &id, &metrics).await {
            Ok(found) => Ok((
                media,
                InsightReport {
                    metrics: found,
                    unavailable: vec![],
                },
            )),
            Err(StoreError::Rejected(_)) if metrics.len() > 1 => {
                let mut report = InsightReport {
                    metrics: vec![],
                    unavailable: vec![],
                };
                for metric in &metrics {
                    let one = std::slice::from_ref(metric);
                    match self.store.media_insights(credential, &id, one).await {
                        Ok(found) => report.metrics.extend(found),
                        Err(StoreError::Rejected(reason)) => {
                            report.unavailable.push(UnavailableMetric {
                                name: metric.as_str().to_string(),
                                reason,
                            })
                        }
                        Err(other) => return Err(other.into()),
                    }
                }
                Ok((media, report))
            }
            Err(other) => Err(other.into()),
        }
    }

    pub async fn comments(
        &self,
        session: &Session,
        media_id: &str,
        page: PageRequest,
    ) -> Result<CommentPage> {
        let media = self.media(session, media_id).await?;
        let id = ObjectId::parse(&media.id)?;
        Ok(self.store.comments(&session.credential, &id, &page).await?)
    }

    pub async fn account_insights(
        &self,
        session: &Session,
        account: Option<&str>,
        request: AccountInsightRequest,
    ) -> Result<InsightReport> {
        let account = session.tenant.account(account)?;
        let metrics = metrics_or(
            &request.metrics,
            crate::domain::insights::default_account_metrics(),
        )?;
        let range = Range::resolve(request.since, request.until, (self.clock)())?;
        let query = AccountInsightQuery {
            metrics,
            period: request.period,
            metric_type: request.metric_type,
            breakdown: request.breakdown,
            range: Some(range),
            timeframe: None,
        };
        self.account_query(session, &account.id, query).await
    }

    pub async fn demographics(
        &self,
        session: &Session,
        account: Option<&str>,
        audience: Audience,
        breakdown: Breakdown,
        timeframe: Timeframe,
    ) -> Result<InsightReport> {
        if !matches!(
            breakdown,
            Breakdown::Age | Breakdown::Gender | Breakdown::Country | Breakdown::City
        ) {
            return Err(ValidationError::Invalid(
                "demographics break down by age, gender, country, or city".into(),
            )
            .into());
        }
        let account = session.tenant.account(account)?;
        let query = AccountInsightQuery {
            metrics: vec![audience.metric()],
            period: Period::Lifetime,
            metric_type: MetricType::TotalValue,
            breakdown: Some(breakdown),
            range: None,
            timeframe: Some(timeframe),
        };
        self.account_query(session, &account.id, query).await
    }

    /// Like media insights, falls back to one metric at a time on rejection.
    async fn account_query(
        &self,
        session: &Session,
        account_id: &str,
        query: AccountInsightQuery,
    ) -> Result<InsightReport> {
        let credential = &session.credential;
        match self
            .store
            .account_insights(credential, account_id, &query)
            .await
        {
            Ok(metrics) => Ok(InsightReport {
                metrics,
                unavailable: vec![],
            }),
            Err(StoreError::Rejected(_)) if query.metrics.len() > 1 => {
                let mut report = InsightReport {
                    metrics: vec![],
                    unavailable: vec![],
                };
                for metric in &query.metrics {
                    let single = AccountInsightQuery {
                        metrics: vec![metric.clone()],
                        ..query.clone()
                    };
                    match self
                        .store
                        .account_insights(credential, account_id, &single)
                        .await
                    {
                        Ok(found) => report.metrics.extend(found),
                        Err(StoreError::Rejected(reason)) => {
                            report.unavailable.push(UnavailableMetric {
                                name: metric.as_str().to_string(),
                                reason,
                            })
                        }
                        Err(other) => return Err(other.into()),
                    }
                }
                Ok(report)
            }
            Err(other) => Err(other.into()),
        }
    }

    pub async fn stories(&self, session: &Session, account: Option<&str>) -> Result<Vec<Media>> {
        let account = session.tenant.account(account)?;
        Ok(self.store.stories(&session.credential, &account.id).await?)
    }

    pub async fn tagged_media(
        &self,
        session: &Session,
        account: Option<&str>,
        page: PageRequest,
    ) -> Result<MediaPage> {
        let account = session.tenant.account(account)?;
        Ok(self
            .store
            .tagged_media(&session.credential, &account.id, &page)
            .await?)
    }

    pub async fn hashtag_media(
        &self,
        session: &Session,
        account: Option<&str>,
        name: &str,
        sort: HashtagSort,
        page: PageRequest,
        caption: Option<&str>,
    ) -> Result<(Hashtag, MediaSearch)> {
        let account = session.tenant.account(account)?;
        let hashtag = self.resolve_hashtag(session, &account.id, name).await?;
        let credential = &session.credential;
        let store = &self.store;
        let (account_id, hashtag_id) = (&account.id, &hashtag.id);
        let found = search(page, caption, |request| async move {
            store
                .hashtag_media(credential, account_id, hashtag_id, sort, &request)
                .await
        })
        .await?;
        Ok((hashtag, found))
    }

    pub async fn hashtag_report(
        &self,
        session: &Session,
        account: Option<&str>,
        name: &str,
        sort: HashtagSort,
        sample: usize,
    ) -> Result<MediaReport> {
        let account = session.tenant.account(account)?;
        let hashtag = self.resolve_hashtag(session, &account.id, name).await?;
        let credential = &session.credential;
        let store = &self.store;
        let (account_id, hashtag_id) = (&account.id, &hashtag.id);
        let media = collect(sample, DISCOVERY_PAGE, |request| async move {
            store
                .hashtag_media(credential, account_id, hashtag_id, sort, &request)
                .await
        })
        .await?;
        Ok(report(None, Some(hashtag), media))
    }

    pub async fn recent_hashtags(
        &self,
        session: &Session,
        account: Option<&str>,
    ) -> Result<Vec<Hashtag>> {
        let account = session.tenant.account(account)?;
        Ok(self
            .store
            .recent_hashtags(&session.credential, &account.id)
            .await?)
    }

    pub async fn discover(
        &self,
        session: &Session,
        account: Option<&str>,
        username: &str,
        page: PageRequest,
    ) -> Result<DiscoveredAccount> {
        let account = session.tenant.account(account)?;
        let username = Username::parse(username)?;
        Ok(self
            .store
            .discover(&session.credential, &account.id, &username, &page)
            .await?)
    }

    pub async fn business_report(
        &self,
        session: &Session,
        account: Option<&str>,
        username: &str,
        sample: usize,
    ) -> Result<MediaReport> {
        let account = session.tenant.account(account)?;
        let username = Username::parse(username)?;
        let credential = &session.credential;
        let mut profile = None;
        let mut media = Vec::new();
        let mut after = None;
        let want = sample.clamp(1, MAX_SAMPLE);
        while media.len() < want {
            let request = PageRequest {
                after: after.take(),
                ..PageRequest::first(DISCOVERY_PAGE.min((want - media.len()) as u32))
            };
            let found = self
                .store
                .discover(credential, &account.id, &username, &request)
                .await?;
            profile.get_or_insert(found.profile);
            media.extend(found.media.media);
            match found.media.next {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        media.truncate(want);
        Ok(report(profile, None, media))
    }

    pub async fn account_report(
        &self,
        session: &Session,
        account: Option<&str>,
        sample: usize,
    ) -> Result<MediaReport> {
        let account = session.tenant.account(account)?;
        let credential = &session.credential;
        let profile = self.store.profile(credential, &account.id).await?;
        let store = &self.store;
        let media = collect(sample, SCAN_PAGE, |request| async move {
            store.media_page(credential, &account.id, &request).await
        })
        .await?;
        Ok(report(Some(profile), None, media))
    }

    async fn resolve_hashtag(
        &self,
        session: &Session,
        account_id: &str,
        name: &str,
    ) -> Result<Hashtag> {
        let name = HashtagName::parse(name)?;
        self.store
            .hashtag(&session.credential, account_id, &name)
            .await?
            .ok_or_else(|| {
                ServiceError::NotFound(format!("#{} does not exist on Instagram.", name.as_str()))
            })
    }
}

fn report(profile: Option<Profile>, hashtag: Option<Hashtag>, media: Vec<Media>) -> MediaReport {
    let followers = profile.as_ref().and_then(|p| p.followers_count);
    let stats = media_stats(&media, followers, hashtag.as_ref().map(|h| h.name.as_str()));
    let top = stats
        .top_media
        .iter()
        .filter_map(|id| media.iter().find(|m| &m.id == id).cloned())
        .collect();
    MediaReport {
        profile,
        hashtag,
        stats,
        top,
    }
}

/// One page, or with a caption filter, pages until `limit` matches are found
/// or `MAX_SCAN` media were read. The cursor continues after the last page
/// read, so every match on it is returned rather than cut at `limit`.
async fn search<F, Fut>(page: PageRequest, caption: Option<&str>, fetch: F) -> Result<MediaSearch>
where
    F: Fn(PageRequest) -> Fut,
    Fut: Future<Output = std::result::Result<MediaPage, StoreError>>,
{
    let Some(query) = caption.map(str::trim).filter(|q| !q.is_empty()) else {
        let found = fetch(page).await?;
        let scanned = found.media.len();
        return Ok(MediaSearch {
            media: found.media,
            next: found.next,
            scanned,
        });
    };
    let want = page.limit as usize;
    let mut request = PageRequest {
        limit: SCAN_PAGE,
        ..page
    };
    let mut out = MediaSearch {
        media: vec![],
        next: None,
        scanned: 0,
    };
    loop {
        let found = fetch(request.clone()).await?;
        out.scanned += found.media.len();
        out.media
            .extend(found.media.into_iter().filter(|m| m.caption_matches(query)));
        out.next = found.next;
        match &out.next {
            Some(next) if out.media.len() < want && out.scanned < MAX_SCAN => {
                request.after = Some(next.clone());
            }
            _ => return Ok(out),
        }
    }
}

/// Up to `sample` media across pages.
async fn collect<F, Fut>(sample: usize, page_size: u32, fetch: F) -> Result<Vec<Media>>
where
    F: Fn(PageRequest) -> Fut,
    Fut: Future<Output = std::result::Result<MediaPage, StoreError>>,
{
    let want = sample.clamp(1, MAX_SAMPLE);
    let mut media = Vec::new();
    let mut request = PageRequest::first(page_size);
    while media.len() < want {
        let found = fetch(request.clone()).await?;
        media.extend(found.media);
        match found.next {
            Some(next) => request.after = Some(next),
            None => break,
        }
    }
    media.truncate(want);
    Ok(media)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::insights::{Metric, MetricName};
    use crate::domain::media::{MediaType, ProductType};
    use crate::domain::tenant::{Credential, Tenant};
    use crate::instagram::store::BoxFut;

    /// Media 1 belongs to account 10; media 2 to someone else.
    #[derive(Default)]
    struct FakeStore {
        insight_calls: Mutex<Vec<usize>>,
    }

    fn media(id: &str, owner: &str, caption: &str) -> Media {
        Media {
            id: id.into(),
            caption: Some(caption.into()),
            media_type: MediaType::Image,
            product_type: Some(ProductType::Feed),
            timestamp: None,
            permalink: None,
            media_url: None,
            thumbnail_url: None,
            like_count: Some(1),
            comments_count: Some(0),
            view_count: None,
            username: None,
            owner_id: Some(owner.into()),
        }
    }

    fn metric(name: &str) -> Metric {
        Metric {
            name: name.into(),
            period: None,
            title: None,
            description: None,
            total: Some(1.0),
            values: vec![],
            breakdowns: vec![],
        }
    }

    impl InstagramStore for FakeStore {
        fn profile<'a>(&'a self, _: &'a Credential, _: &'a str) -> BoxFut<'a, Profile> {
            unimplemented!()
        }
        fn media_page<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a str,
            page: &'a PageRequest,
        ) -> BoxFut<'a, MediaPage> {
            // Three pages of two; one match on each.
            let n: usize = page
                .after
                .as_ref()
                .map_or(0, |c| c.as_str().parse().unwrap());
            Box::pin(async move {
                Ok(MediaPage {
                    media: vec![
                        media(&format!("{n}1"), "10", "pasta night"),
                        media(&format!("{n}2"), "10", "pizza"),
                    ],
                    next: (n < 2).then(|| Cursor::parse(&(n + 1).to_string()).unwrap()),
                })
            })
        }
        fn media<'a>(&'a self, _: &'a Credential, id: &'a ObjectId) -> BoxFut<'a, Media> {
            Box::pin(async move {
                match id.as_str() {
                    "1" => Ok(media("1", "10", "")),
                    "2" => Ok(media("2", "99", "")),
                    _ => Err(StoreError::NotFound),
                }
            })
        }
        fn media_insights<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a ObjectId,
            metrics: &'a [MetricName],
        ) -> BoxFut<'a, Vec<Metric>> {
            self.insight_calls.lock().unwrap().push(metrics.len());
            Box::pin(async move {
                if metrics.iter().any(|m| m.as_str() == "bogus") {
                    return Err(StoreError::Rejected("metric bogus is not supported".into()));
                }
                Ok(metrics.iter().map(|m| metric(m.as_str())).collect())
            })
        }
        fn account_insights<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a str,
            _: &'a AccountInsightQuery,
        ) -> BoxFut<'a, Vec<Metric>> {
            unimplemented!()
        }
        fn stories<'a>(&'a self, _: &'a Credential, _: &'a str) -> BoxFut<'a, Vec<Media>> {
            unimplemented!()
        }
        fn tagged_media<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a str,
            _: &'a PageRequest,
        ) -> BoxFut<'a, MediaPage> {
            unimplemented!()
        }
        fn comments<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a ObjectId,
            _: &'a PageRequest,
        ) -> BoxFut<'a, CommentPage> {
            Box::pin(async {
                Ok(CommentPage {
                    comments: vec![],
                    next: None,
                })
            })
        }
        fn hashtag<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a str,
            _: &'a HashtagName,
        ) -> BoxFut<'a, Option<Hashtag>> {
            unimplemented!()
        }
        fn hashtag_media<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a str,
            _: &'a str,
            _: HashtagSort,
            _: &'a PageRequest,
        ) -> BoxFut<'a, MediaPage> {
            unimplemented!()
        }
        fn recent_hashtags<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a str,
        ) -> BoxFut<'a, Vec<Hashtag>> {
            unimplemented!()
        }
        fn discover<'a>(
            &'a self,
            _: &'a Credential,
            _: &'a str,
            _: &'a Username,
            _: &'a PageRequest,
        ) -> BoxFut<'a, DiscoveredAccount> {
            unimplemented!()
        }
    }

    fn session() -> Session {
        Session {
            tenant: Tenant {
                user_id: "u".into(),
                accounts: vec![IgAccount {
                    id: "10".into(),
                    username: "brand".into(),
                    page_name: "Brand".into(),
                }],
            },
            credential: Credential::new("t".into()),
        }
    }

    fn service() -> (Arc<FakeStore>, InstagramService) {
        let store = Arc::new(FakeStore::default());
        (
            store.clone(),
            InstagramService::new(store, || 1_800_000_000),
        )
    }

    #[tokio::test]
    async fn foreign_media_is_not_found() {
        let (_, svc) = service();
        assert!(svc.media(&session(), "1").await.is_ok());
        for id in ["2", "3"] {
            assert!(matches!(
                svc.media(&session(), id).await,
                Err(ServiceError::NotFound(_))
            ));
        }
        // Comments go through the same check.
        assert!(
            svc.comments(&session(), "2", PageRequest::first(10))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn one_unsupported_metric_does_not_hide_the_rest() {
        let (store, svc) = service();
        let (_, report) = svc
            .media_insights(&session(), "1", &["reach".into(), "bogus".into()])
            .await
            .unwrap();
        assert_eq!(report.metrics.len(), 1);
        assert_eq!(report.unavailable[0].name, "bogus");
        assert_eq!(*store.insight_calls.lock().unwrap(), [2, 1, 1]);
    }

    #[tokio::test]
    async fn caption_search_scans_pages_until_enough_matches() {
        let (_, svc) = service();
        let found = svc
            .media_list(&session(), None, PageRequest::first(2), Some("PASTA"))
            .await
            .unwrap();
        assert_eq!(found.media.len(), 2);
        assert_eq!(found.scanned, 4);
        assert!(found.next.is_some());
    }

    #[tokio::test]
    async fn without_a_filter_one_page_is_returned() {
        let (_, svc) = service();
        let found = svc
            .media_list(&session(), None, PageRequest::first(2), None)
            .await
            .unwrap();
        assert_eq!(found.media.len(), 2);
    }
}
