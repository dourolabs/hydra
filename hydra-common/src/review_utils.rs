use chrono::{DateTime, Utc};

use crate::api::v1::patches::{PatchVersionRecord, Review};
use crate::principal::{Principal, principal_eq};

impl Review {
    /// Per-review staleness check against a precomputed cutoff timestamp.
    ///
    /// - With `cutoff = None`, every review is non-stale (no `commit_range`
    ///   change ever invalidated reviews).
    /// - With `cutoff = Some(t)`, the review is non-stale iff it has a
    ///   `submitted_at` timestamp at or after `t`. Reviews without a
    ///   `submitted_at` are stale whenever a cutoff exists.
    ///
    /// Shared by [`Review::is_non_stale`] (which derives the cutoff from a
    /// patch version history), [`find_latest_review_by_author`], and
    /// [`has_approved_non_dismissed_review`] so the three cannot drift.
    pub(crate) fn is_non_stale_with_cutoff(&self, cutoff: Option<DateTime<Utc>>) -> bool {
        match cutoff {
            Some(t) => self.submitted_at.is_some_and(|s| s >= t),
            None => true,
        }
    }

    /// Returns `true` iff this review is non-stale against the given patch
    /// version history.
    ///
    /// "Non-stale" means the review has not been invalidated by a change to
    /// the patch's `commit_range` since it was submitted. Specifically: if
    /// [`find_last_commit_range_change_timestamp`] returns `None` for
    /// `patch_versions`, every review is non-stale; otherwise the review
    /// must have a `submitted_at` timestamp at or after that change.
    ///
    /// Shared between the server-side `merge_authorization` restriction
    /// and the CLI preflight so a user sees the same staleness verdict
    /// from either side. The semantics match
    /// [`has_approved_non_dismissed_review`].
    pub fn is_non_stale(&self, patch_versions: &[PatchVersionRecord]) -> bool {
        let cutoff = find_last_commit_range_change_timestamp(patch_versions);
        self.is_non_stale_with_cutoff(cutoff)
    }
}

/// Find the latest non-stale review by a given author (matched
/// case-insensitively on the principal's name). When multiple
/// reviews exist from the same author, the one with the latest
/// `submitted_at` timestamp wins. Reviews without a timestamp are
/// treated as older than any review with a timestamp.
///
/// If `staleness_cutoff` is `Some`, reviews whose `submitted_at` is
/// before the cutoff are considered stale and excluded. Reviews
/// without a `submitted_at` are also considered stale when a cutoff
/// is present.
///
/// Matches `Review.author` against `author` using [`principal_eq`]
/// (kind-aware, case-insensitive on the name segments).
pub fn find_latest_review_by_author<'a>(
    reviews: &'a [Review],
    author: &Principal,
    staleness_cutoff: Option<DateTime<Utc>>,
) -> Option<&'a Review> {
    reviews
        .iter()
        .filter(|r| principal_eq(&r.author, author))
        .filter(|r| r.is_non_stale_with_cutoff(staleness_cutoff))
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
    // Collect unique authors keyed on canonical path form (case-folded
    // for User/Agent/External name segments). Using a String key keeps
    // dedup and lookup agree on case-folded equivalence (matches what
    // `find_latest_review_by_author` uses).
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for review in reviews {
        let key = review.author.canonical_key();
        if !seen_keys.insert(key) {
            continue;
        }
        if let Some(latest) =
            find_latest_review_by_author(reviews, &review.author, staleness_cutoff)
        {
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

    fn user(name: &str) -> Principal {
        Principal::User {
            name: crate::api::v1::users::Username::try_new(name)
                .unwrap_or_else(|_| crate::api::v1::users::Username::from(name.to_string())),
        }
    }

    fn make_review(
        contents: &str,
        is_approved: bool,
        author: &str,
        submitted_at: Option<DateTime<Utc>>,
    ) -> Review {
        Review::new(
            contents.to_string(),
            is_approved,
            user(author),
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
            Vec::new(),
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

        let result = find_latest_review_by_author(&reviews, &user("alice"), None).unwrap();
        assert!(result.is_approved);
        assert_eq!(result.contents, "newer");
    }

    #[test]
    fn find_latest_review_case_insensitive() {
        let reviews = vec![make_review("ok", true, "Alice", Some(Utc::now()))];

        let result = find_latest_review_by_author(&reviews, &user("alice"), None);
        assert!(result.is_some());
        assert!(result.unwrap().is_approved);
    }

    #[test]
    fn find_latest_review_no_match() {
        let reviews = vec![make_review("ok", true, "bob", Some(Utc::now()))];

        let result = find_latest_review_by_author(&reviews, &user("alice"), None);
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

        let result = find_latest_review_by_author(&reviews, &user("alice"), Some(cutoff)).unwrap();
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

        let result = find_latest_review_by_author(&reviews, &user("alice"), Some(cutoff));
        assert!(result.is_none());
    }

    #[test]
    fn find_latest_review_no_timestamp_considered_stale_when_cutoff_present() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![make_review("LGTM", true, "alice", None)];

        let result = find_latest_review_by_author(&reviews, &user("alice"), Some(cutoff));
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

    // --- Unit tests for Review::is_non_stale ---

    fn range_a() -> Option<CommitRange> {
        Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ))
    }

    fn range_b() -> Option<CommitRange> {
        Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ))
    }

    #[test]
    fn is_review_non_stale_true_when_no_submitted_at_and_no_commit_range_changes() {
        let now = Utc::now();
        // Single version → no commit_range change → cutoff is None → every
        // review is non-stale, including those without a `submitted_at`.
        let versions = vec![make_version_record(1, now, range_a())];
        let review = make_review("LGTM", true, "alice", None);
        assert!(review.is_non_stale(&versions));
    }

    #[test]
    fn is_review_non_stale_true_with_empty_version_history() {
        // No versions at all → no commit_range change → cutoff is None.
        let review = make_review("LGTM", true, "alice", Some(Utc::now()));
        assert!(review.is_non_stale(&[]));
    }

    #[test]
    fn is_review_non_stale_false_when_commit_range_changed_after_review() {
        let now = Utc::now();
        let review_ts = now - Duration::hours(2);
        let change_ts = now - Duration::hours(1);
        let versions = vec![
            make_version_record(1, now - Duration::hours(3), range_a()),
            make_version_record(2, change_ts, range_b()),
        ];
        let review = make_review("LGTM", true, "alice", Some(review_ts));
        assert!(!review.is_non_stale(&versions));
    }

    #[test]
    fn is_review_non_stale_true_when_commit_range_changed_before_review() {
        let now = Utc::now();
        let change_ts = now - Duration::hours(2);
        let review_ts = now - Duration::hours(1);
        let versions = vec![
            make_version_record(1, now - Duration::hours(3), range_a()),
            make_version_record(2, change_ts, range_b()),
        ];
        let review = make_review("LGTM", true, "alice", Some(review_ts));
        assert!(review.is_non_stale(&versions));
    }

    #[test]
    fn is_review_non_stale_true_when_review_exactly_at_commit_range_change() {
        // Boundary: equal timestamps count as non-stale (matches the
        // `>= cutoff` semantics of `has_approved_non_dismissed_review`).
        let now = Utc::now();
        let change_ts = now - Duration::hours(1);
        let versions = vec![
            make_version_record(1, now - Duration::hours(2), range_a()),
            make_version_record(2, change_ts, range_b()),
        ];
        let review = make_review("LGTM", true, "alice", Some(change_ts));
        assert!(review.is_non_stale(&versions));
    }

    #[test]
    fn is_review_non_stale_false_when_no_submitted_at_but_commit_range_changed() {
        // A review without a `submitted_at` is stale whenever a cutoff exists.
        let now = Utc::now();
        let versions = vec![
            make_version_record(1, now - Duration::hours(2), range_a()),
            make_version_record(2, now - Duration::hours(1), range_b()),
        ];
        let review = make_review("LGTM", true, "alice", None);
        assert!(!review.is_non_stale(&versions));
    }

    #[test]
    fn is_review_non_stale_matches_has_approved_non_dismissed_review() {
        // The shared cutoff means the two predicates agree on every review
        // for any patch version history.
        let now = Utc::now();
        let change_ts = now - Duration::hours(1);
        let versions = vec![
            make_version_record(1, now - Duration::hours(3), range_a()),
            make_version_record(2, change_ts, range_b()),
        ];
        let cutoff = find_last_commit_range_change_timestamp(&versions);

        let stale = make_review("LGTM", true, "alice", Some(now - Duration::hours(2)));
        let fresh = make_review("LGTM", true, "bob", Some(now));

        // Just the stale approval -> no non-dismissed approval.
        assert!(!stale.is_non_stale(&versions));
        assert!(!has_approved_non_dismissed_review(
            std::slice::from_ref(&stale),
            cutoff,
        ));

        // Add the fresh approval from a different author -> one survives.
        assert!(fresh.is_non_stale(&versions));
        assert!(has_approved_non_dismissed_review(&[stale, fresh], cutoff,));
    }
}
