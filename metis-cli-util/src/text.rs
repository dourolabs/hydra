/// Truncates a vector of wrapped lines to a maximum count, appending an ellipsis to the final line
/// when truncation occurs.
pub fn truncate_lines(lines: Vec<String>, max_lines: usize, max_width: usize) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::truncate_lines;

    #[test]
    fn truncates_and_appends_ellipsis() {
        let lines = vec![
            "line1".to_string(),
            "line2".to_string(),
            "line3".to_string(),
            "line4".to_string(),
        ];

        let truncated = truncate_lines(lines, 2, 10);

        assert_eq!(truncated.len(), 2);
        assert_eq!(truncated[0], "line1");
        assert_eq!(truncated[1], "line2...");
    }

    #[test]
    fn does_not_truncate_when_not_needed() {
        let lines = vec!["line1".to_string()];

        let truncated = truncate_lines(lines.clone(), 2, 10);

        assert_eq!(truncated, lines);
    }
}
