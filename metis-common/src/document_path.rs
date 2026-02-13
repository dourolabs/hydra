use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, ops::Deref, str::FromStr};

/// A validated document path that enforces structural constraints:
/// - Must not be empty
/// - Must start with `/` (normalized on construction)
/// - No empty segments (e.g. `foo//bar`)
/// - No hidden segments (path components starting with `.`)
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentPath(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentPathError {
    Empty,
    EmptySegment,
    HiddenSegment(String),
}

impl fmt::Display for DocumentPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentPathError::Empty => f.write_str("document path must not be empty"),
            DocumentPathError::EmptySegment => {
                f.write_str("document path must not contain empty segments")
            }
            DocumentPathError::HiddenSegment(segment) => {
                write!(
                    f,
                    "document path contains a hidden segment \"{segment}\"; \
                     path components starting with '.' are not allowed"
                )
            }
        }
    }
}

impl std::error::Error for DocumentPathError {}

impl DocumentPath {
    /// Returns the path as a string slice (always starts with `/`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DocumentPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for DocumentPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for DocumentPath {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<DocumentPath> for String {
    fn from(value: DocumentPath) -> Self {
        value.0
    }
}

impl FromStr for DocumentPath {
    type Err = DocumentPathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize: ensure leading slash
        let canonical = if s.starts_with('/') {
            s.to_string()
        } else {
            format!("/{s}")
        };

        // After normalization the path is at least "/", so split after the leading slash
        // to get the meaningful segments.
        let after_slash = &canonical[1..];

        if after_slash.is_empty() {
            return Err(DocumentPathError::Empty);
        }

        for segment in after_slash.split('/') {
            if segment.is_empty() {
                return Err(DocumentPathError::EmptySegment);
            }
            if segment.starts_with('.') {
                return Err(DocumentPathError::HiddenSegment(segment.to_string()));
            }
        }

        Ok(Self(canonical))
    }
}

impl TryFrom<String> for DocumentPath {
    type Error = DocumentPathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl Serialize for DocumentPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DocumentPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_path_with_leading_slash() {
        let path: DocumentPath = "/designs/policy-engine.md".parse().unwrap();
        assert_eq!(path.as_str(), "/designs/policy-engine.md");
    }

    #[test]
    fn normalizes_path_without_leading_slash() {
        let path: DocumentPath = "designs/policy-engine.md".parse().unwrap();
        assert_eq!(path.as_str(), "/designs/policy-engine.md");
    }

    #[test]
    fn rejects_empty_string() {
        assert_eq!(DocumentPath::from_str(""), Err(DocumentPathError::Empty));
    }

    #[test]
    fn rejects_slash_only() {
        assert_eq!(DocumentPath::from_str("/"), Err(DocumentPathError::Empty));
    }

    #[test]
    fn rejects_hidden_segment_at_start() {
        let err = DocumentPath::from_str(".hidden/file.md").unwrap_err();
        assert_eq!(err, DocumentPathError::HiddenSegment(".hidden".to_string()));
    }

    #[test]
    fn rejects_hidden_segment_in_middle() {
        let err = DocumentPath::from_str("dir/.hidden/file.md").unwrap_err();
        assert_eq!(err, DocumentPathError::HiddenSegment(".hidden".to_string()));
    }

    #[test]
    fn rejects_hidden_segment_with_leading_slash() {
        let err = DocumentPath::from_str("/.git/config").unwrap_err();
        assert_eq!(err, DocumentPathError::HiddenSegment(".git".to_string()));
    }

    #[test]
    fn rejects_dot_only_segment() {
        let err = DocumentPath::from_str("dir/./file.md").unwrap_err();
        assert_eq!(err, DocumentPathError::HiddenSegment(".".to_string()));
    }

    #[test]
    fn rejects_dotdot_segment() {
        let err = DocumentPath::from_str("dir/../file.md").unwrap_err();
        assert_eq!(err, DocumentPathError::HiddenSegment("..".to_string()));
    }

    #[test]
    fn rejects_empty_segment_double_slash() {
        assert_eq!(
            DocumentPath::from_str("foo//bar"),
            Err(DocumentPathError::EmptySegment)
        );
    }

    #[test]
    fn rejects_trailing_slash() {
        assert_eq!(
            DocumentPath::from_str("docs/"),
            Err(DocumentPathError::EmptySegment)
        );
    }

    #[test]
    fn accepts_simple_filename() {
        let path: DocumentPath = "readme.md".parse().unwrap();
        assert_eq!(path.as_str(), "/readme.md");
    }

    #[test]
    fn accepts_nested_path() {
        let path: DocumentPath = "/a/b/c/d.txt".parse().unwrap();
        assert_eq!(path.as_str(), "/a/b/c/d.txt");
    }

    #[test]
    fn accepts_file_with_dots_in_name() {
        let path: DocumentPath = "docs/file.name.with.dots.md".parse().unwrap();
        assert_eq!(path.as_str(), "/docs/file.name.with.dots.md");
    }

    #[test]
    fn display_matches_as_str() {
        let path: DocumentPath = "/docs/file.md".parse().unwrap();
        assert_eq!(path.to_string(), path.as_str());
    }

    #[test]
    fn into_string_returns_canonical_form() {
        let path: DocumentPath = "docs/file.md".parse().unwrap();
        let s: String = path.into();
        assert_eq!(s, "/docs/file.md");
    }

    #[test]
    fn as_ref_returns_str() {
        let path: DocumentPath = "/docs/file.md".parse().unwrap();
        let s: &str = path.as_ref();
        assert_eq!(s, "/docs/file.md");
    }

    #[test]
    fn try_from_string_works() {
        let path = DocumentPath::try_from("docs/file.md".to_string()).unwrap();
        assert_eq!(path.as_str(), "/docs/file.md");
    }

    #[test]
    fn try_from_string_rejects_invalid() {
        let err = DocumentPath::try_from(".hidden".to_string()).unwrap_err();
        assert_eq!(err, DocumentPathError::HiddenSegment(".hidden".to_string()));
    }

    #[test]
    fn serializes_as_string() {
        let path: DocumentPath = "/docs/file.md".parse().unwrap();
        let value = serde_json::to_value(&path).unwrap();
        assert_eq!(value, serde_json::json!("/docs/file.md"));
    }

    #[test]
    fn deserializes_valid_path() {
        let path: DocumentPath =
            serde_json::from_value(serde_json::json!("/docs/file.md")).unwrap();
        assert_eq!(path.as_str(), "/docs/file.md");
    }

    #[test]
    fn deserializes_and_normalizes() {
        let path: DocumentPath = serde_json::from_value(serde_json::json!("docs/file.md")).unwrap();
        assert_eq!(path.as_str(), "/docs/file.md");
    }

    #[test]
    fn deserialize_rejects_invalid_path() {
        let result: Result<DocumentPath, _> =
            serde_json::from_value(serde_json::json!(".hidden/file.md"));
        assert!(result.is_err());
    }

    #[test]
    fn serde_round_trip() {
        let original: DocumentPath = "/designs/policy-engine.md".parse().unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let restored: DocumentPath = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn document_with_optional_document_path() {
        // Verify Option<DocumentPath> serialization
        let some_path: Option<DocumentPath> = Some("/docs/file.md".parse().unwrap());
        let json = serde_json::to_value(&some_path).unwrap();
        assert_eq!(json, serde_json::json!("/docs/file.md"));

        let none_path: Option<DocumentPath> = None;
        let json = serde_json::to_value(&none_path).unwrap();
        assert_eq!(json, serde_json::json!(null));
    }
}
