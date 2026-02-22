mod harness;

use anyhow::Result;
use harness::{concurrent, test_all_orderings, IssueSummaryAssertions, Step};
use metis_common::issues::IssueStatus;

/// `test_all_orderings` with 2 steps runs exactly 2 permutations (AB, BA).
#[tokio::test]
async fn test_all_orderings_two_steps_runs_two_permutations() -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_verify = counter.clone();

    test_all_orderings(
        {
            let counter = counter.clone();
            move || {
                let c1 = counter.clone();
                let c2 = counter.clone();
                vec![
                    Step::new("step A", move |h| {
                        Box::pin(async move {
                            let _ = h.default_user().create_issue("issue A").await?;
                            c1.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    }),
                    Step::new("step B", move |h| {
                        Box::pin(async move {
                            let _ = h.default_user().create_issue("issue B").await?;
                            c2.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    }),
                ]
            }
        },
        |h| {
            Box::pin(async move {
                let issues = h.default_user().list_issues().await?;
                assert_eq!(issues.issues.len(), 2);
                Ok(())
            })
        },
    )
    .await?;

    // 2 permutations * 2 steps each = 4 step executions total.
    assert_eq!(counter_verify.load(Ordering::SeqCst), 4);

    Ok(())
}

/// `test_all_orderings` with 3 steps runs exactly 6 permutations.
#[tokio::test]
async fn test_all_orderings_three_steps_runs_six_permutations() -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_verify = counter.clone();

    test_all_orderings(
        {
            let counter = counter.clone();
            move || {
                let c1 = counter.clone();
                let c2 = counter.clone();
                let c3 = counter.clone();
                vec![
                    Step::new("step A", move |h| {
                        Box::pin(async move {
                            let _ = h.default_user().create_issue("issue A").await?;
                            c1.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    }),
                    Step::new("step B", move |h| {
                        Box::pin(async move {
                            let _ = h.default_user().create_issue("issue B").await?;
                            c2.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    }),
                    Step::new("step C", move |h| {
                        Box::pin(async move {
                            let _ = h.default_user().create_issue("issue C").await?;
                            c3.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    }),
                ]
            }
        },
        |h| {
            Box::pin(async move {
                let issues = h.default_user().list_issues().await?;
                assert_eq!(issues.issues.len(), 3);
                Ok(())
            })
        },
    )
    .await?;

    // 6 permutations * 3 steps each = 18 step executions total.
    assert_eq!(counter_verify.load(Ordering::SeqCst), 18);

    Ok(())
}

/// Each permutation gets a fresh TestHarness (no state leaks between runs).
#[tokio::test]
async fn test_all_orderings_fresh_harness_per_permutation() -> Result<()> {
    test_all_orderings(
        || {
            vec![
                Step::new("create issue", |h| {
                    Box::pin(async move {
                        h.default_user().create_issue("only issue").await?;
                        Ok(())
                    })
                }),
                Step::new("check isolation", |h| {
                    Box::pin(async move {
                        // Before this step runs, there should be at most 1 issue
                        // (the one created by "create issue" if it ran first).
                        // There should never be issues left over from a previous
                        // permutation.
                        let issues = h.default_user().list_issues().await?;
                        assert!(
                            issues.issues.len() <= 1,
                            "expected at most 1 issue (fresh harness), got {}",
                            issues.issues.len()
                        );
                        Ok(())
                    })
                }),
            ]
        },
        |_h| Box::pin(async move { Ok(()) }),
    )
    .await?;

    Ok(())
}

/// `concurrent` runs both operations and returns both results.
#[tokio::test]
async fn concurrent_runs_both_operations() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let results = concurrent(vec![
        Box::pin(user.create_issue("concurrent issue 1")),
        Box::pin(user.create_issue("concurrent issue 2")),
    ])
    .await?;

    assert_eq!(results.len(), 2, "concurrent should return 2 results");

    // Both issues should exist in the store.
    let issues = user.list_issues().await?;
    assert_eq!(issues.issues.len(), 2);

    Ok(())
}

/// Helper: find the first issue matching a description, or create it.
async fn find_or_create_issue(
    user: &harness::UserHandle,
    description: &str,
) -> Result<metis_common::IssueId> {
    let issues = user.list_issues().await?;
    if let Some(existing) = issues
        .issues
        .iter()
        .find(|i| i.issue.description == description)
    {
        Ok(existing.issue_id.clone())
    } else {
        user.create_issue(description).await
    }
}

/// Integration test: two actors creating children concurrently — both children
/// exist regardless of ordering.
#[tokio::test]
async fn concurrent_child_creation_all_orderings() -> Result<()> {
    test_all_orderings(
        || {
            vec![
                Step::new("user creates child A", |h| {
                    Box::pin(async move {
                        let user = h.default_user();
                        let parent_id = find_or_create_issue(user, "parent").await?;
                        user.create_child_issue(&parent_id, "child A").await?;
                        Ok(())
                    })
                }),
                Step::new("user creates child B", |h| {
                    Box::pin(async move {
                        let user = h.default_user();
                        let parent_id = find_or_create_issue(user, "parent").await?;
                        user.create_child_issue(&parent_id, "child B").await?;
                        Ok(())
                    })
                }),
            ]
        },
        |h| {
            Box::pin(async move {
                let user = h.default_user();
                let issues = user.list_issues().await?;

                // Find the parent issue.
                let parent = issues
                    .issues
                    .iter()
                    .find(|i| i.issue.description == "parent")
                    .expect("parent issue should exist");

                // Both children should exist regardless of ordering.
                parent.assert_has_child_with_status(&issues.issues, "child A", IssueStatus::Open);
                parent.assert_has_child_with_status(&issues.issues, "child B", IssueStatus::Open);

                Ok(())
            })
        },
    )
    .await?;

    Ok(())
}

/// `test_all_orderings` with 0 steps runs the verify function once
/// (1 empty permutation).
#[tokio::test]
async fn test_all_orderings_zero_steps() -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let verify_count = Arc::new(AtomicUsize::new(0));
    let verify_count_clone = verify_count.clone();

    test_all_orderings(Vec::new, move |_h| {
        let count = verify_count_clone.clone();
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    })
    .await?;

    assert_eq!(verify_count.load(Ordering::SeqCst), 1);

    Ok(())
}
