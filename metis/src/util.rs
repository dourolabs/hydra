use chrono::Duration as ChronoDuration;

/// Shortens a list of lines to a maximum count, trimming the final line with an
/// ellipsis when truncation occurs.
pub(crate) fn truncate_lines(
    lines: Vec<String>,
    max_lines: usize,
    max_width: usize,
) -> Vec<String> {
    if max_lines == 0 || lines.len() <= max_lines {
        return lines;
    }

    let mut truncated: Vec<String> = lines.into_iter().take(max_lines).collect();
    if let Some(last) = truncated.last_mut() {
        let ellipsis = "...";
        if max_width <= ellipsis.len() {
            *last = ellipsis.chars().take(max_width).collect();
        } else {
            let keep = max_width - ellipsis.len();
            let mut shortened: String = last.chars().take(keep).collect();
            shortened.push_str(ellipsis);
            *last = shortened;
        }
    }

    truncated
}

/// Formats a duration without spaces (e.g. `5m04s`) for compact column output.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn format_compact_duration(duration: ChronoDuration) -> String {
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
    use super::*;

    #[test]
    fn truncate_lines_limits_and_appends_ellipsis() {
        let lines = vec!["one".into(), "two".into(), "three".into(), "four".into()];
        let truncated = truncate_lines(lines, 3, 10);

        assert_eq!(truncated.len(), 3);
        assert_eq!(truncated.last().unwrap(), "three...");
    }

    #[test]
    fn format_compact_duration_displays_compact_units() {
        assert_eq!(
            format_compact_duration(ChronoDuration::seconds(0)),
            "0s".to_string()
        );
        assert_eq!(
            format_compact_duration(ChronoDuration::seconds(65)),
            "1m05s".to_string()
        );
        assert_eq!(
            format_compact_duration(ChronoDuration::seconds(3661)),
            "1h01m01s".to_string()
        );
    }
}
