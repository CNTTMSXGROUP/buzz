import {
  applyHuddleLifecycleHistory,
  compareHuddleLifecycleEvents,
  fetchHuddleLifecycleHistory,
  huddleSessionId,
  HuddlePresenceTracker,
  HUDDLE_LIFECYCLE_PAGE_LIMIT,
} from "@/features/huddle/lib/huddlePresence";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import { MAX_EXPLICIT_CHANNEL_VALUES } from "@/shared/api/relayClientShared";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_LIVENESS,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";

const LIFECYCLE_KINDS = [
  KIND_HUDDLE_STARTED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_ENDED,
] as const;
const MAX_PENDING_LIVE_EVENTS = 1_000;
const INITIAL_RETRY_DELAY_MS = 1_000;
const MAX_RETRY_DELAY_MS = 30_000;
// Owner leases renew every 10 seconds against a 30-second TTL. Refreshing on
// the same cadence bounds a stale badge to one lease lifetime plus one poll
// when an owner disappears without publishing a lifecycle end event.
const LIVENESS_REFRESH_INTERVAL_MS = 10_000;

type Dispose = () => void | Promise<void>;

export type HuddlePresenceRuntimeDependencies = {
  relaySelfPubkey: string;
  channelIds: readonly string[];
  subscribeLive: (
    filter: RelaySubscriptionFilter,
    onEvent: (event: RelayEvent) => void,
  ) => Promise<Dispose>;
  fetchEvents: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
  subscribeToReconnects: (listener: () => void) => () => void;
  onPresence: (participants: ReadonlySet<string>) => void;
  onError?: (message: string, error: unknown) => void;
  setRetryTimer?: (callback: () => void, delayMs: number) => unknown;
  clearRetryTimer?: (handle: unknown) => void;
  setLivenessTimer?: (callback: () => void, delayMs: number) => unknown;
  clearLivenessTimer?: (handle: unknown) => void;
  nowSeconds?: () => number;
};

/**
 * Keeps community-wide huddle presence convergent across hydration failures,
 * disconnect gaps, and live/history overlap. The runtime fails closed while a
 * complete history rebuild is unavailable and retries without remounting.
 */
export function startHuddlePresenceRuntime(
  dependencies: HuddlePresenceRuntimeDependencies,
): () => void {
  const setRetryTimer =
    dependencies.setRetryTimer ??
    ((callback: () => void, delayMs: number) =>
      window.setTimeout(callback, delayMs));
  const clearRetryTimer =
    dependencies.clearRetryTimer ??
    ((handle: unknown) => window.clearTimeout(handle as number));
  const setLivenessTimer = dependencies.setLivenessTimer ?? setRetryTimer;
  const clearLivenessTimer = dependencies.clearLivenessTimer ?? clearRetryTimer;
  const nowSeconds =
    dependencies.nowSeconds ?? (() => Math.floor(Date.now() / 1_000));

  let disposed = false;
  let liveDispose: Dispose | null = null;
  let connecting = false;
  let reconciling = false;
  let reconcileAgain = false;
  let hydrated = false;
  let tracker = new HuddlePresenceTracker(dependencies.relaySelfPubkey);
  let activeSessionIds = new Set<string>();
  let pendingLiveEvents: RelayEvent[] = [];
  let pendingOverflowed = false;
  let retryHandle: unknown = null;
  let livenessHandle: unknown = null;
  let retryDelayMs = INITIAL_RETRY_DELAY_MS;
  let livenessRequestVersion = 0;

  const channelChunks: string[][] = [];
  const normalizedChannelIds = [...new Set(dependencies.channelIds)].sort();
  for (
    let index = 0;
    index < normalizedChannelIds.length;
    index += MAX_EXPLICIT_CHANNEL_VALUES
  ) {
    channelChunks.push(
      normalizedChannelIds.slice(index, index + MAX_EXPLICIT_CHANNEL_VALUES),
    );
  }

  if (channelChunks.length === 0) {
    dependencies.onPresence(new Set());
    return () => {};
  }

  const clearScheduledRetry = () => {
    if (retryHandle === null) return;
    clearRetryTimer(retryHandle);
    retryHandle = null;
  };

  const clearScheduledLivenessRefresh = () => {
    if (livenessHandle === null) return;
    clearLivenessTimer(livenessHandle);
    livenessHandle = null;
  };

  const scheduleRecovery = (recover: () => void) => {
    if (disposed || retryHandle !== null) return;
    const delay = retryDelayMs;
    retryDelayMs = Math.min(retryDelayMs * 2, MAX_RETRY_DELAY_MS);
    retryHandle = setRetryTimer(() => {
      retryHandle = null;
      recover();
    }, delay);
  };

  const applyLiveEvent = (event: RelayEvent) => {
    if (disposed) return;
    if (!hydrated || reconciling) {
      if (pendingLiveEvents.length >= MAX_PENDING_LIVE_EVENTS) {
        hydrated = false;
        pendingOverflowed = true;
        pendingLiveEvents = [];
        dependencies.onPresence(new Set());
        return;
      }
      pendingLiveEvents.push(event);
      return;
    }
    const changed = tracker.apply(event);
    if (!changed) return;

    const sessionId = huddleSessionId(event);
    if (sessionId) {
      if (event.kind === KIND_HUDDLE_ENDED) activeSessionIds.delete(sessionId);
      else activeSessionIds.add(sessionId);
    }
    // Fence an in-flight authoritative snapshot against every accepted live
    // lifecycle mutation. A stale response queried the old session set and
    // must not replace newer live state.
    livenessRequestVersion += 1;
    dependencies.onPresence(tracker.snapshot(activeSessionIds));
  };

  const fetchActiveSessionIds = async (sessionIds: readonly string[]) => {
    const sessionChunks: string[][] = [];
    for (
      let index = 0;
      index < sessionIds.length;
      index += MAX_EXPLICIT_CHANNEL_VALUES
    ) {
      sessionChunks.push(
        sessionIds.slice(index, index + MAX_EXPLICIT_CHANNEL_VALUES),
      );
    }
    const livenessPages = await Promise.all(
      channelChunks.flatMap((channelIds) =>
        sessionChunks.map((sessions) =>
          dependencies.fetchEvents({
            kinds: [KIND_HUDDLE_LIVENESS],
            "#h": channelIds,
            "#d": sessions,
            limit: sessions.length,
          }),
        ),
      ),
    );
    return new Set(
      livenessPages
        .flat()
        .filter((event) => event.kind === KIND_HUDDLE_LIVENESS)
        .map(huddleSessionId)
        .filter((sessionId): sessionId is string => Boolean(sessionId)),
    );
  };

  const scheduleLivenessRefresh = () => {
    if (disposed || !hydrated || livenessHandle !== null) return;
    livenessHandle = setLivenessTimer(() => {
      livenessHandle = null;
      void refreshLiveness();
    }, LIVENESS_REFRESH_INTERVAL_MS);
  };

  async function refreshLiveness() {
    if (disposed || !hydrated || reconciling) {
      scheduleLivenessRefresh();
      return;
    }
    const requestVersion = livenessRequestVersion;
    try {
      const nextActiveSessionIds = await fetchActiveSessionIds([
        ...activeSessionIds,
      ]);
      if (disposed) return;
      if (requestVersion !== livenessRequestVersion) {
        scheduleLivenessRefresh();
        return;
      }
      activeSessionIds = nextActiveSessionIds;
      dependencies.onPresence(tracker.snapshot(activeSessionIds));
      scheduleLivenessRefresh();
    } catch (error) {
      if (disposed) return;
      if (requestVersion !== livenessRequestVersion) {
        scheduleLivenessRefresh();
        return;
      }
      hydrated = false;
      dependencies.onPresence(new Set());
      dependencies.onError?.("Huddle liveness refresh failed", error);
      scheduleRecovery(recover);
    }
  }

  const reconcile = async () => {
    if (disposed) return;
    if (reconciling) {
      reconcileAgain = true;
      return;
    }
    reconciling = true;
    livenessRequestVersion += 1;
    try {
      const historyPages = await Promise.all(
        channelChunks.map((channelIds) =>
          fetchHuddleLifecycleHistory(dependencies.fetchEvents, channelIds),
        ),
      );
      const history = [
        ...new Map(
          historyPages.flat().map((event) => [event.id, event]),
        ).values(),
      ];
      const sessionIds = [
        ...new Set(
          history
            .map(huddleSessionId)
            .filter((sessionId): sessionId is string => Boolean(sessionId)),
        ),
      ];
      const nextActiveSessionIds = await fetchActiveSessionIds(sessionIds);
      if (disposed) return;
      const nextTracker = new HuddlePresenceTracker(
        dependencies.relaySelfPubkey,
      );
      applyHuddleLifecycleHistory(nextTracker, history);
      if (pendingOverflowed) {
        pendingOverflowed = false;
        pendingLiveEvents = [];
        hydrated = false;
        dependencies.onPresence(new Set());
        reconcileAgain = true;
        return;
      }
      // The liveness snapshot is authoritative for which persisted sessions are
      // current. Replaying buffered events against a fresh tracker still gates
      // each active-set mutation on the tracker's signer/session validation.
      for (const event of [...pendingLiveEvents].sort(
        compareHuddleLifecycleEvents,
      )) {
        if (!nextTracker.apply(event)) continue;
        const sessionId = huddleSessionId(event);
        if (!sessionId) continue;
        if (event.kind === KIND_HUDDLE_ENDED) {
          nextActiveSessionIds.delete(sessionId);
        } else {
          nextActiveSessionIds.add(sessionId);
        }
      }
      pendingLiveEvents = [];
      tracker = nextTracker;
      activeSessionIds = nextActiveSessionIds;
      hydrated = true;
      retryDelayMs = INITIAL_RETRY_DELAY_MS;
      clearScheduledRetry();
      dependencies.onPresence(tracker.snapshot(activeSessionIds));
      clearScheduledLivenessRefresh();
      scheduleLivenessRefresh();
    } catch (error) {
      if (disposed) return;
      hydrated = false;
      pendingLiveEvents = [];
      dependencies.onPresence(new Set());
      dependencies.onError?.("Huddle presence hydration failed", error);
      scheduleRecovery(recover);
    } finally {
      reconciling = false;
      if (reconcileAgain && !disposed) {
        reconcileAgain = false;
        void reconcile();
      }
    }
  };

  const ensureSubscribed = async () => {
    if (disposed || liveDispose || connecting) return;
    connecting = true;
    try {
      const unsubscribes: Dispose[] = [];
      try {
        for (const channelIds of channelChunks) {
          unsubscribes.push(
            await dependencies.subscribeLive(
              {
                kinds: [...LIFECYCLE_KINDS],
                "#h": channelIds,
                since: nowSeconds(),
                limit: HUDDLE_LIFECYCLE_PAGE_LIMIT,
              },
              applyLiveEvent,
            ),
          );
        }
      } catch (error) {
        await Promise.all(unsubscribes.map((unsubscribe) => unsubscribe()));
        throw error;
      }
      const unsubscribe = () =>
        Promise.all(unsubscribes.map((dispose) => dispose())).then(() => {});
      if (disposed) {
        void unsubscribe();
        return;
      }
      liveDispose = unsubscribe;
      retryDelayMs = INITIAL_RETRY_DELAY_MS;
      await reconcile();
    } catch (error) {
      if (disposed) return;
      dependencies.onPresence(new Set());
      dependencies.onError?.("Huddle presence subscription failed", error);
      scheduleRecovery(recover);
    } finally {
      connecting = false;
    }
  };

  function recover() {
    if (disposed) return;
    if (liveDispose) {
      void reconcile();
    } else {
      void ensureSubscribed();
    }
  }

  const unsubscribeReconnect = dependencies.subscribeToReconnects(recover);
  void ensureSubscribed();

  return () => {
    disposed = true;
    clearScheduledRetry();
    clearScheduledLivenessRefresh();
    unsubscribeReconnect();
    if (liveDispose) void liveDispose();
    liveDispose = null;
    pendingLiveEvents = [];
    pendingOverflowed = false;
    activeSessionIds.clear();
  };
}
