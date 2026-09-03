import assert from "node:assert/strict";
import { test } from "node:test";
import { clipForChannel } from "./shareToChannel.ts";

test("clipForChannel bỏ frontmatter", () => {
  assert.equal(clipForChannel("---\nloai: x\n---\n# Tiêu đề\n\nthân"), "# Tiêu đề\n\nthân");
});

test("clipForChannel cắt 3000 ký tự", () => {
  assert.equal(clipForChannel("x".repeat(4000)).length, 3000);
});
