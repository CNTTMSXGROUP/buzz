/**
 * Test BrainPanel: render cây + mở file (mock Tauri invoke qua __TAURI_INTERNALS__).
 * Chạy: node --import ./test-loader.mjs --experimental-strip-types --test src/features/msx-brain/ui/BrainPanel.test.mjs
 * (trong desktop/)
 */
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://localhost" });

before(() => {
  const tauriMock = {
    invoke: async (cmd, args) => {
      if (cmd === "brain_list_tree") {
        return [
          { name: "1. Thu Thập", rel_path: "1. Thu Thập", is_dir: true, area: "1. Thu Thập" },
          { name: "a.md", rel_path: "1. Thu Thập/a.md", is_dir: false, area: "1. Thu Thập" },
          { name: "_meta", rel_path: "_meta", is_dir: true, area: "_meta" },
        ];
      }
      if (cmd === "brain_read_file") {
        if (args?.rel_path === "_meta/nguoi-dung.json")
          return JSON.stringify({
            nguoi: [{ ten: "Thắng", pubkey: "PK1", vai_tro: "chu", khu: "*" }],
          });
        return "# A\n\nnội dung";
      }
      return null;
    },
  };
  dom.window.__TAURI_INTERNALS__ = tauriMock;
  Object.assign(globalThis, {
    __TAURI_INTERNALS__: tauriMock,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("BrainPanel hiển thị cây + mở file khi click", async () => {
  const React = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { BrainPanel } = await import("./BrainPanel.tsx");
  render(React.createElement(BrainPanel, { vaultRoot: "/tmp/vault", myPubkey: "PK1" }));
  assert.ok(await screen.findByText("1. Thu Thập"));
  fireEvent.click(screen.getByText(/a\.md/));
  assert.ok(await screen.findByText("nội dung"));
});

