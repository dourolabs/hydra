use chrono::Duration as ChronoDuration;

/// Formats a duration into a compact string such as `5m04s` or `2h03m15s`.
pub fn format_compact_duration(duration: ChronoDuration) -> String {
    let total_seconds = duration.num_seconds();
    if total_seconds <= 0 {
        return "0s".to_string();
    }

    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h{minutes:02}m{seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::format_compact_duration;
    use chrono::Duration as ChronoDuration;

    #[test]
    fn renders_hours_minutes_seconds() {
        let duration = ChronoDuration::seconds(7_395); // 2h03m15s

        assert_eq!(format_compact_duration(duration), "2h03m15s");
    }

    #[test]
    fn renders_minutes_and_seconds() {
        let duration = ChronoDuration::seconds(305); // 5m05s

        assert_eq!(format_compact_duration(duration), "5m05s");
    }

    #[test]
    fn renders_seconds_only_when_short() {
        let duration = ChronoDuration::seconds(42);

        assert_eq!(format_compact_duration(duration), "42s");
    }
}
