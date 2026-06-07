import { test, expect } from "../fixtures/auth";

test.describe("Patches @patches:view-detail @patches:navigate", () => {
  test("displays patch detail page with title and status @patches:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();
  });

  test("patch detail page shows patch ID in title block @patches:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();

    // patch_id is rendered alongside the title (no Metadata tab anymore).
    // Scope to <main> since the breadcrumb also shows the patch ID.
    await expect(
      page.getByRole("main").getByText("p-seed00001", { exact: true })
    ).toBeVisible();
  });

  test("patches list Repo column links to the linked GitHub PR @patches:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches");
    await expect(page.getByRole("heading", { name: "Patches" })).toBeVisible();

    // p-seed00001 is linked to acme/web-app#142.
    const prLink = page.getByRole("link", { name: "acme/web-app#142" });
    await expect(prLink).toBeVisible();
    await expect(prLink).toHaveAttribute("href", "https://github.com/acme/web-app/pull/142");
    await expect(prLink).toHaveAttribute("target", "_blank");
    await expect(prLink).toHaveAttribute("rel", /noopener/);
    await expect(prLink).toHaveAttribute("rel", /noreferrer/);
  });

  test("can navigate to a patch from an issue's Related tab @patches:navigate", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00002 has patch p-seed00001
    await page.goto("/issues/i-seed00002");
    await expect(
      page.getByRole("heading", { name: "Migrate authentication to OAuth2" })
    ).toBeVisible();

    // Patches are now listed in the right-rail Related tab (default active).
    await expect(page.getByTestId("issue-rail-tab-related")).toBeVisible();
    await page.getByTestId("issue-rail-tab-related").click();

    // The patch row in the Related tab is an ItemRow showing the patch title;
    // clicking it navigates to /patches/<id>.
    await expect(page.getByRole("heading", { name: "Patches" })).toBeVisible();
    await page
      .getByText("Add OAuth2 provider integration")
      .last()
      .click();
    await expect(page).toHaveURL(/\/patches\/p-seed00001/);
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();
  });
});
