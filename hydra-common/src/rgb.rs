use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, str::FromStr};

/// Error type for invalid RGB hex color strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbError {
    /// The value is not exactly 7 characters (# + 6 hex digits).
    InvalidLength { actual: usize },
    /// The value does not start with '#'.
    MissingHash,
    /// The value contains non-hex characters after the '#'.
    InvalidHexDigits,
}

impl fmt::Display for RgbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RgbError::InvalidLength { actual } => {
                write!(
                    f,
                    "RGB color must be exactly 7 characters (#rrggbb), got {actual}"
                )
            }
            RgbError::MissingHash => f.write_str("RGB color must start with '#'"),
            RgbError::InvalidHexDigits => {
                f.write_str("RGB color must contain only hex digits after '#'")
            }
        }
    }
}

impl std::error::Error for RgbError {}

/// A validated RGB hex color value (e.g. `"#e74c3c"`).
///
/// Wraps a `String` guaranteed to be in `#rrggbb` format (6-digit hex, case-insensitive).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
pub struct Rgb(String);

impl Rgb {
    fn validate_str(value: &str) -> Result<(), RgbError> {
        if value.len() != 7 {
            return Err(RgbError::InvalidLength {
                actual: value.len(),
            });
        }
        if !value.starts_with('#') {
            return Err(RgbError::MissingHash);
        }
        if !value[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(RgbError::InvalidHexDigits);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Rgb::try_from(value).map_err(de::Error::custom)
    }
}

impl TryFrom<String> for Rgb {
    type Error = RgbError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Rgb::validate_str(&value)?;
        Ok(Self(value))
    }
}

impl From<Rgb> for String {
    fn from(value: Rgb) -> Self {
        value.0
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Rgb {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for Rgb {
    type Err = RgbError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_hex_values() {
        assert!("#000000".parse::<Rgb>().is_ok());
        assert!("#FFFFFF".parse::<Rgb>().is_ok());
        assert!("#e74c3c".parse::<Rgb>().is_ok());
        assert!("#aAbBcC".parse::<Rgb>().is_ok());
    }

    #[test]
    fn rejects_empty_string() {
        let err = "".parse::<Rgb>().unwrap_err();
        assert!(matches!(err, RgbError::InvalidLength { actual: 0 }));
    }

    #[test]
    fn rejects_missing_hash() {
        let err = "e74c3c0".parse::<Rgb>().unwrap_err();
        assert!(matches!(err, RgbError::MissingHash));
    }

    #[test]
    fn rejects_non_hex_characters() {
        let err = "#xyzxyz".parse::<Rgb>().unwrap_err();
        assert!(matches!(err, RgbError::InvalidHexDigits));
    }

    #[test]
    fn rejects_too_short() {
        let err = "#12345".parse::<Rgb>().unwrap_err();
        assert!(matches!(err, RgbError::InvalidLength { actual: 6 }));
    }

    #[test]
    fn rejects_too_long() {
        let err = "#1234567".parse::<Rgb>().unwrap_err();
        assert!(matches!(err, RgbError::InvalidLength { actual: 8 }));
    }

    #[test]
    fn serde_round_trip() {
        let rgb: Rgb = "#e74c3c".parse().unwrap();
        let json = serde_json::to_string(&rgb).unwrap();
        assert_eq!(json, "\"#e74c3c\"");
        let deserialized: Rgb = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, rgb);
    }

    #[test]
    fn display_and_from_str_round_trip() {
        let rgb: Rgb = "#ABCDEF".parse().unwrap();
        let displayed = rgb.to_string();
        let parsed: Rgb = displayed.parse().unwrap();
        assert_eq!(parsed, rgb);
    }

    #[test]
    fn try_from_string() {
        let rgb = Rgb::try_from("#abcdef".to_string()).unwrap();
        assert_eq!(rgb.as_ref(), "#abcdef");

        let s: String = rgb.into();
        assert_eq!(s, "#abcdef");
    }

    #[test]
    fn deserialize_rejects_invalid() {
        let result = serde_json::from_str::<Rgb>("\"not-a-color\"");
        assert!(result.is_err());
    }
}
