//! Parser for the `--since` / `--until` time-window flags shared by
//! `hydra graph diff` and `hydra graph log`.
//!
//! Accepted forms:
//! - RFC 3339 absolute timestamps (e.g. `2026-05-15T13:00:00Z`).
//! - Relative durations against `now`: `-<N><unit>` where `<unit>` is one of
//!   `s` (seconds), `m` (minutes), `h` (hours), `d` (days). Examples:
//!   `-30s`, `-15m`, `-1h`, `-7d`.
//! - The literal string `now`.

use std::fmt;

use chrono::{DateTime, Duration, Utc};

/// Errors produced by [`parse_window_arg`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeParseError {
    /// The input was empty.
    Empty,
    /// The input did not match any accepted form.
    InvalidFormat(String),
    /// The numeric portion of a relative duration could not be parsed as `i64`.
    InvalidDuration(String),
    /// The duration was a valid number but did not fit a chrono `Duration`.
    DurationOverflow(String),
}

impl fmt::Display for TimeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeParseError::Empty => f.write_str("time-window value is empty"),
            TimeParseError::InvalidFormat(s) => write!(
                f,
                "'{s}' is not a valid time-window value (expected RFC 3339, '-Nh'/'-Nm'/'-Ns'/'-Nd', or 'now')"
            ),
            TimeParseError::InvalidDuration(s) => {
                write!(
                    f,
                    "'{s}' has an invalid duration value (must be a non-negative integer)"
                )
            }
            TimeParseError::DurationOverflow(s) => {
                write!(f, "'{s}' is too large to represent as a time delta")
            }
        }
    }
}

impl std::error::Error for TimeParseError {}

/// Parse a time-window argument against the current wall-clock time.
///
/// See [the module docs](self) for accepted forms.
pub fn parse_window_arg(s: &str) -> Result<DateTime<Utc>, TimeParseError> {
    parse_window_arg_with_now(s, Utc::now())
}

/// Parse a time-window argument against an explicit "now" reference.
///
/// Exposed for tests; production callers should use [`parse_window_arg`].
pub fn parse_window_arg_with_now(
    s: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, TimeParseError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(TimeParseError::Empty);
    }
    if trimmed == "now" {
        return Ok(now);
    }
    if let Some(rest) = trimmed.strip_prefix('-') {
        return parse_relative(rest, now, trimmed);
    }
    DateTime::parse_from_rfc3339(trimmed)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| TimeParseError::InvalidFormat(trimmed.to_string()))
}

fn parse_relative(
    rest: &str,
    now: DateTime<Utc>,
    original: &str,
) -> Result<DateTime<Utc>, TimeParseError> {
    if rest.is_empty() {
        return Err(TimeParseError::InvalidFormat(original.to_string()));
    }
    let (num_part, unit_char) = split_number_unit(rest)
        .ok_or_else(|| TimeParseError::InvalidFormat(original.to_string()))?;
    if num_part.is_empty() {
        return Err(TimeParseError::InvalidFormat(original.to_string()));
    }
    let value: i64 = num_part
        .parse()
        .map_err(|_| TimeParseError::InvalidDuration(original.to_string()))?;
    if value < 0 {
        return Err(TimeParseError::InvalidDuration(original.to_string()));
    }
    let delta = match unit_char {
        's' => Duration::try_seconds(value),
        'm' => Duration::try_minutes(value),
        'h' => Duration::try_hours(value),
        'd' => Duration::try_days(value),
        _ => return Err(TimeParseError::InvalidFormat(original.to_string())),
    }
    .ok_or_else(|| TimeParseError::DurationOverflow(original.to_string()))?;
    now.checked_sub_signed(delta)
        .ok_or_else(|| TimeParseError::DurationOverflow(original.to_string()))
}

/// Splits a string into its leading digit run and a single trailing unit char.
/// Returns `None` if the input doesn't end in a single ASCII alphabetic char or
/// if there is no leading digit portion at all.
fn split_number_unit(s: &str) -> Option<(&str, char)> {
    let last = s.chars().last()?;
    if !last.is_ascii_alphabetic() {
        return None;
    }
    let unit_len = last.len_utf8();
    let num = &s[..s.len() - unit_len];
    Some((num, last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap()
    }

    #[test]
    fn parses_rfc3339_utc() {
        let parsed = parse_window_arg_with_now("2026-05-15T13:00:00Z", fixed_now()).unwrap();
        assert_eq!(parsed, Utc.with_ymd_and_hms(2026, 5, 15, 13, 0, 0).unwrap());
    }

    #[test]
    fn parses_rfc3339_with_timezone_offset() {
        // 2026-05-15T15:00:00+02:00 == 2026-05-15T13:00:00Z
        let parsed = parse_window_arg_with_now("2026-05-15T15:00:00+02:00", fixed_now()).unwrap();
        assert_eq!(parsed, Utc.with_ymd_and_hms(2026, 5, 15, 13, 0, 0).unwrap());
    }

    #[test]
    fn parses_literal_now() {
        let now = fixed_now();
        assert_eq!(parse_window_arg_with_now("now", now).unwrap(), now);
    }

    #[test]
    fn parses_relative_hours() {
        let now = fixed_now();
        assert_eq!(
            parse_window_arg_with_now("-1h", now).unwrap(),
            now - Duration::try_hours(1).unwrap()
        );
    }

    #[test]
    fn parses_relative_minutes() {
        let now = fixed_now();
        assert_eq!(
            parse_window_arg_with_now("-30m", now).unwrap(),
            now - Duration::try_minutes(30).unwrap()
        );
    }

    #[test]
    fn parses_relative_seconds() {
        let now = fixed_now();
        assert_eq!(
            parse_window_arg_with_now("-45s", now).unwrap(),
            now - Duration::try_seconds(45).unwrap()
        );
    }

    #[test]
    fn parses_relative_days() {
        let now = fixed_now();
        assert_eq!(
            parse_window_arg_with_now("-7d", now).unwrap(),
            now - Duration::try_days(7).unwrap()
        );
    }

    #[test]
    fn parses_zero_relative_as_now() {
        let now = fixed_now();
        assert_eq!(parse_window_arg_with_now("-0h", now).unwrap(), now);
    }

    #[test]
    fn trims_whitespace() {
        let now = fixed_now();
        assert_eq!(parse_window_arg_with_now("   now  ", now).unwrap(), now);
    }

    #[test]
    fn rejects_empty_string() {
        let err = parse_window_arg_with_now("", fixed_now()).unwrap_err();
        assert_eq!(err, TimeParseError::Empty);
    }

    #[test]
    fn rejects_whitespace_only() {
        let err = parse_window_arg_with_now("   \t", fixed_now()).unwrap_err();
        assert_eq!(err, TimeParseError::Empty);
    }

    #[test]
    fn rejects_unknown_unit() {
        let err = parse_window_arg_with_now("-5y", fixed_now()).unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidFormat(_)), "got {err}");
    }

    #[test]
    fn rejects_missing_unit() {
        let err = parse_window_arg_with_now("-5", fixed_now()).unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidFormat(_)), "got {err}");
    }

    #[test]
    fn rejects_missing_digits() {
        let err = parse_window_arg_with_now("-h", fixed_now()).unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidFormat(_)), "got {err}");
    }

    #[test]
    fn rejects_non_numeric_duration() {
        let err = parse_window_arg_with_now("-abch", fixed_now()).unwrap_err();
        assert!(
            matches!(err, TimeParseError::InvalidDuration(_)),
            "got {err}"
        );
    }

    #[test]
    fn rejects_positive_relative() {
        // We require a leading '-' to disambiguate from an arbitrary token.
        let err = parse_window_arg_with_now("1h", fixed_now()).unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidFormat(_)), "got {err}");
    }

    #[test]
    fn rejects_doubly_negative_relative() {
        // The grammar is `^-(\d+)(s|m|h|d)$`; "--1h" must not parse.
        let err = parse_window_arg_with_now("--1h", fixed_now()).unwrap_err();
        assert!(
            matches!(err, TimeParseError::InvalidDuration(_)),
            "got {err}"
        );
    }

    #[test]
    fn rejects_bogus_rfc3339() {
        let err = parse_window_arg_with_now("yesterday", fixed_now()).unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidFormat(_)), "got {err}");
    }

    #[test]
    fn duration_overflow_is_reported() {
        // 24-day units multiplied past i64::MAX seconds overflow the Duration.
        let err = parse_window_arg_with_now(&format!("-{}d", i64::MAX), fixed_now()).unwrap_err();
        assert!(
            matches!(err, TimeParseError::DurationOverflow(_)),
            "got {err}"
        );
    }
}
