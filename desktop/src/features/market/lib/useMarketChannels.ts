import { useQueries, useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  marketAnnouncementMatchesProjection,
  projectMarketAnnouncements,
  projectMarketChannel,
  type MarketAnnouncement,
  type MarketProjection,
} from "@/features/market/lib/marketProtocol";
import { useChannelMessagesQuery } from "@/features/messages/hooks";
import { getChannelWindowEvents } from "@/shared/api/channelWindow";
import { getGlobalNotes } from "@/shared/api/social";
import type { Channel } from "@/shared/api/types";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

const MARKET_REFETCH_INTERVAL_MS = 5_000;

export function useMarketAnnouncements(): {
  announcements: MarketAnnouncement[];
  isLoading: boolean;
} {
  const refetchInterval = useFocusedRefetchInterval(MARKET_REFETCH_INTERVAL_MS);
  const query = useQuery({
    queryKey: ["market-announcements"],
    queryFn: () => getGlobalNotes({ limit: 500 }),
    refetchInterval,
    refetchOnWindowFocus: true,
  });
  const announcements = React.useMemo(
    () => projectMarketAnnouncements(query.data?.notes ?? []),
    [query.data?.notes],
  );
  return { announcements, isLoading: query.isLoading };
}

export function useVerifiedMarketAnnouncements(): {
  announcements: MarketAnnouncement[];
  isLoading: boolean;
} {
  const pulse = useMarketAnnouncements();
  const channelQueries = useQueries({
    queries: pulse.announcements.map((announcement) => ({
      queryKey: ["market-channel-contract", announcement.channelId],
      queryFn: () => getChannelWindowEvents(announcement.channelId, null, 100),
      staleTime: 30_000,
    })),
  });
  const announcements = React.useMemo(
    () =>
      pulse.announcements.filter((announcement, index) => {
        const events = channelQueries[index]?.data;
        if (!events) return false;
        const projection = projectMarketChannel(
          events.map((event) => ({
            id: event.id,
            pubkey: event.pubkey,
            createdAt: event.created_at,
            content: event.content,
            tags: event.tags,
          })),
          announcement.channelId,
        );
        return (
          projection !== null &&
          marketAnnouncementMatchesProjection(announcement, projection)
        );
      }),
    [channelQueries, pulse.announcements],
  );
  return {
    announcements,
    isLoading:
      pulse.isLoading || channelQueries.some((query) => query.isPending),
  };
}

export function useMarketAnnouncement(
  channelId: string | null,
): MarketAnnouncement | null {
  const { announcements } = useMarketAnnouncements();
  if (!channelId) return null;
  return (
    announcements.find(
      (announcement) =>
        announcement.channelId.toLowerCase() === channelId.toLowerCase(),
    ) ?? null
  );
}

export function useMarketChannelProjection(
  channel: Channel,
  announcement?: MarketAnnouncement | null,
): MarketProjection | null {
  const messagesQuery = useChannelMessagesQuery(channel);
  return React.useMemo(() => {
    const projection = projectMarketChannel(
      (messagesQuery.data ?? []).map((event) => ({
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.created_at,
        content: event.content,
        tags: event.tags,
      })),
      channel.id,
    );
    if (
      !projection ||
      (announcement &&
        !marketAnnouncementMatchesProjection(announcement, projection))
    ) {
      return null;
    }
    return projection;
  }, [announcement, channel.id, messagesQuery.data]);
}
