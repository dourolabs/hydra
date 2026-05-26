import { test, expect } from "../fixtures/auth";

// Fixture acme/api-gateway carries a non-trivial merge_policy in the mock-server
// seed (see hydra-web/packages/mock-server/fixtures/seed.json).
const REPO = "acme/api-gateway";

test.describe("Repository edit merge policy @repos:edit-merge-policy", () => {
  test("round-trips merge_policy through the edit modal @repos:edit-merge-policy", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/repositories");
    await expect(page.getByRole("heading", { name: "Repositories" })).toBeVisible();

    // Criterion 1: opening the modal pre-fills the textarea with pretty JSON.
    await page
      .getByTestId(`repositories-list-row-${REPO}`)
      .getByRole("button", { name: `Edit ${REPO}` })
      .click();
    let modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    const editor = () => page.getByRole("dialog").getByTestId("merge-policy-editor");
    const initialText = await editor().inputValue();
    const initialPolicy = JSON.parse(initialText);
    expect(initialPolicy).toMatchObject({
      reviewers: [
        { label: "code-review", any_of: ["reviewer", "carol"], count: 2 },
        { label: "human-signoff", any_of: ["alice", "bob"] },
      ],
      mergers: { any_of: ["@patch.author", "alice"] },
    });
    // Pretty-printed with two-space indent.
    expect(initialText).toContain('\n  "reviewers"');

    // Criterion 3: save unchanged → policy preserved on the server.
    await modal.getByRole("button", { name: "Save Changes" }).click();
    await expect(modal).toBeHidden();

    // Reload and confirm the summary still renders the same content.
    await page.reload();
    await expect(page.getByRole("heading", { name: "Repositories" })).toBeVisible();
    const summary = page.getByTestId(`merge-policy-${REPO}`);
    await expect(summary).toBeVisible();
    await expect(summary).toContainText("code-review");
    await expect(summary).toContainText("human-signoff");
    await expect(summary).toContainText("@patch.author");

    // Criterion 6: invalid JSON disables Save and shows inline error.
    await page
      .getByTestId(`repositories-list-row-${REPO}`)
      .getByRole("button", { name: `Edit ${REPO}` })
      .click();
    modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
    await editor().fill("{ not valid json");
    await expect(modal.getByText(/Invalid JSON:/)).toBeVisible();
    await expect(modal.getByRole("button", { name: "Save Changes" })).toBeDisabled();

    // Criterion 4: editing to a valid new policy and saving updates the server.
    const newPolicy = {
      reviewers: [{ any_of: ["users/alice"], count: 1 }],
    };
    await editor().fill(JSON.stringify(newPolicy, null, 2));
    await expect(modal.getByText(/Invalid JSON:/)).toBeHidden();
    await modal.getByRole("button", { name: "Save Changes" }).click();
    await expect(modal).toBeHidden();

    // The repositories query is invalidated on success; the summary should
    // reflect the new policy without an explicit reload.
    const summaryAfter = page.getByTestId(`merge-policy-${REPO}`);
    await expect(summaryAfter).toContainText("users/alice");
    await expect(summaryAfter).not.toContainText("@patch.author");

    // Criterion 5: Clear policy → save sends null → reload shows the
    // "no policy" placeholder via MergePolicySummary (rendered when policy is null).
    await page
      .getByTestId(`repositories-list-row-${REPO}`)
      .getByRole("button", { name: `Edit ${REPO}` })
      .click();
    modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
    await modal.getByTestId("merge-policy-clear").click();
    await expect(editor()).toHaveValue("");
    await modal.getByRole("button", { name: "Save Changes" }).click();
    await expect(modal).toBeHidden();

    await page.reload();
    await expect(page.getByRole("heading", { name: "Repositories" })).toBeVisible();
    await expect(page.getByTestId(`merge-policy-${REPO}-none`)).toBeVisible();
  });
});
