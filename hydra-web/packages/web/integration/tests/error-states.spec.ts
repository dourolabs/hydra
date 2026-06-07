import { test, expect } from "../fixtures/auth";

test.describe("Error States @errors:404 @errors:server-error", () => {
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
});
