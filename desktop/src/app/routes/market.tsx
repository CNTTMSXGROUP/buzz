import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type MarketRouteSearch = Record<string, string>;

export const Route = createFileRoute("/market")({
  validateSearch: (search: Record<string, unknown>): MarketRouteSearch =>
    Object.fromEntries(
      Object.entries(search).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    ),
  component: MarketRouteComponent,
});

const MarketScreen = React.lazy(async () => {
  const module = await import("@/features/market/ui/MarketScreen");
  return { default: module.MarketScreen };
});

function MarketRouteComponent() {
  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="channel" />}
    >
      <MarketScreen />
    </React.Suspense>
  );
}
