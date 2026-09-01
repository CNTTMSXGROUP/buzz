import { useQuery } from "@tanstack/react-query";
import {
  ArrowRight,
  BadgeCheck,
  CircleAlert,
  Radio,
  Store,
} from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { presentMarketEvent } from "@/features/market/lib/marketEventPresentation";
import {
  marketAnnouncementMatchesProjection,
  parseMarketEnvelope,
  projectMarketChannel,
} from "@/features/market/lib/marketProtocol";
import { getChannelWindowEvents } from "@/shared/api/channelWindow";
import { Badge } from "@/shared/ui/badge";

export function MarketEventCard({
  authorPubkey,
  content,
}: {
  authorPubkey: string;
  content: string;
}) {
  const { goChannel } = useAppNavigation();
  const envelope = parseMarketEnvelope(content);
  const announcement = envelope?.type === "announcement" ? envelope : null;
  const verificationQuery = useQuery({
    enabled: announcement !== null,
    queryKey: ["market-announcement-verification", announcement?.channelId],
    queryFn: () =>
      getChannelWindowEvents(announcement?.channelId ?? "", null, 100),
    staleTime: 30_000,
  });
  if (!envelope) return null;

  const presentation = presentMarketEvent(envelope);
  const verification = announcement
    ? verifyAnnouncement(
        announcement,
        authorPubkey,
        verificationQuery.data ?? null,
      )
    : "channel-event";
  const canOpen = announcement !== null && verification === "verified";

  return (
    <section
      className="mt-1.5 max-w-2xl rounded-2xl border bg-card p-4 text-left shadow-xs"
      data-testid={`market-event-${envelope.type}`}
    >
      <div className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-muted text-muted-foreground">
          {announcement ? (
            <Radio className="h-4 w-4" />
          ) : (
            <Store className="h-4 w-4" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Badge className="capitalize" variant="secondary">
              {presentation.eyebrow}
            </Badge>
            {announcement ? <VerificationBadge state={verification} /> : null}
          </div>
          <h3 className="mt-2 font-semibold leading-snug">
            {presentation.title}
          </h3>
          <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
            {presentation.summary}
          </p>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3 text-xs">
            <span className="font-medium text-foreground">
              {presentation.amount ?? "Signed market event"}
            </span>
            {announcement ? (
              <button
                className="inline-flex items-center gap-1 font-medium text-muted-foreground enabled:hover:text-foreground disabled:cursor-not-allowed disabled:opacity-60"
                disabled={!canOpen}
                onClick={() => void goChannel(announcement.channelId)}
                type="button"
              >
                {verification === "checking"
                  ? "Checking channel…"
                  : verification === "verified"
                    ? "Open market channel"
                    : "Channel claim did not verify"}
                {canOpen ? <ArrowRight className="h-3.5 w-3.5" /> : null}
              </button>
            ) : null}
          </div>
        </div>
      </div>
    </section>
  );
}

type VerificationState = "channel-event" | "checking" | "invalid" | "verified";

type Announcement = Extract<
  NonNullable<ReturnType<typeof parseMarketEnvelope>>,
  { type: "announcement" }
>;

function verifyAnnouncement(
  announcement: Announcement,
  authorPubkey: string,
  events: Awaited<ReturnType<typeof getChannelWindowEvents>> | null,
): VerificationState {
  if (!events) return "checking";
  const projection = projectMarketChannel(
    events.map((event) => ({
      id: event.id,
      pubkey: event.pubkey,
      createdAt: event.created_at,
      content: event.content,
      tags: event.tags,
    })),
    announcement.channelId,
  );
  if (!projection) return "invalid";
  return marketAnnouncementMatchesProjection(
    {
      announcementEventId: "",
      channelId: announcement.channelId,
      createdAt: 0,
      listingEventId: announcement.listingEventId,
      listing: announcement.listing,
      publisherPubkey: authorPubkey,
    },
    projection,
  )
    ? "verified"
    : "invalid";
}

function VerificationBadge({ state }: { state: VerificationState }) {
  if (state === "checking")
    return <Badge variant="outline">Checking channel</Badge>;
  if (state === "verified") {
    return (
      <Badge variant="outline">
        <BadgeCheck className="mr-1 h-3 w-3" /> Verified contract
      </Badge>
    );
  }
  return (
    <Badge variant="destructive">
      <CircleAlert className="mr-1 h-3 w-3" /> Unverified claim
    </Badge>
  );
}
