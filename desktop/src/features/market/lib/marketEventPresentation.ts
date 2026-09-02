import type {
  MarketEnvelope,
  MarketListing,
} from "@/features/market/lib/marketProtocol";

export type MarketEventPresentation = {
  amount: string | null;
  eyebrow: string;
  summary: string;
  title: string;
};

function listingAmount(listing: MarketListing): string | null {
  if (listing.priceSats) return `${listing.priceSats} fake sats per unit`;
  if (listing.maxBudgetSats) return `Up to ${listing.maxBudgetSats} fake sats`;
  return null;
}

export function presentMarketEvent(
  envelope: MarketEnvelope,
): MarketEventPresentation {
  switch (envelope.type) {
    case "announcement":
      return {
        amount: listingAmount(envelope.listing),
        eyebrow: `${envelope.listing.direction} · ${envelope.listing.mechanism.replace("-", " ")}`,
        summary: envelope.listing.summary,
        title: envelope.listing.title,
      };
    case "contract":
      return {
        amount: listingAmount(envelope.listing),
        eyebrow: "Market contract opened",
        summary: envelope.listing.summary,
        title: envelope.listing.title,
      };
    case "response":
      return {
        amount: envelope.amountSats
          ? `${envelope.amountSats} fake sats per unit`
          : null,
        eyebrow: "Market response",
        summary: envelope.message,
        title: `${envelope.actorName} responded for ${envelope.quantity} ${envelope.quantity === 1 ? "unit" : "units"}`,
      };
    case "award":
      return {
        amount: `${envelope.amountSats * envelope.quantity} fake sats total`,
        eyebrow: "Market award",
        summary: `${envelope.quantity} ${envelope.quantity === 1 ? "unit" : "units"} awarded at ${envelope.amountSats} fake sats each.`,
        title: `${envelope.actorName} awarded a response`,
      };
    case "fulfillment":
      return {
        amount: null,
        eyebrow: "Market fulfillment",
        summary: envelope.message,
        title: `${envelope.actorName} delivered`,
      };
    case "settlement":
      return {
        amount: `${envelope.amountSats} fake sats`,
        eyebrow: "Settlement",
        summary: `Paid ${envelope.amountSats} fake sats. No real payment moved.`,
        title: `${envelope.actorName} marked the award settled`,
      };
  }
}
