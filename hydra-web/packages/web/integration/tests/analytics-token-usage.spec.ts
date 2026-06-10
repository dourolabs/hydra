import { test, expect } from "../fixtures/auth";

test.describe("Analytics token usage @analytics:token-usage", () => {
  test("sidebar entry navigates to the token usage page", async ({ authenticatedPage: page }) => {
    await page.getByTestId("sidebar-analytics-token-usage").click();
    await expect(page).toHaveURL(/\/analytics\/token-usage/);
    await expect(page.getByTestId("analytics-token-usage-page")).toBeVisible();
    await expect(page.getByTestId("analytics-tokens-section")).toBeVisible();
  });

  test("page renders breadcrumb and title", async ({ authenticatedPage: page }) => {
    await page.goto("/analytics/token-usage");
    await expect(page.getByRole("heading", { name: "Token Usage" })).toBeVisible();
    // Breadcrumb segment from useBreadcrumbs.
    await expect(page.getByRole("link", { name: "Analytics" })).toBeVisible();
  });

  test("renders the tokens-over-time chart with stub data", async ({ authenticatedPage: page }) => {
    await page.goto("/analytics/token-usage");
    const chart = page.getByTestId("chart-tokens-over-time");
    await expect(chart).toBeVisible();
    await expect(chart.getByTestId("tokens-over-time-content")).toBeVisible();
    // All four legend entries render so we know each series mounted.
    for (const key of [
      "input_tokens",
      "output_tokens",
      "cache_read_input_tokens",
      "cache_creation_input_tokens",
    ]) {
      await expect(chart.getByTestId(`tokens-over-time-legend-${key}`)).toBeVisible();
    }
  });

  test("time-range buttons update the URL and re-issue the request with new from/to", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/token-usage");
    await expect(
      page.getByTestId("chart-tokens-over-time").getByTestId("tokens-over-time-content"),
    ).toBeVisible();

    const requests: string[] = [];
    page.on("request", (req) => {
      if (req.url().includes("/v1/analytics/token_usage/over_time")) {
        requests.push(req.url());
      }
    });

    await page.getByTestId("time-range-7d").click();
    await expect(page).toHaveURL(/range=7d/);
    await expect
      .poll(() => requests.some((u) => /from=[^&]+/.test(u) && /to=[^&]+/.test(u)), {
        timeout: 5_000,
      })
      .toBe(true);
    const firstFrom = extractParam(requests.at(-1)!, "from");

    await page.getByTestId("time-range-90d").click();
    await expect(page).toHaveURL(/range=90d/);
    await expect
      .poll(
        () => {
          const latest = requests.at(-1);
          if (!latest) return false;
          return extractParam(latest, "from") !== firstFrom;
        },
        { timeout: 5_000 },
      )
      .toBe(true);

    await page.getByTestId("time-range-all-time").click();
    await expect(page).toHaveURL(/range=all-time/);
  });

  test("chart card is a labeled region for screen readers", async ({ authenticatedPage: page }) => {
    await page.goto("/analytics/token-usage");
    const card = page.getByTestId("chart-tokens-over-time");
    await expect(card).toHaveAttribute("role", "region");
    await expect(card).toHaveAttribute("aria-label", /.+/);
  });

  test("renders the cost-per-agent bar chart with stub data", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/token-usage");
    const chart = page.getByTestId("chart-cost-per-agent");
    await expect(chart).toBeVisible();
    await expect(chart.getByTestId("cost-per-agent-content")).toBeVisible();
    // Y-axis labels match the stub's agent order, including the Ad-hoc bucket.
    for (const label of ["swe", "pm", "reviewer", "Ad-hoc"]) {
      await expect(chart.locator("text", { hasText: new RegExp(`^${label}$`) })).toBeVisible();
    }
  });

  test("renders one scatter point per session in the stub", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/token-usage");
    const chart = page.getByTestId("chart-per-session-cost");
    await expect(chart).toBeVisible();
    await expect(chart.getByTestId("per-session-cost-content")).toBeVisible();
    // Stub has 3 + 2 + 3 + 2 = 10 sessions across the four agents.
    const points = chart.locator(".recharts-scatter-symbol");
    await expect(points).toHaveCount(10);
  });

  test("renders top-issues list in stub order with working issue links", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/token-usage");
    const card = page.getByTestId("chart-top-issues-by-cost");
    await expect(card).toBeVisible();
    const list = card.getByTestId("top-issues-by-cost-content");
    await expect(list).toBeVisible();
    const items = list.locator("li");
    await expect(items).toHaveCount(5);
    // First row's title links to the matching /issues/<id> route.
    const firstLink = items.nth(0).getByRole("link", {
      name: "Token Usage dashboard",
    });
    await expect(firstLink).toHaveAttribute("href", "/issues/i-abcd1234");
  });

  test("time-range picker re-issues all four token-usage requests", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/analytics/token-usage");
    await expect(page.getByTestId("cost-per-agent-content")).toBeVisible();
    await expect(page.getByTestId("per-session-cost-content")).toBeVisible();
    await expect(page.getByTestId("top-issues-by-cost-content")).toBeVisible();

    const seen: Record<string, string[]> = {
      over_time: [],
      cost_per_agent: [],
      top_issues_by_cost: [],
    };
    page.on("request", (req) => {
      const url = req.url();
      for (const key of Object.keys(seen)) {
        if (url.includes(`/v1/analytics/token_usage/${key}`)) {
          seen[key].push(url);
        }
      }
    });

    await page.getByTestId("time-range-7d").click();
    await expect(page).toHaveURL(/range=7d/);
    await expect
      .poll(
        () =>
          seen.over_time.length > 0 &&
          seen.cost_per_agent.length > 0 &&
          seen.top_issues_by_cost.length > 0,
        { timeout: 5_000 },
      )
      .toBe(true);

    for (const key of ["over_time", "cost_per_agent", "top_issues_by_cost"]) {
      const url = seen[key].at(-1);
      expect(url).toBeDefined();
      expect(url).toMatch(/[?&]from=/);
      expect(url).toMatch(/[?&]to=/);
    }
  });
});

function extractParam(url: string, key: string): string | null {
  const match = url.match(new RegExp(`[?&]${key}=([^&]+)`));
  return match ? decodeURIComponent(match[1]) : null;
}
