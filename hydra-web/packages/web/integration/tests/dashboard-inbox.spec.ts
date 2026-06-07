import { test, expect } from "../fixtures/auth";

test.describe("Dashboard Inbox @dashboard:inbox", () => {
  test("shows dropped issues @dashboard:inbox", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to dashboard with your-issues filter (inbox equivalent)
    await page.goto("/?selected=your-issues");

    // The dropped issue "Update deployment documentation" (i-seed00010)
    // has inbox label and dev-user is assignee — should appear in the list
    await expect(
      page.getByText("Update deployment documentation")
    ).toBeVisible();
  });

  test("shows closed issues @dashboard:inbox", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=your-issues");

    // i-seed00007 "Fix login page 500 error on expired sessions" is closed
    // with inbox label and dev-user is creator — should appear in the list
    await expect(
      page.getByText("Fix login page 500 error on expired sessions")
    ).toBeVisible();
  });
});
