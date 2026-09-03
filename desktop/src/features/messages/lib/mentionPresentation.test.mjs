import assert from "node:assert/strict";
import test from "node:test";
import { buildMentionCandidates } from "./buildMentionCandidates.ts";
const A = "a".repeat(64),
  B = "b".repeat(64),
  VIEWER = "f".repeat(64);
function input(overrides = {}) {
  return {
    activeAgentPubkeys: new Set(),
    activePersonaById: new Map(),
    activePersonas: [],
    canSearchGlobalUsers: true,
    currentPubkey: VIEWER,
    isArchived: () => false,
    managedAgentDirectoryReady: true,
    managedAgentNamesByPubkey: new Map(),
    managedAgentPersonaIds: new Set(),
    managedAgentPersonaIdsByPubkey: new Map(),
    managedAgents: [],
    memberPubkeys: new Set(),
    members: [],
    mentionChannelId: "room",
    mentionableAgentPubkeys: new Set(),
    personaNameByPubkey: new Map(),
    profiles: {},
    relayAgentDirectoryReady: true,
    relayAgentNamesByPubkey: new Map(),
    relayAgents: [],
    userSearchResults: [],
    ...overrides,
  };
}
test("union keeps same-named people and marks collisions before the cap", () => {
  const rows = buildMentionCandidates(
    input({
      userSearchResults: [A, B].map((pubkey) => ({
        pubkey,
        displayName: "Sam",
        isAgent: false,
      })),
    }),
  );
  assert.equal(rows.length, 2);
  assert.ok(rows.every((row) => row.hasNameCollision));
});
