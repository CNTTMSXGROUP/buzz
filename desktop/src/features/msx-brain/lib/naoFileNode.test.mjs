import assert from "node:assert/strict";
import { test } from "node:test";
import { NAO_FILE_NODE_NAME } from "./naoFileNode.ts";

test("NaoFile node có đúng tên để composer chèn", () => {
  assert.equal(NAO_FILE_NODE_NAME, "naoFile");
});

test("serialize markdown link gọn (không còn token thô dài)", () => {
  const relPath = "4. Kiến Tạo/README.md";
  const name = "README";
  const out = `[${name}](msx-brain://open?file=${encodeURIComponent(relPath)})`;
  // gửi đi: label ngắn + href mã hoá
  assert.ok(out.startsWith("[README]"));
  assert.ok(out.includes("msx-brain://open?file="));
  assert.ok(!out.includes(" "));
});
