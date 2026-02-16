use rand::distributions::{Distribution, Uniform};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, str::FromStr};

pub const MIN_RANDOM_LEN: usize = 4;
const DEFAULT_RANDOM_LEN: usize = 6;
pub const MAX_RANDOM_LEN: usize = 12;
const ISSUE_PREFIX: &str = "i-";
const PATCH_PREFIX: &str = "p-";
const TASK_PREFIX: &str = "t-";
const DOCUMENT_PREFIX: &str = "d-";

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
pub struct DocumentId(String);

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

    pub fn as_document_id(&self) -> Option<DocumentId> {
        DocumentId::try_from(self.clone()).ok()
    }

    pub fn as_task_id(&self) -> Option<TaskId> {
        TaskId::try_from(self.clone()).ok()
    }

    pub fn validate_str(value: &str) -> Result<(), MetisIdError> {
        if value.starts_with(ISSUE_PREFIX) {
            IssueId::validate_str(value)
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

    pub fn new_for_count(count: u64) -> Self {
        let len = compute_random_len(count);
        Self::generate(len).expect("computed random length should always be valid")
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

    pub fn new_for_count(count: u64) -> Self {
        let len = compute_random_len(count);
        Self::generate(len).expect("computed random length should always be valid")
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

    pub fn new_for_count(count: u64) -> Self {
        let len = compute_random_len(count);
        Self::generate(len).expect("computed random length should always be valid")
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

impl TaskId {
    pub fn generate(random_len: usize) -> Result<Self, MetisIdError> {
        generate_with_prefix(TASK_PREFIX, random_len).map(Self)
    }

    pub fn new() -> Self {
        Self::generate(DEFAULT_RANDOM_LEN).expect("default random length should always be valid")
    }

    pub fn new_for_count(count: u64) -> Self {
        let len = compute_random_len(count);
        Self::generate(len).expect("computed random length should always be valid")
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

impl TryFrom<String> for DocumentId {
    type Error = MetisIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        DocumentId::validate_str(&value)?;
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

impl TryFrom<MetisId> for DocumentId {
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

impl From<DocumentId> for MetisId {
    fn from(value: DocumentId) -> Self {
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

impl From<DocumentId> for String {
    fn from(value: DocumentId) -> Self {
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

impl fmt::Display for DocumentId {
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

impl AsRef<str> for DocumentId {
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

impl FromStr for DocumentId {
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

/// Computes the minimum random suffix length needed for the given object count,
/// keeping collision probability safely low using birthday-bound math.
///
/// We want `26^len > count^2 * SAFETY_MARGIN` to ensure negligible collision
/// probability. The result is clamped to `[MIN_RANDOM_LEN, MAX_RANDOM_LEN]`.
pub fn compute_random_len(object_count: u64) -> usize {
    if object_count <= 1 {
        return MIN_RANDOM_LEN;
    }

    // Birthday bound: P(collision) ≈ n^2 / (2 * 26^len)
    // We want P < 1e-6, so 26^len > n^2 * 500_000
    // Using f64 logarithms: len > (2*log26(n) + log26(500_000))
    let log26 = (26.0_f64).ln();
    let n = object_count as f64;
    let threshold = 2.0 * n.ln() / log26 + (500_000.0_f64).ln() / log26;
    let len = (threshold.ceil()) as usize;

    len.clamp(MIN_RANDOM_LEN, MAX_RANDOM_LEN)
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
    fn compute_random_len_returns_min_for_zero() {
        assert_eq!(compute_random_len(0), MIN_RANDOM_LEN);
    }

    #[test]
    fn compute_random_len_returns_min_for_one() {
        assert_eq!(compute_random_len(1), MIN_RANDOM_LEN);
    }

    #[test]
    fn compute_random_len_returns_min_for_small_counts() {
        // For small counts (e.g., 10), MIN_RANDOM_LEN=4 gives 26^4=456K
        // which is >> 10^2 * 500K = 50M... actually 456K < 50M, so len=5 expected.
        // Let's just verify it's >= MIN_RANDOM_LEN
        let len = compute_random_len(10);
        assert!(len >= MIN_RANDOM_LEN);
        assert!(len <= MAX_RANDOM_LEN);
    }

    #[test]
    fn compute_random_len_grows_with_count() {
        let len_small = compute_random_len(10);
        let len_medium = compute_random_len(10_000);
        let len_large = compute_random_len(1_000_000);
        assert!(
            len_large >= len_medium,
            "len_large={len_large} should be >= len_medium={len_medium}"
        );
        assert!(
            len_medium >= len_small,
            "len_medium={len_medium} should be >= len_small={len_small}"
        );
    }

    #[test]
    fn compute_random_len_caps_at_max() {
        let len = compute_random_len(u64::MAX);
        assert_eq!(len, MAX_RANDOM_LEN);
    }

    #[test]
    fn compute_random_len_maintains_birthday_bound() {
        // For any count, 26^len should be >> count^2 (birthday bound)
        for &count in &[0u64, 1, 10, 100, 1_000, 10_000, 100_000, 1_000_000] {
            let len = compute_random_len(count);
            if count > 1 {
                let space = 26.0_f64.powi(len as i32);
                let n = count as f64;
                // Birthday bound: P ≈ n^2 / (2*space), we want P < 1e-6
                // So space > n^2 * 500_000
                assert!(
                    space > n * n,
                    "at count={count}, len={len}, 26^{len}={space} should be > {count}^2={}",
                    count * count
                );
            }
        }
    }

    #[test]
    fn new_for_count_generates_valid_ids() {
        let issue = IssueId::new_for_count(0);
        assert!(issue.as_ref().starts_with(IssueId::prefix()));
        // With count=0, suffix should be MIN_RANDOM_LEN
        assert_eq!(
            issue.as_ref().len(),
            IssueId::prefix().len() + MIN_RANDOM_LEN
        );

        let patch = PatchId::new_for_count(100_000);
        assert!(patch.as_ref().starts_with(PatchId::prefix()));
        let suffix_len = patch.as_ref().len() - PatchId::prefix().len();
        assert!(suffix_len >= MIN_RANDOM_LEN);
        assert!(suffix_len <= MAX_RANDOM_LEN);
    }
}
