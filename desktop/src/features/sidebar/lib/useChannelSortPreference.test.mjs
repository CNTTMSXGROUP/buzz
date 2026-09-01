import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { makeHookStubs } from "./sidebarSyncTestHelpers.mjs";
import { runWholeBlobHookSuite } from "./wholeBlobHook.shared.test.mjs";

const { act, cleanup, renderHook } = await import("@testing-library/react");
const { relayClient } = await import("@/shared/api/relayClient");
const { useChannelSortPreference } = await import(
  "./useChannelSortPreference.ts"
);
const { readChannelSortOutbox, storageKey } = await import(
  "./channelSortPreference.ts"
);
const { normalizeRelayUrl } = await import("@/shared/lib/normalizeRelayUrl");

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});
after(() => dom.window.close());

const { stubRelay, stubTauri } = makeHookStubs();

runWholeBlobHookSuite({
  label: "sort",
  dTag: "channel-sort",
  useHook: useChannelSortPreference,
  storageKey,
  readOutbox: readChannelSortOutbox,
  makeEdit: (r) => r.setSortModeFor("channels", "alpha"),
  makeB1Store: () =>
    JSON.stringify({ version: 1, groups: { channels: "recent" } }),
  assertNotContainsB1: (r, msg) =>
    assert.equal(r.sortModeFor("channels"), "alpha", msg),
  makeA2Edit: (r) => r.setSortModeFor("starred", "recent"),
  assertA2Derived: (r, msg) =>
    assert.equal(r.sortModeFor("channels"), "alpha", msg),
  makeLiveRemotePayload: () =>
    JSON.stringify({ version: 1, groups: { "remote-group": "recent" } }),
  assertRemoteNotApplied: (r, msg) =>
    assert.equal(r.sortModeFor("channels"), "alpha", msg),
  makeDecryptById: (id) =>
    JSON.stringify({ version: 1, groups: { [id]: "recent" } }),
  assertLowerIdWon: (r, msg) =>
    assert.equal(r.sortModeFor("aaaa"), "recent", msg),
  assertHigherIdLost: (r, msg) =>
    assert.equal(r.sortModeFor("bbbb"), "alpha", msg),
  makeRemotePayload: () =>
    JSON.stringify({ version: 1, groups: { "remote-group": "recent" } }),
  assertLocalAdoptedAway: (r, msg) =>
    assert.equal(r.sortModeFor("channels"), "alpha", msg),
});
// Mutation: writing the consumed marker unconditionally loses the legacy blob.

test("legacy replay whose v2 transfer fails (quota) does not write the consumed marker", async () => {
  const origLocalStorage = window.localStorage;
  const pubkey = "pk-quota";
  const relayUrl = "wss://r.quota";
  const scope = `${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
  const legacyKey = `buzz-channel-sort-outbox.v1:${scope}`;
  const v2Prefix = `buzz-channel-sort-outbox.v1:${scope}:`;
  const legacyRaw = JSON.stringify({
    store: { version: 1, groups: { "legacy-group": "recent" } },
    queuedAt: 0,
  });
  const map = new Map([[legacyKey, legacyRaw]]);
  const throwingStorage = {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => {
      if (k.startsWith(v2Prefix)) throw new Error("QuotaExceededError");
      map.set(k, String(v));
    },
    removeItem: (k) => map.delete(k),
    clear: () => map.clear(),
    get length() {
      return map.size;
    },
    key: (i) => [...map.keys()][i] ?? null,
  };
  const restoreRelay = stubRelay(relayClient);
  const restoreTauri = stubTauri(pubkey, null);
  let hook = null;
  try {
    Object.defineProperty(window, "localStorage", {
      value: throwingStorage,
      configurable: true,
    });
    window.localStorage.setItem(
      `buzz-sync-watermark.v1:channel-sort:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`,
      "1700000000",
    );
    await act(async () => {
      hook = renderHook(() => useChannelSortPreference(pubkey, relayUrl));
      for (let i = 0; i < 4; i++) await Promise.resolve();
    });
    assert.equal(
      [...map.keys()].some((k) => k.includes("-legacy-consumed:")),
      false,
      "consumed marker must not be written after failed v2 transfer",
    );
    const resumed = readChannelSortOutbox(pubkey, relayUrl);
    assert.ok(resumed !== null, "legacy blob must remain replayable");
    assert.equal(resumed.store.groups["legacy-group"], "recent");
    assert.ok(resumed.legacyRawToConsume !== null);
    hook.unmount();
  } finally {
    cleanup();
    Object.defineProperty(window, "localStorage", {
      value: origLocalStorage,
      configurable: true,
    });
    restoreRelay();
    restoreTauri();
  }
});
