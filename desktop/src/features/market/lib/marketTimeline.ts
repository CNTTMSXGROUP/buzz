import * as React from "react";
import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";
import type { MarketProjection } from "@/features/market/lib/marketProtocol";
import { parseMarketEnvelope } from "@/features/market/lib/marketProtocol";
import type { TimelineMessage } from "@/features/messages/types";

/** Keep protocol state out of the market's main chat; negotiation stays in threads. */
export function selectMarketTimelineMessages(
  messages: TimelineMessage[],
  projection: MarketProjection | null,
): TimelineMessage[] {
  if (!projection) return messages;
  return messages.filter((message) => !parseMarketEnvelope(message.body));
}

export function useMarketTimelineMessages(messages: TimelineMessage[]) {
  const projection = useMarketChannel();
  return React.useMemo(
    () => selectMarketTimelineMessages(messages, projection),
    [messages, projection],
  );
}
