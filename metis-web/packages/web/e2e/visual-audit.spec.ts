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
  { name: "issues-list", path: "/issues" },
  { name: "issue-detail", path: "/issues/i-seed00001" },
  { name: "patches-list", path: "/patches" },
  { name: "patch-detail", path: "/patches/p-seed00001" },
  { name: "documents-list", path: "/documents" },
  { name: "document-detail", path: "/documents/d-seed00001" },
  { name: "notifications", path: "/notifications" },
  { name: "settings", path: "/settings" },
  { name: "job-log", path: "/issues/i-seed00005/jobs/t-seed00001/logs" },
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
  await page.screenshot({
    path: path.join(SCREENSHOT_DIR, `${prefix}-${pageName}.png`),
    fullPage: true,
  });
}

base.describe("Visual Audit - Login", () => {
  base("capture login page at desktop and mobile viewports", async ({ page }) => {
    await page.goto("/login");
    await page.waitForSelector('[data-testid="token-input"]');

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
