# Testing

Cross-cutting testing rules that apply to all code in this repo regardless of
language. Per-language testing mechanics (commands, harnesses, framework
conventions) live in [rust/testing.md](rust/testing.md) and
[typescript/testing.md](typescript/testing.md).

## TDD is REQUIRED

**Write all new code test-first.**

1. Write a failing test for the behavior you want.
2. Run it; confirm it fails for the right reason.
3. Write the minimum production code to make it pass.
4. Run it; confirm it passes.
5. Refactor only with the test green.

Each change must be just enough to pass the tests that exist. Do not add
functions, branches, parameters, fields, or error handling unless a failing
test requires them. This covers new code, new branches in existing code, and
bug fixes (reproduce the bug as a failing test, then fix).

**The only exception** is code where a test would be prohibitively difficult
to write *and* would have very low value (e.g. trivial glue over a
third-party API). Justify both halves in the PR description.

## Never widen exports for tests

A module's exports are its public API. Never add an export solely because a
test needs it.

If a test needs internal functionality, or would be significantly cleaner
with access to it, split those internals into their own module and test that
module through its real public API. This keeps every export justified by a
real caller.
