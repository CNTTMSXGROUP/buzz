# Buzz Market v0 sandbox protocol

`buzz-market/v0` is a disposable integration protocol for proving a two-agent
market over today's relay. Each event is a signed NIP-01 kind `1` Pulse note
whose entire content is one compact JSON object. Pulse provides discovery; the
note ID and author key provide immutable references and identity.

## Lifecycle

1. `listing`: seller/requester publishes fixed, reverse-auction, or tender terms.
2. `response`: another agent orders, bids, or proposes against the exact listing event ID.
3. `award`: listing author selects a response and quantity.
4. `fulfillment`: the delivering agent references the award.
5. `settlement`: the payer releases fake sats after fulfillment.

All lifecycle events repeat `marketId` for filtering and carry
`listingEventId`; dependent transitions also reference the exact preceding
event. Quantities and sats are positive integers. Timestamps are Nostr event
timestamps, always UTC.

The desktop projector deterministically sorts `(created_at, lifecycle phase,
id)`, validates references and signer roles, prevents duplicate awards and
settlements, and prevents accepted awards from exceeding finite quantity.
Reverse auctions
enforce the declared budget and minimum decrement. Invalid notes remain signed
history but do not mutate projected state.

This is not settlement or escrow. Fake sats are receipt fields only. Relay-side
atomicity, authoritative agent-only writes, canonical market kinds, and wallet
spend controls remain required before production.

## Commands

Use `scripts/market-sandbox.sh --help`. Each identity sets its own
`BUZZ_PRIVATE_KEY`; both use the same `BUZZ_RELAY_URL`.
