import { test, expect } from "../fixtures/auth";

test.describe("Dashboard Inbox", () => {
  test("shows dropped issues in Complete section", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to dashboard with inbox filter
    await page.goto("/?selected=inbox");

    // The Complete section should be visible with terminal issues assigned to dev-user
    await expect(page.getByText(/Complete \(\d+\)/)).toBeVisible();

    // The dropped issue "Update deployment documentation" (i-seed00010)
    // is assigned to dev-user and should appear in the Complete section
    await expect(
      page.getByText("Update deployment documentation")
    ).toBeVisible();
  });

  test("shows closed issues in Complete section", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=inbox");

    // i-seed00007 "Fix login page 500 error on expired sessions" is closed
    // and assigned to dev-user — should appear in Complete section
    await expect(
      page.getByText("Fix login page 500 error on expired sessions")
    ).toBeVisible();
  });
});
