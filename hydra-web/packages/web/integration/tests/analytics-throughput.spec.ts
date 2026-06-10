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

  test("renders the 4 patches chart cards with content", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/throughput");

    await expect(
      page.getByTestId("chart-patches-over-time").getByTestId("patches-over-time-content"),
    ).toBeVisible();

    const mixCard = page.getByTestId("chart-patches-terminal-mix");
    await expect(mixCard.getByTestId("patches-terminal-mix-content")).toBeVisible();
    // Mock server returns merged=27, closed=4 → total 31, shown in donut center.
    await expect(mixCard.getByTestId("patches-terminal-mix-total")).toHaveText("31");

    const ttmCard = page.getByTestId("chart-patches-time-to-merge");
    await expect(ttmCard.getByTestId("patches-time-to-merge-content")).toBeVisible();
    // Median 18000s = 5h; p95 86400*3s = 3d. Both should land in the callouts row.
    const callouts = ttmCard.getByTestId("patches-time-to-merge-callouts");
    await expect(callouts).toContainText("5h");
    await expect(callouts).toContainText("3d");

    await expect(
      page.getByTestId("chart-patches-in-flight").getByTestId("patches-in-flight-content"),
    ).toBeVisible();
  });

  test("changing the repo slicer triggers a chart refetch", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/throughput");

    await expect(
      page.getByTestId("chart-patches-over-time").getByTestId("patches-over-time-content"),
    ).toBeVisible();

    const requests: string[] = [];
    page.on("request", (req) => {
      if (req.url().includes("/v1/analytics/throughput/patches/over_time")) {
        requests.push(req.url());
      }
    });

    await page.getByTestId("slicer-repo").selectOption({ index: 1 });
    await expect(page).toHaveURL(/repo_name=/);

    await expect
      .poll(() => requests.length, { timeout: 5_000 })
      .toBeGreaterThan(0);
    expect(requests.some((u) => u.includes("repo_name="))).toBe(true);
  });
});
