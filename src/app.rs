//! Composition: builds the object graph and the HTTP application.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, middleware, routing::get};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing::info_span;

use crate::config::Config;
use crate::instagram::service::InstagramService;
use crate::mcp::server::InstagramMcp;
use crate::meta::api::GraphApi;
use crate::meta::identity::MetaIdentity;
use crate::meta::store::MetaInstagramStore;
use crate::oauth::seal::Sealer;
use crate::oauth::service::OAuthService;
use crate::oauth::{bearer, pages, routes};

pub struct Services {
    pub oauth: Arc<OAuthService>,
    pub instagram: Arc<InstagramService>,
}

/// Wires the Meta adapters into the services.
pub fn build_services(cfg: &Config) -> Result<Services, String> {
    let api = GraphApi::new(
        &cfg.meta_graph_url,
        &cfg.meta_graph_version,
        cfg.meta_app_secret.clone(),
    );
    let identity = Arc::new(MetaIdentity::new(
        api.clone(),
        &cfg.meta_web_url,
        &cfg.meta_graph_version,
        cfg.meta_app_id.clone(),
        cfg.meta_app_secret.clone(),
        cfg.meta_login_config_id.clone(),
        unix_now,
    ));
    let store = Arc::new(MetaInstagramStore::new(api));
    let oauth = Arc::new(OAuthService::new(
        Sealer::new(&cfg.token_keys)?,
        identity,
        cfg.public_base_url.clone(),
        unix_now,
    ));
    Ok(Services {
        oauth,
        instagram: Arc::new(InstagramService::new(store, || unix_now() as i64)),
    })
}

/// The whole HTTP surface. `shutdown` drains in-flight MCP streams.
pub fn router(cfg: &Config, services: Services, shutdown: CancellationToken) -> Router {
    let Services { oauth, instagram } = services;

    // No sessions: the token is checked on every request, and any replica can
    // serve any request.
    let mcp_service = StreamableHttpService::new(
        move || Ok(InstagramMcp::new(instagram.clone())),
        NeverSessionManager::default().into(),
        {
            // with_allowed_hosts replaces the default list, so keep the
            // localhost entries for local development and probes.
            let mut hosts = vec!["localhost".to_string(), "127.0.0.1".into(), "::1".into()];
            hosts.extend(cfg.allowed_hosts.iter().cloned());
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true)
                .with_cancellation_token(shutdown)
                .with_allowed_hosts(hosts)
        },
    );

    let mcp = Router::new()
        .nest_service("/mcp", mcp_service)
        .route_layer(middleware::from_fn_with_state(
            oauth.clone(),
            bearer::require_session,
        ))
        .layer(routes::cors());

    Router::new()
        .route("/", get(landing).with_state(oauth.clone()))
        .route("/healthz", get(|| async { "ok\n" }))
        .merge(routes::router(oauth))
        .merge(mcp)
        .layer(
            // Logs the path only: the query of the flow's URLs carries codes
            // and sealed state.
            TraceLayer::new_for_http().make_span_with(|request: &Request| {
                info_span!(
                    "http",
                    method = %request.method(),
                    path = request.uri().path(),
                    request_id = %uuid::Uuid::new_v4(),
                )
            }),
        )
}

async fn landing(State(oauth): State<Arc<OAuthService>>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        pages::landing(&oauth.resource()),
    )
        .into_response()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs()
}
