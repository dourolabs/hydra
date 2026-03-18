use std::fmt::Display;
use std::str::FromStr;

use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Serialize a `Vec<T>` as a comma-separated string, where each element uses
/// its `Display` impl.
pub fn serialize_comma_separated<T, S>(items: &[T], serializer: S) -> Result<S::Ok, S::Error>
where
    T: Display,
    S: Serializer,
{
    let s = items
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

/// Deserialize a comma-separated string into a `Vec<T>`, where each element is
/// parsed via its `FromStr` impl.
pub fn deserialize_comma_separated<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
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

/// Serialize a `Vec<T>` as a comma-separated string, where each element is
/// serialized to JSON first (for types that serialize as a JSON string but
/// don't implement `Display`/`FromStr`).
pub fn serialize_comma_separated_json<T, S>(items: &[T], serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: Serializer,
{
    let s = items
        .iter()
        .map(|item| {
            let v = serde_json::to_value(item).expect("value serializes to JSON");
            v.as_str().expect("value serializes to string").to_string()
        })
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

/// Deserialize a comma-separated string into a `Vec<T>`, where each element is
/// deserialized from a JSON string value (for types that don't implement
/// `FromStr`).
pub fn deserialize_comma_separated_json<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: serde::de::DeserializeOwned,
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
