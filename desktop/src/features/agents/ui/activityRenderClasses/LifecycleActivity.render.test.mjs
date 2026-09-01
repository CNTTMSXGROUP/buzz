import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LifecycleActivity } from "./LifecycleActivity.tsx";
import { buildTranscript } from "../agentSessionTranscript.ts";

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

const BASE_PROPS = {
  agentAvatarUrl: null,
  agentName: "Test Agent",
  agentPubkey: "pubkey123",
};

const BASE_IDENTITY = {
  turnId: "turn-1",
  sessionId: "session-1",
  channelId: "channel-1",
};

/**
 * Build a pending permission lifecycle item with the given options array.
 * The card is actionable (awaiting a user decision) and has a request nonce.
 */
function pendingPermissionItem(options) {
  return {
    id: "perm-1",
    type: "lifecycle",
    renderClass: "permission",
    title: "Tool requires approval",
    text: "Run shell command",
    timestamp: "2026-08-10T00:00:00.000Z",
    requestNonce: "nonce-abc",
    actionable: true,
    options,
    ...BASE_IDENTITY,
  };
}

// ---------------------------------------------------------------------------
// allow_once — renders a green actionable Allow button
// ---------------------------------------------------------------------------

test("test_allow_once_renders_actionable_allow_button", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-allow", kind: "allow_once", label: "Allow once" },
      ]),
    }),
  );

  // The button must be present and labelled correctly.
  assert.ok(
    html.includes("permission-decision-opt-allow"),
    "allow_once option should render a button with its optionId testid",
  );
  assert.ok(
    html.includes("Allow once"),
    "allow_once option should show its label",
  );

  // The persistent-grant badge must NOT appear for a pure allow_once card.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "allow_once card should not render the persistent-grant badge",
  );
});

// ---------------------------------------------------------------------------
// reject_once — renders a red actionable Deny button
// ---------------------------------------------------------------------------

test("test_reject_once_renders_actionable_deny_button", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-deny", kind: "reject_once" },
      ]),
    }),
  );

  assert.ok(
    html.includes("permission-decision-opt-deny"),
    "reject_once option should render a button with its optionId testid",
  );
  // Deny button uses destructive styling; verify at least the testid is there.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "reject_once card should not render the persistent-grant badge",
  );
});

// ---------------------------------------------------------------------------
// allow_always — not actionable, no badge (F3: persistent-grant badge removed)
// ---------------------------------------------------------------------------

test("test_allow_always_renders_no_button_and_no_badge", () => {
  // After F3: allow_always is NOT in ACTIONABLE_KINDS and the persistent-grant
  // badge has been removed. A card with only allow_always renders nothing
  // actionable — no button and no badge — because the two-option contract
  // (allow_once / reject_once only) is enforced at both the Rust sentinel and
  // the observer surface.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-always", kind: "allow_always", label: "Always allow" },
      ]),
    }),
  );

  // No button for allow_always.
  assert.ok(
    !html.includes("permission-decision-opt-always"),
    "allow_always option must not render an actionable button",
  );
  // No <button> element at all — no actionable options.
  assert.ok(
    !html.includes("<button"),
    "allow_always-only card must not render any button element",
  );
  // The persistent-grant badge is gone — it was the only surface that showed
  // allow_always and it has been removed in F3.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "persistent-grant badge must not render after F3 removal",
  );
  assert.ok(
    !html.includes("Permanent grant"),
    "persistent-grant copy must not render after F3 removal",
  );
});

// ---------------------------------------------------------------------------
// Unknown kind — fail closed: renders nothing actionable, no badge
// ---------------------------------------------------------------------------

test("test_unknown_kind_fails_closed_renders_nothing", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-mystery", kind: "future_unknown_verb" },
      ]),
    }),
  );

  // No button for the unknown kind.
  assert.ok(
    !html.includes("permission-decision-opt-mystery"),
    "unknown kind must not render an actionable button",
  );
  // No persistent-grant badge either.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "unknown kind must not render the persistent-grant badge",
  );
  // No button element at all.
  assert.ok(
    !html.includes("<button"),
    "unknown-kind-only card must not render any button element",
  );
  // The outer permission card shell is still rendered (title row etc.).
  assert.ok(
    html.includes("transcript-permission-item"),
    "unknown kind still renders the permission card shell",
  );
});

// ---------------------------------------------------------------------------
// Unknown reject_*-prefixed kind — fail closed: exact allowlist, not prefix
// ---------------------------------------------------------------------------

test("test_unknown_reject_prefixed_kind_fails_closed_renders_nothing", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-reject-future", kind: "reject_later_v2" },
      ]),
    }),
  );

  // A reject-prefixed but unrecognized kind must NOT render a trusted button:
  // recognition is an exact allowlist, not a prefix match.
  assert.ok(
    !html.includes("permission-decision-opt-reject-future"),
    "unknown reject_*-prefixed kind must not render an actionable button",
  );
  assert.ok(
    !html.includes("<button"),
    "unknown reject_*-prefixed-only card must not render any button element",
  );
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "unknown reject_*-prefixed kind must not render the persistent-grant badge",
  );
  // The outer permission card shell is still rendered.
  assert.ok(
    html.includes("transcript-permission-item"),
    "unknown reject_*-prefixed kind still renders the permission card shell",
  );
});

// ---------------------------------------------------------------------------
// reject_always — not actionable (F3: removed from ACTIONABLE_KINDS)
// ---------------------------------------------------------------------------

test("test_reject_always_renders_no_button", () => {
  // After F3: reject_always is removed from ACTIONABLE_KINDS. The thread card
  // cannot grant permanent denial; the ACP read loop accepts only allow_once and
  // reject_once. A reject_always option must render as inert — no clickable
  // button. The outer card shell is still rendered (the activity still appears
  // in the transcript), but no action can be taken on it.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-reject-always", kind: "reject_always" },
      ]),
    }),
  );

  // Must NOT render a clickable button for reject_always.
  assert.ok(
    !html.includes("permission-decision-opt-reject-always"),
    "reject_always must not render an actionable button after F3",
  );
  // No button element at all — no actionable options present.
  assert.ok(
    !html.includes("<button"),
    "reject_always-only card must not render any button element after F3",
  );
  // No persistent-grant badge either.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "reject_always card must not render the persistent-grant badge",
  );
  // The outer card shell IS rendered.
  assert.ok(
    html.includes("transcript-permission-item"),
    "reject_always still renders the outer permission card shell",
  );
});

// ---------------------------------------------------------------------------
// Mixed options — allow_once + allow_always: only allow_once actionable
// No persistent-grant badge after F3 removal
// ---------------------------------------------------------------------------

test("test_mixed_allow_once_and_allow_always_only_allow_once_actionable", () => {
  // After F3: allow_always is not in ACTIONABLE_KINDS and the persistent-grant
  // badge is removed. A mixed card renders only the allow_once button; allow_always
  // is inert context with no UI surface.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-once", kind: "allow_once" },
        { optionId: "opt-always", kind: "allow_always" },
      ]),
    }),
  );

  // allow_once produces a button.
  assert.ok(
    html.includes("permission-decision-opt-once"),
    "allow_once in mixed card should render a button",
  );
  // allow_always does NOT produce a button.
  assert.ok(
    !html.includes("permission-decision-opt-always"),
    "allow_always in mixed card must not render a button",
  );
  // No persistent-grant badge — it has been removed.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "mixed card must not render the persistent-grant badge after F3 removal",
  );
  assert.ok(
    !html.includes("Permanent grant"),
    "mixed card must not render persistent-grant copy after F3 removal",
  );
});

// ---------------------------------------------------------------------------
// F3 contract: all four adapter option kinds — only allow_once + reject_once
// are actionable; allow_always and reject_always are inert.
// ---------------------------------------------------------------------------

test("test_four_option_contract_only_allow_once_and_reject_once_actionable", () => {
  // The two-option contract: the thread card may only action allow_once and
  // reject_once. This test covers all four recognized adapter option kinds in
  // a single card and verifies the exact set of rendered buttons.
  //
  // Mutation proof: removing "reject_once" from ACTIONABLE_KINDS in
  // LifecycleActivity.tsx makes "permission-decision-opt-deny" absent — the
  // assertion on opt-deny goes red. Removing "allow_once" makes opt-allow
  // absent similarly. The ACP read loop on the Rust side accepts only the
  // two option IDs snapshotted into CardActions (allow_once / reject_once);
  // sending an allow_always or reject_always option ID is silently ignored.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-allow", kind: "allow_once", label: "Allow once" },
        { optionId: "opt-deny", kind: "reject_once", label: "Deny" },
        { optionId: "opt-always", kind: "allow_always", label: "Always allow" },
        {
          optionId: "opt-reject-always",
          kind: "reject_always",
          label: "Always deny",
        },
      ]),
    }),
  );

  // Only allow_once and reject_once render buttons.
  assert.ok(
    html.includes("permission-decision-opt-allow"),
    "allow_once must render an actionable button",
  );
  assert.ok(
    html.includes("permission-decision-opt-deny"),
    "reject_once must render an actionable button",
  );

  // allow_always and reject_always must NOT render buttons.
  assert.ok(
    !html.includes("permission-decision-opt-always"),
    "allow_always must not render a button in a mixed four-option card",
  );
  assert.ok(
    !html.includes("permission-decision-opt-reject-always"),
    "reject_always must not render a button in a mixed four-option card",
  );

  // No persistent-grant badge — removed in F3.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "four-option card must not render the persistent-grant badge",
  );
  // Exactly two <button> elements (allow_once + reject_once).
  const buttonCount = (html.match(/<button/g) ?? []).length;
  assert.equal(
    buttonCount,
    2,
    `four-option card must render exactly 2 buttons (allow_once + reject_once); got ${buttonCount}`,
  );
});

// ---------------------------------------------------------------------------
// F3 cross-layer: acp_read → buildTranscript → LifecycleActivity
//
// Starts with all four adapter option kinds in the request payload.
// Drives the event through the full transcript reducer so the card is built
// from the real processing path, not a hand-rolled fixture.
// Then renders via LifecycleActivity and confirms the two-button contract.
// ---------------------------------------------------------------------------

test("test_f3_cross_layer_four_options_acp_read_to_lifecycle_activity_two_buttons", () => {
  // Build an acp_read event carrying all four adapter option kinds.
  // This is the real wire shape the observer feed emits when the agent
  // requests permission with a full four-option set.
  const acpReadEvent = {
    seq: 1,
    timestamp: "2026-09-01T10:00:00.000Z",
    kind: "acp_read",
    agentIndex: 0,
    channelId: "ch-f3-cross",
    sessionId: "sess-f3-cross",
    turnId: "turn-f3-cross",
    payload: {
      jsonrpc: "2.0",
      id: "req-f3",
      method: "session/request_permission",
      params: {
        title: "Tool requires approval",
        toolCallId: "tc-f3",
        // Four option kinds offered by the adapter.
        options: [
          {
            optionId: "opt-allow-once",
            kind: "allow_once",
            name: "Allow once",
          },
          { optionId: "opt-reject-once", kind: "reject_once", name: "Deny" },
          {
            optionId: "opt-allow-always",
            kind: "allow_always",
            name: "Always allow",
          },
          {
            optionId: "opt-reject-always",
            kind: "reject_always",
            name: "Always deny",
          },
        ],
      },
    },
    // Authorization envelope: marks the card as actionable with a nonce.
    authorization: {
      requestNonce: "nonce-f3-cross",
      actionable: true,
    },
  };

  // 1. Drive through the transcript reducer.
  const transcript = buildTranscript([acpReadEvent]);
  const card = transcript.find((item) => item.renderClass === "permission");
  assert.ok(card, "transcript must contain a permission card");
  assert.equal(
    card.requestNonce,
    "nonce-f3-cross",
    "card must carry the request nonce",
  );
  assert.ok(card.actionable, "card must be actionable");
  assert.ok(Array.isArray(card.options), "card must have options");
  assert.equal(card.options.length, 4, "all four options must be on the card");

  // 2. Render via LifecycleActivity and assert the two-button contract.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: card,
    }),
  );

  // Only allow_once and reject_once render buttons (ACTIONABLE_KINDS contract).
  assert.ok(
    html.includes("permission-decision-opt-allow-once"),
    "allow_once must render a button via cross-layer path",
  );
  assert.ok(
    html.includes("permission-decision-opt-reject-once"),
    "reject_once must render a button via cross-layer path",
  );

  // allow_always and reject_always must NOT render buttons.
  assert.ok(
    !html.includes("permission-decision-opt-allow-always"),
    "allow_always must not render a button via cross-layer path",
  );
  assert.ok(
    !html.includes("permission-decision-opt-reject-always"),
    "reject_always must not render a button via cross-layer path",
  );

  // Exactly two <button> elements.
  const buttonCount = (html.match(/<button/g) ?? []).length;
  assert.equal(
    buttonCount,
    2,
    `cross-layer four-option card must render exactly 2 buttons; got ${buttonCount}`,
  );
});
