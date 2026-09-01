// Compact wire-contract adapter for ChannelStarSyncManager.
// Shared engine invariants are in mergeLaneSync.shared.test.mjs.
// This file asserts only stars-specific wiring: event kind, d-tag, payload shape, parser delegation, and typed API.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  parseStarPayload,
  readChannelStarsOutbox,
} from "./channelStarsStorage.ts";
import { ChannelStarSyncManager } from "./channelStarsSync.ts";
import {
  installEchoTauri,
  installFakeWindow,
  makeFakeWindow,
} from "./sidebarSyncTestHelpers.mjs";

const RELAY = "wss://r.test";

test("stars wire: kind=30078, d-tag='channel-stars', payload has 'channels' not 'sections'", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  let publishedEvent = null;
  mock.method(relayClient, "publishEvent", (evt) => {
    publishedEvent = evt;
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-wire-stars");
  try {
    const m = new ChannelStarSyncManager("pk-wire-stars", RELAY);
    m.publishStars({
      version: 1,
      channels: { ch1: { starred: true, updatedAt: 1, rev: 0 } },
    });
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));
    assert.ok(publishedEvent !== null, "publish must have been called");
    assert.equal(publishedEvent.kind, 30078, "kind must be 30078");
    const dTag = publishedEvent.tags.find((t) => t[0] === "d")?.[1];
    assert.equal(dTag, "channel-stars", "d-tag must be 'channel-stars'");
    const plaintext = tauri.capturedPlaintext();
    const parsed = JSON.parse(plaintext);
    assert.ok("channels" in parsed, "payload must have 'channels' field");
    assert.ok(!("sections" in parsed), "payload must not have 'sections'");
    assert.ok(!("groups" in parsed), "payload must not have 'groups'");
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("stars wire: parser delegates to parseStarPayload (starred field, version guard)", () => {
  const valid = {
    version: 1,
    channels: { a: { starred: true, updatedAt: 1, rev: 0 } },
  };
  const parsed = parseStarPayload(valid);
  assert.ok(parsed !== null, "valid star payload parses");
  assert.equal(parsed.channels.a.starred, true, "starred field present");
  const mutePayload = {
    version: 1,
    channels: { a: { muted: true, updatedAt: 1, rev: 0 } },
  };
  const rejected = parseStarPayload(mutePayload);
  assert.deepEqual(
    rejected?.channels ?? {},
    {},
    "muted entry rejected by stars parser",
  );
});

test("stars wire: outbox/subsumption callbacks are wired to stars storage (not mutes)", async () => {
  // Drive the full manager publish cycle: publish sets the stars outbox;
  // a confirming fetch returns a subsuming head → discardPending clears stars outbox.
  // A copy/paste mutation wiring mutes outbox/subsumption to the stars config would:
  //   (a) write to the mutes outbox prefix (readChannelStarsOutbox returns null), OR
  //   (b) check isMutesStoreSubsumedBy instead of isStarsStoreSubsumedBy, failing to
  //       confirm subsumption on a starred head → outbox never clears.
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-outbox-stars");
  try {
    const m = new ChannelStarSyncManager("pk-outbox-stars", RELAY);
    const store = {
      version: 1,
      channels: { ch: { starred: true, updatedAt: 100, rev: 1 } },
    };
    m.publishStars(store);
    // Outbox must be written synchronously on publish — proves writeChannelStarsOutbox is wired.
    assert.ok(
      readChannelStarsOutbox("pk-outbox-stars", RELAY) !== null,
      "publishStars must write to the stars outbox (not mutes)",
    );
    // Drive through a full publish cycle: fire debounce, return a subsuming head on
    // both fetches (fetchOwnBlob + confirmRetainedHeadSubsumes). After discardPending,
    // the stars outbox must be cleared — proves clearChannelStarsOutbox + isStarsStoreSubsumedBy
    // are wired (a mutes-subsumption mutation would see muted=undefined → not subsumed → no clear).
    const subsumingHead = tauri.mintHead(store, 50, "evt-sub");
    subsumingHead.tags = [["d", "channel-stars"]];
    mock.method(relayClient, "fetchEvents", () =>
      Promise.resolve([subsumingHead]),
    );
    fw._fireTimer(); // fires debounce → doPublish → fetchOwnBlob → publish → confirmRetained
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(readChannelStarsOutbox("pk-outbox-stars", RELAY), null);
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("stars wire: typed API (publishStars, getPendingStarStore, fetchRemoteStars, cancelPendingStarPublish)", () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const m = new ChannelStarSyncManager("pk-api", RELAY);
    assert.equal(m.getPendingStarStore(), null, "no pending initially");
    m.publishStars({
      version: 1,
      channels: { c: { starred: true, updatedAt: 1, rev: 0 } },
    });
    assert.ok(m.getPendingStarStore() !== null, "publishStars sets pending");
    m.cancelPendingStarPublish();
    assert.ok(typeof m.cancelPendingStarPublish === "function");
    assert.ok(
      typeof m.fetchRemoteStars === "function",
      "fetchRemoteStars exists",
    );
    m.destroy();
  } finally {
    restore();
    mock.reset();
  }
});
