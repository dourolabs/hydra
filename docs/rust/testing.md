# Testing

Cross-cutting testing rules (TDD-first, never widen exports for tests) live
in [../testing.md](../testing.md) and apply here as well.

## The pre-PR gate

All three must pass before opening a PR:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`-D warnings` is on by design — fix the lints, don't `#[allow]` them.

## Async tests

Use `#[tokio::test]` for async code and give tests descriptive names that read
as a sentence about behavior. The test name is what shows up in a failure
report; "test_1" is useless there.

```rust
// wrong
#[tokio::test]
async fn test_logs() { ... }

// correct
#[tokio::test]
async fn logs_returns_latest_chunks() { ... }
```

## Regression test per bug fix

Every bug fix lands with a test that fails before the fix and passes after.
This is non-negotiable — without it the same regression returns the next time
someone refactors the surrounding code. Cover new branches you introduce in
the same PR (especially job-state transitions and Kubernetes interactions).

## Don't test third-party libraries

Don't write a test whose only assertion is "this well-used library behaves
the way its docs say." The library author already has those tests.

| Don't write | Why not |
|---|---|
| `serde` round-trip on a derived type (`from_str(to_string(x)) == x`) | Tests serde, not your wire contract. |
| `chrono` parse-of-format symmetry | Tests chrono. |
| `Uuid::parse_str(uuid.to_string())` | Tests `uuid`. |

Useful coverage at the *same* boundary:

- **Wire-format shape tests** that assert specific JSON tag literals — the
  wire format is *our* contract, not serde's.
- **`ts-rs` export tests** — they verify *our* type-export pipeline.
- **Integration tests** that drive the type through a real codepath (HTTP
  route, WS frame, DB write/read).

## Integration tests use `worker_run` + the hydra CLI

End-to-end tests must drive the system the way a real agent does — invoking
the `hydra` CLI inside a `worker_run` harness. Don't shortcut to internal
APIs from the test body; status transitions (e.g. setting an issue to
`Failed`) happen via the CLI inside a worker, not via direct store calls.
When testing failure/rejection cascades, include dependent issues
(blocked-on, children) so cascade behavior is actually exercised.

## `HydraClient` forward-compatibility

When you add a method to `HydraClient`, add forward-compatibility coverage in
`hydra/tests/hydra_client_forward_compat.rs`. The test asserts that the
client tolerates new enum variants and extra fields in server responses, so
an older client doesn't crash against a newer server.

See [style.md](style.md) for naming and [errors-and-logging.md](errors-and-logging.md)
for what tests should assert about error paths.
