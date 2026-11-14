//! Tool definitions. All are read-only. The connected accounts a tool may act
//! as come from the caller's token; `account` only picks one of them.

use axum::http::request::Parts;
use rmcp::{
    ErrorData as McpError,
    handler::server::{tool::Extension, wrapper::Parameters},
    model::CallToolResult,
    tool, tool_router,
};

use crate::domain::insights::{MetricType, Period};
use crate::instagram::service::{AccountInsightRequest, ServiceError};
use crate::mcp::dto::{
    AccountArgs, AccountDto, AccountInsightsArgs, AccountReportArgs, BusinessArgs,
    BusinessReportArgs, CommentPageDto, CommentsArgs, DemographicsArgs, DiscoveredDto, HashtagDto,
    HashtagMediaArgs, HashtagMediaDto, HashtagReportArgs, InsightsDto, MediaDto, MediaIdArgs,
    MediaInsightsArgs, MediaListArgs, MediaPageDto, PageArgs, ProfileDto, ReportDto, day_end,
    day_start, max_discovery_page, max_page, page_request, sample,
};
use crate::mcp::server::{InstagramMcp, session, tool_result};

type ToolResult = Result<CallToolResult, McpError>;

fn non_empty(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

#[tool_router(router = own_router, vis = "pub")]
impl InstagramMcp {
    #[tool(
        name = "accounts_list",
        description = "List the Instagram professional accounts connected to this server, with follower and media counts. Use a username from here as `account` in other tools.",
        annotations(read_only_hint = true)
    )]
    pub async fn accounts_list(&self, Extension(parts): Extension<Parts>) -> ToolResult {
        let session = session(&parts)?;
        let accounts: Vec<AccountDto> = self
            .instagram
            .accounts(session)
            .await
            .into_iter()
            .map(Into::into)
            .collect();
        tool_result("accounts_list", Ok(accounts))
    }

    #[tool(
        name = "account_profile",
        description = "Profile of a connected account: name, biography, website, followers, following, media count.",
        annotations(read_only_hint = true)
    )]
    pub async fn account_profile(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<AccountArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = self
            .instagram
            .profile(session, args.account.as_deref())
            .await
            .map(ProfileDto::from);
        tool_result("account_profile", result)
    }

    #[tool(
        name = "media_list",
        description = "List a connected account's posts and reels, newest first, with likes and comments. Filter by date, or by caption keywords with caption_contains.",
        annotations(read_only_hint = true)
    )]
    pub async fn media_list(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<MediaListArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = async {
            let page = args.page()?;
            let filtered = non_empty(args.caption_contains.as_deref());
            let found = self
                .instagram
                .media_list(
                    session,
                    args.account.as_deref(),
                    page,
                    args.caption_contains.as_deref(),
                )
                .await?;
            Ok::<_, ServiceError>(MediaPageDto::from_search(found, filtered))
        }
        .await;
        tool_result("media_list", result)
    }

    #[tool(
        name = "media_get",
        description = "Read one media item of a connected account: caption, kind, likes, comments, links.",
        annotations(read_only_hint = true)
    )]
    pub async fn media_get(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<MediaIdArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = self
            .instagram
            .media(session, &args.media_id)
            .await
            .map(MediaDto::from);
        tool_result("media_get", result)
    }

    #[tool(
        name = "media_insights",
        description = "Performance of one post, reel, or story of a connected account: views, reach, likes, comments, shares, saves, watch time and more. Story insights exist only for 24 hours after posting.",
        annotations(read_only_hint = true)
    )]
    pub async fn media_insights(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<MediaInsightsArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let metrics = args.metrics.unwrap_or_default();
        let result = self
            .instagram
            .media_insights(session, &args.media_id, &metrics)
            .await
            .map(|(media, report)| InsightsDto::new(Some(media), report));
        tool_result("media_insights", result)
    }

    #[tool(
        name = "comments_list",
        description = "Comments on a media item of a connected account, newest first.",
        annotations(read_only_hint = true)
    )]
    pub async fn comments_list(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<CommentsArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = async {
            let page = page_request(args.limit, args.after.as_deref(), max_page())?;
            let found = self
                .instagram
                .comments(session, &args.media_id, page)
                .await?;
            Ok::<_, ServiceError>(CommentPageDto::from(found))
        }
        .await;
        tool_result("comments_list", result)
    }

    #[tool(
        name = "stories_list",
        description = "A connected account's stories that are live right now (last 24 hours).",
        annotations(read_only_hint = true)
    )]
    pub async fn stories_list(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<AccountArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = self
            .instagram
            .stories(session, args.account.as_deref())
            .await
            .map(|stories| stories.into_iter().map(MediaDto::from).collect::<Vec<_>>());
        tool_result("stories_list", result)
    }

    #[tool(
        name = "tagged_media",
        description = "Media by other accounts in which a connected account is tagged.",
        annotations(read_only_hint = true)
    )]
    pub async fn tagged_media(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<PageArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = async {
            let page = page_request(args.limit, args.after.as_deref(), max_page())?;
            let found = self
                .instagram
                .tagged_media(session, args.account.as_deref(), page)
                .await?;
            Ok::<_, ServiceError>(MediaPageDto::from(found))
        }
        .await;
        tool_result("tagged_media", result)
    }

    #[tool(
        name = "account_insights",
        description = "Account-level analytics for a date range of up to 30 days within the last 2 years: views, reach, engagement, interactions, follows and unfollows, profile link taps. Use metric_type time_series with reach or follower_count for daily values.",
        annotations(read_only_hint = true)
    )]
    pub async fn account_insights(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<AccountInsightsArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = async {
            let request = AccountInsightRequest {
                metrics: args.metrics.unwrap_or_default(),
                period: args.period.map_or(Period::Day, Into::into),
                metric_type: args.metric_type.map_or(MetricType::TotalValue, Into::into),
                breakdown: args.breakdown.map(Into::into),
                since: args.since.as_deref().map(day_start).transpose()?,
                until: args.until.as_deref().map(day_end).transpose()?,
            };
            let report = self
                .instagram
                .account_insights(session, args.account.as_deref(), request)
                .await?;
            Ok::<_, ServiceError>(InsightsDto::new(None, report))
        }
        .await;
        tool_result("account_insights", result)
    }

    #[tool(
        name = "audience_demographics",
        description = "Who follows or engages with a connected account, by age, gender, country, or city. Needs at least 100 followers or engaged accounts.",
        annotations(read_only_hint = true)
    )]
    pub async fn audience_demographics(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<DemographicsArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let (account, audience, breakdown, timeframe) = args.into_parts();
        let result = self
            .instagram
            .demographics(session, account.as_deref(), audience, breakdown, timeframe)
            .await
            .map(|report| InsightsDto::new(None, report));
        tool_result("audience_demographics", result)
    }

    #[tool(
        name = "account_report",
        description = "Content analytics for a connected account over its recent media: engagement per post and rate, posting frequency, best formats, weekdays and hours (UTC), most used hashtags, and top posts.",
        annotations(read_only_hint = true)
    )]
    pub async fn account_report(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<AccountReportArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = self
            .instagram
            .account_report(session, args.account.as_deref(), sample(args.sample_size))
            .await
            .map(ReportDto::from);
        tool_result("account_report", result)
    }
}

#[tool_router(router = public_router, vis = "pub")]
impl InstagramMcp {
    #[tool(
        name = "hashtag_media",
        description = "Public posts for a hashtag: top (most popular) or recent (last 24 hours). Optionally filtered by caption keywords. Each looked-up hashtag counts towards 30 distinct hashtags per account per 7 days.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn hashtag_media(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<HashtagMediaArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = async {
            let page = page_request(args.limit, args.after.as_deref(), max_discovery_page())?;
            let filtered = non_empty(args.caption_contains.as_deref());
            let (hashtag, found) = self
                .instagram
                .hashtag_media(
                    session,
                    args.account.as_deref(),
                    &args.hashtag,
                    args.sort.unwrap_or_default().into(),
                    page,
                    args.caption_contains.as_deref(),
                )
                .await?;
            Ok::<_, ServiceError>(HashtagMediaDto {
                hashtag: hashtag.into(),
                page: MediaPageDto::from_search(found, filtered),
            })
        }
        .await;
        tool_result("hashtag_media", result)
    }

    #[tool(
        name = "hashtag_report",
        description = "Analyze a hashtag over a sample of its top or recent posts: typical likes and comments, formats, posting times (UTC), related hashtags used alongside it, and top posts. Counts as one hashtag lookup.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn hashtag_report(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<HashtagReportArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = self
            .instagram
            .hashtag_report(
                session,
                args.account.as_deref(),
                &args.hashtag,
                args.sort.unwrap_or_default().into(),
                sample(args.sample_size),
            )
            .await
            .map(ReportDto::from);
        tool_result("hashtag_report", result)
    }

    #[tool(
        name = "hashtags_searched",
        description = "Hashtags a connected account looked up in the last 7 days. Reusing these does not spend the 30-per-week hashtag budget.",
        annotations(read_only_hint = true)
    )]
    pub async fn hashtags_searched(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<AccountArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = self
            .instagram
            .recent_hashtags(session, args.account.as_deref())
            .await
            .map(|tags| tags.into_iter().map(HashtagDto::from).collect::<Vec<_>>());
        tool_result("hashtags_searched", result)
    }

    #[tool(
        name = "business_profile",
        description = "Public profile and recent posts of another Instagram business or creator account, e.g. a competitor: followers, media count, biography, and per-post likes, comments, and reel views.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn business_profile(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<BusinessArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = async {
            let page = page_request(
                args.media_limit,
                args.after.as_deref(),
                max_discovery_page(),
            )?;
            let found = self
                .instagram
                .discover(session, args.account.as_deref(), &args.username, page)
                .await?;
            Ok::<_, ServiceError>(DiscoveredDto::from(found))
        }
        .await;
        tool_result("business_profile", result)
    }

    #[tool(
        name = "business_report",
        description = "Benchmark another business or creator account over its recent posts: engagement per post and rate, posting frequency, formats, timing (UTC), hashtags, and top posts. Compare with account_report.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn business_report(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(args): Parameters<BusinessReportArgs>,
    ) -> ToolResult {
        let session = session(&parts)?;
        let result = self
            .instagram
            .business_report(
                session,
                args.account.as_deref(),
                &args.username,
                sample(args.sample_size),
            )
            .await
            .map(ReportDto::from);
        tool_result("business_report", result)
    }
}
