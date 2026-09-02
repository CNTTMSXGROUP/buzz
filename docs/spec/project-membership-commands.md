# Project Related-Channel Commands

Status: draft architecture decision for the first collaborative Project-membership slice.

## Problem

`kind:30621` gives a Project one owner-controlled identity and metadata event. That works for the Project name, description, visibility, home channel, and repository references, but it does not let Project collaborators link existing channels without the owner's key.

The first requirement is deliberately narrow: a Project owner or home-channel administrator can link and unlink one existing related channel from Desktop or the `buzz` CLI. The implementation must not make Project metadata collaboratively replaceable, introduce a global Project revision, or pre-build a dormant protocol for future member types.

## Decision

Keep `kind:30621` as owner-controlled Project identity and metadata. Represent each collaborative related-channel change as an actor-signed regular event that the authoritative host relay validates and stores. The relay maintains one durable per-channel override row for enforcement and publishes a bounded relay-signed projection of effective related channels.

The accepted command events remain the attributable history. The projection is derived read state, not a replacement for Project identity or metadata.

This is a relay-local extension of NIP-MP, not a replacement for its portable owner-authored container. Linking a channel grants no authority over the Project, channel, or repository.

## Phase 1 event

Use regular `kind:47010`, signed by the human or agent performing the action. One event addresses exactly one related channel.

An add contains:

```json
{
  "kind": 47010,
  "content": "",
  "tags": [
    ["a", "30621:<project-owner>:<project-d>"],
    ["op", "add"],
    ["d", "<channel-uuid>"]
  ]
}
```

A remove uses the same shape with `op=remove`:

```json
{
  "kind": 47010,
  "content": "",
  "tags": [
    ["a", "30621:<project-owner>:<project-d>"],
    ["op", "remove"],
    ["d", "<channel-uuid>"]
  ]
}
```

The Project coordinate uses the standard lowercase `a` tag. The target channel uses a canonical lowercase, hyphenated UUID in `d`, making reverse lookup available through a standard single-letter Nostr tag filter. `kind:47010` remains a regular event rather than a parameterized-replaceable event. `h` is forbidden because the event is global Project metadata rather than a channel-scoped message.

The protocol parser requires empty content and exactly one tag of each required type. The relay may also accept its existing ambient transport authorization tag, but rejects duplicated or unknown protocol tags.

## Authorization

The relay accepts a command when the effective actor is one of:

- the Project signer;
- the registered human owner of an agent-owned Project, using the existing NIP-OA ownership relation;
- an active owner or administrator of the Project's live, unarchived home channel, including the existing rule that the registered human owner of an active owner-role agent may act.

The implementation must reuse one channel-administrator authority predicate rather than create a slightly different Project-only interpretation. The human owner of an admin-role agent does not inherit that agent's authority.

A Project without a resolvable active home channel remains manageable only by the Project signer or its registered human owner. Assigning or changing the home channel remains an owner-authored `30621` update.

Adding requires a live, non-home target channel in the same community. The actor must be an active member of the target channel so a guessed private UUID cannot be published. Removing remains possible after the target is archived or deleted, or after the actor loses target-channel membership, so cleanup is never hidden.

Authorization is evaluated inside the same database transaction as the event and override write. WebSocket, HTTP, and imported events all use this boundary. A copied command has no authority merely because its signature is valid on another relay.

At the transport gate, `kind:47010` requires `channels:write`. Global Project routing is independent of the OAuth/NIP-OA scope name; using `repos:write` would block the standard CLI channel-management token from the intended home-administrator workflow.

## Per-channel concurrency

The relay serializes Project replacements, deletions, and commands under the existing Project-coordinate lock. Channel-membership locks serialize role checks, and channel row locks prevent archive or deletion from racing active-channel validation. The override itself is scoped to one `(community, Project coordinate, channel UUID)` relation.

- Exact signed-event replay succeeds idempotently.
- Adding an already-present relation succeeds without storing a no-op event.
- Removing an absent relation succeeds without storing a no-op event.

Each command is a desired-state toggle. Clients do not need to read a relation before changing it, and the relay decides state changes and no-ops under the same Project-coordinate lock. The lock gives commands a single authoritative acceptance order without a client-maintained predecessor protocol.

Changes to unrelated channels do not share a revision. The relay rejects a command that would add a 65th effective related channel. During migration, owner-authored `30621` events and their derived snapshots use the same 64-channel bound.

## Persistence and reads

Accepted commands are inserted into the normal event store. In the same database transaction, the relay upserts a channel-specific override keyed by community, Project owner, Project `d`, and channel UUID:

- no row: inherit the live `30621` `buzz-related-channel` tag;
- present: an accepted add wins;
- absent: an accepted remove wins.

Absent overrides are retained so an ordinary owner metadata update cannot silently resurrect a removed legacy tag. The owner can deliberately reassert it with an add command.

Effective membership is the live legacy seed overlaid by the command overrides. The override table is the durable current-state authority. Accepted commands remain the attributable audit history, but neither authorization nor current-state reads replay that history.

The relay also publishes one parameterized-replaceable `kind:30623` snapshot per Project, signed by its NIP-11 `self` key. Its tags are ordered exactly as deterministic `d=sha256("buzz:project-related-channels:v1" || NUL || Project coordinate)`, then `a=<Project coordinate>`, then at most 64 strictly sorted `c=[channel UUID]` tags. This is a bounded derived read model, not Project identity or command authority. If an owner metadata replacement makes the effective legacy-plus-override set exceed 64, the relay keeps the Project writable, logs the truncation, and publishes the first 64 canonical entries.

An applied `47010` command, its override, and the new `30623` snapshot commit in one transaction. The relay's specialized `30621` replacement path commits an owner-authored Project head and its regenerated snapshot in the same transaction; generic replacement storage is not sufficient for Project writes. Project deletion tombstones the Project and its snapshot atomically. A no-op writes none of them. The relay signs the snapshot while the transaction's Project-coordinate guard is still held, so there is no post-commit reconciliation gap.

Clients derive each snapshot's deterministic `d`, then query `kind:30623` by `d` and relay author in bounded chunks. A valid snapshot replaces legacy related-channel tags in the read model. During rollout, a missing snapshot leaves the legacy seed intact. An invalid snapshot fails closed to an empty related-channel projection for that Project rather than resurrecting legacy relations or failing the whole Projects surface. Because the relay may advance a replacement event's `created_at` beyond wall-clock time to preserve last-write-wins ordering, clients must not add an `until=now` bound to this current-state query.

A Project coordinate is its stable identity. While no live `30621` exists, commands are rejected and prior overrides are ineffective. A genuinely new Project uses a new `d`; Phase 1 adds no Project epoch or special deletion lifecycle.

Generic NIP-09 deletion must not erase accepted `47010` command facts. Undoing a relationship is another authorized command.

## Agent and Desktop surface

The first implementation exposes singular CLI operations:

```text
buzz projects link-channel --project <coordinate> --channel <uuid>
buzz projects unlink-channel --project <coordinate> --channel <uuid>
buzz projects related-channels --project <coordinate>
```

These names do not conflict with the existing `projects add-channel` command, which drafts a new channel for Desktop to create. The CLI returns the canonical write result. The read command queries the relay-authored snapshot by its deterministic `d` value and relay signer.

Desktop uses the same signed desired-state event. It reads the bounded relay-authored snapshot, with the rollout fallback described above. Its existing owner-only create-channel flow continues to add the new channel to owner metadata. A separate Desktop delivery slice adds link-existing and unlink controls for authorized collaborators.

## Bounds and recovery

- Validate and bound tags before allocating proportional collections.
- Event insertion, override mutation, and snapshot replacement are one database transaction; a failure stores none and is never acknowledged as success.
- The accepted signed event is the durable authorization/audit fact. Post-commit generic audit dispatch is not claimed to be atomic.
- Rebuild processing is bounded, and clients read one bounded current-state snapshot rather than replaying command history.

## Deferred product direction

Later work may generalize explicit Project membership to other types, including tasks that belong directly to Projects. That work should be designed from an implemented workflow rather than pre-built into this channel-only protocol.

Dynamically discovered content is not membership. A pull request mentioned in a member thread or a document attached to a member meeting remains derived context until explicitly added.

The migration overlay is temporary. After legacy `buzz-related-channel` tags have been backfilled once into durable overrides, effective membership should stop consulting those tags so the collaborative model has one current-state source.

Linking is a deliberate visibility action: an authorized administrator who is a member of a target channel may expose that channel UUID to people who can read the Project. Target membership prevents blind UUID guessing; it does not promise that a deliberately linked private channel remains undiscoverable through the Project.

Cold-start reverse resolution from a related channel to its Projects is deferred. The relay snapshot's indexed `c` tags make that bounded lookup possible, while the first Desktop slice learns related channels through the normal Projects enumeration.

## Rejected alternatives

### Relay-authored Project identity

Rejected because Project identity and unrelated metadata must remain owner-authored and portable. The narrow `30623` related-channel snapshot is acceptable only as derived state; it does not replace the Project event.

### Per-author addressable membership events with timestamp last-write-wins

Rejected because addressable coordinates include the signer, so administrators cannot replace one another's state. Cross-author last-write-wins would depend on client-controlled timestamps and could strand a relation behind a future-dated event.

### Project generations

Rejected for Phase 1 because the Project coordinate is already stable identity and current deletion/recreation semantics do not provide an epoch primitive. Adding one would require invasive interception of generic Project replacement and NIP-09 deletion paths.

### Generic member rows and batched commands

Rejected because Phase 1 has one implemented member type and one user action. A channel-specific row and one-channel command are sufficient and make partial failure impossible.

## Delivery slices

1. Relay persistence, authorization, focused tests, and `buzz` CLI link/unlink commands.
2. Desktop link-existing and unlink UI using the same commands.
3. Other explicit member types only with their product workflow.

Each slice must be independently usable and contain no dormant protocol path.
