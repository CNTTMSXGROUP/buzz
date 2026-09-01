import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

async function openThread(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem("buzz.channels.threadViewMode", "split");
  });
  await installMockBridge(page);
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  await page.evaluate(() => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      parentEventId: "mock-general-welcome",
      content: [
        "The thread should use the available panel width in both layouts. Expanding it keeps the channel visible behind the drawer, while messages and the reply composer retain their normal edge spacing.",
        "## Validation notes",
        "| Check | Result |\n| --- | --- |\n| Message margins | Match the split pane |\n| Reply composer | Uses the same edge spacing |\n| Channel sliver | Still visible behind the drawer |\n| Long content | Wraps within the panel |",
        "This longer paragraph makes the reading measure visible. Switching back to the split pane should preserve the conversation and return to the same pane width, without changing the channel navigation or the drawer’s outer position.",
      ].join("\n\n"),
    });
  });
  await page.getByTestId("channel-general").click();
  const root = page.locator(
    '[data-testid="message-row"][data-message-id="mock-general-welcome"]',
  );
  await root.hover();
  await root.getByRole("button", { name: "Reply", exact: true }).click();
  await expect(page.getByTestId("message-thread-body")).toBeVisible();
}

async function geometry(page: Page) {
  return page.getByTestId("message-thread-body").evaluate((body) => {
    const panel = body.closest('[data-testid="message-thread-panel"]');
    const row = body.querySelector('[data-message-id="mock-general-welcome"]');
    const composer = panel?.querySelector('[data-testid="message-composer"]');
    if (!panel || !row || !composer) throw new Error("Missing thread geometry");
    const outer = panel.getBoundingClientRect();
    const message = row.getBoundingClientRect();
    const input = composer.getBoundingClientRect();
    return {
      panel: { x: outer.x, width: outer.width },
      row: {
        left: message.left - outer.left,
        right: outer.right - message.right,
      },
      composer: {
        left: input.left - outer.left,
        right: outer.right - input.right,
      },
      overflow: body.scrollWidth - body.clientWidth,
    };
  });
}

for (const width of [1280, 1720, 2560]) {
  test(`focus content keeps split gutters at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openThread(page);
    await waitForAnimations(page);
    const split = await geometry(page);
    await page.screenshot({ path: testInfo.outputPath("split.png") });

    await page
      .getByRole("button", { name: "Expand thread", exact: true })
      .click();
    await expect(page.getByTestId("focus-thread-drawer")).toBeVisible();
    await waitForAnimations(page);
    // Motion's transform can finish on a later animation frame than CSS animations.
    await expect
      .poll(async () => {
        const drawer = await page
          .getByTestId("focus-thread-drawer")
          .boundingBox();
        const overlay = await page
          .getByTestId("focus-thread-drawer-overlay")
          .boundingBox();
        return Math.abs((drawer?.x ?? 0) - (overlay?.x ?? 0) - 72);
      })
      .toBeLessThan(0.01);
    const focus = await geometry(page);
    await page.screenshot({ path: testInfo.outputPath("focus.png") });
    await testInfo.attach("geometry", {
      body: JSON.stringify({ split, focus }, null, 2),
      contentType: "application/json",
    });

    expect(focus.panel.width).toBeGreaterThan(split.panel.width);
    expect(focus.overflow).toBe(0);
    for (const edge of ["left", "right"] as const) {
      expect(focus.row[edge]).toBeCloseTo(split.row[edge], 1);
      expect(focus.composer[edge]).toBeCloseTo(split.composer[edge], 1);
    }

    // Keep the outer drawer position: the existing 72px channel sliver.
    const overlay = await page
      .getByTestId("focus-thread-drawer-overlay")
      .boundingBox();
    expect(overlay).not.toBeNull();
    expect(focus.panel.x - (overlay?.x ?? 0)).toBeCloseTo(72, 1);
    expect(focus.panel.width).toBeCloseTo((overlay?.width ?? 0) - 72, 1);

    await page
      .getByRole("button", { name: "Show thread beside channel", exact: true })
      .click();
    await expect(page.getByTestId("focus-thread-drawer")).toHaveCount(0);
    await waitForAnimations(page);
    expect(await geometry(page)).toEqual(split);
  });
}
