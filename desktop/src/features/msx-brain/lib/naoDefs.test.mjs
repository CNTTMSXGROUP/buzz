import assert from "node:assert/strict";
import { test } from "node:test";
import { filterByNao, khuForNao, naoCoversPath, normalizeNaoList } from "./naoDefs.ts";

test("normalizeNaoList: string cũ -> {id,path} trong Nao Bo Phan", () => {
  assert.deepEqual(normalizeNaoList(["mkt", "chung"]), [
    { id: "mkt", path: "Nao Bo Phan/mkt" },
    { id: "chung", path: "Nao Bo Phan/chung" },
  ]);
});

test("normalizeNaoList: object mới giữ path tuỳ ý", () => {
  assert.deepEqual(normalizeNaoList([{ id: "x", path: "Du An/X" }]), [{ id: "x", path: "Du An/X" }]);
});

test("filterByNao: giữ trong não + tổ tiên để điều hướng, bỏ não khác", () => {
  const entries = [
    { name: "a.md", rel_path: "Du An/X/a.md", is_dir: false, area: "Du An" },
    { name: "z.md", rel_path: "Du An/Khac/z.md", is_dir: false, area: "Du An" },
    { name: "Du An", rel_path: "Du An", is_dir: true, area: "Du An" },
    { name: "Du An/X", rel_path: "Du An/X", is_dir: true, area: "Du An" },
    { name: "1. Thu Thập", rel_path: "1. Thu Thập", is_dir: true, area: "1. Thu Thập" },
  ];
  const got = filterByNao(entries, { id: "x", path: "Du An/X" }, ["Du An/X", "Du An/Khac"]);
  assert.deepEqual(got.map((e) => e.rel_path), [
    "Du An/X/a.md",
    "Du An",
    "Du An/X",
    "1. Thu Thập",
  ]);
});

test("khuForNao: prefix path cho Rust", () => {
  const defs = [
    { id: "x", path: "Du An/X" },
    { id: "y", path: "Nao Bo Phan/y" },
  ];
  assert.equal(khuForNao(defs, ["x", "y"]), "Du An/X,Nao Bo Phan/y");
});

test("naoCoversPath: khớp đúng prefix, không khớp lân cận", () => {
  assert.ok(naoCoversPath({ id: "x", path: "Du An/X" }, "Du An/X/a.md"));
  assert.ok(!naoCoversPath({ id: "x", path: "Du An/X" }, "Du An/XY/a.md"));
});
