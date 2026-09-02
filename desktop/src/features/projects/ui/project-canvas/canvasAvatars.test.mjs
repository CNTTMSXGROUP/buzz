import assert from "node:assert/strict";
import test from "node:test";

import {
  CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET,
  selectAvatarsWithinBudget,
} from "./canvasAvatars.ts";
import { PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES } from "./projectCanvasProtocol.ts";

/** A data URL of exactly `length` characters. */
function avatar(length) {
  const prefix = "data:image/webp;base64,";
  return prefix + "A".repeat(Math.max(0, length - prefix.length));
}

function totalLength(dataUrls) {
  return dataUrls.reduce((sum, value) => sum + (value?.length ?? 0), 0);
}

test("the combined avatar budget leaves room inside one rpc message", () => {
  // The whole response — avatars, names, pubkeys, envelope — must fit the port
  // ceiling, so the avatar share has to stay strictly under it.
  assert.ok(
    CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET < PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES,
  );
});

test("avatars are kept in order until the budget is spent", () => {
  const rows = [avatar(10_000), avatar(10_000), avatar(10_000)];

  const selected = selectAvatarsWithinBudget(rows, 25_000);

  assert.deepEqual(
    selected.map((value) => value !== null),
    [true, true, false],
  );
  assert.ok(totalLength(selected) <= 25_000);
});

test("a lookup that would overrun the port ceiling is trimmed under it", () => {
  // 32 people is the lookup maximum; at the per-avatar cap they would total
  // 512 KiB and the frame would get `too-large` instead of the result.
  const rows = Array.from({ length: 32 }, () => avatar(16 * 1_024));

  const selected = selectAvatarsWithinBudget(rows);

  assert.ok(totalLength(selected) <= CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET);
  assert.ok(totalLength(selected) < PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES);
  assert.ok(selected.some((value) => value !== null));
});

test("a single avatar larger than the whole budget is dropped", () => {
  const selected = selectAvatarsWithinBudget([avatar(50_000)], 40_000);

  assert.deepEqual(selected, [null]);
});

test("missing avatars pass through without consuming budget", () => {
  const selected = selectAvatarsWithinBudget(
    [null, avatar(30_000), null, avatar(9_000)],
    40_000,
  );

  assert.deepEqual(
    selected.map((value) => value !== null),
    [false, true, false, true],
  );
});

test("the result is index-aligned with its input so rows can be zipped back", () => {
  const rows = [avatar(100), null, avatar(100)];

  assert.equal(selectAvatarsWithinBudget(rows).length, rows.length);
});
