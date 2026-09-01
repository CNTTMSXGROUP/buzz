import * as React from "react";

import type { MarketProjection } from "@/features/market/lib/marketProtocol";

const MarketChannelContext = React.createContext<MarketProjection | null>(null);

export function MarketChannelProvider({
  children,
  projection,
}: {
  children: React.ReactNode;
  projection: MarketProjection;
}) {
  return (
    <MarketChannelContext.Provider value={projection}>
      {children}
    </MarketChannelContext.Provider>
  );
}

export function useMarketChannel(): MarketProjection | null {
  return React.useContext(MarketChannelContext);
}
