//! Who a request acts for, and which Instagram accounts it may read.

use serde::{Deserialize, Serialize};

/// The Meta user token a request runs with. Opaque, never logged, and only
/// handed to the Meta adapter. Hence the redacting `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Credential(String);

impl Credential {
    pub fn new(token: String) -> Self {
        Self(token)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

/// An Instagram professional account the user granted access to, through the
/// Facebook Page it is linked to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgAccount {
    pub id: String,
    pub username: String,
    pub page_name: String,
}

/// The identity every data access is scoped to. Only ever built from a
/// validated access token, never from tool arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    /// App-scoped Meta user id.
    pub user_id: String,
    pub accounts: Vec<IgAccount>,
}

/// An authenticated request: who, and with which upstream credential.
#[derive(Debug, Clone)]
pub struct Session {
    pub tenant: Tenant,
    pub credential: Credential,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error(
        "No Instagram professional account is connected. Link one to a Facebook Page and reconnect."
    )]
    NoAccounts,
    #[error("Several accounts are connected ({0}). Name one with the account argument.")]
    Ambiguous(String),
    #[error("@{0} is not one of your connected accounts ({1}).")]
    Unknown(String, String),
}

impl Tenant {
    /// Resolves the `account` tool argument. Without one, the only connected
    /// account is used. Matching is by username, ignoring case and a leading @.
    pub fn account(&self, username: Option<&str>) -> Result<&IgAccount, AccountError> {
        let wanted = username
            .map(|name| name.trim().trim_start_matches('@'))
            .filter(|name| !name.is_empty());
        match (wanted, self.accounts.as_slice()) {
            (_, []) => Err(AccountError::NoAccounts),
            (None, [only]) => Ok(only),
            (None, _) => Err(AccountError::Ambiguous(self.usernames())),
            (Some(name), accounts) => accounts
                .iter()
                .find(|account| account.username.eq_ignore_ascii_case(name))
                .ok_or_else(|| AccountError::Unknown(name.to_string(), self.usernames())),
        }
    }

    /// Whether a media owner id belongs to this tenant.
    pub fn owns(&self, ig_user_id: &str) -> bool {
        self.accounts.iter().any(|account| account.id == ig_user_id)
    }

    fn usernames(&self) -> String {
        self.accounts
            .iter()
            .map(|account| format!("@{}", account.username))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, username: &str) -> IgAccount {
        IgAccount {
            id: id.into(),
            username: username.into(),
            page_name: "Page".into(),
        }
    }

    fn tenant(accounts: Vec<IgAccount>) -> Tenant {
        Tenant {
            user_id: "1".into(),
            accounts,
        }
    }

    #[test]
    fn a_single_account_is_the_default() {
        let t = tenant(vec![account("17841", "brand")]);
        assert_eq!(t.account(None).unwrap().id, "17841");
        assert_eq!(t.account(Some("  ")).unwrap().id, "17841");
    }

    #[test]
    fn several_accounts_need_a_name() {
        let t = tenant(vec![account("1", "brand"), account("2", "Shop")]);
        assert!(matches!(t.account(None), Err(AccountError::Ambiguous(_))));
        assert_eq!(t.account(Some("@shop")).unwrap().id, "2");
    }

    #[test]
    fn foreign_accounts_are_refused() {
        let t = tenant(vec![account("1", "brand")]);
        assert!(matches!(
            t.account(Some("competitor")),
            Err(AccountError::Unknown(..))
        ));
        assert!(!t.owns("2"));
        assert!(t.owns("1"));
    }

    #[test]
    fn no_accounts_is_explained() {
        assert_eq!(tenant(vec![]).account(None), Err(AccountError::NoAccounts));
    }

    #[test]
    fn credentials_never_print() {
        let c = Credential::new("EAAB-secret".into());
        assert!(!format!("{c:?}").contains("secret"));
    }
}
