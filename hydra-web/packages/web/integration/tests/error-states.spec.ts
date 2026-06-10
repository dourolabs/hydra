import { test, expect } from "../fixtures/auth";

test.describe("Error States @errors:404 @errors:server-error @errors:route-not-found", () => {
  test("shows not-found message for non-existent issue @errors:404", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-nonexistent");
    await expect(page.getByText(/not found/i)).toBeVisible();
  });

  test("shows not-found message for non-existent patch @errors:404", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-nonexistent");
    await expect(page.getByText(/not found/i)).toBeVisible();
  });

  test("shows not-found message for non-existent document @errors:404", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents/d-nonexistent");
    await expect(page.getByText(/not found/i)).toBeVisible();
  });

  test("shows error message when server returns 500 on issue detail @errors:server-error", async ({
    authenticatedPage: page,
  }) => {
    // Intercept API calls for issue detail and simulate a 500 error
    await page.route("**/api/v1/issues/i-seed00001", (route) => {
      route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ error: "Internal Server Error" }),
      });
    });

    await page.goto("/issues/i-seed00001");
    // The page should show some error indication
    await expect(page.getByText(/error/i)).toBeVisible();
  });

  test("shows error message when server returns 500 on documents list @errors:server-error", async ({
    authenticatedPage: page,
  }) => {
    // Intercept API calls for documents list and simulate a 500 error
    await page.route("**/api/v1/documents**", (route) => {
      route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ error: "Internal Server Error" }),
      });
    });

    await page.goto("/documents");
    // The page should show some error indication
    await expect(page.getByText(/error/i)).toBeVisible();
  });

  test("renders styled NotFound page inside AppLayout for unmatched routes @errors:route-not-found", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/does-not-exist");

    // The styled 404 heading is visible — not React Router's developer fallback.
    await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
    await expect(page.getByText(/Unexpected Application Error/i)).toHaveCount(0);

    // The AppLayout chrome (sidebar) is still rendered.
    await expect(page.getByTestId("sidebar")).toBeVisible();

    // Plant a sentinel on `window`: a full reload wipes it; a client-side
    // navigate via React Router keeps the existing window/document. This is
    // how we confirm the "Go to dashboard" action stays in-SPA.
    await page.evaluate(() => {
      (window as unknown as { __notFoundNav: boolean }).__notFoundNav = true;
    });
    await page.getByRole("button", { name: "Go to dashboard" }).click();
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/$/);
    const sentinelSurvived = await page.evaluate(
      () => (window as unknown as { __notFoundNav?: boolean }).__notFoundNav === true,
    );
    expect(sentinelSurvived).toBe(true);
  });
});
