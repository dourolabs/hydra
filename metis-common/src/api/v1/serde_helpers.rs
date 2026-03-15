use crate::task_status::Status;
use serde::{Deserialize, Deserializer, Serializer, de};
use serde_json::Value;
use std::fmt::Display;
use std::str::FromStr;

pub(crate) fn serialize_comma_separated<T: Display, S: Serializer>(
    items: &[T],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let s = items
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

pub(crate) fn deserialize_comma_separated<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: FromStr,
    T::Err: Display,
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
