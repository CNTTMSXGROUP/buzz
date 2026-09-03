// Carl-round regression suite for WholeBlobSyncManager (P1, P2a-1, P2b).
//
// Three causal regressions confirmed at source in Duncan's analysis pass:
//
// T-P1  (hook layer): failed bootstrap → restored outbox (old queuedAt) →
//   bootstrap `.then()` replay → pre-publish fetch finds newer peer head B
//   retained while device was closed → must ADOPT (never publish stale edit
//   over B).
//   Mutation: remove `!this.pendingIsRestoredReplay` guard from the
//   failed-bootstrap exception → exception fires, folds B in, publishes over it.
//
// T-P2a1 (hook layer): blocked bootstrap H100 → click → live H102 arrives and
//   is suppressed (hasPendingEdit) → bootstrap resolves H100 → hook `.then()`
//   calls publishSections/publishSortPrefs(outbox, isRestoredReplay=true) →
//   replay must NOT re-freeze publishBaseline from mutable lastRemoteHead →
//   pre-publish fetch returns H102 → ADOPT (H102 is a genuine advance).
//   Mutation: remove `if (!isRestoredReplay)` guard from publish() →
//   publishBaseline refreezes to H102 → pre-publish sees equality → publishes
//   pre-H102 content over H102, H102's changes lost.
//
// T-P2b (manager layer): periodic/reconnect fetchRemoteBlob path records
//   lastRemoteHead BEFORE decryptAndParse returns; a concurrent click freezes
//   publishBaseline against the pre-decrypt head; the pre-publish fetch then
//   sees no advance and publishes stale content over the fetched event.
//   Mutation: move recordRemoteHead back above decryptAndParse (revert P2b fix)
//   → publishBaseline freezes against undecrypted head → pre-publish fetch
//   returns same head → no advance → publish-over.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  makeHookTimerBed,
  installEchoTauri,
} from "./sidebarSyncTestHelpers.mjs";

const RELAY = "wss://r.carl";

/**
 * Register the Carl-round P1/P2a-1 hook-layer regressions for a single lane.
 *
 * @param {object} opts
 * @param {string} opts.label         — "sections"|"sort"
 * @param {Function} opts.Manager     — concrete manager class
 * @param {Function} opts.publishEdit — (manager, store) => void (fresh click, isRestoredReplay=false)
 * @param {Function} opts.publishReplay — (manager, store) => void (restored replay, isRestoredReplay=true)
 * @param {Function} opts.subscribe   — (manager, cb) => Promise<unsubscribe>
 * @param {Function} opts.makeNonEmptyStore  — () => non-empty mount store
 * @param {Function} opts.makeEditStore      — () => user-click store (distinct from mount)
 * @param {Function} opts.makeRemoteStore    — () => peer relay head store
 */
export function runWholeBlobCarlSuite({
  label,
  Manager,
  publishEdit,
  publishReplay,
  subscribe,
  makeNonEmptyStore,
  makeEditStore,
  makeRemoteStore,
}) {
  // T-P1: failed bootstrap → hook replay of restored outbox → peer head B was
  //   retained while device was closed → replay must ADOPT, never publish over B.
  //
  // Production sequence:
  //   1. A edits at t=100, queuedAt=100, quits before publish.
  //   2. Peer retains head B at t=200 while A is offline.
  //   3. A reopens: bootstrap fetch fails → bootstrapFailed=true.
  //   4. Hook `.then()` reads outbox (queuedAt=100 < B.createdAt=200 → shouldReplay
  //      guard in the hook would be false for apply-remote; but bootstrap=hold here
  //      so shouldReplay is always true). The replay calls publish(store,
  //      isRestoredReplay=true) → pendingIsRestoredReplay=true.
  //   5. Debounce fires. fetchOwnBlobBeforePublish returns B.
  //      remoteAdvancedSince(B, {0,""}) = true → would normally adopt.
  //      Failed-bootstrap exception: bootstrapFailed=true,
  //      !bootstrapFailedExternalHeadObserved=true, publishBaseline={0,""}.
  //      With the fix: pendingIsRestoredReplay=true → exception suppressed
  //      → normal adopt fires.
  //      Mutation: exception fires → fold B into baseline → publish over B.
  //
  // The test uses a `hold` bootstrap (failed fetch) so the hook always replays.
  // `shouldReplay` is true for `hold`. The outbox queuedAt (100) is older than
  // B (200) — without the fix, the exception would override and publish over B.
  test(`P1 ${label}: failed-bootstrap hook replay must adopt a newer peer head retained while device was closed, never publish over it`, async () => {
    let publishCalls = 0;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p1-${label}`);

    const peerHead = tauri.mintHead(
      makeRemoteStore(),
      200,
      `evt-p1-peer-${label}`,
    );

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // Call 1 = bootstrap fetch: fail → bootstrapFailed=true.
      if (fetchCalls === 1) return Promise.reject(new Error("bootstrap fail"));
      // Subsequent calls (pre-publish fetch, retry): return peerHead.
      return Promise.resolve([peerHead]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p1-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Bootstrap fails synchronously.
      await manager.bootstrap(makeNonEmptyStore());

      // Hook .then() replays the outbox: publishEdit with isRestoredReplay=true.
      // The manager sees bootstrapResolved=true and schedules the debounce.
      publishReplay(manager, makeEditStore());

      // Debounce fires.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // The pre-publish fetch returns peerHead (createdAt=200) which is newer
      // than publishBaseline={0,""}. With the fix: pendingIsRestoredReplay=true
      // → failed-bootstrap exception is suppressed → normal adopt fires.
      // Mutation: exception fires → fold peerHead into baseline → publish over it.
      assert.equal(
        publishCalls,
        0,
        `P1 ${label}: restored replay must ADOPT newer peer head, never publish over it`,
      );
      assert.equal(
        adopted.length,
        1,
        `P1 ${label}: peer head must be adopted after restored replay`,
      );
      assert.equal(
        manager.getPendingStore(),
        null,
        `P1 ${label}: pending cleared after adopt`,
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // T-P2a1: blocked bootstrap H100 → click → live H102 decrypts (suppressed by
  //   hasPendingEdit) → bootstrap resolves H100 → hook .then() replay
  //   (isRestoredReplay=true) → H102 must be adopted as a genuine advance.
  //
  // This tests that the hook-level replay path (which calls publish with
  // isRestoredReplay=true) does NOT re-freeze publishBaseline from lastRemoteHead
  // (which now holds H102 after the live delivery). releaseDeferred already set
  // publishBaseline = canonicalMax(click-time, H100) = H100 (click-time baseline
  // was {0,""}). The replay's publish(store, true) must keep that baseline.
  // Pre-publish fetch returns H102 (createdAt=400 > H100.createdAt=200) →
  // remoteAdvancedSince(H102, H100) = true → ADOPT.
  //
  // Mutation: publish() re-freezes publishBaseline = lastRemoteHead = H102 →
  // pre-publish sees equality → publishes pre-H102 content over H102 (lose H102).
  test(`P2a-1 ${label}: hook replay after blocked bootstrap must not re-freeze baseline from live peer head H102; H102 must be adopted`, async () => {
    let publishCalls = 0;
    let liveCallback = null;
    let releaseBootstrap = null;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a1-${label}`);

    const H100 = tauri.mintHead(makeRemoteStore(), 200, `evt-h100-${label}`);
    const H102 = tauri.mintHead(makeRemoteStore(), 400, `evt-h102-${label}`);

    mock.method(relayClient, "subscribeLive", (_f, cb) => {
      liveCallback = cb;
      return Promise.resolve(async () => {});
    });

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // Call 1 = bootstrap fetch (blocked).
      if (fetchCalls === 1)
        return new Promise((res) => {
          releaseBootstrap = () => res([H100]);
        });
      // Subsequent (pre-publish fetch): return H102.
      return Promise.resolve([H102]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a1-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Subscribe to capture live callback.
      await subscribe(manager, () => {});

      // Bootstrap blocked; fresh click BEFORE bootstrap resolves.
      // (No existing outbox — this simulates a click during the bootstrap window,
      // with the hook seeing an existing pending edit before .then() runs.)
      const bootstrapPromise = manager.bootstrap(makeNonEmptyStore());
      // Fresh click: publish(store, isRestoredReplay=false) — normal path, freezes
      // publishBaseline = {0,""} (lastRemoteHead before any live event).
      publishEdit(manager, makeEditStore());

      // Live H102 arrives BEFORE bootstrap resolves (suppressed by hasPendingEdit).
      while (liveCallback === null) await Promise.resolve();
      liveCallback(H102);
      for (let i = 0; i < 20; i++) await Promise.resolve();
      // lastRemoteHead is now H102.

      // Bootstrap resolves with H100. releaseDeferred sets publishBaseline =
      // canonicalMax({0,""}, H100) = H100.
      while (releaseBootstrap === null) await Promise.resolve();
      releaseBootstrap();
      await bootstrapPromise;
      for (let i = 0; i < 50; i++) await Promise.resolve();
      // bootstrapResolved=true. The timer was scheduled by releaseDeferred.

      // Hook .then() would replay the outbox edit. Simulate: call publish with
      // isRestoredReplay=true as the hook's .then() callback does.
      // With the fix: baseline is unchanged (still H100).
      // With the mutation: publishBaseline is reset to lastRemoteHead=H102.
      publishReplay(manager, makeEditStore());

      // Fire the debounce.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // Pre-publish returns H102 (createdAt=400 > H100.createdAt=200).
      // Fix: publishBaseline=H100 → remoteAdvancedSince(H102, H100) = true → ADOPT.
      // Mutation: publishBaseline=H102 → remoteAdvancedSince(H102, H102) = false
      //   → publish-over fires, H102's changes lost.
      assert.equal(
        publishCalls,
        0,
        `P2a-1 ${label}: must ADOPT H102 (genuine advance), not publish over it`,
      );
      assert.equal(
        adopted.length,
        1,
        `P2a-1 ${label}: H102 must be adopted as genuine remote advance`,
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });
}

/**
 * Register the Carl-round P2b manager-level regression (fetchRemoteBlob
 * decrypt gap — periodic/reconnect path).
 *
 * @param {object} opts
 * @param {string} opts.label     — "sections"|"sort"
 * @param {Function} opts.Manager — concrete manager class
 * @param {Function} opts.publishEdit — (manager, store) => void
 * @param {Function} opts.makeEditStore  — () => store
 * @param {Function} opts.makeRemoteStore — () => store
 */
export function runWholeBlobP2bSuite({
  label,
  Manager,
  publishEdit,
  makeEditStore,
  makeRemoteStore,
}) {
  // T-P2b: periodic/reconnect fetchRemoteBlob records lastRemoteHead BEFORE
  //   decryptAndParse; a concurrent click (whose publish() fires during the
  //   async decrypt gap) freezes publishBaseline against the pre-decrypt head;
  //   the pre-publish fetch returns the same head; no advance → publish-over.
  //
  // Sequence:
  //   1. fetchRemoteBlob is in-flight, has fetched the event but has not yet
  //      decrypted it. Before the fix, recordRemoteHead(event.id) fires here →
  //      lastRemoteHead advances to H.
  //   2. User click: publish() freezes publishBaseline = lastRemoteHead = H.
  //   3. decryptAndParse resolves → store updated to H's content.
  //   4. Debounce fires. fetchOwnBlobBeforePublish returns H.
  //      remoteAdvancedSince(H, H) = false → publishes. BUT: our store is the
  //      click's intent, NOT H's content + click merged in → H's remote-only
  //      changes are lost.
  //   With the fix: recordRemoteHead deferred to after decrypt → at step 2
  //   lastRemoteHead is still {createdAt: 0} → publishBaseline={0,""} → at
  //   step 4 remoteAdvancedSince(H, {0,""}) = true → ADOPT (or use exception
  //   path for a failed-bootstrap scenario). Either way, the clicked edit
  //   doesn't silently overwrite H's remote-only changes.
  //
  // This test exercises the race by controlling the decrypt resolution.
  test(`P2b ${label}: fetchRemoteBlob must not advance lastRemoteHead before decryptAndParse succeeds; click during decrypt gap must not publish over fetched head`, async () => {
    let publishCalls = 0;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2b-${label}`);

    // H: the pre-existing relay head with some remote-only content.
    const H = tauri.mintHead(makeRemoteStore(), 200, `evt-p2b-h-${label}`);

    // A controlled decrypt gate: we resolve the decrypt promise AFTER the click.
    let releaseDecrypt = null;
    const origInvoke = globalThis.window?.__TAURI_INTERNALS__?.invoke;
    const orig = globalThis.window.__TAURI_INTERNALS__;
    // Wrap decrypt: the first call (for H in fetchRemoteBlob) is gated.
    let decryptCallCount = 0;
    globalThis.window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => {
        if (cmd === "nip44_decrypt_from_self") {
          decryptCallCount++;
          if (decryptCallCount === 1) {
            // First decrypt (fetchRemoteBlob's call for H): hold it.
            return new Promise((res, rej) => {
              releaseDecrypt = () =>
                orig.invoke(cmd, args).then(res).catch(rej);
            });
          }
          return orig.invoke(cmd, args);
        }
        return orig.invoke(cmd, args);
      },
    };

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // Bootstrap fetch: absent (no remote head at mount).
      if (fetchCalls === 1) return Promise.resolve([]);
      // fetchRemoteBlob (periodic/reconnect) call: return H.
      if (fetchCalls === 2) return Promise.resolve([H]);
      // Pre-publish fetch: return H (relay still has H as head).
      return Promise.resolve([H]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2b-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Bootstrap: absent → bootstrapFailed=false, publishBaseline={0,""}.
      await manager.bootstrap(makeEditStore());

      // Trigger a fetchRemoteBlob (simulates the periodic/reconnect path).
      // This call will block at the decrypt await.
      const fetchPromise = manager.fetchRemoteBlob();
      // Wait for the fetch to have collected the event but not yet decrypted.
      // The fetch awaits decryptAndParse at this point.
      for (let i = 0; i < 10; i++) await Promise.resolve();

      // Click during the decrypt gap.
      // With the fix: lastRemoteHead still {createdAt:0} (decrypt not done yet)
      //   → publishBaseline = {0,""}.
      // With the mutation: lastRemoteHead = {200, H.id} (pre-decrypt record)
      //   → publishBaseline = {200, H.id}.
      publishEdit(manager, makeEditStore());

      // Release the decrypt.
      assert.ok(releaseDecrypt !== null, "decrypt gate must have been hit");
      releaseDecrypt();
      await fetchPromise;
      for (let i = 0; i < 20; i++) await Promise.resolve();

      // Debounce fires. Pre-publish fetch returns H (createdAt=200, same id).
      // With the fix: publishBaseline={0,""} → remoteAdvancedSince(H,{0,""})=true
      //   → bootstrapFailed=false so no exception → normal ADOPT.
      // With the mutation: publishBaseline=H → remoteAdvancedSince(H,H)=false
      //   → publish fires, H's remote-only content silently lost.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(
        publishCalls,
        0,
        `P2b ${label}: click during fetchRemoteBlob decrypt gap must not publish over the fetched head`,
      );
      assert.equal(
        adopted.length >= 1,
        true,
        `P2b ${label}: the fetched head must be adopted (genuine advance)`,
      );
      manager.destroy();
    } finally {
      globalThis.window.__TAURI_INTERNALS__ = orig;
      restore();
      mock.reset();
    }
  });
}
