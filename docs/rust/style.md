# Style

Formatting, naming, and small spelling rules that show up in review. See
[idioms.md](idioms.md) for design-level patterns.

## Formatting gate

`cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
must both pass. See [testing.md](testing.md) for the full pre-PR checklist.

## Naming

| Kind | Convention |
|---|---|
| Modules, files, functions, locals | `snake_case` |
| Types, traits, enum variants | `UpperCamelCase` |
| Constants and statics | `SCREAMING_SNAKE_CASE` |

## Identifiers

Use the `HydraId` type alias (and the more specific aliases like `IssueId`,
`PatchId`, `SessionId`) from `hydra-common` for any Hydra entity id. Raw
`String` for an id is wrong — it loses the type-level distinction between an
issue id and a patch id and skips parse-time validation.

```rust
// wrong
fn find_issue(id: String) -> Option<Issue> { ... }

// correct
fn find_issue(id: &IssueId) -> Option<Issue> { ... }
```

## Git operations: libgit2, not the shell

CLI git operations go through the `git2` crate; do not shell out to the `git`
binary. Shelling out depends on the host having `git` installed and on its
PATH, and produces brittle text-parsing code.

```rust
// wrong
Command::new("git").args(["rev-parse", "HEAD"]).output()?;

// correct
let repo = git2::Repository::discover(pwd)?;
let head = repo.head()?.peel_to_commit()?.id();
```

## Environment variables: declare on the arg struct

When a CLI command needs an env var, declare it as an `#[arg(env = ...)]` on
the command's `clap` struct and read the value from the parsed args. Do not
call `std::env::var` inside the command body.

```rust
// wrong
fn run() -> Result<()> {
    let id = std::env::var("HYDRA_ISSUE_ID")?;
    ...
}

// correct
#[derive(Args)]
struct CreateArgs {
    #[arg(long = "current-issue-id", env = ENV_HYDRA_ISSUE_ID)]
    current_issue_id: Option<IssueId>,
}
```

Env-var names live in `hydra-common/src/constants.rs` (so the server and CLI
agree on the literal); CLI-only path constants live in `hydra/src/constants.rs`.

## `///` doc comments

Document only non-obvious public behavior. Don't restate the function name as a
sentence, and don't paraphrase the type signature. Prefer a single line that
captures a non-trivial invariant, error condition, or interaction.

```rust
// wrong: redundant with the signature
/// Returns the issue.
pub fn get_issue(id: &IssueId) -> Option<Issue> { ... }

// correct: documents a non-obvious behavior
/// Returns `None` if the issue was soft-deleted; callers that need deleted
/// rows should use `get_issue_including_deleted`.
pub fn get_issue(id: &IssueId) -> Option<Issue> { ... }
```
