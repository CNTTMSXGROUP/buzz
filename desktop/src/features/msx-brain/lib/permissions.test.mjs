/**
 * Unit tests cho phân quyền Não MSX — mirror logic khu_ok của msx_brain.rs.
 * Chạy: pnpm -C desktop test -- src/features/msx-brain/lib/permissions.test.mjs
 */
import { test } from "node:test";
import assert from "node:assert/strict";

import { canReadPath } from "./permissions.ts";

const owner = { ten: "Thắng", vaiTro: "chu", khu: "*" };
const nvmkt = { ten: "Văn A", vaiTro: "nhan-vien", khu: "mkt" };

test("owner đọc hết trừ _mat và thư mục hệ thống", () => {
  assert.equal(canReadPath(owner, "2. Tinh Lọc/x.md"), true);
  assert.equal(canReadPath(owner, "MSX Knowledge/a.md"), true);
  assert.equal(canReadPath(owner, "_mat/lương.md"), false);
  assert.equal(canReadPath(owner, ".git/config"), false);
  assert.equal(canReadPath(owner, "a/.obsidian/b.json"), false);
});

test("nhân viên mkt chỉ thấy khu mkt + khu chung", () => {
  assert.equal(canReadPath(nvmkt, "mkt/ghi nhanh.md"), true);
  assert.equal(canReadPath(nvmkt, "1. Thu Thập/a.md"), true);
  assert.equal(canReadPath(nvmkt, "MSX Knowledge/x.md"), true);
  assert.equal(canReadPath(nvmkt, "tech/runbook.md"), false);
});

test("_mat bị chặn dù khu trùng", () => {
  assert.equal(canReadPath(nvmkt, "mkt/_mat/x.md"), false);
  assert.equal(canReadPath(owner, "mkt/_mat/x.md"), false);
});
