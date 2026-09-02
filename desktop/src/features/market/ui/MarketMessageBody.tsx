import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";
import { presentMarketEvent } from "@/features/market/lib/marketEventPresentation";
import { parseMarketEnvelope } from "@/features/market/lib/marketProtocol";
import { MarketEventCard } from "@/features/market/ui/MarketEventCard";
import { Badge } from "@/shared/ui/badge";

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

/**
 * Inside a market channel, lifecycle envelopes read as ordinary chat
 * messages: the agent's words as the body, with a small inline badge naming
 * the signed transition. Outside market channels (Pulse, forwarded events)
 * the full card treatment remains.
 */
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
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge className="capitalize" variant="secondary">
          {presentation.eyebrow}
        </Badge>
        {presentation.amount ? (
          <span className="text-xs font-medium text-muted-foreground">
            {presentation.amount}
          </span>
        ) : null}
        {rejection ? (
          <Badge data-testid="market-message-rejected" variant="destructive">
            Rejected · {rejection.reason}
          </Badge>
        ) : null}
      </div>
      <p className="mt-1 max-w-full text-message">{body}</p>
    </div>
  );
}
