import { test, expect } from "../../fixtures/auth";

test.describe("Mobile Swipe to Archive @mobile:swipe-archive", () => {
  test("swiping an inbox item past threshold archives it @mobile:swipe-archive", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=inbox");

    const itemText = "Update deployment documentation";
    await expect(page.getByText(itemText)).toBeVisible();

    // The row containing the item has an archive reveal div with "Archive" text
    // and a swipeContent div that handles touch gestures
    const row = page.locator("li", { hasText: itemText });

    // Verify the archive reveal background is rendered with "Archive" text
    await expect(row.getByText("Archive")).toBeAttached();

    // Get the swipe content element (second child div of the row)
    const swipeContent = row.locator("> div:nth-child(2)");
    const box = await swipeContent.boundingBox();
    expect(box).toBeTruthy();

    const startX = box!.x + box!.width - 30;
    const centerY = box!.y + box!.height / 2;

    // Simulate touch swipe: touchstart, touchmove past 100px threshold, then touchend
    await swipeContent.evaluate(
      (el, { startX, centerY }) => {
        const createTouch = (x: number, y: number) =>
          new Touch({ identifier: 0, target: el, clientX: x, clientY: y });

        el.dispatchEvent(
          new TouchEvent("touchstart", {
            bubbles: true,
            touches: [createTouch(startX, centerY)],
          }),
        );

        // Move in steps past the 100px commit threshold
        for (let dx = 0; dx <= 120; dx += 20) {
          el.dispatchEvent(
            new TouchEvent("touchmove", {
              bubbles: true,
              touches: [createTouch(startX - dx, centerY)],
            }),
          );
        }
      },
      { startX, centerY },
    );

    // During the swipe, the swipe content has moved left revealing the archive background
    // Verify the archive reveal is in the DOM (it's always rendered behind the content)
    await expect(row.getByText("Archive")).toBeVisible();

    // Complete the swipe by dispatching touchend
    await swipeContent.evaluate((el) => {
      el.dispatchEvent(
        new TouchEvent("touchend", {
          bubbles: true,
          touches: [],
        }),
      );
    });

    // After the swipe commits and the archive mutation fires,
    // the item should be removed from the inbox list
    await expect(page.getByText(itemText)).not.toBeVisible({ timeout: 5000 });
  });
});
