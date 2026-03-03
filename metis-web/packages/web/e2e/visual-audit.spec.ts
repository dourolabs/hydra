import { test as base } from "@playwright/test";
import type { Page } from "@playwright/test";
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

async function authenticate(page: Page) {
  await page.goto("/login");
  await page.fill('[data-testid="token-input"]', "dev-token-12345");
  await page.click('[data-testid="login-button"]');
  await page.waitForFunction(
    () => !window.location.pathname.startsWith("/login"),
  );
}

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

base.describe("Visual Audit - Authenticated Pages", () => {
  for (const { name, path: pagePath } of AUTHENTICATED_PAGES) {
    base(`capture ${name} at desktop and mobile viewports`, async ({ page }) => {
      await authenticate(page);
      await page.goto(pagePath);
      // Wait for network to settle so content is loaded
      await page.waitForLoadState("networkidle");

      await captureScreenshot(page, name, DESKTOP_VIEWPORT, "desktop");
      await captureScreenshot(page, name, MOBILE_VIEWPORT, "mobile");
    });
  }
});
