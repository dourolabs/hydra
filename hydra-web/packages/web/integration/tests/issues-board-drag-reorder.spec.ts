import { test, expect } from "../fixtures/auth";
import type { Page } from "@playwright/test";

// Real-DOM drag wrapper. dnd-kit's `PointerSensor` activates only after the
// pointer moves past its `activationConstraint.distance` (4 px in this
// codebase), and the project bar drag synchronously collapses every section
// to its bar mid-drag — so the drop target's coordinates must be measured
// against that collapsed layout. We step the pointer in small increments
// from start → end so the activation distance is consistently crossed and
// the move events drive dnd-kit's measuring pipeline.
async function dragWithMouse(
  page: Page,
  startX: number,
  startY: number,
  endX: number,
  endY: number,
): Promise<void> {
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  const steps = 25;
  for (let i = 1; i <= steps; i++) {
    const x = startX + ((endX - startX) * i) / steps;
    const y = startY + ((endY - startY) * i) / steps;
    await page.mouse.move(x, y);
  }
  await page.mouse.up();
}

test.describe("Issues board drag-to-reorder @issues:board-drag-reorder", () => {
  test("dragging a project bar fires PUT /v1/projects/<id> with the new priority and persists across reload @issues:board-drag-reorder", async ({
    authenticatedPage: page,
  }) => {
    const projectPuts: Array<{ url: string; body: string | null; status: number }> = [];
    page.on("response", async (resp) => {
      const req = resp.request();
      const url = resp.url();
      if (
        req.method() === "PUT" &&
        /\/v1\/projects\/[^/]+$/.test(new URL(url).pathname)
      ) {
        projectPuts.push({
          url: new URL(url).pathname,
          body: req.postData(),
          status: resp.status(),
        });
      }
    });

    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // Seed has two real projects: `default` (priority 1000) and
    // `engineering-v2` (priority 2000). They render in that order.
    const defaultBar = page.getByTestId("board-project-bar-default");
    const engBar = page.getByTestId("board-project-bar-engineering-v2");
    await expect(defaultBar).toBeVisible();
    await expect(engBar).toBeVisible();

    const initialOrder = await page
      .locator("[data-testid^='board-project-bar-']")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
    expect(initialOrder).toEqual([
      "board-project-bar-default",
      "board-project-bar-engineering-v2",
    ]);

    // Drag engineering-v2's bar above default's. Aim a few pixels above the
    // top edge of the default bar's box so dnd-kit's collision detection
    // resolves engineering-v2 as the new first item.
    //
    // The default project's seed has 23 issues across 5 status columns, so
    // engineering-v2's bar sits well below the 720px viewport on first paint
    // and `mouse.move` at its native y never lands on it. Scroll it into view
    // before measuring so the drag origin is inside the viewport — once the
    // drag activates, every section's body collapses (see ProjectSection's
    // `!dragActive` guard) and the document shrinks back to a tight column of
    // bars, so the browser auto-snaps scroll to 0 and the cursor passes over
    // the default bar's new (visible) position during the step-through.
    await engBar.scrollIntoViewIfNeeded();
    const srcBox = await engBar.boundingBox();
    const tgtBox = await defaultBar.boundingBox();
    expect(srcBox && tgtBox).toBeTruthy();
    if (!srcBox || !tgtBox) throw new Error("missing bounding box");
    await dragWithMouse(
      page,
      srcBox.x + srcBox.width / 2,
      srcBox.y + srcBox.height / 2,
      tgtBox.x + tgtBox.width / 2,
      tgtBox.y + 4,
    );

    // Settles after the optimistic reorder + PUT.
    await expect(async () => {
      expect(projectPuts.length).toBeGreaterThanOrEqual(1);
    }).toPass({ timeout: 3000 });

    // Exactly one project-level PUT, against engineering-v2 (the moved
    // project), carrying a numeric `priority`.
    expect(projectPuts).toHaveLength(1);
    const put = projectPuts[0];
    expect(put.status).toBe(200);
    expect(put.url).toBe("/api/v1/projects/j-engv2");
    const body = put.body ? (JSON.parse(put.body) as { priority: number }) : null;
    expect(body).not.toBeNull();
    expect(typeof body!.priority).toBe("number");
    // Top-of-list extension uses the existing `computeReorderPriority` rule:
    // `left.priority - PROJECT_PRIORITY_STEP` = 1000 - 1024 = -24.
    expect(body!.priority).toBe(-24);

    // Optimistic UI must show the new order before reload.
    const afterDragOrder = await page
      .locator("[data-testid^='board-project-bar-']")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
    expect(afterDragOrder).toEqual([
      "board-project-bar-engineering-v2",
      "board-project-bar-default",
    ]);

    // Reload — the server-sorted list (`priority ASC`) must keep
    // engineering-v2 ahead of default.
    await page.reload();
    await page.getByTestId("issues-layout-board").click();
    await expect(
      page.getByTestId("board-project-bar-engineering-v2"),
    ).toBeVisible();
    const afterReloadOrder = await page
      .locator("[data-testid^='board-project-bar-']")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
    expect(afterReloadOrder).toEqual([
      "board-project-bar-engineering-v2",
      "board-project-bar-default",
    ]);
  });

  test("dragging a status column head fires sequential PUT /v1/projects/<ref>/statuses/<key> calls with new positions and persists across reload @issues:board-drag-reorder", async ({
    authenticatedPage: page,
  }) => {
    const statusPuts: Array<{
      url: string;
      key: string | null;
      position: number | null;
      status: number;
    }> = [];
    page.on("response", async (resp) => {
      const req = resp.request();
      const url = resp.url();
      const pathname = new URL(url).pathname;
      if (
        req.method() === "PUT" &&
        /\/v1\/projects\/[^/]+\/statuses\/[^/]+$/.test(pathname)
      ) {
        const body = req.postData();
        const parsed = body
          ? (JSON.parse(body) as { key?: string; position?: number })
          : null;
        statusPuts.push({
          url: pathname,
          key: parsed?.key ?? null,
          position: typeof parsed?.position === "number" ? parsed.position : null,
          status: resp.status(),
        });
      }
    });

    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // Default project has 5 statuses in fixed seed order.
    const initialColumns = await page
      .locator("[data-testid^='board-col-head-default-']")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
    expect(initialColumns).toEqual([
      "board-col-head-default-open",
      "board-col-head-default-in-progress",
      "board-col-head-default-closed",
      "board-col-head-default-dropped",
      "board-col-head-default-failed",
    ]);

    // Drag the `open` column past the `in-progress` column.
    const openHead = page.getByTestId("board-col-head-default-open");
    const inProgressHead = page.getByTestId(
      "board-col-head-default-in-progress",
    );
    const srcBox = await openHead.boundingBox();
    const tgtBox = await inProgressHead.boundingBox();
    expect(srcBox && tgtBox).toBeTruthy();
    if (!srcBox || !tgtBox) throw new Error("missing bounding box");
    await dragWithMouse(
      page,
      srcBox.x + srcBox.width / 2,
      srcBox.y + srcBox.height / 2,
      tgtBox.x + tgtBox.width / 2 + 30,
      tgtBox.y + tgtBox.height / 2,
    );

    // One PUT per status in the column row (the reorder mutation persists
    // every position).
    await expect(async () => {
      expect(statusPuts.length).toBe(5);
    }).toPass({ timeout: 3000 });

    for (const put of statusPuts) {
      expect(put.status).toBe(200);
      expect(put.url.startsWith("/api/v1/projects/j-defaul/statuses/")).toBe(true);
      expect(put.position).not.toBeNull();
    }
    // Positions are recomputed as `index * 100` against the new column order
    // (`in-progress`, `open`, `closed`, `dropped`, `failed`).
    const byKey = new Map(statusPuts.map((p) => [p.key, p.position]));
    expect(byKey.get("in-progress")).toBe(0);
    expect(byKey.get("open")).toBe(100);
    expect(byKey.get("closed")).toBe(200);
    expect(byKey.get("dropped")).toBe(300);
    expect(byKey.get("failed")).toBe(400);

    const afterDragColumns = await page
      .locator("[data-testid^='board-col-head-default-']")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
    expect(afterDragColumns).toEqual([
      "board-col-head-default-in-progress",
      "board-col-head-default-open",
      "board-col-head-default-closed",
      "board-col-head-default-dropped",
      "board-col-head-default-failed",
    ]);

    await page.reload();
    await page.getByTestId("issues-layout-board").click();
    await expect(
      page.getByTestId("board-col-head-default-in-progress"),
    ).toBeVisible();
    const afterReloadColumns = await page
      .locator("[data-testid^='board-col-head-default-']")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
    expect(afterReloadColumns).toEqual([
      "board-col-head-default-in-progress",
      "board-col-head-default-open",
      "board-col-head-default-closed",
      "board-col-head-default-dropped",
      "board-col-head-default-failed",
    ]);
  });
});
