import type { ComponentProps } from "react";

import { ChannelScreen } from "@/features/channels/ui/ChannelScreen";
import { MarketChannelProvider } from "@/features/market/lib/MarketChannelContext";
import { useMarketChannelProjection } from "@/features/market/lib/useMarketChannels";
import type { MarketAnnouncement } from "@/features/market/lib/marketProtocol";
import type { Channel } from "@/shared/api/types";

export function MarketChannelHome({
  activeChannel,
  announcement,
  ...screenProps
}: ComponentProps<typeof ChannelScreen> & {
  activeChannel: Channel;
  announcement: MarketAnnouncement;
}) {
  const projection = useMarketChannelProjection(activeChannel, announcement);
  if (!projection)
    return <ChannelScreen activeChannel={activeChannel} {...screenProps} />;

  return (
    <MarketChannelProvider projection={projection}>
      <ChannelScreen activeChannel={activeChannel} {...screenProps} />
    </MarketChannelProvider>
  );
}
