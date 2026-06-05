# Queued (Planned) E2E Scenarios

Files in this directory are **paper specs queued for activation**. They describe end-to-end behavior the system does not yet ship but will soon; they are checked in so the spec lives next to the code that will eventually exercise it.

## How the skip works

The tester's canonical enumeration is `ls tests/e2e/scenarios/*.md` — a non-recursive glob that skips this `planned/` subdirectory by design. That is the entire skip mechanism. No prompt change, no priority tier, no in-file `Status: planned` flag.

## Activating a scenario

When the listed runtime gates are met, activate a scenario with a single move:

```
git mv tests/e2e/scenarios/planned/<file>.md tests/e2e/scenarios/<file>.md
```

The PR doing the move should reference the gate-PRs that unlocked it.

## Currently queued

_None._ The two previously queued scenarios were activated under [[i-bhaxdizc]] — `triggers-one-shot-via-chat.md` and `per-project-status-pipeline.md` now live in `tests/e2e/scenarios/`. Both retain their in-scenario skip-if pre-checks, so they self-skip on instances where the remaining wiring (chat-prompt allowlist for `hydra triggers`; runner-side `engineering-v2` project + `pm-v2`/`swe-v2`/`reviewer-v2` cloned-agent seeding) is not yet in place.
