use async_trait::async_trait;

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};

/// Restricts login to users who belong to at least one of the configured
/// GitHub organizations. When `allowed_orgs` is empty the restriction is a
/// no-op, preserving the previous open-access behaviour.
pub struct GithubOrgCheckRestriction {
    allowed_orgs: Vec<String>,
}

impl GithubOrgCheckRestriction {
    pub fn new(params: Option<&toml::Value>) -> Result<Self, String> {
        let allowed_orgs = if let Some(params) = params {
            let table = params
                .as_table()
                .ok_or("github_org_check params must be a table")?;
            if let Some(orgs) = table.get("allowed_orgs") {
                let arr = orgs.as_array().ok_or("allowed_orgs must be an array")?;
                let mut result = Vec::new();
                for v in arr {
                    let s = v.as_str().ok_or("allowed_orgs entries must be strings")?;
                    result.push(s.to_string());
                }
                result
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        Ok(Self { allowed_orgs })
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
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!(
                    "GitHub user '{username}' is not a member of any allowed organization. \
                     Allowed organizations: {}.",
                    self.allowed_orgs.join(", ")
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

    fn restriction_with_orgs(orgs: Vec<&str>) -> GithubOrgCheckRestriction {
        let mut table = toml::map::Map::new();
        let arr: Vec<toml::Value> = orgs
            .into_iter()
            .map(|s| toml::Value::String(s.to_string()))
            .collect();
        table.insert("allowed_orgs".to_string(), toml::Value::Array(arr));
        GithubOrgCheckRestriction::new(Some(&toml::Value::Table(table))).unwrap()
    }

    fn login_payload(username: &str, orgs: Vec<&str>) -> OperationPayload {
        OperationPayload::Login {
            username: username.to_string(),
            github_org_logins: orgs.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn allows_login_when_no_orgs_configured() {
        let restriction = GithubOrgCheckRestriction::new(None).unwrap();
        let store = MemoryStore::new();
        let payload = login_payload("alice", vec!["some-org"]);
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn allows_login_when_user_in_allowed_org() {
        let restriction = restriction_with_orgs(vec!["dourolabs", "other-org"]);
        let store = MemoryStore::new();
        let payload = login_payload("alice", vec!["dourolabs"]);
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn allows_login_case_insensitive() {
        let restriction = restriction_with_orgs(vec!["DouroLabs"]);
        let store = MemoryStore::new();
        let payload = login_payload("alice", vec!["dourolabs"]);
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_login_when_user_not_in_allowed_org() {
        let restriction = restriction_with_orgs(vec!["dourolabs"]);
        let store = MemoryStore::new();
        let payload = login_payload("bob", vec!["other-org"]);
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
        assert!(violation.message.contains("dourolabs"));
    }

    #[tokio::test]
    async fn rejects_login_when_user_has_no_orgs() {
        let restriction = restriction_with_orgs(vec!["dourolabs"]);
        let store = MemoryStore::new();
        let payload = login_payload("bob", vec![]);
        let ctx = RestrictionContext {
            operation: Operation::Login,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_err());
    }

    #[tokio::test]
    async fn ignores_non_login_operations() {
        let restriction = restriction_with_orgs(vec!["dourolabs"]);
        let store = MemoryStore::new();
        let payload = OperationPayload::Issue {
            issue_id: None,
            new: crate::domain::issues::Issue::new(
                crate::domain::issues::IssueType::Task,
                "test".to_string(),
                crate::domain::users::Username::from("alice"),
                String::new(),
                crate::domain::issues::IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }
}
