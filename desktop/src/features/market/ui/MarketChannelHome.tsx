import * as React from "react";
import type { ComponentProps } from "react";
import { WalletCards } from "lucide-react";

import { AppTopChromePortal } from "@/app/AppTopChromePortal";

import { ChannelScreen } from "@/features/channels/ui/ChannelScreen";
import { MarketChannelProvider } from "@/features/market/lib/MarketChannelContext";
import { useMarketChannelProjection } from "@/features/market/lib/useMarketChannels";
import type { MarketAnnouncement } from "@/features/market/lib/marketProtocol";
import { AgentWalletPanel } from "@/features/market/ui/AgentWalletPanel";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { useOptionalSidebar } from "@/shared/ui/sidebar";

export function MarketChannelHome({
  activeChannel,
  announcement,
  ...screenProps
}: ComponentProps<typeof ChannelScreen> & {
  activeChannel: Channel;
  announcement: MarketAnnouncement;
}) {
  const projection = useMarketChannelProjection(activeChannel, announcement);
  const sidebar = useOptionalSidebar();
  const [walletOpen, setWalletOpen] = React.useState(false);
  if (!projection)
    return <ChannelScreen activeChannel={activeChannel} {...screenProps} />;

  const walletToggle = (
    <AppTopChromePortal>
      <div
        className="ml-auto flex shrink-0 items-center"
        data-tauri-drag-region
      >
        <Button
          aria-label={walletOpen ? "Hide agent wallet" : "Show agent wallet"}
          aria-pressed={walletOpen}
          className="h-7 w-7 rounded text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
          data-testid="market-wallet-toggle"
          onClick={() => setWalletOpen((open) => !open)}
          size="icon"
          title={walletOpen ? "Hide agent wallet" : "Show agent wallet"}
          type="button"
          variant="ghost"
        >
          <WalletCards data-testid="market-wallet-toggle-icon" />
        </Button>
      </div>
    </AppTopChromePortal>
  );

  return (
    <MarketChannelProvider projection={projection}>
      <div
        className={cn(
          "relative flex min-h-0 min-w-0 flex-1 overflow-hidden bg-sidebar pb-2 pr-2 pt-px",
          sidebar?.open === false && "pl-2",
        )}
        data-buzz-context-detached="true"
        data-testid="market-channel-home"
      >
        {walletToggle}
        <div className="ml-px flex min-h-0 min-w-60 flex-1 flex-col overflow-hidden rounded-2xl bg-background">
          <ChannelScreen {...screenProps} activeChannel={activeChannel} />
        </div>
        <AgentWalletPanel open={walletOpen} projection={projection} />
      </div>
    </MarketChannelProvider>
  );
}
