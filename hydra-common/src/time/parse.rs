//! Parse the `--since` / `--until` CLI flag values used by `hydra graph` time
//! window subcommands.
//!
//! Accepted forms:
//! - RFC 3339 absolute timestamps, e.g. `2026-05-15T13:00:00Z`.
//! - Relative durations against `now`: `-<N>(s|m|h|d)`, e.g. `-30m`, `-7d`.
//! - The literal `now`.

use chrono::{DateTime, Duration, Utc};
use std::fmt;
use std::str::FromStr;

/// CLI time-window value parsed from `--since` / `--until` flags.
///
/// Implements [`FromStr`] (and [`Display`]) so clap can parse it directly as
/// an argument type — callers should declare `since: HydraTime` /
/// `until: Option<HydraTime>` on their arg structs rather than parsing
/// strings manually in the command body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HydraTime(DateTime<Utc>);

impl HydraTime {
    /// Construct from an existing UTC timestamp.
    pub const fn from_utc(ts: DateTime<Utc>) -> Self {
        Self(ts)
    }

    /// Extract the underlying UTC timestamp.
    pub const fn into_inner(self) -> DateTime<Utc> {
        self.0
    }

    /// Borrow the underlying UTC timestamp.
    pub const fn as_utc(&self) -> &DateTime<Utc> {
        &self.0
    }
}

impl From<DateTime<Utc>> for HydraTime {
    fn from(ts: DateTime<Utc>) -> Self {
        Self(ts)
    }
}

impl From<HydraTime> for DateTime<Utc> {
    fn from(t: HydraTime) -> Self {
        t.0
    }
}

impl FromStr for HydraTime {
    type Err = TimeParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_window_arg(s).map(Self)
    }
}

impl fmt::Display for HydraTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.to_rfc3339())
    }
}

/// Errors returned by [`parse_window_arg`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeParseError {
    /// The input was the empty string.
    Empty,
    /// The relative duration's numeric portion failed to parse as `u64`.
    InvalidNumber(String),
    /// The relative duration's unit suffix was missing or unrecognized.
    InvalidUnit(String),
    /// The input did not match any accepted form.
    Unrecognized(String),
}

impl fmt::Display for TimeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeParseError::Empty => {
                write!(f, "time value is empty")
            }
            TimeParseError::InvalidNumber(s) => {
                write!(f, "invalid number in relative duration: '{s}'")
            }
            TimeParseError::InvalidUnit(s) => {
                write!(
                    f,
                    "invalid unit in relative duration '{s}' (expected one of s, m, h, d)"
                )
            }
            TimeParseError::Unrecognized(s) => write!(
                f,
                "unrecognized time value '{s}'; expected RFC 3339 timestamp, \
                 relative duration like '-1h' / '-30m' / '-7d', or 'now'"
            ),
        }
    }
}

impl std::error::Error for TimeParseError {}

/// Parse a `--since` / `--until` argument into an absolute UTC timestamp.
///
/// The relative-duration arm is resolved against [`Utc::now()`]. For test
/// determinism, see [`parse_window_arg_with_now`].
pub fn parse_window_arg(s: &str) -> Result<DateTime<Utc>, TimeParseError> {
    parse_window_arg_with_now(s, Utc::now())
}

/// Same as [`parse_window_arg`], but resolves relative durations and the
/// literal `now` against the supplied `now` value. Used by tests.
pub fn parse_window_arg_with_now(
    s: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, TimeParseError> {
    if s.is_empty() {
        return Err(TimeParseError::Empty);
    }

    if s == "now" {
        return Ok(now);
    }

    if let Some(rest) = s.strip_prefix('-') {
        return parse_relative_duration(rest, now);
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    Err(TimeParseError::Unrecognized(s.to_string()))
}

fn parse_relative_duration(
    rest: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, TimeParseError> {
    if rest.is_empty() {
        return Err(TimeParseError::Unrecognized(format!("-{rest}")));
    }
    // Split into numeric prefix and single-char unit suffix.
    let unit_idx = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| i);
    let Some(unit_idx) = unit_idx else {
        // All digits, no unit.
        return Err(TimeParseError::InvalidUnit(format!("-{rest}")));
    };
    let (num_str, unit_str) = rest.split_at(unit_idx);
    if num_str.is_empty() {
        return Err(TimeParseError::InvalidNumber(format!("-{rest}")));
    }
    let n: u64 = num_str
        .parse()
        .map_err(|_| TimeParseError::InvalidNumber(format!("-{rest}")))?;
    let n = n as i64;
    let dur = match unit_str {
        "s" => Duration::seconds(n),
        "m" => Duration::minutes(n),
        "h" => Duration::hours(n),
        "d" => Duration::days(n),
        _ => return Err(TimeParseError::InvalidUnit(format!("-{rest}"))),
    };
    Ok(now - dur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap()
    }

    #[test]
    fn parses_now_literal() {
        let now = fixed_now();
        let result = parse_window_arg_with_now("now", now).unwrap();
        assert_eq!(result, now);
    }

    #[test]
    fn parses_rfc3339_timestamp_with_z() {
        let result = parse_window_arg_with_now("2026-05-15T13:00:00Z", fixed_now()).unwrap();
        assert_eq!(result, Utc.with_ymd_and_hms(2026, 5, 15, 13, 0, 0).unwrap());
    }

    #[test]
    fn parses_rfc3339_timestamp_with_offset() {
        let result = parse_window_arg_with_now("2026-05-15T13:00:00+02:00", fixed_now()).unwrap();
        assert_eq!(result, Utc.with_ymd_and_hms(2026, 5, 15, 11, 0, 0).unwrap());
    }

    #[test]
    fn parses_relative_seconds() {
        let now = fixed_now();
        let result = parse_window_arg_with_now("-30s", now).unwrap();
        assert_eq!(result, now - Duration::seconds(30));
    }

    #[test]
    fn parses_relative_minutes() {
        let now = fixed_now();
        let result = parse_window_arg_with_now("-30m", now).unwrap();
        assert_eq!(result, now - Duration::minutes(30));
    }

    #[test]
    fn parses_relative_hours() {
        let now = fixed_now();
        let result = parse_window_arg_with_now("-1h", now).unwrap();
        assert_eq!(result, now - Duration::hours(1));
    }

    #[test]
    fn parses_relative_days() {
        let now = fixed_now();
        let result = parse_window_arg_with_now("-7d", now).unwrap();
        assert_eq!(result, now - Duration::days(7));
    }

    #[test]
    fn rejects_empty_string() {
        let err = parse_window_arg_with_now("", fixed_now()).unwrap_err();
        assert_eq!(err, TimeParseError::Empty);
    }

    #[test]
    fn rejects_missing_unit() {
        let err = parse_window_arg_with_now("-1", fixed_now()).unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidUnit(_)), "got {err:?}");
    }

    #[test]
    fn rejects_unknown_unit() {
        let err = parse_window_arg_with_now("-1y", fixed_now()).unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidUnit(_)), "got {err:?}");
    }

    #[test]
    fn rejects_missing_number() {
        let err = parse_window_arg_with_now("-h", fixed_now()).unwrap_err();
        assert!(
            matches!(err, TimeParseError::InvalidNumber(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_garbage() {
        let err = parse_window_arg_with_now("yesterday", fixed_now()).unwrap_err();
        assert!(
            matches!(err, TimeParseError::Unrecognized(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_just_minus() {
        let err = parse_window_arg_with_now("-", fixed_now()).unwrap_err();
        assert!(
            matches!(err, TimeParseError::Unrecognized(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_non_rfc3339_date_only() {
        let err = parse_window_arg_with_now("2026-05-15", fixed_now()).unwrap_err();
        assert!(
            matches!(err, TimeParseError::Unrecognized(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn hydra_time_from_str_parses_rfc3339() {
        let t: HydraTime = "2026-05-15T13:00:00Z".parse().unwrap();
        assert_eq!(
            t.into_inner(),
            Utc.with_ymd_and_hms(2026, 5, 15, 13, 0, 0).unwrap()
        );
    }

    #[test]
    fn hydra_time_from_str_rejects_garbage() {
        let err = "yesterday".parse::<HydraTime>().unwrap_err();
        assert!(
            matches!(err, TimeParseError::Unrecognized(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn hydra_time_display_round_trips_through_from_str() {
        let original = HydraTime::from_utc(Utc.with_ymd_and_hms(2026, 5, 15, 13, 0, 0).unwrap());
        let s = original.to_string();
        let parsed: HydraTime = s.parse().unwrap();
        assert_eq!(parsed, original);
    }
}
