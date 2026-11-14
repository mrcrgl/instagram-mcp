//! `IdentityProvider` over Facebook Login for Business.

use url::Url;

use crate::domain::tenant::{Credential, IgAccount};
use crate::instagram::store::StoreError;
use crate::meta::api::GraphApi;
use crate::meta::mapping;
use crate::meta::models::{PageRow, Paged, TokenRow, UserRow};
use crate::oauth::identity::{BoxFut, GrantedToken, IdentityError, IdentityProvider, IdentityUser};

/// Asked for when no Facebook Login for Business configuration is set.
/// Read-only, except that Meta bundles reading comments into
/// `instagram_manage_comments`.
pub const DEFAULT_SCOPES: &str = "instagram_basic,instagram_manage_insights,instagram_manage_comments,pages_show_list,pages_read_engagement,business_management";
/// More Pages than anyone manages by hand; bounds the sign-in's requests.
const MAX_ACCOUNT_PAGES: usize = 5;

/// Holds the app secret, hence no `Debug`.
pub struct MetaIdentity {
    api: GraphApi,
    dialog_url: String,
    app_id: String,
    app_secret: String,
    /// Facebook Login for Business configuration; replaces `scope` when set.
    login_config_id: Option<String>,
    clock: fn() -> u64,
}

impl MetaIdentity {
    pub fn new(
        api: GraphApi,
        web_url: &str,
        version: &str,
        app_id: String,
        app_secret: String,
        login_config_id: Option<String>,
        clock: fn() -> u64,
    ) -> Self {
        Self {
            api,
            dialog_url: format!("{}/{version}/dialog/oauth", web_url.trim_end_matches('/')),
            app_id,
            app_secret,
            login_config_id,
            clock,
        }
    }

    fn granted(&self, row: TokenRow) -> GrantedToken {
        GrantedToken {
            credential: Credential::new(row.access_token),
            expires_at: row.expires_in.map(|seconds| (self.clock)() + seconds),
        }
    }

    async fn long_lived(&self, short_lived: &str) -> Result<GrantedToken, IdentityError> {
        let row: TokenRow = self
            .api
            .get_public(
                &["oauth", "access_token"],
                &[
                    ("grant_type", "fb_exchange_token"),
                    ("client_id", &self.app_id),
                    ("client_secret", &self.app_secret),
                    ("fb_exchange_token", short_lived),
                ],
            )
            .await
            .map_err(identity_error)?;
        Ok(self.granted(row))
    }
}

fn identity_error(error: StoreError) -> IdentityError {
    match error {
        StoreError::RateLimited(reason) | StoreError::Unavailable(reason) => {
            IdentityError::Unavailable(reason)
        }
        _ => IdentityError::Rejected,
    }
}

impl IdentityProvider for MetaIdentity {
    fn authorize_url(&self, state: &str, redirect_uri: &str) -> String {
        let mut url = Url::parse(&self.dialog_url).expect("dialog URL is built from a valid URL");
        url.query_pairs_mut()
            .append_pair("client_id", &self.app_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("response_type", "code");
        match &self.login_config_id {
            Some(config_id) => url.query_pairs_mut().append_pair("config_id", config_id),
            None => url.query_pairs_mut().append_pair("scope", DEFAULT_SCOPES),
        };
        url.into()
    }

    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        redirect_uri: &'a str,
    ) -> BoxFut<'a, GrantedToken> {
        Box::pin(async move {
            let short: TokenRow = self
                .api
                .get_public(
                    &["oauth", "access_token"],
                    &[
                        ("client_id", &self.app_id),
                        ("client_secret", &self.app_secret),
                        ("redirect_uri", redirect_uri),
                        ("code", code),
                    ],
                )
                .await
                .map_err(identity_error)?;
            self.long_lived(&short.access_token).await
        })
    }

    fn extend<'a>(&'a self, credential: &'a Credential) -> BoxFut<'a, GrantedToken> {
        Box::pin(async move { self.long_lived(credential.expose()).await })
    }

    fn user<'a>(&'a self, credential: &'a Credential) -> BoxFut<'a, IdentityUser> {
        Box::pin(async move {
            let row: UserRow = self
                .api
                .get(credential, &["me"], &[("fields", "id".into())])
                .await
                .map_err(identity_error)?;
            Ok(IdentityUser { id: row.id })
        })
    }

    fn accounts<'a>(&'a self, credential: &'a Credential) -> BoxFut<'a, Vec<IgAccount>> {
        Box::pin(async move {
            let mut pages: Vec<PageRow> = Vec::new();
            let mut after: Option<String> = None;
            for _ in 0..MAX_ACCOUNT_PAGES {
                let mut query = vec![
                    (
                        "fields",
                        "name,instagram_business_account{id,username}".to_string(),
                    ),
                    ("limit", "100".to_string()),
                ];
                if let Some(cursor) = &after {
                    query.push(("after", cursor.clone()));
                }
                let page: Paged<PageRow> = self
                    .api
                    .get(credential, &["me", "accounts"], &query)
                    .await
                    .map_err(identity_error)?;
                pages.extend(page.data);
                match mapping::next_cursor(page.paging.as_ref()) {
                    Some(next) => after = Some(next.as_str().to_string()),
                    None => break,
                }
            }
            Ok(mapping::accounts(pages))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(config: Option<&str>) -> MetaIdentity {
        MetaIdentity::new(
            GraphApi::new("https://graph.example", "v26.0", "s".into()),
            "https://www.facebook.example/",
            "v26.0",
            "123".into(),
            "s".into(),
            config.map(String::from),
            || 0,
        )
    }

    #[test]
    fn the_dialog_asks_for_scopes_or_a_configuration() {
        let plain = identity(None).authorize_url("st", "https://mcp.example/cb");
        assert!(plain.starts_with("https://www.facebook.example/v26.0/dialog/oauth?client_id=123"));
        assert!(plain.contains("scope=instagram_basic"));
        let configured = identity(Some("987")).authorize_url("st", "https://mcp.example/cb");
        assert!(configured.contains("config_id=987"));
        assert!(!configured.contains("scope="));
    }
}
