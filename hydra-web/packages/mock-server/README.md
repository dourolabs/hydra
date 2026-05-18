# @hydra/mock-server

In-process mock of the Hydra HTTP API. Used by the `@hydra/web` dev experience and by Vitest contract tests in this package. It implements the same routes as the real `hydra-server` (and the BFF proxy paths) backed by an in-memory store seeded from a JSON fixture.

Fixtures live in [`fixtures/seed.json`](./fixtures/seed.json) and are reloaded by `POST /v1/dev/reset`.

## Running

```bash
pnpm --filter @hydra/mock-server dev        # standalone server (default port 3030)
pnpm --filter @hydra/mock-server test       # run contract + store tests
pnpm --filter @hydra/mock-server typecheck
```

The web dev server (`pnpm --filter @hydra/web dev`) wires the mock server in automatically — open `http://localhost:3000` and any of the issue IDs below.

## Seed issues with forms

The form schema is defined in `hydra-common/src/api/v1/form.rs` and mirrored to TypeScript under `packages/api/src/generated/{Form,Field,Input,Action,Effect,SelectOption,FormResponse}.ts`. When working on `FormPanel` or anything form-adjacent, open the issue IDs below — between them they cover every `Input` variant, both `Effect` types, and every `ActionStyle`.

| Issue ID | Type | What it demonstrates |
|---|---|---|
| `i-seed00011` | review-request | Code-review form: dropdown `select`, `text` (with `pattern`), `textarea`, `checkbox`. `primary` + `default` actions, both `record_only` and `update_issue` effects. |
| `i-seed00023` | review-request | ADR review form: radio `select`, `text`, `textarea`, `checkbox`. `primary` + `danger` actions, both effect variants. |
| `i-seed00024` | review-request | Same shape as 00023 but with a populated `form_response` — the read-only rendering path. |
| `i-seed00025` | task (Survey) | Every flavor of text input: `text` (with `placeholder`, `min_length`, `max_length`, `pattern`) and `textarea` (default rows and `rows: 8`), with and without `default`. |
| `i-seed00026` | task (Survey) | Every selection input: `select` with `radio: true`, `select` with `radio: false`, and three `checkbox` fields (defaulted on, defaulted off, no default). |
| `i-seed00027` | task (Survey) | Every flavor of `number` input: fully bounded with `step`, fractional `step` without `default`, max-only, and fully unbounded. |
| `i-seed00028` | task (Survey) | Action / effect coverage: `primary` + `danger` + `default` action styles, both `record_only` and `update_issue` effects, plus a populated `form_response` (read-only path on a non-review issue). |

`Survey:`-prefixed issues are top-level tasks — there is no `survey` value in `IssueType`, so the type signals the category in the title only.
