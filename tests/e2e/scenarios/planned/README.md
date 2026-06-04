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

- **`triggers-one-shot-via-chat.md`** — queued on:
  1. **PR 6 [[i-fhmpscam]]** — the `hydra triggers {create,get,list,update,delete,test}` CLI (currently in flight).
  2. **Chat agent prompt update** — `$DOC_STORE/agents/chat/prompt.md` must add `hydra triggers` to its allowlist so the chat agent can translate "set up a trigger" into a real create call. No issue filed yet; tracked as a follow-up under parent [[i-fqrjpnhi]].
