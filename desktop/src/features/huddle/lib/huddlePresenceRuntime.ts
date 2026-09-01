import {
  compareHuddleLifecycleEvents,
  fetchHuddleLifecycleHistory,
  huddleLifecycleGeneration,
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
  let activeSessionGenerations = new Map<string, string>();
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
      if (event.kind === KIND_HUDDLE_ENDED) {
        activeSessionGenerations.delete(sessionId);
      } else if (event.kind === KIND_HUDDLE_PARTICIPANT_JOINED) {
        // A START is published before audio admission and is not proof of a live
        // room. An authenticated relay JOIN may activate it immediately.
        activeSessionGenerations.set(
          sessionId,
          huddleLifecycleGeneration(event) ??
            activeSessionGenerations.get(sessionId) ??
            "pending",
        );
      }
    }
    // Fence an in-flight authoritative snapshot against every accepted live
    // lifecycle mutation. A stale response queried the old session set and
    // must not replace newer live state.
    livenessRequestVersion += 1;
    dependencies.onPresence(
      tracker.snapshot(new Set(activeSessionGenerations.keys())),
    );
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
    const generations = new Map<string, string>();
    for (const event of livenessPages.flat()) {
      if (event.kind !== KIND_HUDDLE_LIVENESS) continue;
      const sessionId = huddleSessionId(event);
      if (!sessionId) continue;
      try {
        const generation = (
          JSON.parse(event.content) as { generation?: unknown }
        ).generation;
        if (typeof generation === "string" && generation.length > 0) {
          generations.set(sessionId, generation);
        }
      } catch {
        // A malformed synthetic response is not authoritative liveness.
      }
    }
    return generations;
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
      const nextActiveSessionGenerations = await fetchActiveSessionIds([
        ...activeSessionGenerations.keys(),
      ]);
      if (disposed) return;
      if (requestVersion !== livenessRequestVersion) {
        const requestedGenerations = new Map(activeSessionGenerations);
        for (const [sessionId, generation] of nextActiveSessionGenerations) {
          if (!requestedGenerations.has(sessionId)) continue;
          requestedGenerations.set(sessionId, generation);
        }
        tracker.reconcileLiveness(
          requestedGenerations,
          activeSessionGenerations,
        );
        activeSessionGenerations = requestedGenerations;
        dependencies.onPresence(
          tracker.snapshot(new Set(activeSessionGenerations.keys())),
        );
        scheduleLivenessRefresh();
        return;
      }
      tracker.reconcileLiveness(
        nextActiveSessionGenerations,
        activeSessionGenerations,
      );
      activeSessionGenerations = nextActiveSessionGenerations;
      dependencies.onPresence(
        tracker.snapshot(new Set(activeSessionGenerations.keys())),
      );
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
          [...history, ...pendingLiveEvents]
            .map(huddleSessionId)
            .filter((sessionId): sessionId is string => Boolean(sessionId)),
        ),
      ];
      const nextActiveSessionGenerations =
        await fetchActiveSessionIds(sessionIds);
      if (disposed) return;
      const nextTracker = new HuddlePresenceTracker(
        dependencies.relaySelfPubkey,
      );
      if (pendingOverflowed) {
        pendingOverflowed = false;
        pendingLiveEvents = [];
        hydrated = false;
        dependencies.onPresence(new Set());
        reconcileAgain = true;
        return;
      }
      const bufferedEvents = pendingLiveEvents;
      pendingLiveEvents = [];
      const bufferedEventIds = new Set(bufferedEvents.map((event) => event.id));
      const combinedEvents = [
        ...new Map(
          [...history, ...bufferedEvents].map((event) => [event.id, event]),
        ).values(),
      ].sort(compareHuddleLifecycleEvents);
      // Globally order persisted history with the live overlap before applying
      // either source. Accepted buffered events may activate or end sessions;
      // rejected events cannot mutate the liveness gate.
      for (const event of combinedEvents) {
        if (!nextTracker.apply(event)) continue;
        if (!bufferedEventIds.has(event.id)) continue;
        const sessionId = huddleSessionId(event);
        if (!sessionId) continue;
        if (event.kind === KIND_HUDDLE_ENDED) {
          nextActiveSessionGenerations.delete(sessionId);
        } else if (event.kind === KIND_HUDDLE_PARTICIPANT_JOINED) {
          nextActiveSessionGenerations.set(
            sessionId,
            huddleLifecycleGeneration(event) ??
              nextActiveSessionGenerations.get(sessionId) ??
              "pending",
          );
        }
      }
      pendingLiveEvents = [];
      nextTracker.reconcileLiveness(nextActiveSessionGenerations);
      tracker = nextTracker;
      activeSessionGenerations = nextActiveSessionGenerations;
      hydrated = true;
      retryDelayMs = INITIAL_RETRY_DELAY_MS;
      clearScheduledRetry();
      dependencies.onPresence(
        tracker.snapshot(new Set(activeSessionGenerations.keys())),
      );
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
    activeSessionGenerations.clear();
  };
}
