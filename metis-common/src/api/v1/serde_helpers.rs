use crate::api::v1::issues::IssueGraphFilter;
use crate::task_status::Status;
use crate::{IssueId, LabelId};
use serde::{Deserialize, Deserializer, Serializer, de};
use serde_json::Value;

pub(crate) fn serialize_issue_ids<S>(ids: &[IssueId], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

pub(crate) fn deserialize_issue_ids<'de, D>(deserializer: D) -> Result<Vec<IssueId>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(',')
        .map(|part| part.trim().parse().map_err(de::Error::custom))
        .collect()
}

pub(crate) fn serialize_label_ids<S>(ids: &[LabelId], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

pub(crate) fn deserialize_label_ids<'de, D>(deserializer: D) -> Result<Vec<LabelId>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(',')
        .map(|part| part.trim().parse().map_err(de::Error::custom))
        .collect()
}

pub(crate) fn serialize_statuses<S>(statuses: &[Status], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = statuses
        .iter()
        .map(|status| {
            let v = serde_json::to_value(status).expect("Status serializes to JSON");
            v.as_str().expect("Status serializes to string").to_string()
        })
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

pub(crate) fn deserialize_statuses<'de, D>(deserializer: D) -> Result<Vec<Status>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(',')
        .map(|part| {
            let trimmed = part.trim();
            serde_json::from_value(Value::String(trimmed.to_string())).map_err(de::Error::custom)
        })
        .collect()
}

pub(crate) fn serialize_graph_filters<S>(
    filters: &[IssueGraphFilter],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = filters
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

pub(crate) fn deserialize_graph_filters<'de, D>(
    deserializer: D,
) -> Result<Vec<IssueGraphFilter>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(',')
        .map(|part| part.parse().map_err(de::Error::custom))
        .collect()
}
