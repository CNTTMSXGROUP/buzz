import { z } from "zod";

import type {
  MarketActivity,
  MarketScenario,
  MarketScenarioId,
} from "@/features/market/lib/marketPrototypeData";
import type { UserNote } from "@/shared/api/socialTypes";
import { truncatePubkey } from "@/shared/lib/pubkey";

export const MARKET_PROTOCOL = "buzz-market/v0" as const;

const EventId = z.string().regex(/^[0-9a-f]{64}$/);
const MarketId = z.string().min(1).max(120);
const ActorName = z.string().min(1).max(80);
const PositiveInteger = z.number().int().positive();
const NonnegativeInteger = z.number().int().nonnegative();

const BaseEnvelope = z.object({
  protocol: z.literal(MARKET_PROTOCOL),
  marketId: MarketId,
});

const ListingEnvelope = BaseEnvelope.extend({
  type: z.literal("listing"),
  version: z.literal(1),
  listing: z.object({
    actorName: ActorName,
    direction: z.enum(["offer", "request"]),
    mechanism: z.enum(["fixed", "reverse-auction", "tender"]),
    title: z.string().min(1).max(160),
    summary: z.string().min(1).max(2_000),
    quantity: z.union([PositiveInteger, z.literal("unlimited")]),
    priceSats: PositiveInteger.optional(),
    maxBudgetSats: PositiveInteger.optional(),
    closesAt: NonnegativeInteger.optional(),
    deliveryMinutes: PositiveInteger.optional(),
    minimumDecrementSats: PositiveInteger.optional(),
  }),
});

const ResponseEnvelope = BaseEnvelope.extend({
  type: z.literal("response"),
  listingEventId: EventId,
  actorName: ActorName,
  quantity: PositiveInteger,
  amountSats: PositiveInteger.optional(),
  message: z.string().min(1).max(2_000),
});

const AwardEnvelope = BaseEnvelope.extend({
  type: z.literal("award"),
  listingEventId: EventId,
  responseEventId: EventId,
  actorName: ActorName,
  quantity: PositiveInteger,
  amountSats: PositiveInteger,
});

const FulfillmentEnvelope = BaseEnvelope.extend({
  type: z.literal("fulfillment"),
  listingEventId: EventId,
  awardEventId: EventId,
  actorName: ActorName,
  message: z.string().min(1).max(2_000),
});

const SettlementEnvelope = BaseEnvelope.extend({
  type: z.literal("settlement"),
  listingEventId: EventId,
  awardEventId: EventId,
  fulfillmentEventId: EventId,
  actorName: ActorName,
  amountSats: PositiveInteger,
});

export const MarketEnvelopeSchema = z.discriminatedUnion("type", [
  ListingEnvelope,
  ResponseEnvelope,
  AwardEnvelope,
  FulfillmentEnvelope,
  SettlementEnvelope,
]);

export type MarketEnvelope = z.infer<typeof MarketEnvelopeSchema>;
export type MarketListingEnvelope = z.infer<typeof ListingEnvelope>;

export type MarketProjection = {
  listingEventId: string;
  scenario: MarketScenario;
  rejected: Array<{ eventId: string; reason: string }>;
};

export function parseMarketEnvelope(content: string): MarketEnvelope | null {
  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch {
    return null;
  }
  const parsed = MarketEnvelopeSchema.safeParse(value);
  return parsed.success ? parsed.data : null;
}

function scenarioIdForListing(
  listing: MarketListingEnvelope["listing"],
  settled: boolean,
): MarketScenarioId {
  if (settled) return "awarded";
  if (listing.mechanism === "reverse-auction") return "auction";
  if (listing.mechanism === "tender") return "tender";
  return listing.quantity === "unlimited" ? "unlimited" : "finite";
}

function formatAt(timestamp: number): string {
  return new Intl.DateTimeFormat("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "UTC",
    timeZoneName: "short",
  }).format(new Date(timestamp * 1_000));
}

function actorDetail(pubkey: string): string {
  return `agent · ${truncatePubkey(pubkey)}`;
}

function listingPrice(listing: MarketListingEnvelope["listing"]): string {
  if (listing.mechanism === "fixed") {
    return `${listing.priceSats ?? 0} sats per unit`;
  }
  if (listing.mechanism === "reverse-auction") {
    return `Maximum ${listing.maxBudgetSats ?? 0} sats`;
  }
  return listing.maxBudgetSats
    ? `Budget up to ${listing.maxBudgetSats} sats`
    : "Judged on published criteria";
}

function validateListing(
  listing: MarketListingEnvelope["listing"],
): string | null {
  if (listing.mechanism === "fixed" && !listing.priceSats) {
    return "fixed listing requires priceSats";
  }
  if (listing.mechanism === "reverse-auction" && !listing.maxBudgetSats) {
    return "reverse auction requires maxBudgetSats";
  }
  return null;
}

function responseIsValid(
  listing: MarketListingEnvelope["listing"],
  response: Extract<MarketEnvelope, { type: "response" }>,
  bestAuctionBid: number | null,
): string | null {
  if (
    listing.quantity !== "unlimited" &&
    response.quantity > listing.quantity
  ) {
    return "response quantity exceeds listing quantity";
  }
  if (
    listing.mechanism === "fixed" &&
    response.amountSats !== listing.priceSats
  ) {
    return "fixed-price response must match priceSats";
  }
  if (listing.mechanism === "reverse-auction") {
    if (!response.amountSats)
      return "reverse-auction response requires amountSats";
    if (listing.maxBudgetSats && response.amountSats > listing.maxBudgetSats) {
      return "bid exceeds maximum budget";
    }
    const decrement = listing.minimumDecrementSats ?? 1;
    if (
      bestAuctionBid !== null &&
      response.amountSats > bestAuctionBid - decrement
    ) {
      return "bid does not meet minimum decrement";
    }
  }
  return null;
}

function envelopePhase(envelope: MarketEnvelope): number {
  switch (envelope.type) {
    case "listing":
      return 0;
    case "response":
      return 1;
    case "award":
      return 2;
    case "fulfillment":
      return 3;
    case "settlement":
      return 4;
  }
}

export function projectMarketNotes(
  notes: UserNote[],
  marketId?: string,
): MarketProjection | null {
  const parsed = notes
    .map((note) => ({ note, envelope: parseMarketEnvelope(note.content) }))
    .filter(
      (entry): entry is { note: UserNote; envelope: MarketEnvelope } =>
        entry.envelope !== null &&
        (marketId === undefined || entry.envelope.marketId === marketId),
    )
    .sort(
      (left, right) =>
        left.note.createdAt - right.note.createdAt ||
        envelopePhase(left.envelope) - envelopePhase(right.envelope) ||
        left.note.id.localeCompare(right.note.id),
    );
  const listingEntry = parsed.find(
    (entry) =>
      entry.envelope.type === "listing" &&
      validateListing(entry.envelope.listing) === null,
  );
  if (listingEntry?.envelope.type !== "listing") return null;

  const listingEventId = listingEntry.note.id;
  const listing = listingEntry.envelope;
  const marketEvents = parsed.filter(
    (entry) =>
      entry.envelope.marketId === listing.marketId &&
      entry.note.createdAt >= listingEntry.note.createdAt,
  );
  const responses = new Map<
    string,
    { note: UserNote; envelope: Extract<MarketEnvelope, { type: "response" }> }
  >();
  const awards = new Map<
    string,
    { note: UserNote; envelope: Extract<MarketEnvelope, { type: "award" }> }
  >();
  const fulfillments = new Map<
    string,
    {
      note: UserNote;
      envelope: Extract<MarketEnvelope, { type: "fulfillment" }>;
    }
  >();
  const settlements = new Map<string, UserNote>();
  const rejected: MarketProjection["rejected"] = [];
  let awardedQuantity = 0;
  let bestAuctionBid: number | null = null;

  for (const entry of marketEvents) {
    const { envelope, note } = entry;
    if (envelope.type === "listing") continue;
    if (envelope.listingEventId !== listingEventId) {
      rejected.push({
        eventId: note.id,
        reason: "listingEventId does not target current listing",
      });
      continue;
    }
    if (listing.listing.closesAt && note.createdAt > listing.listing.closesAt) {
      rejected.push({
        eventId: note.id,
        reason: "event arrived after listing close",
      });
      continue;
    }

    if (envelope.type === "response") {
      const reason = responseIsValid(listing.listing, envelope, bestAuctionBid);
      if (reason) {
        rejected.push({ eventId: note.id, reason });
        continue;
      }
      responses.set(note.id, { note, envelope });
      if (listing.listing.mechanism === "reverse-auction") {
        bestAuctionBid = envelope.amountSats ?? bestAuctionBid;
      }
      continue;
    }

    if (envelope.type === "award") {
      const response = responses.get(envelope.responseEventId);
      const available =
        listing.listing.quantity === "unlimited"
          ? Number.POSITIVE_INFINITY
          : listing.listing.quantity - awardedQuantity;
      const reason =
        note.pubkey !== listingEntry.note.pubkey
          ? "only listing author may award"
          : !response
            ? "award references unknown response"
            : awards.has(envelope.responseEventId)
              ? "response already awarded"
              : envelope.quantity > response.envelope.quantity
                ? "award quantity exceeds response quantity"
                : envelope.quantity > available
                  ? "award quantity exceeds available quantity"
                  : listing.listing.mechanism !== "tender" &&
                      envelope.amountSats !== response.envelope.amountSats
                    ? "award amount differs from response"
                    : null;
      if (reason) {
        rejected.push({ eventId: note.id, reason });
        continue;
      }
      awards.set(envelope.responseEventId, { note, envelope });
      awardedQuantity += envelope.quantity;
      continue;
    }

    if (envelope.type === "fulfillment") {
      const award = [...awards.values()].find(
        (value) => value.note.id === envelope.awardEventId,
      );
      const response = award
        ? responses.get(award.envelope.responseEventId)
        : null;
      const fulfiller =
        listing.listing.direction === "offer"
          ? listingEntry.note.pubkey
          : response?.note.pubkey;
      const reason = !award
        ? "fulfillment references unknown award"
        : note.pubkey !== fulfiller
          ? "fulfillment author is not the delivering agent"
          : fulfillments.has(envelope.awardEventId)
            ? "award already fulfilled"
            : null;
      if (reason) {
        rejected.push({ eventId: note.id, reason });
        continue;
      }
      fulfillments.set(envelope.awardEventId, { note, envelope });
      continue;
    }

    const award = [...awards.values()].find(
      (value) => value.note.id === envelope.awardEventId,
    );
    const response = award
      ? responses.get(award.envelope.responseEventId)
      : null;
    const fulfillment = fulfillments.get(envelope.awardEventId);
    const payer =
      listing.listing.direction === "offer"
        ? response?.note.pubkey
        : listingEntry.note.pubkey;
    const reason =
      !award || !fulfillment
        ? "settlement requires a fulfilled award"
        : envelope.fulfillmentEventId !== fulfillment.note.id
          ? "settlement references wrong fulfillment"
          : note.pubkey !== payer
            ? "settlement author is not the payer"
            : envelope.amountSats !==
                award.envelope.amountSats * award.envelope.quantity
              ? "settlement amount differs from award total"
              : settlements.has(envelope.awardEventId)
                ? "award already settled"
                : null;
    if (reason) {
      rejected.push({ eventId: note.id, reason });
      continue;
    }
    settlements.set(envelope.awardEventId, note);
  }

  const accepted = marketEvents.filter((entry) => {
    if (entry.envelope.type === "listing")
      return entry.note.id === listingEventId;
    return !rejected.some((item) => item.eventId === entry.note.id);
  });
  const activities: MarketActivity[] = accepted
    .map(({ note, envelope }): MarketActivity => {
      if (envelope.type === "listing") {
        return {
          actor: envelope.listing.actorName,
          at: formatAt(note.createdAt),
          detail: `Contract v${envelope.version} is signed and open on Pulse.`,
          state: "accepted",
          title: "Market opened",
        };
      }
      if (envelope.type === "response") {
        return {
          actor: envelope.actorName,
          at: formatAt(note.createdAt),
          detail: envelope.message,
          state: "discussion",
          title:
            listing.listing.mechanism === "reverse-auction"
              ? "Bid submitted"
              : "Response submitted",
        };
      }
      if (envelope.type === "award") {
        return {
          actor: envelope.actorName,
          at: formatAt(note.createdAt),
          detail: `Awarded ${envelope.quantity} unit for ${envelope.amountSats} sats each.`,
          state: "terminal",
          title: "Response awarded",
        };
      }
      if (envelope.type === "fulfillment") {
        return {
          actor: envelope.actorName,
          at: formatAt(note.createdAt),
          detail: envelope.message,
          state: "terminal",
          title: "Delivery fulfilled",
        };
      }
      return {
        actor: envelope.actorName,
        at: formatAt(note.createdAt),
        detail: `${envelope.amountSats} fake sats released for the fulfilled award.`,
        state: "terminal",
        title: "Sandbox settlement complete",
      };
    })
    .reverse();

  const settled = settlements.size > 0;
  const fulfilled = fulfillments.size;
  const scenarioId = scenarioIdForListing(listing.listing, settled);
  const quantity = listing.listing.quantity;
  const available =
    quantity === "unlimited"
      ? "Unlimited"
      : `${quantity - awardedQuantity} of ${quantity}`;
  const directionLabel =
    listing.listing.direction === "offer" ? "Seller" : "Requester";
  const closeAt = listing.listing.closesAt
    ? `Closes ${new Date(listing.listing.closesAt * 1_000).toISOString().replace("T", " ").slice(0, 16)} UTC`
    : quantity === "unlimited"
      ? "No quantity limit"
      : "No scheduled close · UTC";
  const status: MarketScenario["status"] = settled
    ? "Fulfilled"
    : awards.size > 0
      ? "Awarded"
      : "Open";

  return {
    listingEventId,
    rejected,
    scenario: {
      id: scenarioId,
      eyebrow: `${listing.listing.direction === "offer" ? "Offer" : "Request"} · ${listing.listing.mechanism.replace("-", " ")} · ${quantity === "unlimited" ? "unlimited" : "finite"}`,
      title: listing.listing.title,
      summary: listing.listing.summary,
      direction:
        listing.listing.direction === "offer"
          ? `Buyer pays ${listingPrice(listing.listing)} · Seller delivers`
          : `Requester pays ${listingPrice(listing.listing)} · Winning agent delivers`,
      mode:
        listing.listing.mechanism === "fixed"
          ? "Fixed price"
          : listing.listing.mechanism === "reverse-auction"
            ? "Reverse auction"
            : "Qualitative tender",
      status,
      statusDetail: `Pulse state ${accepted.at(-1)?.note.id.slice(0, 8) ?? listingEventId.slice(0, 8)} · ${formatAt(accepted.at(-1)?.note.createdAt ?? listingEntry.note.createdAt)}`,
      closeAt,
      contractId: `${MARKET_PROTOCOL}:${listing.marketId}:v${listing.version}`,
      primaryAction:
        listing.listing.mechanism === "reverse-auction"
          ? `Bid below ${bestAuctionBid ?? listing.listing.maxBudgetSats ?? 0} sats`
          : `Respond for ${listing.listing.priceSats ?? listing.listing.maxBudgetSats ?? 0} sats`,
      terms: [
        {
          label: directionLabel,
          value: listing.listing.actorName,
          detail: actorDetail(listingEntry.note.pubkey),
        },
        {
          label: listing.listing.direction === "offer" ? "Price" : "Reward",
          value: listingPrice(listing.listing),
          detail: "fake-sats sandbox",
        },
        { label: "Initial quantity", value: String(quantity) },
        ...(listing.listing.deliveryMinutes
          ? [
              {
                label: "Delivery deadline",
                value: `${listing.listing.deliveryMinutes} minutes after award`,
              },
            ]
          : []),
        {
          label: "Contract version",
          value: `v${listing.version} · event ${listingEventId.slice(0, 8)}`,
        },
        { label: "Settlement", value: "Signed fake-sats receipt" },
      ],
      liveMetrics: [
        { label: "Available", value: available },
        { label: "Responses", value: String(responses.size) },
        { label: "Awarded", value: String(awards.size) },
        {
          label: "Fulfilled / settled",
          value: `${fulfilled} / ${settlements.size}`,
        },
      ],
      activity: activities,
    },
  };
}
