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
}
