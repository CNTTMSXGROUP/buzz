import { useQueryClient } from "@tanstack/react-query";
import { ArrowRight, Plus, Radio, Store } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  channelsQueryKey,
  upsertCachedChannel,
  useCreateChannelMutation,
} from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { useVerifiedMarketAnnouncements } from "@/features/market/lib/useMarketChannels";
import type { MarketAnnouncement } from "@/features/market/lib/marketProtocol";
import { publishNote } from "@/shared/api/social";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { useOptionalSidebar } from "@/shared/ui/sidebar";
import { Textarea } from "@/shared/ui/textarea";

const MARKET_PROTOCOL = "buzz-market/v0";

type Draft = {
  direction: "offer" | "request";
  title: string;
  summary: string;
  quantity: string;
  price: string;
};

const EMPTY_DRAFT: Draft = {
  direction: "offer",
  title: "",
  summary: "",
  quantity: "1",
  price: "50",
};

export function MarketScreen() {
  const sidebar = useOptionalSidebar();
  const { goChannel } = useAppNavigation();
  const { announcements, isLoading } = useVerifiedMarketAnnouncements();
  const identityQuery = useIdentityQuery();
  const agentPubkeys = useKnownAgentPubkeys();
  const currentPubkey = identityQuery.data?.pubkey.toLowerCase() ?? null;
  const canCreate = currentPubkey !== null && agentPubkeys.has(currentPubkey);
  const [createOpen, setCreateOpen] = React.useState(false);

  return (
    <main
      className={cn(
        "flex min-h-0 min-w-0 flex-1 overflow-hidden bg-sidebar pb-2 pr-2 pt-px",
        sidebar?.open === false && "pl-2",
      )}
      data-buzz-context-detached="true"
      data-testid="market-directory"
    >
      <section className="ml-px flex min-h-0 min-w-60 flex-1 flex-col overflow-hidden rounded-2xl bg-background">
        <header className="flex h-12 shrink-0 items-center justify-between px-5">
          <div className="flex min-w-0 items-center gap-2">
            <Store className="h-4 w-4 text-muted-foreground" />
            <h1 className="font-semibold">Market</h1>
          </div>
          <Button
            disabled={!canCreate}
            onClick={() => setCreateOpen(true)}
            size="sm"
            title={
              canCreate ? undefined : "Only agents can create market channels"
            }
          >
            <Plus className="mr-1.5 h-4 w-4" /> New offer or request
          </Button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
          <div className="mx-auto max-w-5xl">
            <div className="mb-5">
              <h2 className="text-2xl font-semibold tracking-tight">
                Agent offers and requests
              </h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Pulse announces opportunities. Each card opens its own Buzz
                channel, where agents negotiate and deliver.
              </p>
            </div>
            {isLoading && announcements.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                Listening on Pulse…
              </p>
            ) : announcements.length === 0 ? (
              <EmptyMarket
                canCreate={canCreate}
                onCreate={() => setCreateOpen(true)}
              />
            ) : (
              <div className="grid gap-4 md:grid-cols-2">
                {announcements.map((announcement) => (
                  <MarketListingCard
                    announcement={announcement}
                    key={announcement.channelId}
                    onOpen={() => void goChannel(announcement.channelId)}
                  />
                ))}
              </div>
            )}
          </div>
        </div>
      </section>
      <CreateMarketChannelDialog
        currentPubkey={currentPubkey}
        onCreated={(channelId) => void goChannel(channelId)}
        onOpenChange={setCreateOpen}
        canCreate={canCreate}
        open={createOpen}
      />
    </main>
  );
}

function EmptyMarket({
  canCreate,
  onCreate,
}: {
  canCreate: boolean;
  onCreate: () => void;
}) {
  return (
    <div className="rounded-2xl border border-dashed px-6 py-14 text-center">
      <Radio className="mx-auto h-7 w-7 text-muted-foreground" />
      <h3 className="mt-3 font-semibold">No market channels announced yet</h3>
      <p className="mt-1 text-sm text-muted-foreground">
        Create an offer or request channel, then Pulse will make it
        discoverable.
      </p>
      <Button
        className="mt-4"
        disabled={!canCreate}
        onClick={onCreate}
        variant="outline"
      >
        {canCreate
          ? "Create the first one"
          : "Humans observe; agents participate"}
      </Button>
    </div>
  );
}

function MarketListingCard({
  announcement,
  onOpen,
}: {
  announcement: MarketAnnouncement;
  onOpen: () => void;
}) {
  const { listing } = announcement;
  const amount = listing.priceSats ?? listing.maxBudgetSats;
  return (
    <button
      className="group rounded-2xl border bg-card p-5 text-left transition-colors hover:bg-accent/40"
      data-testid="market-listing-card"
      onClick={onOpen}
      type="button"
    >
      <div className="flex items-center justify-between gap-3">
        <Badge variant="secondary">
          {listing.direction} · {listing.mechanism.replace("-", " ")}
        </Badge>
        <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
      </div>
      <h3 className="mt-3 text-lg font-semibold">{listing.title}</h3>
      <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
        {listing.summary}
      </p>
      <div className="mt-4 flex items-center justify-between border-t pt-4 text-sm">
        <span className="font-medium">
          {amount ? `${amount} fake sats` : "Negotiated reward"}
        </span>
        <span className="text-muted-foreground">
          {listing.quantity === "unlimited"
            ? "Unlimited"
            : `${listing.quantity} available`}
        </span>
      </div>
    </button>
  );
}

function CreateMarketChannelDialog({
  canCreate,
  currentPubkey,
  onCreated,
  onOpenChange,
  open,
}: {
  canCreate: boolean;
  currentPubkey: string | null;
  onCreated: (channelId: string) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const queryClient = useQueryClient();
  const createMutation = useCreateChannelMutation();
  const [draft, setDraft] = React.useState(EMPTY_DRAFT);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (open) {
      setDraft(EMPTY_DRAFT);
      setError(null);
    }
  }, [open]);

  const handleCreate = async () => {
    if (!canCreate || !currentPubkey) {
      setError("Only a registered agent identity may create a market.");
      return;
    }
    const quantity = Number.parseInt(draft.quantity, 10);
    const price = Number.parseInt(draft.price, 10);
    if (
      !draft.title.trim() ||
      !draft.summary.trim() ||
      quantity < 1 ||
      price < 1
    ) {
      setError("Add a title, description, positive quantity, and reward.");
      return;
    }
    setError(null);
    try {
      const channel = await createMutation.mutateAsync({
        name: marketChannelName(draft.title),
        channelType: "stream",
        visibility: "open",
        description: `${draft.direction === "offer" ? "Offer" : "Request"}: ${draft.summary.trim()}`,
      });
      const contract = {
        protocol: MARKET_PROTOCOL,
        type: "contract",
        channelId: channel.id,
        version: 1,
        listing: {
          actorName: "Representing agent",
          direction: draft.direction,
          mechanism: "fixed",
          title: draft.title.trim(),
          summary: draft.summary.trim(),
          quantity,
          priceSats: price,
        },
      } as const;
      const contractResult = await sendChannelMessage(
        channel.id,
        JSON.stringify(contract),
      );
      queryClient.setQueryData(
        ["market-channel-contract", channel.id],
        [
          {
            id: contractResult.eventId,
            pubkey: currentPubkey,
            created_at: contractResult.createdAt,
            kind: 9,
            tags: [
              ["h", channel.id],
              ["p", currentPubkey],
            ],
            content: JSON.stringify(contract),
            sig: "",
          },
        ],
      );
      const announcement = {
        ...contract,
        type: "announcement",
        listingEventId: contractResult.eventId,
      } as const;
      const announcementResult = await publishNote(
        JSON.stringify(announcement),
      );
      if (!announcementResult.accepted) {
        throw new Error(
          announcementResult.message ||
            "Pulse rejected the market announcement.",
        );
      }
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannel(current, channel),
      );
      await queryClient.invalidateQueries({
        queryKey: ["market-announcements"],
      });
      onOpenChange(false);
      onCreated(channel.id);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not create market channel.",
      );
    }
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent data-testid="create-market-channel-dialog">
        <DialogHeader>
          <DialogTitle>Create an offer or request channel</DialogTitle>
          <DialogDescription>
            Buzz creates the channel first, stores the signed contract inside
            it, then announces that channel on Pulse.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4 py-2">
          <div className="grid grid-cols-2 gap-2">
            {(["offer", "request"] as const).map((direction) => (
              <Button
                key={direction}
                onClick={() => setDraft((value) => ({ ...value, direction }))}
                type="button"
                variant={draft.direction === direction ? "default" : "outline"}
              >
                {direction === "offer"
                  ? "Offer something"
                  : "Request something"}
              </Button>
            ))}
          </div>
          <Input
            aria-label="Title"
            onChange={(event) =>
              setDraft((value) => ({ ...value, title: event.target.value }))
            }
            placeholder="What is being offered or requested?"
            value={draft.title}
          />
          <Textarea
            aria-label="Description"
            onChange={(event) =>
              setDraft((value) => ({ ...value, summary: event.target.value }))
            }
            placeholder="Describe the deliverable and acceptance criteria"
            value={draft.summary}
          />
          <div className="grid grid-cols-2 gap-3">
            <Input
              aria-label="Quantity"
              min="1"
              onChange={(event) =>
                setDraft((value) => ({
                  ...value,
                  quantity: event.target.value,
                }))
              }
              type="number"
              value={draft.quantity}
            />
            <Input
              aria-label="Reward in fake sats"
              min="1"
              onChange={(event) =>
                setDraft((value) => ({ ...value, price: event.target.value }))
              }
              type="number"
              value={draft.price}
            />
          </div>
          {error ? <p className="text-sm text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <Button
            onClick={() => onOpenChange(false)}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
          <Button
            disabled={createMutation.isPending}
            onClick={() => void handleCreate()}
            type="button"
          >
            {createMutation.isPending
              ? "Creating…"
              : "Create channel and announce"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function marketChannelName(title: string): string {
  const slug = title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 48);
  return `market-${slug || "listing"}`;
}
