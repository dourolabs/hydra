pub fn join_api_url(base: &str, path: &str) -> String {
    let trimmed_base = base.trim_end_matches('/');
    let trimmed_path = path.trim_start_matches('/');
    format!("{trimmed_base}/{trimmed_path}")
}

#[cfg(test)]
mod tests {
    use super::join_api_url;

    #[test]
    fn join_api_url_removes_extra_slashes() {
        let url = join_api_url("https://api.metis.local/", "/v1/jobs/");
        assert_eq!(url, "https://api.metis.local/v1/jobs/");
    }

    #[test]
    fn join_api_url_preserves_single_slash() {
        let url = join_api_url("https://api.metis.local", "v1/agents");
        assert_eq!(url, "https://api.metis.local/v1/agents");
    }
}
