import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile the sidebar drawer is open by default and intercepts pointer
// events on the page content underneath. Persist "hidden" before navigation
// so the drawer stays closed for assertions on the main column. Mirrors the
// pattern in list-pages-overflow.spec.ts.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

const AUTH_HEADER = { Authorization: "Bearer dev-token-12345" };

// Seed enough patches to overflow the mobile viewport. The bug being guarded
// against (mobile patches/sessions rows squished to a sliver on initial load)
// only surfaces when the unfiltered list exceeds the mobile list container's
// height — the flex column container would otherwise have spare space and
// rows render at natural height regardless.
async function seedManyPatches(count: number) {
  for (let i = 0; i < count; i++) {
    const res = await fetch("http://localhost:8080/v1/patches", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...AUTH_HEADER },
      body: JSON.stringify({
        patch: {
          title: `Mobile overlap regression patch ${i}`,
          base_branch: "main",
          branch_name: `feat/overlap-regression-${i}`,
        },
      }),
    });
    if (!res.ok) {
      throw new Error(`seed patch failed: ${res.status} ${await res.text()}`);
    }
  }
}

test.describe("Mobile list rows render at natural height @mobile:list-row-overlap", () => {
  test.use({ viewport: { width: 375, height: 667 } });

  test("patches list rows do not collapse below content height on initial load", async ({
    authenticatedPage: page,
  }) => {
    // 30 extra patches + 10 in the seed = 40 rows; comfortably overflows a
    // 667px-tall viewport at the natural ~47px rail-row height.
    await seedManyPatches(30);
    await setSidebarHidden(page);
    await page.goto("/patches");

    await expect(
      page.getByRole("heading", { name: "Patches", level: 1 }),
    ).toBeVisible();
    await page.waitForLoadState("networkidle");

    const measurements = await page.evaluate(() => {
      const mlist = document.querySelector('[class*="mobileList"]');
      if (!mlist) return null;
      const rows = Array.from(mlist.children) as HTMLElement[];
      if (rows.length === 0) return null;
      return {
        rowCount: rows.length,
        scrolls: mlist.scrollHeight > mlist.clientHeight + 1,
        minRowHeight: Math.min(
          ...rows.map((r) => r.getBoundingClientRect().height),
        ),
      };
    });

    expect(measurements, "mobile patches list did not render").not.toBeNull();
    expect(
      measurements!.rowCount,
      "need enough seeded patches to overflow the viewport for this regression",
    ).toBeGreaterThan(10);
    // Without `flex-shrink: 0` on the rail row, the flex algorithm would
    // distribute the container's height across all rows, squishing each one
    // to ~16px and clipping its content. The natural rail-row height is well
    // above 30px (one title line + one meta line + padding).
    expect(
      measurements!.minRowHeight,
      "rail rows squished below natural content height — flex shrink not pinned",
    ).toBeGreaterThan(30);
    expect(
      measurements!.scrolls,
      "container should scroll when row count exceeds viewport",
    ).toBe(true);
  });
});
