import { test, expect } from "../fixtures/auth";

test.describe("Analytics throughput @analytics:throughput", () => {
  test("sidebar entry navigates to the throughput page", async ({
    authenticatedPage: page,
  }) => {
    await page.getByTestId("sidebar-analytics-throughput").click();
    await expect(page).toHaveURL(/\/analytics\/throughput/);
    await expect(page.getByTestId("analytics-throughput-page")).toBeVisible();
    await expect(page.getByTestId("analytics-patches-section")).toBeVisible();
    await expect(page.getByTestId("analytics-issues-section")).toBeVisible();
  });

  test("/analytics redirects to /analytics/throughput", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics");
    await expect(page).toHaveURL(/\/analytics\/throughput/);
  });

  test("time-range buttons update the URL search params", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/throughput");
    await page.getByTestId("time-range-7d").click();
    await expect(page).toHaveURL(/range=7d/);
    await page.getByTestId("time-range-90d").click();
    await expect(page).toHaveURL(/range=90d/);
    await page.getByTestId("time-range-all-time").click();
    await expect(page).toHaveURL(/range=all-time/);
  });

  test("changing the project filter writes to the URL and enables status options", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/throughput");

    // Initially the project-scoped issue cards advertise that a project is required.
    await expect(
      page.getByTestId("chart-issues-time-in-status").getByTestId("chart-card-disabled"),
    ).toBeVisible();

    await page.getByTestId("slicer-project").selectOption({ index: 1 });

    await expect(page).toHaveURL(/project_id=/);
    await expect(
      page.getByTestId("chart-issues-time-in-status").getByTestId("chart-card-disabled"),
    ).toHaveCount(0);
  });
});
