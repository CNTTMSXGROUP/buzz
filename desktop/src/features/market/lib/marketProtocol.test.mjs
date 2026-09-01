import assert from "node:assert/strict";
import test from "node:test";

import {
  MARKET_PROTOCOL,
  parseMarketEnvelope,
  projectMarketNotes,
} from "./marketProtocol.ts";

const SELLER = "a".repeat(64);
const BUYER = "b".repeat(64);
const LISTING_ID = "1".repeat(64);
const RESPONSE_ID = "2".repeat(64);
const AWARD_ID = "3".repeat(64);
const FULFILLMENT_ID = "4".repeat(64);

function note(id, pubkey, createdAt, envelope) {
  return { id, pubkey, createdAt, content: JSON.stringify(envelope), tags: [] };
}

function listing(overrides = {}) {
  return {
    protocol: MARKET_PROTOCOL,
    type: "listing",
    marketId: "incident-report",
    version: 1,
    listing: {
      actorName: "Seller Agent",
      direction: "offer",
      mechanism: "fixed",
      title: "Incident report",
      summary: "A cited report delivered after award.",
      quantity: 1,
      priceSats: 50,
      deliveryMinutes: 120,
      ...overrides,
    },
  };
}

test("parseMarketEnvelope accepts v0 and rejects unrelated Pulse notes", () => {
  assert.equal(parseMarketEnvelope("hello Pulse"), null);
  assert.equal(parseMarketEnvelope('{"protocol":"other"}'), null);
  assert.equal(parseMarketEnvelope(JSON.stringify(listing()))?.type, "listing");
});

test("projectMarketNotes folds offer through fake settlement", () => {
  const notes = [
    note(LISTING_ID, SELLER, 100, listing()),
    note(RESPONSE_ID, BUYER, 101, {
      protocol: MARKET_PROTOCOL,
      type: "response",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      actorName: "Buyer Agent",
      quantity: 1,
      amountSats: 50,
      message: "I reserve the report.",
    }),
    note(AWARD_ID, SELLER, 102, {
      protocol: MARKET_PROTOCOL,
      type: "award",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      responseEventId: RESPONSE_ID,
      actorName: "Seller Agent",
      quantity: 1,
      amountSats: 50,
    }),
    note(FULFILLMENT_ID, SELLER, 103, {
      protocol: MARKET_PROTOCOL,
      type: "fulfillment",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      awardEventId: AWARD_ID,
      actorName: "Seller Agent",
      message: "Report delivered with cited evidence.",
    }),
    note("5".repeat(64), BUYER, 104, {
      protocol: MARKET_PROTOCOL,
      type: "settlement",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      awardEventId: AWARD_ID,
      fulfillmentEventId: FULFILLMENT_ID,
      actorName: "Buyer Agent",
      amountSats: 50,
    }),
  ];

  const projection = projectMarketNotes(notes);
  assert.ok(projection);
  assert.equal(projection.scenario.status, "Fulfilled");
  assert.equal(projection.scenario.title, "Incident report");
  assert.equal(projection.scenario.activity.length, 5);
  assert.equal(
    projection.scenario.activity[0].title,
    "Sandbox settlement complete",
  );
  assert.deepEqual(projection.rejected, []);
});

test("projectMarketNotes selects an explicit market", () => {
  const other = listing();
  other.marketId = "other-market";
  other.listing.title = "Other listing";
  const projection = projectMarketNotes(
    [
      note(LISTING_ID, SELLER, 100, listing()),
      note("9".repeat(64), BUYER, 99, other),
    ],
    "incident-report",
  );
  assert.ok(projection);
  assert.equal(projection.scenario.title, "Incident report");
});

test("projectMarketNotes rejects overselling and unauthorized awards", () => {
  const notes = [
    note(LISTING_ID, SELLER, 100, listing()),
    note(RESPONSE_ID, BUYER, 101, {
      protocol: MARKET_PROTOCOL,
      type: "response",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      actorName: "Buyer Agent",
      quantity: 1,
      amountSats: 50,
      message: "Reserve one.",
    }),
    note(AWARD_ID, BUYER, 102, {
      protocol: MARKET_PROTOCOL,
      type: "award",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      responseEventId: RESPONSE_ID,
      actorName: "Buyer Agent",
      quantity: 1,
      amountSats: 50,
    }),
  ];

  const projection = projectMarketNotes(notes);
  assert.ok(projection);
  assert.equal(projection.scenario.status, "Open");
  assert.equal(projection.rejected[0].reason, "only listing author may award");
});

test("reverse auction enforces the minimum decrement", () => {
  const auction = listing({
    direction: "request",
    mechanism: "reverse-auction",
    quantity: 1,
    priceSats: undefined,
    maxBudgetSats: 100,
    minimumDecrementSats: 10,
  });
  const notes = [
    note(LISTING_ID, SELLER, 100, auction),
    note(RESPONSE_ID, BUYER, 101, {
      protocol: MARKET_PROTOCOL,
      type: "response",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      actorName: "Bidder One",
      quantity: 1,
      amountSats: 90,
      message: "I bid 90.",
    }),
    note("6".repeat(64), "c".repeat(64), 102, {
      protocol: MARKET_PROTOCOL,
      type: "response",
      marketId: "incident-report",
      listingEventId: LISTING_ID,
      actorName: "Bidder Two",
      quantity: 1,
      amountSats: 85,
      message: "I bid 85.",
    }),
  ];

  const projection = projectMarketNotes(notes);
  assert.ok(projection);
  assert.equal(projection.scenario.liveMetrics[1].value, "1");
  assert.equal(
    projection.rejected[0].reason,
    "bid does not meet minimum decrement",
  );
});
