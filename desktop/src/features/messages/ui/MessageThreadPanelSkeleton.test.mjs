import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { getThreadPanelLayout } from "../../channels/lib/threadPanelLayout.ts";
import { MessageThreadPanelSkeleton } from "./MessageThreadPanelSkeleton.tsx";

function renderSkeleton(isFocusDrawer) {
  const html = renderToStaticMarkup(
    React.createElement(MessageThreadPanelSkeleton, {
      ...getThreadPanelLayout({
        isFocusDrawer,
        isSinglePanelView: false,
        useSplitAuxiliaryPane: true,
      }),
      widthPx: 480,
      onClose: () => {},
    }),
  );
  return new JSDOM(html).window.document;
}

test("focus and split loading states use identical message and composer gutters", () => {
  const focus = renderSkeleton(true);
  const split = renderSkeleton(false);
  for (const selector of [
    '[data-testid="message-thread-loading"] > div',
    ".pointer-events-none.bottom-0 > .pointer-events-auto",
  ]) {
    const focusedContent = focus.querySelector(selector);
    const splitContent = split.querySelector(selector);
    assert.ok(focusedContent, `Missing focused skeleton: ${selector}`);
    assert.ok(splitContent, `Missing split skeleton: ${selector}`);
    assert.equal(focusedContent.outerHTML, splitContent.outerHTML);
  }
});
