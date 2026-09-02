import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";
import { presentMarketEvent } from "@/features/market/lib/marketEventPresentation";
import { parseMarketEnvelope } from "@/features/market/lib/marketProtocol";
import { MarketEventCard } from "@/features/market/ui/MarketEventCard";

import type { TimelineMessage } from "@/features/messages/types";

export function MarketMessageBody({
  body: content,
  pubkey: authorPubkey,
  id: eventId,
  parentId,
}: Pick<TimelineMessage, "body" | "pubkey" | "id" | "parentId">) {
  return parseMarketEnvelope(content) ? (
    <MarketEventMessage
      authorPubkey={authorPubkey ?? ""}
      content={content}
      eventId={eventId}
      isThreadReply={parentId != null}
    />
  ) : null;
}

/** Pulse keeps the listing card; inside its channel signed transitions read as chat. */
function MarketEventMessage({
  authorPubkey,
  content,
  eventId,
  isThreadReply,
}: {
  authorPubkey: string;
  content: string;
  eventId?: string;
  isThreadReply: boolean;
}) {
  const projection = useMarketChannel();
  const envelope = parseMarketEnvelope(content);
  if (!envelope) return null;
  if (!projection) {
    return <MarketEventCard authorPubkey={authorPubkey} content={content} />;
  }
  if (isThreadReply) {
    return (
      <p className="max-w-full text-message">
        {envelope.type === "contract"
          ? envelope.listing.summary
          : presentMarketEvent(envelope).summary}
      </p>
    );
  }
  if (envelope.type !== "response") return null;
  const rejection = eventId
    ? projection.rejected.find((entry) => entry.eventId === eventId)
    : undefined;
  const body = envelope.message;

  return (
    <div
      className="min-w-0 max-w-full"
      data-testid={`market-message-${envelope.type}`}
    >
      <p className="max-w-full text-message">{body}</p>
      {rejection ? (
        <p
          className="mt-1 text-xs text-destructive"
          data-testid="market-message-rejected"
        >
          Not accepted: {rejection.reason}
        </p>
      ) : null}
    </div>
  );
}
