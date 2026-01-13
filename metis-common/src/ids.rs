use rand::distributions::{Distribution, Uniform};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, str::FromStr};

const MIN_RANDOM_LEN: usize = 4;
const DEFAULT_RANDOM_LEN: usize = 6;
const MAX_RANDOM_LEN: usize = 12;
const ISSUE_PREFIX: &str = "i-";
const PATCH_PREFIX: &str = "p-";
const TASK_PREFIX: &str = "t-";

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
pub struct MetisId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct IssueId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct PatchId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct TaskId(String);

impl MetisId {
    pub fn as_issue_id(&self) -> Option<IssueId> {
        IssueId::try_from(self.clone()).ok()
    }

    pub fn as_patch_id(&self) -> Option<PatchId> {
        PatchId::try_from(self.clone()).ok()
    }

    pub fn as_task_id(&self) -> Option<TaskId> {
        TaskId::try_from(self.clone()).ok()
    }

    pub fn validate_str(value: &str) -> Result<(), MetisIdError> {
        if value.starts_with(ISSUE_PREFIX) {
            IssueId::validate_str(value)
        } else if value.starts_with(PATCH_PREFIX) {
            PatchId::validate_str(value)
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

impl TryFrom<String> for TaskId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        TaskId::validate_str(&value)?;
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

impl TryFrom<MetisId> for TaskId {
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

impl From<TaskId> for MetisId {
    fn from(value: TaskId) -> Self {
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

impl From<TaskId> for String {
    fn from(value: TaskId) -> Self {
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

impl fmt::Display for TaskId {
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

impl AsRef<str> for TaskId {
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

impl FromStr for TaskId {
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
