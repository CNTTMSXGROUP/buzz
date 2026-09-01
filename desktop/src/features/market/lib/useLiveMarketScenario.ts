import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import { projectMarketNotes } from "@/features/market/lib/marketProtocol";
import type { MarketScenario } from "@/features/market/lib/marketPrototypeData";
import { getGlobalNotes } from "@/shared/api/social";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

const MARKET_REFETCH_INTERVAL_MS = 5_000;

export function useLiveMarketScenario(
  fallback: MarketScenario,
  marketId?: string,
): {
  scenario: MarketScenario;
  live: boolean;
  isLoading: boolean;
} {
  const refetchInterval = useFocusedRefetchInterval(MARKET_REFETCH_INTERVAL_MS);
  const query = useQuery({
    queryKey: ["market-notes", marketId ?? "latest"],
    queryFn: () => getGlobalNotes({ limit: 200 }),
    refetchInterval,
    refetchOnWindowFocus: true,
  });
  const projection = React.useMemo(
    () => projectMarketNotes(query.data?.notes ?? [], marketId),
    [marketId, query.data?.notes],
  );
  return {
    scenario: projection?.scenario ?? fallback,
    live: projection !== null,
    isLoading: query.isLoading,
  };
}
