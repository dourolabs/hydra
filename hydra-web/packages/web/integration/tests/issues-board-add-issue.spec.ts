import { test, expect } from "../fixtures/auth";

test.describe("Issues board '+ Add issue' button @issues:board", () => {
  test("hovering a column reveals an add-issue button that opens the create modal prepopulated to that column @issues:board", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");

    // Switch to the board layout. The Table button is selected by default;
    // the Board tab renders <IssuesBoard>.
    await page.getByTestId("issues-layout-board").click();

    // The Default project ships with an "open" status column. The add-issue
    // button is rendered with `visibility: hidden` by default — Playwright
    // auto-hovers before clicking, which triggers `.col:hover` and makes the
    // button interactable.
    const addButton = page.getByTestId("board-col-add-issue-default-open");
    await expect(addButton).toBeAttached();

    await page.getByTestId("board-col-default-open").hover();
    await addButton.click();

    // The shared new-issue modal opens with the column's project + status
    // prepopulated. The Project picker shows the column's project key; the
    // Status picker shows the column's status chip.
    const modal = page.getByRole("dialog", { name: "Create issue" });
    await expect(modal).toBeVisible();

    await expect(
      modal.getByTestId("issue-create-project-picker").getByText("default"),
    ).toBeVisible();
    await expect(
      modal.getByTestId("issue-create-status-picker").getByText("Open"),
    ).toBeVisible();
  });

  test("no 'No issues' placeholder is rendered for empty columns @issues:board", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // Wait for the board to settle.
    await expect(page.getByTestId("board-col-default-open")).toBeVisible();

    // Per the issue, the "No issues" placeholder is removed unconditionally —
    // empty columns must not render it anywhere on the board.
    await expect(page.getByText("No issues", { exact: true })).toHaveCount(0);
  });
});
