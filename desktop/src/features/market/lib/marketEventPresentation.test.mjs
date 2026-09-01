import assert from "node:assert/strict";
import test from "node:test";

import { presentMarketEvent } from "./marketEventPresentation.ts";
import { MARKET_PROTOCOL, parseMarketEnvelope } from "./marketProtocol.ts";

const channelId = "123e4567-e89b-12d3-a456-426614174000";
const listingEventId = "1".repeat(64);

function parsed(value) {
  const envelope = parseMarketEnvelope(JSON.stringify(value));
  assert.ok(envelope);
  return envelope;
}

test("announcement presentation exposes useful listing fields, not JSON", () => {
  const presentation = presentMarketEvent(
    parsed({
      protocol: MARKET_PROTOCOL,
      type: "announcement",
      channelId,
      version: 1,
      listingEventId,
      listing: {
        actorName: "Moneypenny",
        direction: "offer",
        mechanism: "fixed",
        title: "Daily standup digest",
        summary: "One cited digest delivered as a channel message.",
        quantity: 1,
        priceSats: 50,
      },
    }),
  );

  assert.deepEqual(presentation, {
    amount: "50 fake sats per unit",
    eyebrow: "offer · fixed",
    summary: "One cited digest delivered as a channel message.",
    title: "Daily standup digest",
  });
});

test("channel lifecycle events get human-readable cards", () => {
  const presentation = presentMarketEvent(
    parsed({
      protocol: MARKET_PROTOCOL,
      type: "fulfillment",
      channelId,
      listingEventId,
      awardEventId: "2".repeat(64),
      actorName: "Seller Agent",
      message: "The cited digest is attached.",
    }),
  );

  assert.equal(presentation.eyebrow, "Market fulfillment");
  assert.equal(presentation.title, "Seller Agent delivered");
  assert.equal(presentation.summary, "The cited digest is attached.");
});
