#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use super::TestHarness;

/// The type of an async action that borrows a `TestHarness`.
type StepAction = Box<dyn FnOnce(&TestHarness) -> Pin<Box<dyn Future<Output = Result<()>> + '_>>>;

/// A named step for permutation testing.
///
/// Each step has a descriptive name (for error messages) and an async closure
/// that operates on a `TestHarness`. Steps are run in every possible ordering
/// by [`test_all_orderings`].
pub struct Step {
    pub name: String,
    pub action: StepAction,
}

impl Step {
    /// Create a new named step from an async closure.
    ///
    /// # Example
    ///
    /// ```ignore
    /// Step::new("create issue", |h| Box::pin(async move {
    ///     h.default_user().create_issue("test").await?;
    ///     Ok(())
    /// }))
    /// ```
    pub fn new<F>(name: &str, action: F) -> Self
    where
        F: FnOnce(&TestHarness) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> + 'static,
    {
        Self {
            name: name.to_string(),
            action: Box::new(action),
        }
    }
}

/// Generate all permutations of indices `0..n`.
///
/// Uses Heap's algorithm. Panics if `n > 6` (720 permutations) to guard
/// against accidentally running an excessive number of test iterations.
fn generate_permutations(n: usize) -> Vec<Vec<usize>> {
    assert!(
        n <= 6,
        "test_all_orderings: too many steps ({n}). Maximum is 6 (720 permutations). \
         Reduce the number of steps or use targeted orderings instead."
    );

    let mut result = Vec::new();
    let mut indices: Vec<usize> = (0..n).collect();
    let mut c = vec![0usize; n];

    result.push(indices.clone());

    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                indices.swap(0, i);
            } else {
                indices.swap(c[i], i);
            }
            result.push(indices.clone());
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }

    result
}

/// Run a verify function for every permutation of the provided steps.
///
/// Each permutation gets a fresh [`TestHarness`] (cheap — in-memory store),
/// runs the steps in that permutation's order, then calls the verify function.
///
/// This is the primary tool for testing concurrent operation orderings. For
/// N steps, N! permutations are tested (2 steps = 2, 3 steps = 6, etc.).
///
/// # Panics
///
/// Panics if `steps` has more than 6 elements (720 permutations).
///
/// # Example
///
/// ```ignore
/// test_all_orderings(
///     || vec![
///         Step::new("user creates child", |h| Box::pin(async move {
///             h.default_user().create_child_issue(&parent, "child 1").await?;
///             Ok(())
///         })),
///         Step::new("agent creates child", |h| Box::pin(async move {
///             h.user("agent").create_child_issue(&parent, "child 2").await?;
///             Ok(())
///         })),
///     ],
///     |h| Box::pin(async move {
///         let issues = h.default_user().list_issues().await?;
///         assert_eq!(issues.issues.len(), 3); // parent + 2 children
///         Ok(())
///     }),
/// ).await?;
/// ```
pub async fn test_all_orderings<S, V>(make_steps: S, verify: V) -> Result<()>
where
    S: Fn() -> Vec<Step>,
    V: for<'a> Fn(&'a TestHarness) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>,
{
    // Determine number of steps from a probe call.
    let probe = make_steps();
    let n = probe.len();
    // Drop the probe — steps contain FnOnce closures that we can't reuse.
    drop(probe);

    let permutations = generate_permutations(n);
    let total = permutations.len();

    for (perm_idx, ordering) in permutations.iter().enumerate() {
        // Create fresh steps for this permutation.
        let steps = make_steps();

        // Collect step names before consuming the steps.
        let all_names: Vec<String> = steps.iter().map(|s| s.name.clone()).collect();
        let step_names: Vec<&str> = ordering.iter().map(|&i| all_names[i].as_str()).collect();
        let ordering_desc = step_names.join(" -> ");

        // Create a fresh harness for this permutation.
        let harness = TestHarness::new().await.map_err(|e| {
            anyhow::anyhow!(
                "permutation {}/{}: failed to create TestHarness: {}",
                perm_idx + 1,
                total,
                e
            )
        })?;

        // Take ownership of each step's action. Since FnOnce closures can only
        // be called once, we wrap them in Option and take them in order.
        let mut actions: Vec<Option<StepAction>> =
            steps.into_iter().map(|s| Some(s.action)).collect();

        for (step_num, &idx) in ordering.iter().enumerate() {
            let action = actions[idx].take().expect("step already consumed");
            action(&harness).await.map_err(|e| {
                anyhow::anyhow!(
                    "permutation {}/{} [{}]: step {} '{}' failed: {}",
                    perm_idx + 1,
                    total,
                    ordering_desc,
                    step_num + 1,
                    step_names[step_num],
                    e
                )
            })?;
        }

        // Run verification.
        verify(&harness).await.map_err(|e| {
            anyhow::anyhow!(
                "permutation {}/{} [{}]: verification failed: {}",
                perm_idx + 1,
                total,
                ordering_desc,
                e
            )
        })?;
    }

    Ok(())
}

/// Run multiple async operations concurrently and collect results.
///
/// All futures are polled concurrently on the current task using
/// `FuturesUnordered`. This works with `!Send` futures (which is common
/// in tests that hold references to `TestHarness`).
///
/// Note: the results are returned in completion order, not submission order.
///
/// # Example
///
/// ```ignore
/// let results = concurrent(vec![
///     Box::pin(async { harness.default_user().create_issue("issue 1").await }),
///     Box::pin(async { harness.default_user().create_issue("issue 2").await }),
/// ]).await?;
/// assert_eq!(results.len(), 2);
/// ```
pub async fn concurrent<'a, T>(
    futures: Vec<Pin<Box<dyn Future<Output = Result<T>> + 'a>>>,
) -> Result<Vec<T>> {
    use futures::stream::{FuturesUnordered, StreamExt};
    let mut unordered: FuturesUnordered<_> = futures.into_iter().collect();

    let mut results = Vec::new();
    while let Some(result) = unordered.next().await {
        results.push(result?);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permutations_of_0() {
        let perms = generate_permutations(0);
        assert_eq!(perms.len(), 1); // One empty permutation
        assert_eq!(perms[0], Vec::<usize>::new());
    }

    #[test]
    fn permutations_of_1() {
        let perms = generate_permutations(1);
        assert_eq!(perms.len(), 1);
        assert_eq!(perms[0], vec![0]);
    }

    #[test]
    fn permutations_of_2() {
        let perms = generate_permutations(2);
        assert_eq!(perms.len(), 2);
        let mut sorted = perms.clone();
        sorted.sort();
        assert_eq!(sorted, vec![vec![0, 1], vec![1, 0]]);
    }

    #[test]
    fn permutations_of_3() {
        let perms = generate_permutations(3);
        assert_eq!(perms.len(), 6);
        let mut sorted = perms.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec![
                vec![0, 1, 2],
                vec![0, 2, 1],
                vec![1, 0, 2],
                vec![1, 2, 0],
                vec![2, 0, 1],
                vec![2, 1, 0],
            ]
        );
    }

    #[test]
    fn permutations_of_6() {
        let perms = generate_permutations(6);
        assert_eq!(perms.len(), 720);
        // Verify all permutations are unique.
        let mut sorted = perms.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 720);
    }

    #[test]
    #[should_panic(expected = "too many steps (7)")]
    fn permutations_of_7_panics() {
        generate_permutations(7);
    }
}
