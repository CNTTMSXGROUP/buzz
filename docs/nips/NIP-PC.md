NIP-PC
======

Collaborative Project Changes
-----------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format and addressable events), NIP-09 (event deletion), NIP-29 (home-channel authority), and [NIP-MP](NIP-MP.md) (owner-signed Project identity)

## Abstract

This NIP separates a Project's stable identity from its collaboratively editable effective state. The owner-signed NIP-MP `kind:30621` remains the Project identity and recovery source. An authorized actor requests an atomic change with `kind:47010`, and the relay exposes the resulting relational state as a relay-signed addressable `kind:30623` projection.

Version 1 changes only the bounded `buzz-related-channel` collection. The command and projection are versioned so later revisions can add fields without turning `kind:30621` into a shared-signing event or forging an owner-authored replacement.

## Non-Goals

This NIP does not give a Project authority over a repository, channel, or other referenced entity.

This NIP does not make `kind:30621` relay-signed or collaboratively replaceable. Only its owner can replace it under NIP-01.

This NIP does not define Project creation, deletion, or owner recovery as `kind:47010` commands. Creation and recovery use owner-signed `kind:30621`; deletion uses the owner or NIP-OA-owner NIP-09 paths defined by NIP-MP. Home-channel owners and admins cannot delete a Project through the related-channel-only version 1 command.

This NIP does not implement relational persistence, relay execution, or client behavior. It fixes the wire contract those layers will implement.

## Kinds

| Kind | Name | Signer | Class | Purpose |
|------|------|--------|-------|---------|
| `47010` | Project Change | authorized actor | regular command | Atomically add or remove related channels at an expected revision |
| `30623` | Project State | relay | addressable projection | Publish the relay's current effective Project state |

`kind:47010` is a global transactional command. It is stored as immutable attribution only when accepted; it is not effective state by itself.

`kind:30623` is relay-only. A relay MUST reject client-submitted events of this kind, including an event whose signer happens to equal the configured relay pubkey but which arrived through ordinary client ingest.

Client submission of `kind:47010` requires `repos:write`. `kind:30623` is relay-authored and therefore has no client write scope. Neither kind is channel-scoped; a stray `h` tag does not select a Project or a community.

## Project Coordinate

Both events identify a Project by its canonical NIP-01 coordinate:

```text
30621:<owner-pubkey-hex>:<project-d>
```

The kind segment is the literal `30621`, the owner is exactly 64 lowercase hexadecimal characters, and the Project `d` value is non-empty and preserved verbatim. Parsing splits on the first two colons so a `d` value containing a colon remains addressable.

## Project Change Event

```jsonc
{
  "kind": 47010,
  "pubkey": "<actor-pubkey-hex>",
  "tags": [
    ["a", "30621:<owner-pubkey-hex>:<project-d>"],
    ["expected-revision", "7"],
    ["auth", "<owner-pubkey-hex>", "kind=47010", "<signature-hex>"]
  ],
  "content": "{\"v\":1,\"patch\":{\"related_channels\":{\"add\":[\"11111111-1111-4111-8111-111111111111\"],\"remove\":[]}}}"
}
```

The event MUST contain exactly one two-element `a` tag and exactly one two-element `expected-revision` tag. It MAY additionally contain exactly one canonical four-element NIP-OA `auth` tag; no other tags are permitted. When present, the relay MUST verify the credential and every condition against this command event. The expected revision is a canonical base-10 integer in `1..=9223372036854775807`: digits only, no sign, and no leading zero. The command compares this value with the authoritative relational revision, never with a Project State event id.

The JSON content has this version 1 shape:

```json
{
  "v": 1,
  "patch": {
    "related_channels": {
      "add": ["<canonical-uuid>"],
      "remove": ["<canonical-uuid>"]
    }
  }
}
```

Every UUID MUST equal its lowercase hyphenated `Uuid::to_string()` form. `add` and `remove` are both required arrays, may each contain at most 64 entries, and are interpreted as sets. An event is invalid if either array contains a duplicate, the arrays overlap, both are empty, the resulting collection exceeds 64 channels, or the requested related channel is the current home channel.

Adding an already-related channel or removing an absent channel is invalid. A command that produces no semantic change MUST NOT advance the revision. Retrying the exact same accepted event id is instead an idempotent success, even after later changes have advanced the Project.

Version 1 decoders MUST reject unknown JSON fields. This prevents a newer client from receiving success when an older relay ignored its intent. A later version may add typed field patches while preserving the same coordinate and CAS envelope.

## Authorization

The command signer is the actor and remains the sole event author. A valid current NIP-OA credential does not rewrite authorship; it identifies an optional delegated authorization principal. The relay MUST use that principal only when it also matches the actor's immutable registered owner in the current community. A stored relationship without a valid credential on this command grants no authority.

The actor is authorized when the actor is the Project owner. The delegated principal, when present and registered, is independently authorized when it is the Project owner. Otherwise, the effective Project state must have a resolvable home channel and either the actor or the delegated principal must independently be an active `owner` or `admin` of that channel. Privileges from the two principals MUST NOT be combined. An archived or deleted home channel grants no authority. Authorization reads the current relational Project state and current channel membership under the same serialization boundary as the CAS; an event projection or stale client snapshot is not an authorization source.

No Project relationship grants authority over a related channel. Adding a channel records grouping metadata only.

## Relational State and CAS

The authoritative row is keyed by `(community, Project owner, Project d)` and carries a monotonic signed-64-bit revision, deletion state, the current owner identity event id, the last accepted change or lifecycle event id, and the effective Project document.

The first accepted owner-signed `kind:30621` materializes revision `1`. Every accepted Project Change, owner recovery replacement, deletion, or recreation increments the revision by one. Revisions never reset, including after deletion and recreation, which prevents an ABA conflict from making an old command current again. Overflow rejects the mutation.

Applying a Project Change is one transaction: verify authorization, compare `expected-revision`, apply the entire patch to a copy, validate all invariants, insert the immutable command event, and CAS the authoritative row. Any failure leaves both the event log and Project state unchanged.

## Owner Identity and Recovery

An accepted newer owner-signed `kind:30621` is a full recovery snapshot. It replaces the effective NIP-MP fields and extension tags with those carried by that owner event, advances the relational revision, and leaves the stable Project coordinate unchanged. A duplicate or superseded `kind:30621` does not advance the revision.

Unknown NIP-MP extension tags are preserved byte-for-byte across collaborative version 1 changes. Transport-only `auth` tags are not Project metadata and are not carried into effective state. Owner recovery may intentionally replace or remove extension tags because the owner event is the recovery source.

An accepted owner-authorized NIP-09 deletion advances the row to a deleted tombstone. It does not delete member repositories or related channels. A later valid owner-signed `kind:30621` recreates the Project at the next revision rather than resetting to revision `1`.

## Project State Projection

```jsonc
{
  "kind": 30623,
  "pubkey": "<relay-identity-pubkey-hex>",
  "tags": [
    ["d", "<sha256-project-coordinate-hex>"],
    ["a", "30621:<owner-pubkey-hex>:<project-d>"],
    ["rev", "8"],
    ["e", "<current-kind-30621-event-id>", "", "identity"],
    ["e", "<last-change-or-lifecycle-event-id>", "", "change"]
  ],
  "content": "{\"v\":1,\"deleted\":false,\"project_tags\":[[\"d\",\"<project-d>\"],[\"buzz-related-channel\",\"11111111-1111-4111-8111-111111111111\"]]}"
}
```

The address `d` is the lowercase SHA-256 hex digest of the UTF-8 canonical Project coordinate. Hashing keeps the projection key fixed at 64 bytes even when the Project's own `d` approaches its 1024-byte bound. The `a` tag carries the unhashed coordinate. The `rev` tag uses the same canonical integer grammar as `expected-revision`.

The `identity` event id names the current owner-signed `kind:30621`. The `change` event id names the exact command, owner recovery, deletion, or recreation that produced this revision. Both are lowercase 64-character event ids. On initial materialization or owner recovery they may name the same `kind:30621` event.

For a live Project, `project_tags` is the complete effective NIP-MP-compatible tag set, including exactly one Project-slug `d` tag and the effective `buzz-related-channel` tags. Known set-valued fields are emitted in deterministic lexical order. Preserved unknown extension tags retain their byte values and relative order. Array order carries no Project membership semantics.

A deleted Project is projected as:

```json
{"v":1,"deleted":true,"project_tags":[]}
```

Clients MUST use `rev` as the CAS token. Projection repair may sign a newer `kind:30623` event with the same revision and content, so the projection event id is not a revision.

## Publication and Reconciliation

The relational row remains authoritative if projection publication or fan-out fails. The relay MUST retain a durable retry or reconciliation path and MUST NOT report a committed mutation as rolled back merely because derived publication failed. Reconciliation republishes the current row without advancing its revision.

Every newly signed projection for one address MUST have a `created_at` strictly greater than every previously accepted projection at that address, including a repair that republishes the same revision. Allocation MUST occur under the coordinate's serialization boundary as `max(current Unix time, previous projection created_at + 1)`; wall-clock time alone is insufficient. If no greater timestamp can be represented, publication fails without changing authoritative state.

## Schema Evolution

Command and projection content have independent explicit versions. A decoder that does not understand a command version MUST reject it. A client that does not understand a Project State version MUST ignore that projection rather than guess at its state.

Future command versions should add typed field patches. They must keep omitted fields unchanged, use collection deltas rather than whole-list replacement where independent edits are expected, preserve unknown extension tags, and retain the same monotonic CAS and provenance rules.

## Security Considerations

Clients MUST accept Project State only from the relay identity advertised through the relay's trusted identity mechanism. Structural parsing alone does not establish that trust.

The command signer remains attributable through the immutable `kind:47010` event. The relay-signed projection states what became effective; it does not rewrite history to make the Project owner appear to have signed a collaborator's change.

The Project coordinate selects state only inside the host-bound community. A relay MUST never derive the community from a client-supplied Project tag.
