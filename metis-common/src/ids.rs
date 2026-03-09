use rand::distributions::{Distribution, Uniform};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, str::FromStr};

const MIN_RANDOM_LEN: usize = 4;
const DEFAULT_RANDOM_LEN: usize = 6;
const MAX_RANDOM_LEN: usize = 12;
const ISSUE_PREFIX: &str = "i-";
const MESSAGE_PREFIX: &str = "m-";
const PATCH_PREFIX: &str = "p-";
const TASK_PREFIX: &str = "t-";
const DOCUMENT_PREFIX: &str = "d-";
const LABEL_PREFIX: &str = "l-";
const NOTIFICATION_PREFIX: &str = "nf-";
const AUDIT_EVENT_PREFIX: &str = "ae-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetisIdError {
    InvalidPrefix(String),
    InvalidLength {
        min: usize,
        max: usize,
        actual: usize,
    },
    InvalidCharacters,
}

impl fmt::Display for MetisIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetisIdError::InvalidPrefix(value) => {
                write!(f, "id '{value}' is missing a supported prefix")
            }
            MetisIdError::InvalidLength { min, max, actual } => write!(
                f,
                "id length must be between {min} and {max} characters (got {actual})"
            ),
            MetisIdError::InvalidCharacters => {
                f.write_str("id suffix must contain only ASCII alphabetic characters")
            }
        }
    }
}

impl std::error::Error for MetisIdError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct MetisId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct IssueId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct PatchId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct DocumentId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct MessageId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct TaskId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct NotificationId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct LabelId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct AuditEventId(String);

impl MetisId {
    pub fn as_issue_id(&self) -> Option<IssueId> {
        IssueId::try_from(self.clone()).ok()
    }

    pub fn as_patch_id(&self) -> Option<PatchId> {
        PatchId::try_from(self.clone()).ok()
    }

    pub fn as_document_id(&self) -> Option<DocumentId> {
        DocumentId::try_from(self.clone()).ok()
    }

    pub fn as_message_id(&self) -> Option<MessageId> {
        MessageId::try_from(self.clone()).ok()
    }

    pub fn as_task_id(&self) -> Option<TaskId> {
        TaskId::try_from(self.clone()).ok()
    }

    pub fn as_notification_id(&self) -> Option<NotificationId> {
        NotificationId::try_from(self.clone()).ok()
    }

    pub fn as_label_id(&self) -> Option<LabelId> {
        LabelId::try_from(self.clone()).ok()
    }

    pub fn as_audit_event_id(&self) -> Option<AuditEventId> {
        AuditEventId::try_from(self.clone()).ok()
    }

    pub fn validate_str(value: &str) -> Result<(), MetisIdError> {
        // Check longer prefixes first to avoid ambiguity (e.g., "nf-" before single-char prefixes)
        if value.starts_with(AUDIT_EVENT_PREFIX) {
            AuditEventId::validate_str(value)
        } else if value.starts_with(NOTIFICATION_PREFIX) {
            NotificationId::validate_str(value)
        } else if value.starts_with(ISSUE_PREFIX) {
            IssueId::validate_str(value)
        } else if value.starts_with(LABEL_PREFIX) {
            LabelId::validate_str(value)
        } else if value.starts_with(MESSAGE_PREFIX) {
            MessageId::validate_str(value)
        } else if value.starts_with(PATCH_PREFIX) {
            PatchId::validate_str(value)
        } else if value.starts_with(DOCUMENT_PREFIX) {
            DocumentId::validate_str(value)
        } else if value.starts_with(TASK_PREFIX) {
            TaskId::validate_str(value)
        } else {
            Err(MetisIdError::InvalidPrefix(value.to_string()))
        }
    }
}

impl<'de> Deserialize<'de> for MetisId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        MetisId::try_from(value).map_err(de::Error::custom)
    }
}

impl IssueId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(ISSUE_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        ISSUE_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, ISSUE_PREFIX)
    }
}

impl Default for IssueId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for IssueId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        IssueId::try_from(value).map_err(de::Error::custom)
    }
}

impl PatchId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(PATCH_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        PATCH_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, PATCH_PREFIX)
    }
}

impl Default for PatchId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for PatchId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        PatchId::try_from(value).map_err(de::Error::custom)
    }
}

impl DocumentId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(DOCUMENT_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        DOCUMENT_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, DOCUMENT_PREFIX)
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for DocumentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        DocumentId::try_from(value).map_err(de::Error::custom)
    }
}

impl MessageId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(MESSAGE_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        MESSAGE_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, MESSAGE_PREFIX)
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for MessageId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        MessageId::try_from(value).map_err(de::Error::custom)
    }
}

impl TaskId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(TASK_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        TASK_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, TASK_PREFIX)
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for TaskId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        TaskId::try_from(value).map_err(de::Error::custom)
    }
}

impl NotificationId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(NOTIFICATION_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        NOTIFICATION_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, NOTIFICATION_PREFIX)
    }
}

impl Default for NotificationId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for NotificationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        NotificationId::try_from(value).map_err(de::Error::custom)
    }
}

impl LabelId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(LABEL_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        LABEL_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, LABEL_PREFIX)
    }
}

impl Default for LabelId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for LabelId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        LabelId::try_from(value).map_err(de::Error::custom)
    }
}

impl AuditEventId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(AUDIT_EVENT_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub const fn prefix() -> &'static str {
        AUDIT_EVENT_PREFIX
    }

    fn validate_str(value: &str) -> Result<(), MetisIdError> {
        validate_with_prefix(value, AUDIT_EVENT_PREFIX)
    }
}

impl Default for AuditEventId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for AuditEventId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        AuditEventId::try_from(value).map_err(de::Error::custom)
    }
}

impl TryFrom<String> for MetisId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        MetisId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for IssueId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        IssueId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for PatchId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        PatchId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for DocumentId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        DocumentId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for MessageId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        MessageId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for TaskId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        TaskId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for NotificationId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        NotificationId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for LabelId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        LabelId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for AuditEventId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        AuditEventId::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<MetisId> for IssueId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl TryFrom<MetisId> for PatchId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl TryFrom<MetisId> for DocumentId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl TryFrom<MetisId> for MessageId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl TryFrom<MetisId> for TaskId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl TryFrom<MetisId> for NotificationId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl TryFrom<MetisId> for LabelId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl TryFrom<MetisId> for AuditEventId {
    type Error = MetisIdError;

    fn try_from(value: MetisId) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

impl From<IssueId> for MetisId {
    fn from(value: IssueId) -> Self {
        Self(value.0)
    }
}

impl From<PatchId> for MetisId {
    fn from(value: PatchId) -> Self {
        Self(value.0)
    }
}

impl From<DocumentId> for MetisId {
    fn from(value: DocumentId) -> Self {
        Self(value.0)
    }
}

impl From<MessageId> for MetisId {
    fn from(value: MessageId) -> Self {
        Self(value.0)
    }
}

impl From<TaskId> for MetisId {
    fn from(value: TaskId) -> Self {
        Self(value.0)
    }
}

impl From<NotificationId> for MetisId {
    fn from(value: NotificationId) -> Self {
        Self(value.0)
    }
}

impl From<LabelId> for MetisId {
    fn from(value: LabelId) -> Self {
        Self(value.0)
    }
}

impl From<AuditEventId> for MetisId {
    fn from(value: AuditEventId) -> Self {
        Self(value.0)
    }
}

impl From<IssueId> for String {
    fn from(value: IssueId) -> Self {
        value.0
    }
}

impl From<PatchId> for String {
    fn from(value: PatchId) -> Self {
        value.0
    }
}

impl From<DocumentId> for String {
    fn from(value: DocumentId) -> Self {
        value.0
    }
}

impl From<MessageId> for String {
    fn from(value: MessageId) -> Self {
        value.0
    }
}

impl From<TaskId> for String {
    fn from(value: TaskId) -> Self {
        value.0
    }
}

impl From<NotificationId> for String {
    fn from(value: NotificationId) -> Self {
        value.0
    }
}

impl From<LabelId> for String {
    fn from(value: LabelId) -> Self {
        value.0
    }
}

impl From<AuditEventId> for String {
    fn from(value: AuditEventId) -> Self {
        value.0
    }
}

impl From<MetisId> for String {
    fn from(value: MetisId) -> Self {
        value.0
    }
}

impl fmt::Display for MetisId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for IssueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for PatchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for NotificationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for LabelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for AuditEventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for MetisId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for IssueId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for PatchId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for DocumentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for MessageId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for TaskId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for NotificationId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LabelId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for AuditEventId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for MetisId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for IssueId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for PatchId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for DocumentId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for MessageId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for TaskId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for NotificationId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for LabelId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

impl FromStr for AuditEventId {
    type Err = MetisIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

fn validate_random_length(len: usize) -> Result<(), MetisIdError> {
    if (MIN_RANDOM_LEN..=MAX_RANDOM_LEN).contains(&len) {
        Ok(())
    } else {
        Err(MetisIdError::InvalidLength {
            min: MIN_RANDOM_LEN,
            max: MAX_RANDOM_LEN,
            actual: len,
        })
    }
}

fn validate_suffix(suffix: &str) -> Result<(), MetisIdError> {
    validate_random_length(suffix.len())?;
    if suffix.chars().all(|ch| ch.is_ascii_alphabetic()) {
        Ok(())
    } else {
        Err(MetisIdError::InvalidCharacters)
    }
}

fn validate_with_prefix(value: &str, prefix: &str) -> Result<(), MetisIdError> {
    value
        .strip_prefix(prefix)
        .ok_or_else(|| MetisIdError::InvalidPrefix(value.to_string()))
        .and_then(validate_suffix)
}

fn generate_with_prefix(prefix: &str, random_len: usize) -> Result<String, MetisIdError> {
    validate_random_length(random_len)?;

    let distribution = Uniform::from(0..26);
    let mut rng = rand::thread_rng();

    let mut id = String::with_capacity(prefix.len() + random_len);
    id.push_str(prefix);
    for _ in 0..random_len {
        let offset = distribution.sample(&mut rng);
        let letter = (b'a' + offset as u8) as char;
        id.push(letter);
    }

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_id_uses_expected_prefix_and_length() {
        let document_id = DocumentId::generate(MIN_RANDOM_LEN).expect("valid length");
        assert!(document_id.as_ref().starts_with(DocumentId::prefix()));
        assert_eq!(
            document_id.as_ref().len(),
            DocumentId::prefix().len() + MIN_RANDOM_LEN
        );
    }

    #[test]
    fn document_id_rejects_invalid_prefix() {
        let err = DocumentId::try_from("x-invalid".to_string()).expect_err("expected error");
        match err {
            MetisIdError::InvalidPrefix(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn document_id_rejects_invalid_length() {
        let err = DocumentId::try_from(format!("{}abc", DocumentId::prefix()))
            .expect_err("expected invalid length");
        match err {
            MetisIdError::InvalidLength { .. } => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn metis_id_converts_to_document_id() {
        let document_id = DocumentId::new();
        let metis_id: MetisId = document_id.clone().into();
        let converted = metis_id.as_document_id().expect("document id");
        assert_eq!(converted, document_id);
    }

    #[test]
    fn document_id_round_trips_through_serde() {
        let document_id = DocumentId::new();
        let serialized = serde_json::to_string(&document_id).expect("serialize");
        let deserialized: DocumentId = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized, document_id);
    }

    #[test]
    fn message_id_generate_produces_m_prefix() {
        let message_id = MessageId::new();
        assert!(
            message_id.as_ref().starts_with(MessageId::prefix()),
            "MessageId should start with 'm-', got: {message_id}",
        );
    }

    #[test]
    fn message_id_round_trips_through_serde() {
        let message_id = MessageId::new();
        let serialized = serde_json::to_string(&message_id).expect("serialize");
        let deserialized: MessageId = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized, message_id);
    }

    #[test]
    fn message_id_rejects_invalid_prefix() {
        let err = MessageId::try_from("x-invalid".to_string()).expect_err("expected error");
        match err {
            MetisIdError::InvalidPrefix(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
