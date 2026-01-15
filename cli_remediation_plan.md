# CLI remediation plan (Jayant feedback)

Scope: Address the HIGH and MEDIUM items from the CLI audit; LOW items are intentionally deferred.

- [HIGH] Align jobs create git revision help with default behavior (`i-weuneg`)
  - Fix the mismatch where help says `--rev` is required for git URLs but the CLI defaults to `main`; either enforce the requirement or update help/docs to match the default with consistent errors.
  - Add a regression test for the updated help or flag handling; document any behavior changes.
- [MEDIUM] Add machine-readable output to `jobs list` (`i-omfjdd`)
  - Introduce a structured/JSON flag aligned with `issues list` and `patches list`, including fields for id, status, timestamps, and paging.
  - Keep the default human output unchanged; add tests for populated and empty results.
- [MEDIUM] Clean up `patches list` empty-output handling (`i-doonqn`)
  - Return an empty list on stdout (no stderr noise) with a success exit code when no patches exist.
  - Cover the empty-state behavior with a regression test.
- [MEDIUM] Standardize job identifier flag naming across job commands (`i-vdoshn`)
  - Choose a single spelling (e.g., `--job-id`) across all job subcommands; update help/examples and add backward-compatible aliases if needed.
  - Add tests for at least two subcommands using the canonical flag.
- [MEDIUM] Nest `worker-run` under the jobs namespace (`i-cxzrvd`)
  - Expose `metis jobs worker-run` for consistency; decide whether the top-level command remains as an alias or is deprecated.
  - Ensure help/docs reflect the new routing and add a regression test for the jobs namespace invocation.
