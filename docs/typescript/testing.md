# Testing

Cross-cutting testing rules (TDD-first, never widen exports for tests) live in [../testing.md](../testing.md) and apply here as well.

Two layers: Vitest (unit + contract) and Playwright (integration + visual audit). The Playwright configs declare a `webServer` block that owns the dev-stack lifecycle, so agents don't manage processes by hand.

## Integration tests with `pnpm integration`

From `hydra-web/`:

```bash
pnpm install
pnpm --filter @hydra/web exec playwright install chromium   # one-time, not needed in worker image
pnpm integration
```

`packages/web/playwright.config.ts` spins up the mock server on `:8080` (with `MOCK_SYNTHETIC_EVENTS=0` for deterministic event tails) and Vite on `:3000`, then tears them down when the run ends.

```bash
pnpm integration                                        # all integration tests
pnpm --filter @hydra/web exec playwright test login     # one test file
pnpm --filter @hydra/web exec playwright test --headed  # visible browser
```

### Don't manage dev servers manually

```bash
# wrong — orphans mock-server / Vite when the agent session ends
./scripts/dev-test.sh --test &
./scripts/dev-test.sh &
pnpm dev &

# correct — Playwright owns the lifecycle
pnpm integration
```

`./scripts/dev-test.sh` ends in `wait` and relies on Ctrl-C to fire its cleanup trap. Backgrounded from a non-interactive caller, neither the wait nor the trap fires and the dev servers outlive the agent. Use `pnpm integration` (or `pnpm visual-audit`) instead.

`reuseExistingServer` is true for local runs, so manually-started servers on `:8080` / `:3000` will silently be reused but won't be cleaned up — another reason to let Playwright start them.

## Reset and error injection

The mock server exposes three dev-only knobs:

```bash
# Reset store to seed data — call between tests for a clean slate
curl -X POST http://localhost:8080/v1/dev/reset

# Force any request to return a specific HTTP status — for testing error paths
curl -H "X-Mock-Error: 503" http://localhost:8080/v1/issues

# Append a SessionEvent to a running session and broadcast its SSE notification
# — for seeding deterministic event tails without running a full agent loop
curl -X POST -H "Content-Type: application/json" \
  -d '{"kind":"ToolUse","name":"Read","input":{}}' \
  http://localhost:8080/v1/dev/sessions/<id>/events
```

## Visual audit

`pnpm --filter @hydra/web visual-audit` captures every major page at desktop (1280×720) and mobile (375×812) viewports via `playwright-visual-audit.config.ts` — same lifecycle story as `pnpm integration`, no manual servers. Output lands in `packages/web/test-results/visual-audit/` as `{viewport}-{page}.png`. Run it before and after any CSS / layout change and diff the two sets.

## Contract tests

`packages/mock-server/src/__tests__/contract.test.ts` validates the mock's responses against the generated `@hydra/api` types. They run as part of `pnpm test` (root) and catch mock-vs-real-server drift before it lands in integration.

## Debugging failures

- Screenshots: `packages/web/test-results/` on failure (config sets `screenshot: "only-on-failure"`).
- Traces: recorded on first retry. View with `pnpm --filter @hydra/web exec playwright show-trace <file>`.
- Headed mode: `--headed` to watch the browser drive the test.

## Mobile testing

- Default visual-regression viewport: **375×667** (iPhone SE / standard mobile width). `pnpm visual-audit` already captures mobile at 375×812 per `playwright-visual-audit.config.ts` — keep that contract.
- Stress-test at **320×568** (smallest common mobile) for any layout you suspect of wrapping issues. Anything that wraps weirdly at 375px breaks at 320px.
- For safe-area / keyboard / overlay behavior, test on **real iOS Safari**, not Chrome DevTools mobile emulation. The emulator doesn't fire the real `viewport-fit` / `env(safe-area-inset-*)` paths, and on-screen-keyboard occlusion behavior diverges from real iOS.
- **Horizontal scroll check:** at every list-page root, assert `document.documentElement.scrollWidth === window.innerWidth` at 375px. Common enough as a regression class to warrant its own assertion.

## See also

- [packages.md](./packages.md) — `@hydra/mock-server` package layout.
- [react-query-and-sse.md](./react-query-and-sse.md) — what to assert about cache-update behaviour.
