use chrono::{DateTime, Utc};

use crate::api::v1::patches::{PatchVersionRecord, Review};

/// Find the latest non-stale review by a given author (case-insensitive match).
/// When multiple reviews exist from the same author, the one with the latest
/// `submitted_at` timestamp wins. Reviews without a timestamp are treated
/// as older than any review with a timestamp.
///
/// If `staleness_cutoff` is `Some`, reviews whose `submitted_at` is before the
/// cutoff are considered stale and excluded. Reviews without a `submitted_at`
/// are also considered stale when a cutoff is present.
pub fn find_latest_review_by_author<'a>(
    reviews: &'a [Review],
    author: &str,
    staleness_cutoff: Option<DateTime<Utc>>,
) -> Option<&'a Review> {
    reviews
        .iter()
        .filter(|r| r.author.eq_ignore_ascii_case(author))
        .filter(|r| {
            // If there is a staleness cutoff, only keep reviews submitted at or after it.
            // Reviews without a submitted_at timestamp are considered stale when a cutoff exists.
            match staleness_cutoff {
                Some(cutoff) => r.submitted_at.is_some_and(|t| t >= cutoff),
                None => true,
            }
        })
        .max_by(|a, b| {
            // Reviews with submitted_at are always newer than those without
            match (&a.submitted_at, &b.submitted_at) {
                (Some(a_time), Some(b_time)) => a_time.cmp(b_time),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                // If neither has a timestamp, use position (later in vec = newer)
                (None, None) => std::cmp::Ordering::Less,
            }
        })
}

/// Finds the timestamp of the last version where the patch's `commit_range` changed.
///
/// Walks the version history in order and returns the timestamp of the most
/// recent version where `commit_range` differs from the previous version.
/// Returns `None` if there is only one version or the `commit_range` never changed.
pub fn find_last_commit_range_change_timestamp(
    versions: &[PatchVersionRecord],
) -> Option<DateTime<Utc>> {
    let mut last_change_timestamp = None;
    for window in versions.windows(2) {
        let prev = &window[0];
        let curr = &window[1];
        if curr.patch.commit_range != prev.patch.commit_range {
            last_change_timestamp = Some(curr.timestamp);
        }
    }
    last_change_timestamp
}

/// Returns `true` if there is at least one approved review that is not
/// dismissed/stale according to the given staleness cutoff.
///
/// This checks across ALL reviewers. A review is considered non-stale if:
/// - There is no staleness cutoff, OR
/// - The review has a `submitted_at` timestamp at or after the cutoff.
///
/// For each author with a non-stale approved review, we verify that their
/// *latest* non-stale review is still approving (i.e., they haven't submitted
/// a newer non-approving review that supersedes the approval).
pub fn has_approved_non_dismissed_review(
    reviews: &[Review],
    staleness_cutoff: Option<DateTime<Utc>>,
) -> bool {
    // Collect unique authors (case-insensitive)
    let authors: std::collections::HashSet<String> = reviews
        .iter()
        .map(|r| r.author.to_ascii_lowercase())
        .collect();

    for author in &authors {
        if let Some(latest) = find_latest_review_by_author(reviews, author, staleness_cutoff) {
            if latest.is_approved {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::v1::patches::{
        CommitRange, GitOid, Patch, PatchStatus, PatchVersionRecord, Review,
    };
    use crate::versioning::VersionNumber;
    use crate::{PatchId, RepoName};
    use chrono::Duration;
    use std::str::FromStr;

    fn make_review(
        contents: &str,
        is_approved: bool,
        author: &str,
        submitted_at: Option<DateTime<Utc>>,
    ) -> Review {
        Review::new(
            contents.to_string(),
            is_approved,
            author.to_string(),
            submitted_at,
        )
    }

    fn make_api_patch(commit_range: Option<CommitRange>) -> Patch {
        Patch::new(
            "test patch".to_string(),
            "desc".to_string(),
            String::new(),
            PatchStatus::Open,
            false,
            None,
            "test-creator".into(),
            vec![],
            RepoName::new("test", "repo").unwrap(),
            None,
            false,
            None,
            commit_range,
            None,
        )
    }

    fn make_version_record(
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        commit_range: Option<CommitRange>,
    ) -> PatchVersionRecord {
        PatchVersionRecord::new(
            PatchId::new(),
            version,
            timestamp,
            make_api_patch(commit_range),
            None,
            timestamp,
        )
    }

    // --- Unit tests for find_latest_review_by_author ---

    #[test]
    fn find_latest_review_picks_most_recent() {
        let now = Utc::now();
        let reviews = vec![
            make_review("old", false, "alice", Some(now - Duration::hours(3))),
            make_review("newer", true, "alice", Some(now - Duration::hours(1))),
            make_review("other", true, "bob", Some(now)),
        ];

        let result = find_latest_review_by_author(&reviews, "alice", None).unwrap();
        assert!(result.is_approved);
        assert_eq!(result.contents, "newer");
    }

    #[test]
    fn find_latest_review_case_insensitive() {
        let reviews = vec![make_review("ok", true, "Alice", Some(Utc::now()))];

        let result = find_latest_review_by_author(&reviews, "alice", None);
        assert!(result.is_some());
        assert!(result.unwrap().is_approved);
    }

    #[test]
    fn find_latest_review_no_match() {
        let reviews = vec![make_review("ok", true, "bob", Some(Utc::now()))];

        let result = find_latest_review_by_author(&reviews, "alice", None);
        assert!(result.is_none());
    }

    #[test]
    fn find_latest_review_filters_stale_reviews() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![
            // Review before the cutoff (stale)
            make_review("old LGTM", true, "alice", Some(now - Duration::hours(2))),
            // Review after the cutoff (fresh)
            make_review("changes needed", false, "alice", Some(now)),
        ];

        let result = find_latest_review_by_author(&reviews, "alice", Some(cutoff)).unwrap();
        assert!(!result.is_approved);
        assert_eq!(result.contents, "changes needed");
    }

    #[test]
    fn find_latest_review_all_stale_returns_none() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![make_review(
            "old LGTM",
            true,
            "alice",
            Some(now - Duration::hours(2)),
        )];

        let result = find_latest_review_by_author(&reviews, "alice", Some(cutoff));
        assert!(result.is_none());
    }

    #[test]
    fn find_latest_review_no_timestamp_considered_stale_when_cutoff_present() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![make_review("LGTM", true, "alice", None)];

        let result = find_latest_review_by_author(&reviews, "alice", Some(cutoff));
        assert!(result.is_none());
    }

    // --- Unit tests for find_last_commit_range_change_timestamp ---

    #[test]
    fn commit_range_change_timestamp_single_version() {
        let now = Utc::now();
        let versions = vec![make_version_record(1, now, None)];

        assert_eq!(find_last_commit_range_change_timestamp(&versions), None);
    }

    #[test]
    fn commit_range_change_timestamp_no_change() {
        let now = Utc::now();
        let range = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let versions = vec![
            make_version_record(1, now - Duration::hours(2), range.clone()),
            make_version_record(2, now - Duration::hours(1), range),
        ];

        assert_eq!(find_last_commit_range_change_timestamp(&versions), None);
    }

    #[test]
    fn commit_range_change_timestamp_detects_change() {
        let now = Utc::now();
        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));
        let change_ts = now - Duration::hours(1);
        let versions = vec![
            make_version_record(1, now - Duration::hours(2), range_v1),
            make_version_record(2, change_ts, range_v2),
        ];

        assert_eq!(
            find_last_commit_range_change_timestamp(&versions),
            Some(change_ts)
        );
    }

    #[test]
    fn commit_range_change_timestamp_picks_last_change() {
        let now = Utc::now();
        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));
        let range_v3 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("dddddddddddddddddddddddddddddddddddddddd").unwrap(),
        ));
        let ts_second_change = now - Duration::minutes(30);
        let versions = vec![
            make_version_record(1, now - Duration::hours(2), range_v1),
            make_version_record(2, now - Duration::hours(1), range_v2),
            make_version_record(3, ts_second_change, range_v3),
        ];

        assert_eq!(
            find_last_commit_range_change_timestamp(&versions),
            Some(ts_second_change)
        );
    }

    // --- Unit tests for has_approved_non_dismissed_review ---

    #[test]
    fn has_approved_review_with_no_staleness() {
        let reviews = vec![make_review("LGTM", true, "alice", Some(Utc::now()))];

        assert!(has_approved_non_dismissed_review(&reviews, None));
    }

    #[test]
    fn no_approved_review() {
        let reviews = vec![make_review(
            "changes needed",
            false,
            "alice",
            Some(Utc::now()),
        )];

        assert!(!has_approved_non_dismissed_review(&reviews, None));
    }

    #[test]
    fn empty_reviews() {
        assert!(!has_approved_non_dismissed_review(&[], None));
    }

    #[test]
    fn approved_review_is_stale() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![make_review(
            "LGTM",
            true,
            "alice",
            Some(now - Duration::hours(2)),
        )];

        assert!(!has_approved_non_dismissed_review(&reviews, Some(cutoff)));
    }

    #[test]
    fn approved_review_superseded_by_non_approving() {
        let now = Utc::now();
        let reviews = vec![
            make_review("LGTM", true, "alice", Some(now - Duration::hours(2))),
            make_review(
                "actually, changes needed",
                false,
                "alice",
                Some(now - Duration::hours(1)),
            ),
        ];

        assert!(!has_approved_non_dismissed_review(&reviews, None));
    }

    #[test]
    fn one_approver_one_non_approver() {
        let now = Utc::now();
        let reviews = vec![
            make_review("LGTM", true, "alice", Some(now)),
            make_review("changes needed", false, "bob", Some(now)),
        ];

        assert!(has_approved_non_dismissed_review(&reviews, None));
    }

    #[test]
    fn fresh_approval_among_stale_reviews() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![
            // Stale approval from alice
            make_review("LGTM", true, "alice", Some(now - Duration::hours(2))),
            // Fresh approval from bob
            make_review("LGTM", true, "bob", Some(now)),
        ];

        assert!(has_approved_non_dismissed_review(&reviews, Some(cutoff)));
    }

    #[test]
    fn approval_without_timestamp_stale_when_cutoff_present() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![make_review("LGTM", true, "alice", None)];

        assert!(!has_approved_non_dismissed_review(&reviews, Some(cutoff)));
    }
}
