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

The same rule applies to any non-`HydraId` typed newtype that exists for an
identifier — e.g. `RepoName`, `Username`, `ActorId`, `StatusKey`, `LabelName`.
Function signatures should take the typed form (`&RepoName`, `&Username`, …)
rather than raw `&str`. Parse at the boundary (DB rows, CLI input, JSON
deserialization) and thread typed refs internally. See [[p-efmgbyhp]] for the
canonical "parse at the DB-row boundary" example.

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

This rule applies self-consistently to rustdoc `# Arguments` and `# Returns`
stanzas: avoid stanzas that restate parameter or return types, because the
well-named identifiers and signature already convey them. Only include such a
stanza when there is non-obvious context — unit conventions, invariants,
side effects. See [[p-ljunbvev]] (16+/107− diff on
`hydra-server/src/store/mod.rs`) for evidence of the prevalent restate
pattern this rule is meant to prevent.

## Comments are self-contained

A comment must carry its meaning at the point where it sits. Don't defer to
an external doc ("see §3 of the migration design", "see the cutover plan for
why") — external references rot when the target moves, gets renamed, or gets
deleted, and the comment goes stale silently. If a fact matters at the
comment, write it at the comment.

```rust
// wrong: the load-bearing reason lives somewhere else
// See §3 of the migration-testing design for why we skip the cutover branch.
if state.is_cutover() { return Ok(()); }

// correct: the reason is here
// Cutover already applied the migration on the writer; replaying it on the
// reader would double-apply the schema change.
if state.is_cutover() { return Ok(()); }
```

## Prefer methods over free functions

When a function has a natural receiver — an obvious value it operates on —
hang it off that type as a method. `issue.is_blocked()` reads better than
`is_blocked(&issue)`, is IDE-discoverable, doesn't pollute the module
namespace, and lets the impl evolve with the type. Free functions are fine
when there's no natural receiver.

```rust
// wrong
fn issue_is_blocked(issue: &Issue) -> bool { ... }
if issue_is_blocked(&issue) { ... }

// correct
impl Issue {
    fn is_blocked(&self) -> bool { ... }
}
if issue.is_blocked() { ... }
```
