import { test as base } from "@playwright/test";
import type { Page } from "@playwright/test";
import { test } from "./fixtures/auth";
import path from "path";

const SCREENSHOT_DIR = path.join("test-results", "visual-audit");

const DESKTOP_VIEWPORT = { width: 1280, height: 720 };
const MOBILE_VIEWPORT = { width: 375, height: 812 };

// Pages that require authentication
const AUTHENTICATED_PAGES = [
  { name: "dashboard", path: "/" },
  { name: "issue-detail", path: "/issues/i-seed00001" },
  { name: "patch-detail", path: "/patches/p-seed00001" },
  { name: "documents-list", path: "/documents" },
  { name: "document-detail", path: "/documents/d-seed00001" },
  { name: "session-log", path: "/issues/i-seed00005/sessions/t-seed00001/logs" },
  { name: "session-detail", path: "/sessions/t-seed00001" },
  { name: "sessions-list", path: "/sessions" },
  { name: "chats-list", path: "/chat" },
  { name: "chat-detail", path: "/chat/c-seed00001" },
  { name: "repositories", path: "/repositories" },
  { name: "agents", path: "/agents" },
  { name: "secrets", path: "/secrets" },
];

async function captureScreenshot(
  page: Page,
  pageName: string,
  viewport: { width: number; height: number },
  prefix: string,
) {
  await page.setViewportSize(viewport);
  // Allow layout to settle after viewport change
  await page.waitForTimeout(500);

  // Disable CSS animations before taking the screenshot. Nested fadeIn
  // animations (on .main and .page) can compound and re-trigger at the
  // compositor level during fullPage capture, causing a dark tint overlay.
  await page.addStyleTag({
    content: "*, *::before, *::after { animation: none !important; }",
  });
  // Allow one frame for the style to take effect
  await page.waitForTimeout(50);

  await page.screenshot({
    path: path.join(SCREENSHOT_DIR, `${prefix}-${pageName}.png`),
    fullPage: true,
  });
}

base.describe("Visual Audit - Login", () => {
  base("capture login page at desktop and mobile viewports", async ({ page }) => {
    await page.goto("/login");
    await page.waitForSelector('[data-testid="github-login-button"]');

    await captureScreenshot(page, "login", DESKTOP_VIEWPORT, "desktop");
    await captureScreenshot(page, "login", MOBILE_VIEWPORT, "mobile");
  });
});

test.describe("Visual Audit - Authenticated Pages", () => {
  for (const { name, path: pagePath } of AUTHENTICATED_PAGES) {
    test(`capture ${name} at desktop and mobile viewports`, async ({ authenticatedPage }) => {
      await authenticatedPage.goto(pagePath);
      // Wait for DOM to be ready then allow content to render
      // (networkidle doesn't work here due to SSE connections)
      await authenticatedPage.waitForLoadState("domcontentloaded");
      await authenticatedPage.waitForTimeout(2000);

      await captureScreenshot(authenticatedPage, name, DESKTOP_VIEWPORT, "desktop");
      await captureScreenshot(authenticatedPage, name, MOBILE_VIEWPORT, "mobile");
    });
  }
});

// ── ChatActivityLine inline activity indicator ─────────────────────────────
// Captures the three reference states from the spec (live mid-tool-use,
// expanded feed, terminal done). Seeds tool_use events into the chat's
// session via the mock-server's `/v1/dev/sessions/:id/events` endpoint so
// the on-disk fixture is left alone — only this test sees the injected
// stream.

const ACTIVITY_CONVERSATION_ID = "c-seed00001";
const ACTIVITY_SESSION_ID = "t-seed00016";
const AUTH_HEADER = { Authorization: "Bearer dev-token-12345" };

async function appendSessionEvent(sessionId: string, event: unknown) {
  const res = await fetch(`http://localhost:8080/v1/dev/sessions/${sessionId}/events`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...AUTH_HEADER },
    body: JSON.stringify(event),
  });
  if (!res.ok) {
    throw new Error(
      `dev append-event failed (${res.status}): ${await res.text()}`,
    );
  }
}

test.describe("Visual Audit - Chat activity line", () => {
  test("capture live, expanded, and done states for the inline activity indicator", async ({
    authenticatedPage,
  }) => {
    const page = authenticatedPage;

    // 1. Seed a multi-step run that ends on a tool_use (live tail).
    const baseTs = Date.parse("2026-05-14T12:00:00Z");
    await appendSessionEvent(ACTIVITY_SESSION_ID, {
      type: "user_message",
      content: "Look at the failing tests and propose a fix.",
      timestamp: new Date(baseTs).toISOString(),
    });
    await appendSessionEvent(ACTIVITY_SESSION_ID, {
      type: "tool_use",
      tool_name: "Grep",
      payload: { description: '"refreshToken" in src/auth' },
      timestamp: new Date(baseTs + 800).toISOString(),
    });
    await appendSessionEvent(ACTIVITY_SESSION_ID, {
      type: "tool_use",
      tool_name: "Read",
      payload: { description: "src/auth/oauth.ts" },
      timestamp: new Date(baseTs + 3_200).toISOString(),
    });
    await appendSessionEvent(ACTIVITY_SESSION_ID, {
      type: "tool_use",
      tool_name: "Edit",
      payload: { description: "Restore refreshToken default in oauth.ts" },
      timestamp: new Date(baseTs + 8_400).toISOString(),
    });

    await page.goto(`/chat/${ACTIVITY_CONVERSATION_ID}`);
    await page.waitForLoadState("domcontentloaded");
    await page.waitForSelector('[data-testid="chat-activity-line"]', {
      timeout: 5_000,
    });
    // Let CSS-only animations land in a deterministic frame before disabling
    // animations in `captureScreenshot`.
    await page.waitForTimeout(500);

    await captureScreenshot(page, "chat-activity-line-live", DESKTOP_VIEWPORT, "desktop");
    await captureScreenshot(page, "chat-activity-line-live", MOBILE_VIEWPORT, "mobile");

    // 2. Expanded feed.
    await page.setViewportSize(DESKTOP_VIEWPORT);
    await page.locator('[data-testid="chat-activity-line-toggle"]').click();
    await page.waitForSelector('[data-testid="chat-activity-line-feed"]');
    await page.waitForTimeout(200);
    await captureScreenshot(
      page,
      "chat-activity-line-expanded",
      DESKTOP_VIEWPORT,
      "desktop",
    );
    await captureScreenshot(
      page,
      "chat-activity-line-expanded",
      MOBILE_VIEWPORT,
      "mobile",
    );

    // 3. Terminal "done" — append an assistant_message and reload.
    await appendSessionEvent(ACTIVITY_SESSION_ID, {
      type: "assistant_message",
      content: "Restored the default and re-ran the suite — green.",
      timestamp: new Date(baseTs + 18_400).toISOString(),
    });
    await page.reload();
    await page.waitForLoadState("domcontentloaded");
    await page.waitForSelector('[data-testid="chat-activity-line"]', {
      timeout: 5_000,
    });
    await page
      .locator('[data-testid="chat-activity-line-toggle"]')
      .click(); // open the feed so the done summary shows the steps.
    await page.waitForSelector('[data-testid="chat-activity-line-feed"]');
    await page.waitForTimeout(200);
    await captureScreenshot(page, "chat-activity-line-done", DESKTOP_VIEWPORT, "desktop");
    await captureScreenshot(page, "chat-activity-line-done", MOBILE_VIEWPORT, "mobile");
  });
});
