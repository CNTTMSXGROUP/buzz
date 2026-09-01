import assert from "node:assert/strict";
import test from "node:test";

import { startHuddlePresenceRuntime } from "./huddlePresenceRuntime.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CAROL = "d".repeat(64);
const RELAY = "c".repeat(64);

function event({
  id,
  kind,
  pubkey = ALICE,
  tags = [],
  admissionId,
  rosterRevision,
  createdAt = Number(id),
  session = "room",
}) {
  return {
    id,
    kind,
    pubkey,
    content: JSON.stringify({
      ephemeral_channel_id: session,
      admission_id: admissionId,
      roster_revision: rosterRevision,
    }),
    tags,
    created_at: createdAt,
    sig: "",
  };
}

function participantEvent(options) {
  return event({ pubkey: RELAY, tags: [["p", BOB]], ...options });
}

function livenessEvent(session = "room") {
  return event({
    id: `live-${session}`,
    kind: 48104,
    createdAt: 1_000,
    session,
  });
}

async function settle() {
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
}

function runtimeHarness(initialHistory) {
  let history = initialHistory;
  let liveSessions = ["room"];
  let reconnect;
  let liveHandler;
  let livenessTimer;
  let livenessDelay;
  let liveDisposed = false;
  let reconnectDisposed = false;
  const snapshots = [];
  const filters = [];
  const runtime = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general", "design"],
    subscribeLive: async (filter, handler) => {
      filters.push(filter);
      liveHandler = handler;
      return () => {
        liveDisposed = true;
      };
    },
    fetchEvents: async (filter) =>
      filter.kinds?.includes(48104) ? liveSessions.map(livenessEvent) : history,
    subscribeToReconnects: (listener) => {
      reconnect = listener;
      return () => {
        reconnectDisposed = true;
      };
    },
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback, delayMs) => {
      livenessTimer = callback;
      livenessDelay = delayMs;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
    nowSeconds: () => 1_000,
  });

  return {
    dispose: runtime,
    emit: (next) => liveHandler(next),
    filters,
    reconnect: () => reconnect(),
    refreshLiveness: () => livenessTimer(),
    setHistory: (next) => {
      history = next;
    },
    setLiveSessions: (next) => {
      liveSessions = next;
    },
    snapshots,
    livenessDelay: () => livenessDelay,
    wasDisposed: () => liveDisposed && reconnectDisposed,
  };
}

test("hydrates lifecycle history in global phase and revision order", async () => {
  const harness = runtimeHarness([
    participantEvent({
      id: "join",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
      createdAt: 10,
    }),
    event({ id: "start", kind: 48100, createdAt: 10 }),
  ]);

  await settle();

  assert.equal(harness.snapshots.at(-1).has(ALICE), true);
  assert.equal(harness.snapshots.at(-1).has(BOB), true);
  assert.equal(harness.filters[0].limit > 0, true);
  assert.equal(harness.filters[0].since, 1_000);
  assert.deepEqual(harness.filters[0]["#h"], ["design", "general"]);
  harness.dispose();
});

test("reconciles joins, leaves, and ends missed during disconnect", async () => {
  const start = event({ id: "1", kind: 48100 });
  const join = participantEvent({
    id: "2",
    kind: 48101,
    admissionId: "desktop",
    rosterRevision: 1,
  });
  const left = participantEvent({
    id: "3",
    kind: 48102,
    admissionId: "desktop",
    rosterRevision: 2,
  });
  const ended = event({ id: "4", kind: 48103, pubkey: RELAY });
  const harness = runtimeHarness([start]);
  await settle();

  harness.setHistory([join, start]);
  harness.reconnect();
  await settle();
  assert.equal(harness.snapshots.at(-1).has(BOB), true);

  harness.setHistory([left, join, start]);
  harness.reconnect();
  await settle();
  assert.equal(harness.snapshots.at(-1).has(BOB), false);
  assert.equal(harness.snapshots.at(-1).has(ALICE), true);

  harness.setHistory([ended, left, join, start]);
  harness.reconnect();
  await settle();
  assert.deepEqual([...harness.snapshots.at(-1)], []);
  harness.dispose();
});

test("applies channel-scoped live joins, leaves, and ends without reconnecting", async () => {
  const start = event({
    id: "1",
    kind: 48100,
    tags: [["h", "general"]],
  });
  const harness = runtimeHarness([start]);
  await settle();

  harness.emit(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
      tags: [
        ["h", "general"],
        ["p", BOB],
      ],
    }),
  );
  assert.equal(harness.snapshots.at(-1).has(BOB), true);

  harness.emit(
    participantEvent({
      id: "3",
      kind: 48102,
      admissionId: "desktop",
      rosterRevision: 2,
      tags: [
        ["h", "general"],
        ["p", BOB],
      ],
    }),
  );
  assert.equal(harness.snapshots.at(-1).has(BOB), false);
  assert.equal(harness.snapshots.at(-1).has(ALICE), true);

  harness.emit(
    event({
      id: "4",
      kind: 48103,
      pubkey: RELAY,
      tags: [["h", "general"]],
    }),
  );
  assert.deepEqual([...harness.snapshots.at(-1)], []);
  harness.dispose();
});

test("clears stale presence on the lease-cadence liveness refresh", async () => {
  const harness = runtimeHarness([
    event({ id: "1", kind: 48100 }),
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
    }),
  ]);
  await settle();

  assert.equal(harness.snapshots.at(-1).has(ALICE), true);
  assert.equal(harness.snapshots.at(-1).has(BOB), true);
  assert.equal(harness.livenessDelay(), 10_000);

  harness.setLiveSessions([]);
  harness.refreshLiveness();
  await settle();

  assert.deepEqual([...harness.snapshots.at(-1)], []);
  harness.dispose();
});

test("keeps a live session added while an older liveness refresh is in flight", async () => {
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const queriedSessions = [];
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [event({ id: "1", kind: 48100 })];
      }
      livenessRequests += 1;
      queriedSessions.push([...filter["#d"]]);
      if (livenessRequests === 1) return [livenessEvent()];
      if (livenessRequests === 2) {
        return new Promise((resolve) => {
          resolveRefresh = resolve;
        });
      }
      return filter["#d"].map(livenessEvent);
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();

  livenessTimer();
  await settle();
  liveHandler(
    event({
      id: "2",
      kind: 48100,
      pubkey: CAROL,
      session: "new-room",
    }),
  );
  assert.equal(snapshots.at(-1).has(CAROL), true);

  resolveRefresh([livenessEvent()]);
  await settle();
  assert.equal(snapshots.at(-1).has(CAROL), true);
  assert.equal(typeof livenessTimer, "function");

  livenessTimer();
  await settle();
  assert.deepEqual(queriedSessions.at(-1), ["room", "new-room"]);
  assert.equal(snapshots.at(-1).has(CAROL), true);
  dispose();
});

test("retries a failed hydration and tears down every recovery path", async () => {
  let attempts = 0;
  let retry;
  let reconnect;
  let liveDisposed = false;
  let reconnectDisposed = false;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async () => () => {
      liveDisposed = true;
    },
    fetchEvents: async (filter) => {
      if (filter.kinds?.includes(48104)) return [livenessEvent()];
      attempts += 1;
      if (attempts === 1) throw new Error("temporary timeout");
      return [event({ id: "1", kind: 48100 })];
    },
    subscribeToReconnects: (listener) => {
      reconnect = listener;
      return () => {
        reconnectDisposed = true;
      };
    },
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setRetryTimer: (callback) => {
      retry = callback;
      return callback;
    },
    clearRetryTimer: () => {
      retry = undefined;
    },
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.deepEqual([...snapshots.at(-1)], []);
  assert.equal(typeof retry, "function");

  retry();
  await settle();
  assert.equal(snapshots.at(-1).has(ALICE), true);

  dispose();
  assert.equal(liveDisposed, true);
  assert.equal(reconnectDisposed, true);
  reconnect();
  await settle();
  assert.equal(attempts, 2);
});

test("fails closed when persisted lifecycle has no authoritative live room", async () => {
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) =>
      filter.kinds?.includes(48104)
        ? []
        : [
            event({ id: "1", kind: 48100 }),
            participantEvent({
              id: "2",
              kind: 48101,
              admissionId: "before-restart",
              rosterRevision: 1,
            }),
          ],
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.deepEqual([...snapshots.at(-1)], []);
  dispose();
});
