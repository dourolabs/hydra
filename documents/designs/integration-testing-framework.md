# Integration Testing Framework Design

## Problem Statement

Metis needs a robust integration testing framework for testing interactions between the `metis` CLI, `metis-server`, and git remotes. The current test suite (`metis/tests/`) works but has several pain points:

1. **Verbosity**: Tests are 50-100+ lines of boilerplate before the interesting logic begins. Creating a GitHub mock server alone takes ~100 lines of setup per test.
2. **No concurrency testing**: Tests run operations sequentially. There's no way to express "user A does X while agent B does Y" and verify all interleavings.
3. **Indirect worker execution**: Workers are invoked via `BashCommands` (a mock of `WorkerCommands`) rather than through the actual scheduling/spawning pipeline, so the real scheduling, task state transitions, and branch management aren't exercised end-to-end.
4. **Scattered helpers**: Test setup is split across `test_helpers.rs`, `bash_commands.rs`, server-side `test_utils/`, and ad-hoc per-test helpers. There's no unified vocabulary for building test scenarios.
5. **Fragile assertions**: Tests assert on string contents of error messages and manually poll for status changes with hardcoded timeouts.

## Goals

- **Concise test authoring**: A test that exercises "create issue, run worker, verify patch" should be ~20 lines, not 70.
- **Concurrency testing**: Express concurrent actions by multiple actors (users, agents) and verify correctness under all interleavings.
- **Maximum realism**: Worker contexts must be set up exactly as in production -- same git branch setup, same env vars, same scheduling pipeline. The only things mocked are Kubernetes (via `MockJobEngine`) and GitHub API (via `httpmock`).
- **Composability**: Common scenarios (issue creation, worker run, patch creation, GitHub review) are reusable building blocks.
- **Deterministic control over async**: Background workers (scheduler, spawner, GitHub poller) should be manually steppable, not running on timers, so tests are deterministic.

## Non-Goals

- Testing the Kubernetes job engine itself (remains mocked via `MockJobEngine`).
- Testing the web UI (`metis-ui`).
- Performance/load testing.
- Testing against a real PostgreSQL database (tests use `MemoryStore`).

## Proposed Approach

### Core Abstraction: `TestHarness`

A single entry point that owns all test infrastructure and exposes a fluent API for building scenarios.

```rust
pub struct TestHarness {
    server: TestServer,
    state: AppState,
    store: Arc<dyn Store>,
    engine: Arc<MockJobEngine>,
    agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>,
    tempdir: TempDir,
    // Pre-configured actors
    users: HashMap<String, UserHandle>,
    // Git remote state
    remotes: HashMap<RepoName, GitRemote>,
    // Optional GitHub mock
    github: Option<GitHubMock>,
}
```

**`TestHarness` provides:**
- Factory method with sensible defaults: `TestHarness::new().await`
- Builder for customization: `TestHarness::builder().with_repo("acme/app").with_github().build().await`
- Named user/agent handles for multi-actor scenarios
- Direct access to `AppState` for manual background worker stepping

### Actor Handles

Each actor (user or agent) gets a handle that encapsulates authentication and provides a typed API:

```rust
pub struct UserHandle {
    name: String,
    client: MetisClient,
    auth_token: String,
}

impl UserHandle {
    // Issue operations
    async fn create_issue(&self, desc: &str) -> Result<IssueId>;
    async fn create_child_issue(&self, parent: &IssueId, desc: &str) -> Result<IssueId>;
    async fn update_issue_status(&self, id: &IssueId, status: IssueStatus) -> Result<()>;
    async fn get_issue(&self, id: &IssueId) -> Result<IssueRecord>;

    // Patch operations
    async fn create_patch(&self, title: &str, desc: &str) -> Result<PatchId>;
    async fn get_patch(&self, id: &PatchId) -> Result<PatchRecord>;

    // Job operations (create via CLI-like API)
    async fn create_job(&self, repo: &RepoName, prompt: &str) -> Result<TaskId>;

    // CLI operations (subprocess, for testing CLI behavior)
    async fn cli(&self, args: &[&str]) -> Result<CliOutput>;
    async fn cli_expect_failure(&self, args: &[&str]) -> Result<CliOutput>;
}
```

The key insight is that `UserHandle` wraps `MetisClient` and pre-fills authentication, issue context, and server URL. Tests never manually construct environment variables or auth tokens.

### Worker Execution

Workers are executed via the real `worker_run::run()` code path, but with `BashCommands` as the `WorkerCommands` implementation. This preserves the full worker lifecycle (git clone, branch setup, env var injection, auto-commit, branch push, patch creation, status update) while replacing only the AI model invocation.

```rust
impl TestHarness {
    /// Run a worker for the given job, executing the provided shell commands
    /// in place of the AI model. Uses the real worker_run pipeline.
    async fn run_worker(
        &self,
        job_id: &TaskId,
        commands: Vec<&str>,
    ) -> Result<WorkerResult>;

    /// Run a worker that is expected to fail
    async fn run_worker_expect_failure(
        &self,
        job_id: &TaskId,
        commands: Vec<&str>,
    ) -> Result<WorkerFailure>;
}

pub struct WorkerResult {
    pub outputs: Vec<CommandOutput>,
    pub patches_created: Vec<PatchId>,
    pub final_status: Status,
}
```

### Background Worker Stepping

Instead of running background workers on timers (which introduces non-determinism), the harness exposes manual stepping:

```rust
impl TestHarness {
    /// Run one iteration of the spawner (finds ready issues, creates tasks)
    async fn step_spawner(&self) -> Result<Vec<TaskId>>;

    /// Run one iteration of the pending job processor (transitions Created -> Pending -> Running)
    async fn step_pending_jobs(&self) -> Result<Vec<TaskId>>;

    /// Run one iteration of the GitHub poller
    async fn step_github_sync(&self) -> Result<()>;

    /// Convenience: step spawner + pending jobs (common pattern)
    async fn step_schedule(&self) -> Result<Vec<TaskId>>;
}
```

This lets tests control exactly when scheduling happens, enabling deterministic interleaving tests.

### Concurrency Testing

For testing concurrent operations, the harness provides a `concurrent` combinator:

```rust
impl TestHarness {
    /// Run multiple async operations concurrently and collect results.
    /// All operations start at the same time via tokio::join!.
    async fn concurrent<T>(
        &self,
        operations: Vec<impl Future<Output = Result<T>>>,
    ) -> Result<Vec<T>>;
}
```

Example usage:

```rust
// Two users updating the same issue concurrently
let results = harness.concurrent(vec![
    user_a.update_issue_status(&issue_id, IssueStatus::InProgress),
    user_b.update_issue_status(&issue_id, IssueStatus::Closed),
]).await?;
```

For testing all interleavings of a sequence of operations, we provide a permutation runner:

```rust
/// Run a test function for every permutation of the provided steps.
/// Each step is a named async closure. The test function receives the harness
/// after all steps in that permutation have executed.
async fn test_all_orderings<F, Fut>(
    steps: Vec<(&str, Box<dyn FnOnce(&TestHarness) -> BoxFuture<'_, Result<()>>>)>,
    verify: F,
) where
    F: Fn(&TestHarness) -> Fut,
    Fut: Future<Output = Result<()>>;
```

This creates a fresh `TestHarness` for each permutation, runs the steps in that order, then calls the verify function. This is feasible because `TestHarness` setup is fast (in-memory store, no I/O beyond tempdir creation).

### GitHub Mock Builder

The current GitHub mock setup is ~100 lines per test. A builder reduces this to ~5 lines:

```rust
pub struct GitHubMockBuilder {
    server: MockServer,
    installations: Vec<MockInstallation>,
}

impl GitHubMockBuilder {
    fn new() -> Self;

    /// Add a repository installation with default mocks
    fn with_installation(self, owner: &str, repo: &str) -> Self;

    /// Configure a PR with specific state
    fn with_pr(self, owner: &str, repo: &str, pr: MockPr) -> Self;

    /// Configure reviews for a PR
    fn with_reviews(self, owner: &str, repo: &str, pr_number: u64, reviews: Vec<MockReview>) -> Self;

    /// Build the mock server and return the Octocrab client
    fn build(self) -> Result<(MockServer, Octocrab)>;
}

pub struct MockPr {
    pub number: u64,
    pub state: &str,       // "open", "closed"
    pub merged: bool,
    pub head_branch: String,
    pub head_sha: String,
}

pub struct MockReview {
    pub author: String,
    pub state: &str,       // "APPROVED", "CHANGES_REQUESTED", "COMMENTED"
    pub body: String,
}
```

### Git Remote Helpers

Simplify git remote setup and verification:

```rust
pub struct GitRemote {
    url: String,
    tempdir: TempDir,
}

impl GitRemote {
    /// Create a bare remote with an initial commit on main
    fn new() -> Result<Self>;

    /// Create a branch with a commit
    fn create_branch(&self, name: &str, file: &str, content: &str) -> Result<String>;

    /// Get the current HEAD SHA of a branch
    fn branch_sha(&self, branch: &str) -> Result<String>;

    /// Check if a branch exists
    fn branch_exists(&self, branch: &str) -> bool;

    /// Get diff between two branches
    fn diff(&self, base: &str, head: &str) -> Result<String>;

    /// Read a file from a specific branch
    fn read_file(&self, branch: &str, path: &str) -> Result<String>;
}
```

### Assertions

Replace fragile string matching with structured assertions:

```rust
pub trait IssueAssertions {
    fn assert_status(&self, expected: IssueStatus);
    fn assert_has_child_with_status(&self, desc_contains: &str, status: IssueStatus);
    fn assert_todo_count(&self, expected: usize);
    fn assert_has_patch(&self);
}

pub trait PatchAssertions {
    fn assert_status(&self, expected: PatchStatus);
    fn assert_review_from(&self, author: &str, state: &str);
    fn assert_diff_contains(&self, text: &str);
}

pub trait JobAssertions {
    fn assert_status(&self, expected: Status);
    fn assert_env_var(&self, key: &str, value: &str);
}

/// Wait for a condition with configurable timeout and polling interval
async fn wait_until<F, Fut>(
    timeout: Duration,
    poll_interval: Duration,
    description: &str,
    condition: F,
) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>;
```

## Key Design Decisions

### 1. Keep using `BashCommands` for worker command execution

**Decision**: Continue using `BashCommands` to replace the AI model, rather than trying to mock at a different level.

**Rationale**: The `WorkerCommands` trait is the natural seam. Everything above it (git setup, branch management, env vars, patch creation, status updates) is exercised with real code. Everything below it (Claude/Codex invocation) is replaced with deterministic shell commands. This gives maximum realism without requiring an AI model in tests.

### 2. In-process server, not subprocess

**Decision**: Continue running `metis-server` in-process via `spawn_test_server_with_state()`.

**Rationale**: In-process gives us direct access to `AppState` for manual stepping of background workers and inspection of internal state. Subprocess would be more realistic but would make deterministic concurrency testing impossible.

### 3. Manual stepping over timer-based background workers

**Decision**: Background workers (spawner, scheduler, GitHub poller) are stepped manually in tests rather than running on timers.

**Rationale**: Timer-based workers introduce non-determinism -- tests become flaky depending on timing. Manual stepping lets tests control exactly when scheduling decisions happen, which is essential for concurrency testing. The trade-off is that tests must explicitly call `step_spawner()`, but this also makes tests more readable (the reader sees exactly what triggers each state transition).

### 4. CLI subprocess invocation preserved for CLI-specific tests

**Decision**: The `UserHandle::cli()` method still invokes the `metis` binary as a subprocess.

**Rationale**: Some tests specifically need to verify CLI behavior (argument parsing, output formatting, env var handling). For these, subprocess invocation is necessary. For tests that only care about business logic (issue status transitions, patch creation), the typed API methods on `UserHandle` (which call `MetisClient` directly) are preferred -- they're faster and produce better error messages.

### 5. Permutation testing for concurrency

**Decision**: Use a permutation-based approach (run all orderings of N steps) rather than randomized property testing.

**Rationale**: With small step counts (2-4 concurrent operations), permutation testing is exhaustive and fast. For N=3, there are only 6 permutations. Each permutation gets a fresh `TestHarness` (cheap -- in-memory store, tempdir). This is simpler to implement than randomized testing and provides stronger guarantees (every ordering tested, not just random samples).

### 6. Single `TestHarness` entry point

**Decision**: One struct owns all test infrastructure, rather than composing separate concerns.

**Rationale**: The current codebase has setup logic scattered across `TestEnvironment`, `TestStateHandles`, `test_state_handles()`, `init_test_server_with_remote()`, etc. A single `TestHarness` with a builder pattern simplifies the API and makes it obvious how to configure tests. Internal composition is still possible -- `TestHarness` delegates to `GitRemote`, `GitHubMockBuilder`, etc.

## Key Files and Directories

### New files

| Path | Purpose |
|------|---------|
| `metis/tests/harness/mod.rs` | `TestHarness` struct, builder, core lifecycle methods |
| `metis/tests/harness/user_handle.rs` | `UserHandle` for actor operations |
| `metis/tests/harness/git_remote.rs` | `GitRemote` helper for git operations |
| `metis/tests/harness/github_mock.rs` | `GitHubMockBuilder` for GitHub API mocking |
| `metis/tests/harness/assertions.rs` | Assertion traits and `wait_until` |
| `metis/tests/harness/concurrency.rs` | `test_all_orderings` and concurrent combinators |

### Modified files

| Path | Change |
|------|--------|
| `metis/tests/common/` | Gradually deprecated as tests migrate to `harness/` |
| `metis-server/src/test_utils/mod.rs` | May need minor additions (e.g., expose `RunSpawnersWorker` construction) |

### Unchanged files

- `metis-server/src/test_utils/job_engine.rs` -- `MockJobEngine` continues as-is
- `metis-server/src/test_utils/store.rs` -- `FailingStore` continues as-is
- `metis/src/worker_commands.rs` -- `WorkerCommands` trait unchanged
- `metis/tests/common/bash_commands.rs` -- `BashCommands` reused within harness

## Example: Before and After

### Before (current style, ~70 lines)

```rust
#[tokio::test]
async fn worker_run_creates_patch_via_override_command() -> Result<()> {
    let env = init_test_server_with_remote("acme/worker-test").await?;
    let prompt = "worker integration patch flow";
    let repo_arg = env.service_repo_name.to_string();
    let server_url = env.server.base_url();

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} --var METIS_ISSUE_ID={} --var {}={} {}",
        repo_arg, server_url, env.current_issue_id, ENV_METIS_TOKEN, env.auth_token, prompt
    )]).await?;

    let job_id = job_id_for_prompt(&env.client, prompt).await?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    env.run_as_worker(
        vec![
            "echo \"worker content\" >> README.md".to_string(),
            "git add README.md".to_string(),
            "git commit -m \"worker changes\"".to_string(),
            "metis patches create --title \"integration worker patch\" --description \"created by worker\"".to_string(),
        ],
        job_id.clone(),
    ).await?;

    let patches = env.client.list_patches(&SearchPatchesQuery::new(None, None)).await?.patches;
    let patch = patches.iter()
        .find(|p| !p.patch.is_automatic_backup)
        .ok_or_else(|| anyhow!("expected patch"))?;
    assert_eq!(patch.patch.title, "integration worker patch");

    let jobs = env.client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
    let status = jobs.iter().find(|j| j.id == job_id).unwrap().task.status;
    assert_eq!(status, Status::Complete);
    Ok(())
}
```

### After (proposed style, ~20 lines)

```rust
#[tokio::test]
async fn worker_creates_patch() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/worker-test")
        .build().await?;
    let user = harness.default_user();

    let issue_id = user.create_issue("worker patch flow").await?;
    let tasks = harness.step_schedule().await?;
    assert_eq!(tasks.len(), 1);

    let result = harness.run_worker(&tasks[0], vec![
        "echo 'worker content' >> README.md",
        "git add README.md",
        "git commit -m 'worker changes'",
        "metis patches create --title 'integration worker patch' --description 'created by worker'",
    ]).await?;

    assert_eq!(result.final_status, Status::Complete);
    assert_eq!(result.patches_created.len(), 1);

    let patch = user.get_patch(&result.patches_created[0]).await?;
    patch.assert_status(PatchStatus::Open);
    assert_eq!(patch.patch.title, "integration worker patch");
    Ok(())
}
```

### Concurrency example

```rust
#[tokio::test]
async fn concurrent_issue_updates_are_safe() -> Result<()> {
    test_all_orderings(
        vec![
            ("user creates child", |h| Box::pin(async move {
                let user = h.default_user();
                user.create_child_issue(&h.root_issue(), "child 1").await?;
                Ok(())
            })),
            ("agent creates child", |h| Box::pin(async move {
                let agent = h.user("agent");
                agent.create_child_issue(&h.root_issue(), "child 2").await?;
                Ok(())
            })),
            ("user updates status", |h| Box::pin(async move {
                let user = h.default_user();
                user.update_issue_status(&h.root_issue(), IssueStatus::InProgress).await?;
                Ok(())
            })),
        ],
        |h| async move {
            let issue = h.default_user().get_issue(&h.root_issue()).await?;
            // Both children should exist regardless of ordering
            assert_eq!(issue.children.len(), 2);
            Ok(())
        },
    ).await
}
```

## Risks and Open Questions

1. **Worker run imports**: The `run()` function in `worker_run.rs` is currently a free function that takes many parameters. The harness needs to call it with the right setup. This may require refactoring `run()` to accept a context struct, or the harness may need to replicate some of the parameter assembly logic. Either way, this is a minor code change.

2. **CLI binary availability in tests**: The `CARGO_BIN_EXE_metis` approach for subprocess CLI invocation requires that the binary is built. This is already the case for integration tests (Cargo builds it automatically), but it means `cargo test --lib` won't build the binary. This is the existing behavior and is acceptable.

3. **Permutation test scalability**: For N steps, there are N! permutations. Beyond N=5 (120 permutations), each with full harness setup, this could get slow. Recommendation: keep permutation tests to 2-4 steps. For larger scenarios, use targeted orderings rather than exhaustive permutation.

4. **Migration strategy**: Existing tests should be migrated incrementally. The harness can coexist with the current `common/test_helpers.rs` infrastructure. New tests use the harness; old tests are migrated as they are touched.

5. **Background worker exposure**: The `RunSpawnersWorker` and `ProcessPendingJobsWorker` constructors may need to be made `pub` or gated behind `#[cfg(feature = "test-utils")]` so the harness can construct them. This is a small change in `metis-server`.

6. **`metis patches create` from worker context**: Currently, the `BashCommands` approach runs `metis patches create` as a subprocess, which works because the binary is available. This should continue to work. However, the harness needs to ensure the correct environment variables are set (METIS_SERVER_URL, METIS_TOKEN, METIS_ISSUE_ID) for the subprocess -- this is already handled by `worker_run::run()`.
