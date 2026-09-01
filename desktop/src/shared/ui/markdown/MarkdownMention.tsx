import type * as React from "react";

import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";
import { InlineChip } from "@/shared/ui/InlineChip";

import { useMarkdownRuntime } from "./runtimeContext";

/**
 * A rendered `@Name` chip.
 *
 * The chip drops the `@` for display, so the identity it resolved is also
 * published as inert `data-*` attributes: the timeline copy handler reads them
 * to restore the sigil and to carry the exact pubkey into the clipboard's HTML
 * flavor. They change neither the visuals nor the screen-reader output.
 */
export function MarkdownMention({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const { agentMentionPubkeysByName, mentionPubkeysByName } =
    useMarkdownRuntime();
  const mentionText = String(children ?? "");
  const mentionName = mentionText.replace(/^@/, "").trim().toLowerCase();
  const pubkey = mentionPubkeysByName?.[mentionName];
  const isAgentMention =
    pubkey !== undefined && agentMentionPubkeysByName?.[mentionName] === pubkey;
  const mentionLabel = mentionText.replace(/^@/, "");
  // Only chips that actually open a profile get the clickable affordance.
  // A mention whose pubkey didn't resolve stays a plain chip — a pointer
  // cursor there promises a click that does nothing.
  const opensProfile = interactive && pubkey !== undefined;
  const mentionNode = (
    <InlineChip
      data-mention=""
      data-mention-kind={
        pubkey === undefined ? undefined : isAgentMention ? "agent" : "human"
      }
      data-mention-label={mentionLabel}
      data-mention-pubkey={pubkey}
      className={cn(isAgentMention && "agent-mention-highlight")}
      icon={isAgentMention ? "agent" : "human"}
      interactive={opensProfile}
    >
      {mentionLabel}
    </InlineChip>
  );

  return opensProfile ? (
    <UserProfilePopover
      botIdenticonValue={mentionLabel}
      pubkey={pubkey}
      role={isAgentMention ? "bot" : undefined}
      triggerElement="span"
    >
      {mentionNode}
    </UserProfilePopover>
  ) : (
    mentionNode
  );
}
