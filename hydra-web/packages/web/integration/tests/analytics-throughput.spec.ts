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

    // Initially both project-scoped issue cards advertise that a project is required.
    await expect(
      page.getByTestId("chart-issues-time-in-status").getByTestId("chart-card-disabled"),
    ).toBeVisible();
    await expect(
      page.getByTestId("chart-issues-per-status").getByTestId("chart-card-disabled"),
    ).toBeVisible();

    await page.getByTestId("slicer-project").selectOption({ index: 1 });

    await expect(page).toHaveURL(/project_id=/);
    await expect(
      page.getByTestId("chart-issues-time-in-status").getByTestId("chart-card-disabled"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("chart-issues-per-status").getByTestId("chart-card-disabled"),
    ).toHaveCount(0);
  });

  test("renders the 4 issues chart cards with content (project-scoped after slicer)", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/throughput");

    // The two cross-project issues charts render right away.
    await expect(
      page.getByTestId("chart-issues-over-time").getByTestId("issues-over-time-content"),
    ).toBeVisible();

    const cycleCard = page.getByTestId("chart-issues-cycle-time");
    await expect(cycleCard.getByTestId("issues-cycle-time-content")).toBeVisible();
    // Mock server returns median 86400s (1d), p95 604800s (7d), count 9.
    const cycleCallouts = cycleCard.getByTestId("issues-cycle-time-callouts");
    await expect(cycleCallouts).toContainText("1d");
    await expect(cycleCallouts).toContainText("7d");
    await expect(cycleCallouts).toContainText("9");

    // Pick a project to unlock the per-project issue charts.
    await page.getByTestId("slicer-project").selectOption({ index: 1 });

    const breakdownCard = page.getByTestId("chart-issues-time-in-status");
    await expect(breakdownCard.getByTestId("issues-time-in-status-content")).toBeVisible();
    // Mock server returns three statuses; the open + in-progress segments
    // render (the closed segment has 0% width and is skipped from the bar).
    await expect(
      breakdownCard.getByTestId("issues-time-in-status-segment-open"),
    ).toBeVisible();
    await expect(
      breakdownCard.getByTestId("issues-time-in-status-segment-in-progress"),
    ).toBeVisible();
    // Legend keeps every status, including the 0-time terminal.
    await expect(
      breakdownCard.getByTestId("issues-time-in-status-legend-closed"),
    ).toBeVisible();

    const perStatusCard = page.getByTestId("chart-issues-per-status");
    await expect(perStatusCard.getByTestId("issues-per-status-content")).toBeVisible();
    await expect(
      perStatusCard.getByTestId("issues-per-status-card-open"),
    ).toBeVisible();
    await expect(
      perStatusCard.getByTestId("issues-per-status-card-in-progress"),
    ).toBeVisible();
  });

  test("each chart card is a labeled region for screen readers", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/throughput");

    for (const testId of [
      "chart-patches-over-time",
      "chart-patches-terminal-mix",
      "chart-patches-time-to-merge",
      "chart-patches-in-flight",
      "chart-issues-over-time",
      "chart-issues-cycle-time",
      "chart-issues-time-in-status",
      "chart-issues-per-status",
    ]) {
      const card = page.getByTestId(testId);
      await expect(card).toHaveAttribute("role", "region");
      await expect(card).toHaveAttribute("aria-label", /.+/);
    }
  });

  test("mobile viewport renders without horizontal scroll", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 375, height: 800 });
    await page.goto("/analytics/throughput");
    await expect(page.getByTestId("analytics-throughput-page")).toBeVisible();

    const { scrollWidth, clientWidth } = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }));
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth);
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

  test("issue-type slicer is multi-select and writes issue_types to the URL", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/throughput");

    // Wait for initial fetches to settle so we only capture refetches.
    await expect(
      page.getByTestId("chart-issues-cycle-time").getByTestId("issues-cycle-time-content"),
    ).toBeVisible();

    const requests: string[] = [];
    page.on("request", (req) => {
      if (req.url().includes("/v1/analytics/throughput/issues/cycle_time")) {
        requests.push(req.url());
      }
    });

    // Single-checkbox subcase: one tick → `issue_types=feature`.
    await page.getByTestId("slicer-issue-type-feature").click();
    await expect(page).toHaveURL(/issue_types=feature(?!%2C|,)/);
    await expect
      .poll(() => requests.some((u) => /issue_types=feature(?!%2C|,)/.test(u)), {
        timeout: 5_000,
      })
      .toBe(true);

    // Tick a second checkbox: URL carries both values, comma-joined
    // (order-insensitive — the URL-encoded comma is `%2C`).
    await page.getByTestId("slicer-issue-type-bug").click();
    await expect(page).toHaveURL(
      /issue_types=(feature%2Cbug|bug%2Cfeature|feature,bug|bug,feature)/,
    );
    await expect
      .poll(
        () => {
          return requests.some((u) => {
            const match = u.match(/issue_types=([^&]+)/);
            if (!match) return false;
            const decoded = decodeURIComponent(match[1]).split(",").sort();
            return decoded.length === 2 && decoded[0] === "bug" && decoded[1] === "feature";
          });
        },
        { timeout: 5_000 },
      )
      .toBe(true);

    // Unchecking both clears the param.
    await page.getByTestId("slicer-issue-type-feature").click();
    await page.getByTestId("slicer-issue-type-bug").click();
    await expect(page).not.toHaveURL(/issue_types=/);
    await expect(page).not.toHaveURL(/issue_type=/);
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
