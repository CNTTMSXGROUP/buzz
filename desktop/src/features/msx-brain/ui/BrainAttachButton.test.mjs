/** Kiểm tra token não — parse lại từ text tin nhắn (hợp lệ: [nao:rel_path]) */
import assert from "node:assert/strict";
import { test } from "node:test";

test("token nao format hợp lệ", () => {
  const rel = "2. Tinh Lọc/Kiến Thức Nguồn/Hướng dẫn bán hàng.md";
  const token = `[nao:${rel}]`;
  assert.ok(token.startsWith("[nao:") && token.endsWith("]"));
  const m = token.match(/^\[nao:(.+)\]$/);
  assert.ok(m);
  assert.equal(m[1], rel);
});
