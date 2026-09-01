import { parseMarketEnvelope } from "@/features/market/lib/marketProtocol";
import { MarketEventCard } from "@/features/market/ui/MarketEventCard";

export function MarketMessageBody(content: string, authorPubkey?: string) {
  return parseMarketEnvelope(content) ? (
    <MarketEventCard authorPubkey={authorPubkey ?? ""} content={content} />
  ) : null;
}
