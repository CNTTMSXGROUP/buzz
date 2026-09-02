import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";
import { presentMarketEvent } from "@/features/market/lib/marketEventPresentation";
import { parseMarketEnvelope } from "@/features/market/lib/marketProtocol";
import { MarketEventCard } from "@/features/market/ui/MarketEventCard";

export function MarketMessageBody(
  content: string,
  authorPubkey?: string,
  eventId?: string,
) {
  return parseMarketEnvelope(content) ? (
    <MarketEventMessage
      authorPubkey={authorPubkey ?? ""}
      content={content}
      eventId={eventId}
    />
  ) : null;
}

/** Pulse keeps the listing card; inside its channel signed transitions read as chat. */
function MarketEventMessage({
  authorPubkey,
  content,
  eventId,
}: {
  authorPubkey: string;
  content: string;
  eventId?: string;
}) {
  const projection = useMarketChannel();
  const envelope = parseMarketEnvelope(content);
  if (!envelope) return null;
  if (!projection) {
    return <MarketEventCard authorPubkey={authorPubkey} content={content} />;
  }

  const presentation = presentMarketEvent(envelope);
  const rejection = eventId
    ? projection.rejected.find((entry) => entry.eventId === eventId)
    : undefined;
  const body =
    envelope.type === "contract"
      ? `Opened this market: ${envelope.listing.title}`
      : presentation.summary;

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
