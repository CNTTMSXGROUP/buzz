import assert from "node:assert/strict";
import { test } from "node:test";
import { stripGhiNhanh } from "./approve.ts";

test("stripGhiNhanh bỏ tiền tố + đuôi .md", () => {
  assert.equal(stripGhiNhanh("GHI NHANH — demo.md"), "demo");
  assert.equal(stripGhiNhanh("Kế hoạch A.md"), "Kế hoạch A");
});
