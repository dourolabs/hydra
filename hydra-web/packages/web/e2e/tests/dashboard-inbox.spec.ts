import { test, expect } from "../fixtures/auth";

test.describe("Dashboard Inbox @dashboard:inbox", () => {
  test("shows dropped issues in Complete section @dashboard:inbox", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to dashboard with inbox filter
    await page.goto("/?selected=inbox");

    // The Complete section should be visible with terminal issues that have the inbox label
    await expect(page.getByText(/Complete \(\d+\)/)).toBeVisible();

    // The dropped issue "Update deployment documentation" (i-seed00010)
    // has inbox label and dev-user is assignee — should appear in Complete section
    await expect(
      page.getByText("Update deployment documentation")
    ).toBeVisible();
  });

  test("shows closed issues in Complete section @dashboard:inbox", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=inbox");

    // i-seed00007 "Fix login page 500 error on expired sessions" is closed
    // with inbox label and dev-user is creator — should appear in Complete section
    await expect(
      page.getByText("Fix login page 500 error on expired sessions")
    ).toBeVisible();
  });
});
