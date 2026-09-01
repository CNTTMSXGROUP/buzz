import type * as React from "react";

import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";
import { MarketContractCard } from "@/features/market/ui/MarketContractCard";

export function MarketChannelIntro(): React.ReactNode {
  const projection = useMarketChannel();
  return projection ? (
    <MarketContractCard scenario={projection.scenario} />
  ) : undefined;
}
