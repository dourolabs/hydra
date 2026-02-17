mod harness;

use anyhow::Result;
use harness::{test_all_orderings, IssueAssertions, Step};
use metis_common::issues::IssueStatus;

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

/// Scenario 9: Concurrent issue creation (ordering safety).
///
/// Tests that multiple actors creating children and updating issues
/// concurrently produces a consistent final state regardless of ordering.
///
/// Three concurrent steps:
///   - Step A: user-a creates child issue "task from A"
///   - Step B: user-b creates child issue "task from B"
///   - Step C: user-a updates parent status to in-progress
///
/// Runs all 3! = 6 permutations via `test_all_orderings`.
///
/// Verifies after each permutation:
///   - Parent has exactly 2 children
///   - Both children exist with correct descriptions
///   - Parent is in-progress
///   - No duplicate children, no lost updates
#[tokio::test]
async fn concurrent_child_creation_and_parent_update_all_orderings() -> Result<()> {
    test_all_orderings(
        || {
            vec![
                Step::new("user-a creates child A", |h| {
                    Box::pin(async move {
                        let user = h.default_user();
                        let parent_id = find_or_create_issue(user, "concurrent parent").await?;
                        user.create_child_issue(&parent_id, "task from A").await?;
                        Ok(())
                    })
                }),
                Step::new("user-b creates child B", |h| {
                    Box::pin(async move {
                        let user = h.default_user();
                        let parent_id = find_or_create_issue(user, "concurrent parent").await?;
                        user.create_child_issue(&parent_id, "task from B").await?;
                        Ok(())
                    })
                }),
                Step::new("user-a updates parent to in-progress", |h| {
                    Box::pin(async move {
                        let user = h.default_user();
                        let parent_id = find_or_create_issue(user, "concurrent parent").await?;
                        user.update_issue_status(&parent_id, IssueStatus::InProgress)
                            .await?;
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
                    .find(|i| i.issue.description == "concurrent parent")
                    .expect("parent issue should exist");

                // Verify parent is in-progress.
                parent.assert_status(IssueStatus::InProgress);

                // Verify both children exist.
                parent.assert_has_child_with_status(
                    &issues.issues,
                    "task from A",
                    IssueStatus::Open,
                );
                parent.assert_has_child_with_status(
                    &issues.issues,
                    "task from B",
                    IssueStatus::Open,
                );

                // Verify exactly 2 children (no duplicates).
                let children_count = issues
                    .issues
                    .iter()
                    .filter(|i| {
                        i.issue.dependencies.iter().any(|d| {
                            d.dependency_type == metis_common::issues::IssueDependencyType::ChildOf
                                && d.issue_id == parent.issue_id
                        })
                    })
                    .count();
                assert_eq!(
                    children_count, 2,
                    "expected exactly 2 children, got {children_count}"
                );

                Ok(())
            })
        },
    )
    .await?;

    Ok(())
}
