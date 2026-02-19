use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct RepoName {
    pub organization: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoNameError {
    MissingSeparator,
    EmptyOrganization,
    EmptyRepository,
    TooManySegments,
    InvalidOrganization,
    InvalidRepository,
}

impl fmt::Display for RepoNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepoNameError::MissingSeparator => f.write_str(
                "repository name must include an organization and repo separated by '/'",
            ),
            RepoNameError::EmptyOrganization => {
                f.write_str("repository organization must not be empty")
            }
            RepoNameError::EmptyRepository => f.write_str("repository name must not be empty"),
            RepoNameError::TooManySegments => {
                f.write_str("repository name must use the form 'org/repo'")
            }
            RepoNameError::InvalidOrganization => {
                f.write_str("repository organization must not contain whitespace")
            }
            RepoNameError::InvalidRepository => {
                f.write_str("repository name must not contain whitespace")
            }
        }
    }
}

impl std::error::Error for RepoNameError {}

impl RepoName {
    pub fn new(
        organization: impl Into<String>,
        repo: impl Into<String>,
    ) -> Result<Self, RepoNameError> {
        let organization = organization.into();
        let repo = repo.into();
        validate_segment(&organization, Segment::Organization)?;
        validate_segment(&repo, Segment::Repository)?;

        Ok(Self { organization, repo })
    }

    pub fn as_str(&self) -> String {
        format!("{}/{}", self.organization, self.repo)
    }
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl From<RepoName> for String {
    fn from(value: RepoName) -> Self {
        value.as_str()
    }
}

impl FromStr for RepoName {
    type Err = RepoNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.contains('/') {
            return Err(RepoNameError::MissingSeparator);
        }

        let mut segments = s.split('/');
        let organization = segments
            .next()
            .ok_or(RepoNameError::EmptyOrganization)?
            .to_string();
        let repo = segments
            .next()
            .ok_or(RepoNameError::EmptyRepository)?
            .to_string();

        if segments.next().is_some() {
            return Err(RepoNameError::TooManySegments);
        }

        validate_segment(&organization, Segment::Organization)?;
        validate_segment(&repo, Segment::Repository)?;

        Ok(Self { organization, repo })
    }
}

impl TryFrom<String> for RepoName {
    type Error = RepoNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl Serialize for RepoName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for RepoName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

enum Segment {
    Organization,
    Repository,
}

fn validate_segment(value: &str, segment: Segment) -> Result<(), RepoNameError> {
    if value.is_empty() {
        return match segment {
            Segment::Organization => Err(RepoNameError::EmptyOrganization),
            Segment::Repository => Err(RepoNameError::EmptyRepository),
        };
    }

    if value.chars().any(char::is_whitespace) {
        return match segment {
            Segment::Organization => Err(RepoNameError::InvalidOrganization),
            Segment::Repository => Err(RepoNameError::InvalidRepository),
        };
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_repo_name() {
        let repo_name: RepoName = "dourolabs/metis".parse().unwrap();
        assert_eq!(repo_name.organization, "dourolabs");
        assert_eq!(repo_name.repo, "metis");
        assert_eq!(repo_name.as_str(), "dourolabs/metis");
        assert_eq!(repo_name.to_string(), "dourolabs/metis");
    }

    #[test]
    fn rejects_invalid_segments() {
        assert!(matches!(
            RepoName::from_str(""),
            Err(RepoNameError::MissingSeparator)
        ));
        assert!(matches!(
            RepoName::from_str("dourolabs"),
            Err(RepoNameError::MissingSeparator)
        ));
        assert!(matches!(
            RepoName::from_str("/repo"),
            Err(RepoNameError::EmptyOrganization)
        ));
        assert!(matches!(
            RepoName::from_str("dourolabs/"),
            Err(RepoNameError::EmptyRepository)
        ));
        assert!(matches!(
            RepoName::from_str("dourolabs/metis/core"),
            Err(RepoNameError::TooManySegments)
        ));
        assert!(matches!(
            RepoName::from_str("douro labs/metis"),
            Err(RepoNameError::InvalidOrganization)
        ));
        assert!(matches!(
            RepoName::from_str("dourolabs/metis repo"),
            Err(RepoNameError::InvalidRepository)
        ));
    }

    #[test]
    fn serializes_as_string() {
        let repo_name: RepoName = "dourolabs/metis".parse().unwrap();
        let value = serde_json::to_value(&repo_name).unwrap();
        assert_eq!(value, json!("dourolabs/metis"));
    }
}
