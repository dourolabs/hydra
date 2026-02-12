use async_trait::async_trait;

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};

/// Restricts login to users who are members of at least one allowed GitHub
/// organization.
///
/// When `allowed_orgs` is empty, all users are permitted (the restriction is
/// effectively disabled).
pub struct GithubOrgCheckRestriction {
    allowed_orgs: Vec<String>,
}

impl GithubOrgCheckRestriction {
    pub fn new(params: Option<&toml::Value>) -> Result<Self, String> {
        let allowed_orgs = if let Some(params) = params {
            let table = params
                .as_table()
                .ok_or("github_org_check params must be a table")?;
            table
                .get("allowed_orgs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self { allowed_orgs })
    }

    /// Create with an explicit list of allowed organizations (for use in
    /// server startup when reading from AppConfig rather than TOML policy
    /// config).
    pub fn with_allowed_orgs(allowed_orgs: Vec<String>) -> Self {
        Self { allowed_orgs }
    }
}

#[async_trait]
impl Restriction for GithubOrgCheckRestriction {
    fn name(&self) -> &str {
        "github_org_check"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        if ctx.operation != Operation::Login {
            return Ok(());
        }

        if self.allowed_orgs.is_empty() {
            return Ok(());
        }

        let OperationPayload::Login {
            username,
            github_org_logins,
        } = ctx.payload
        else {
            return Ok(());
        };

        let is_allowed = github_org_logins.iter().any(|org| {
            self.allowed_orgs
                .iter()
                .any(|allowed| org.eq_ignore_ascii_case(allowed))
        });

        if !is_allowed {
            let org_list = self.allowed_orgs.join(", ");
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!(
                    "User {username} is not a member of any allowed organization. \
                     Allowed orgs: {org_list}. Contact your administrator for access."
                ),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::MemoryStore;

    #[tokio::test]
    async fn allows_user_in_allowed_org() {
        let restriction = GithubOrgCheckRestriction::with_allowed_orgs(vec!["my-org".to_string()]);
        let store = MemoryStore::new();

        let payload = OperationPayload::Login {
            username: "alice".to_string(),
            github_org_logins: vec!["my-org".to_string(), "other-org".to_string()],
        };
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_user_not_in_allowed_org() {
        let restriction = GithubOrgCheckRestriction::with_allowed_orgs(vec!["my-org".to_string()]);
        let store = MemoryStore::new();

        let payload = OperationPayload::Login {
            username: "bob".to_string(),
            github_org_logins: vec!["other-org".to_string()],
        };
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "github_org_check");
        assert!(violation.message.contains("bob"));
        assert!(violation.message.contains("my-org"));
        assert!(violation.message.contains("Contact your administrator"));
    }

    #[tokio::test]
    async fn allows_all_when_no_orgs_configured() {
        let restriction = GithubOrgCheckRestriction::with_allowed_orgs(Vec::new());
        let store = MemoryStore::new();

        let payload = OperationPayload::Login {
            username: "anyone".to_string(),
            github_org_logins: vec![],
        };
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn case_insensitive_org_matching() {
        let restriction = GithubOrgCheckRestriction::with_allowed_orgs(vec!["My-Org".to_string()]);
        let store = MemoryStore::new();

        let payload = OperationPayload::Login {
            username: "alice".to_string(),
            github_org_logins: vec!["my-org".to_string()],
        };
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn skips_non_login_operations() {
        let restriction = GithubOrgCheckRestriction::with_allowed_orgs(vec!["my-org".to_string()]);
        let store = MemoryStore::new();

        let patch = crate::domain::patches::Patch::new(
            "test".to_string(),
            "desc".to_string(),
            String::new(),
            crate::domain::patches::PatchStatus::Open,
            false,
            None,
            Vec::new(),
            metis_common::RepoName::new("test", "repo").unwrap(),
            None,
        );

        let payload = OperationPayload::Patch {
            patch_id: None,
            new: patch,
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreatePatch,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_user_with_no_orgs() {
        let restriction = GithubOrgCheckRestriction::with_allowed_orgs(vec!["my-org".to_string()]);
        let store = MemoryStore::new();

        let payload = OperationPayload::Login {
            username: "loner".to_string(),
            github_org_logins: vec![],
        };
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert!(violation.message.contains("loner"));
    }

    #[tokio::test]
    async fn from_toml_params() {
        let toml_str = r#"allowed_orgs = ["acme-corp", "widgets-inc"]"#;
        let value: toml::Value = toml_str.parse().unwrap();
        let restriction = GithubOrgCheckRestriction::new(Some(&value)).unwrap();

        let store = MemoryStore::new();
        let payload = OperationPayload::Login {
            username: "dev".to_string(),
            github_org_logins: vec!["acme-corp".to_string()],
        };
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }
}
