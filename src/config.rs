//! Process configuration. Deliberately not `Debug`: it holds secrets.

use url::Url;

const DEFAULT_GRAPH_URL: &str = "https://graph.facebook.com";
const DEFAULT_WEB_URL: &str = "https://www.facebook.com";
/// Meta supports each Graph API version for about two years. Pin it, and
/// move it forward deliberately: metrics change between versions.
const DEFAULT_GRAPH_VERSION: &str = "v26.0";

pub struct Config {
    pub port: u16,
    /// Externally visible origin, without a trailing slash. Issuer, resource
    /// identifier, and redirect URIs are all derived from it.
    pub public_base_url: String,
    /// Hostnames accepted in the `Host` header by the MCP transport.
    pub allowed_hosts: Vec<String>,
    pub meta_app_id: String,
    pub meta_app_secret: String,
    /// Facebook Login for Business configuration id. Without it, the dialog
    /// asks for the default scopes.
    pub meta_login_config_id: Option<String>,
    pub meta_graph_version: String,
    pub meta_graph_url: String,
    pub meta_web_url: String,
    /// Base64url-encoded 32-byte keys. The first seals; all of them open.
    pub token_keys: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(&|k| std::env::var(k).ok())
    }

    /// Env access goes through a lookup fn so tests don't mutate process env.
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let port = match get("PORT") {
            Some(p) => p
                .parse::<u16>()
                .map_err(|_| format!("PORT is not a valid port: {p}"))?,
            None => 10080,
        };
        let optional = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let require =
            |k: &str, hint: &str| optional(k).ok_or_else(|| format!("{k} must be set ({hint})"));

        let public_base_url = normalize_base_url(&require(
            "PUBLIC_BASE_URL",
            "the externally visible origin, e.g. https://instagram-mcp.example.com",
        )?)?;
        let allowed_hosts = Url::parse(&public_base_url)
            .ok()
            .and_then(|url| {
                url.host_str().map(|host| match url.port() {
                    Some(port) => vec![host.to_string(), format!("{host}:{port}")],
                    None => vec![host.to_string()],
                })
            })
            .ok_or("PUBLIC_BASE_URL has no host")?;

        let token_keys: Vec<String> = require(
            "TOKEN_KEYS",
            "comma-separated base64url 32-byte keys, newest first",
        )?
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(String::from)
        .collect();

        let meta_graph_version =
            optional("META_GRAPH_VERSION").unwrap_or_else(|| DEFAULT_GRAPH_VERSION.into());
        let valid_version = meta_graph_version
            .strip_prefix('v')
            .and_then(|rest| rest.split_once('.'))
            .is_some_and(|(major, minor)| {
                !major.is_empty() && major.bytes().all(|b| b.is_ascii_digit()) && minor == "0"
            });
        if !valid_version {
            return Err(format!(
                "META_GRAPH_VERSION must look like v26.0, got {meta_graph_version}"
            ));
        }

        let meta_app_id = require("META_APP_ID", "the Meta app's numeric id")?;
        if !meta_app_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err("META_APP_ID must be numeric".into());
        }

        Ok(Self {
            port,
            public_base_url,
            allowed_hosts,
            meta_app_id,
            meta_app_secret: require("META_APP_SECRET", "the Meta app secret")?,
            meta_login_config_id: optional("META_LOGIN_CONFIG_ID"),
            meta_graph_version,
            meta_graph_url: optional("META_GRAPH_URL")
                .unwrap_or_else(|| DEFAULT_GRAPH_URL.into())
                .trim_end_matches('/')
                .to_string(),
            meta_web_url: optional("META_WEB_URL")
                .unwrap_or_else(|| DEFAULT_WEB_URL.into())
                .trim_end_matches('/')
                .to_string(),
            token_keys,
        })
    }
}

fn normalize_base_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|e| format!("PUBLIC_BASE_URL is not a URL: {e}"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("PUBLIC_BASE_URL must use https outside of localhost".into());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("PUBLIC_BASE_URL must be an origin without a path".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &[(&str, &str)] = &[
        ("PUBLIC_BASE_URL", "https://ig.example.com/"),
        ("META_APP_ID", "12345"),
        ("META_APP_SECRET", "secret"),
        ("TOKEN_KEYS", "new, old"),
    ];

    fn config(pairs: &[(&str, &str)]) -> Result<Config, String> {
        Config::from_lookup(&|k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.to_string())
        })
    }

    fn with(key: &'static str, value: &'static str) -> Vec<(&'static str, &'static str)> {
        let mut pairs: Vec<_> = BASE.iter().copied().filter(|(k, _)| *k != key).collect();
        pairs.push((key, value));
        pairs
    }

    #[test]
    fn defaults_point_at_meta_and_derive_the_host() {
        let cfg = config(BASE).unwrap();
        assert_eq!(cfg.public_base_url, "https://ig.example.com");
        assert_eq!(cfg.allowed_hosts, ["ig.example.com"]);
        assert_eq!(cfg.meta_graph_url, "https://graph.facebook.com");
        assert_eq!(cfg.meta_graph_version, DEFAULT_GRAPH_VERSION);
        assert!(cfg.meta_login_config_id.is_none());
        assert_eq!(cfg.token_keys, ["new", "old"]);
        assert_eq!(cfg.port, 10080);
    }

    #[test]
    fn missing_settings_name_the_variable() {
        let pairs: Vec<_> = BASE
            .iter()
            .copied()
            .filter(|(k, _)| *k != "META_APP_SECRET")
            .collect();
        assert!(config(&pairs).err().unwrap().contains("META_APP_SECRET"));
    }

    #[test]
    fn plain_http_is_only_accepted_on_loopback() {
        assert!(config(&with("PUBLIC_BASE_URL", "http://ig.example.com")).is_err());
        let local = config(&with("PUBLIC_BASE_URL", "http://localhost:10080")).unwrap();
        assert_eq!(local.allowed_hosts, ["localhost", "localhost:10080"]);
    }

    #[test]
    fn graph_version_and_app_id_are_checked() {
        assert!(config(&with("META_GRAPH_VERSION", "26")).is_err());
        assert!(config(&with("META_GRAPH_VERSION", "v25.0")).is_ok());
        assert!(config(&with("META_APP_ID", "abc")).is_err());
    }

    #[test]
    fn an_empty_login_config_counts_as_unset() {
        assert!(
            config(&with("META_LOGIN_CONFIG_ID", " "))
                .unwrap()
                .meta_login_config_id
                .is_none()
        );
    }
}
