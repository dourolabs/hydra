import type { Page } from "@playwright/test";
import { test, expect } from "../fixtures/auth";

// `@chat:proxy-tab` exercises the right-panel Proxy tab in `ChatRightPanel`:
//
//   - The tab is HIDDEN when the conversation's active session has no
//     advertised proxy targets (fresh conversation in the seed data).
//   - With `listProxyTargets` populated (a worker advertised port 3000), the
//     tab appears with a `ready` status badge once the HEAD probe lands and
//     the status row settles to `ready`.
//   - Clicking "Open in new tab" mints the cookie via
//     `POST /api/v1/conversations/<cid>/proxy-auth` and then opens a new tab
//     at `https://3000-<cid>.proxy.<host>/`.
//
// The mock server has no real proxy reach, so we intercept the HEAD probe to
// the proxy subdomain and assert the call/URL shape directly. This matches
// the e2e contract described in [[i-pdwsvquo]] acceptance criteria — the
// real backend's proxy router is integration-tested in PR 2.

const SEED_CONVERSATION_ID = "c-seed00001"; // dev-user's active conversation
const SEED_SESSION_ID = "t-seed00016";

/**
 * Shim the `useSSE` `["proxyTargets", session_id]` invalidation by driving
 * `queryClient.invalidateQueries` directly through the dev-only
 * `window.__hydraQueryClient` export. Mirrors the
 * `invalidateSessionEvents` helper in `chat-activity-status.spec.ts`. SSE
 * delivery through vite's dev proxy is unreliable; the useSSE handler
 * itself is unit-tested.
 */
async function invalidateProxyTargets(page: Page, sessionId: string): Promise<void> {
  await page.evaluate((sid) => {
    const qc = (
      window as unknown as {
        __hydraQueryClient?: {
          invalidateQueries: (opts: { queryKey: unknown[] }) => Promise<void>;
        };
      }
    ).__hydraQueryClient;
    qc?.invalidateQueries({ queryKey: ["proxyTargets", sid] });
  }, sessionId);
}

test.describe("Proxy tab @chat:proxy-tab", () => {
  test("is hidden when the conversation's session has no proxy targets @chat:proxy-tab", async ({
    authenticatedPage: page,
  }) => {
    await page.goto(`/chat/${SEED_CONVERSATION_ID}`);
    await expect(page.getByTestId("chat-pane")).toBeVisible();

    // Related + Details only — Proxy is hidden because the seeded session
    // has no `proxy_targets`.
    await expect(page.getByTestId("chat-rail-tab-related")).toBeVisible();
    await expect(page.getByTestId("chat-rail-tab-details")).toBeVisible();
    await expect(page.getByTestId("chat-rail-tab-proxy")).toHaveCount(0);
  });

  test("appears with a ready status row and a working Open-in-new-tab affordance once a target is advertised @chat:proxy-tab", async ({
    authenticatedPage: page,
  }) => {
    // Stub the HEAD probe so the status flips to `ready` deterministically
    // — the real cross-origin fetch would fail DNS in the test env.
    await page.route(/3000-c-seed00001\./, (route) => {
      route.fulfill({ status: 200, body: "" });
    });

    let mintCalls = 0;
    await page.route(
      `**/api/v1/conversations/${SEED_CONVERSATION_ID}/proxy-auth`,
      (route) => {
        mintCalls += 1;
        route.fulfill({ status: 204, body: "" });
      },
    );

    await page.goto(`/chat/${SEED_CONVERSATION_ID}`);
    await expect(page.getByTestId("chat-pane")).toBeVisible();

    // Sanity: with no targets yet advertised, the tab is hidden — this is the
    // pre-condition for the live-update path below.
    await expect(page.getByTestId("chat-rail-tab-proxy")).toHaveCount(0);

    // Worker advertises port 3000 on the active session AFTER the page has
    // mounted, so the live-update path is exercised rather than the
    // initial-mount `refetchOnMount: "always"` fallback. In prod, the mock
    // server's `session_updated` SSE notification would drive `useSSE` to
    // invalidate `["proxyTargets", session_id]`. SSE delivery is unreliable
    // through the dev `vite` proxy (see chat-activity-status.spec.ts for the
    // same workaround), so we shim the invalidation via the dev-only
    // `window.__hydraQueryClient` export. The useSSE → invalidate handler
    // itself is covered by unit tests; this e2e covers the downstream
    // query-refetch → ProxyTab-appears path.
    const advertiseRes = await page.request.post(
      `http://localhost:8080/v1/sessions/${SEED_SESSION_ID}/proxy-targets`,
      {
        headers: {
          Authorization: "Bearer dev-token-12345",
          "Content-Type": "application/json",
        },
        data: { port: 3000, ready_path: "/" },
      },
    );
    expect(advertiseRes.status()).toBe(204);
    await invalidateProxyTargets(page, SEED_SESSION_ID);

    // Proxy tab appears in the right-rail tabs.
    const proxyTabButton = page.getByTestId("chat-rail-tab-proxy");
    await expect(proxyTabButton).toBeVisible();
    await proxyTabButton.click();

    // Row for port 3000 is visible and the status badge settles to `ready`
    // once the HEAD probe resolves.
    const row = page.getByTestId("proxy-row-3000");
    await expect(row).toBeVisible();
    await expect(row).toContainText("port 3000");
    await expect(page.getByTestId("proxy-status-3000")).toHaveText(/Ready/i, {
      timeout: 5000,
    });

    // Capture the popup target URL and assert the click flow:
    //   1. POST /v1/conversations/<cid>/proxy-auth lands first.
    //   2. window.open is invoked with the conv-id-shaped proxy URL.
    const popupPromise = page.waitForEvent("popup");
    await page.getByTestId("proxy-open-3000").click();
    const popup = await popupPromise;
    expect(popup.url()).toMatch(/^https?:\/\/3000-c-seed00001\.proxy\./);
    expect(mintCalls).toBe(1);
  });
});
