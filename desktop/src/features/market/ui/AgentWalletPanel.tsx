import { CircleCheck, Radio, WalletCards, Zap } from "lucide-react";
import type * as React from "react";

import type { MarketScenarioId } from "@/features/market/lib/marketPrototypeData";
import { ProjectContextRail } from "@/features/projects/ui/ProjectContextRail";
import { ProjectHomeColumn } from "@/features/projects/ui/ProjectHomeColumn";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { SIDEBAR_WIDTH_MIN } from "@/shared/layout/sidebarLayout";

const MARKET_WALLET_WIDTH_KEY = "buzz.desktop.market-wallet-width";

const WALLET_DETAILS: Record<
  MarketScenarioId,
  {
    account: string;
    balance: string;
    balanceAmount: string;
    settlement: string;
  }
> = {
  finite: {
    account: "escrow1report…13c8",
    balance: "Funded for 3 reports",
    balanceAmount: "150",
    settlement: "1 paid · 2 reserved",
  },
  unlimited: {
    account: "wallet1mapper…c241",
    balance: "Available for settlement",
    balanceAmount: "80",
    settlement: "760 sats paid",
  },
  auction: {
    account: "escrow1strings…9f10",
    balance: "Auction reserve funded",
    balanceAmount: "600",
    settlement: "Held until award",
  },
  tender: {
    account: "escrow1tender…1b73",
    balance: "Tender reward funded",
    balanceAmount: "2,000",
    settlement: "Held during selection",
  },
  awarded: {
    account: "escrow1tender…1b73",
    balance: "Award held in escrow",
    balanceAmount: "1,750",
    settlement: "Release on signed receipt",
  },
};

export function AgentWalletPanel({
  open,
  scenarioId,
}: {
  open: boolean;
  scenarioId: MarketScenarioId;
}) {
  const width = useThreadPanelWidth(undefined, {
    minWidthPx: SIDEBAR_WIDTH_MIN,
    sessionKey: MARKET_WALLET_WIDTH_KEY,
  });
  const wallet = WALLET_DETAILS[scenarioId];

  return (
    <ProjectContextRail
      open={open}
      panelWidthPx={width.widthPx}
      resizing={width.isResizing}
      testId="market-wallet-rail"
    >
      <ProjectHomeColumn
        bodyClassName="overflow-hidden"
        canResetWidth={width.canReset}
        onResetWidth={width.onResetWidth}
        onResizeStart={width.onResizeStart}
        testId="market-agent-wallet"
        widthPx={width.widthPx}
      >
        <div className="flex h-full min-h-0 flex-col bg-sidebar text-sidebar-foreground">
          <header className="flex h-14 shrink-0 items-center gap-2 px-5">
            <WalletCards className="h-4 w-4 text-sidebar-foreground/65" />
            <h2 className="text-sm font-semibold">Agent wallet</h2>
            <span className="ml-auto inline-flex items-center gap-1 text-2xs text-sidebar-foreground/60">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              Ready
            </span>
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-6">
            <section className="relative overflow-hidden rounded-xl border border-sidebar-border/70 bg-sidebar-accent/60 p-4">
              <div
                aria-hidden="true"
                className="absolute -right-6 -top-8 h-24 w-24 rounded-full border-[14px] border-sidebar-foreground/[0.035]"
              />
              <div className="relative flex items-center gap-2 text-xs text-sidebar-foreground/60">
                <Zap className="h-3.5 w-3.5" /> Sandbox balance
              </div>
              <p className="relative mt-3 flex items-baseline gap-1.5">
                <span className="text-2xl font-semibold tracking-tight">
                  {wallet.balanceAmount}
                </span>
                <span className="text-xs text-sidebar-foreground/60">sats</span>
              </p>
              <p className="relative mt-1 text-xs text-sidebar-foreground/60">
                {wallet.balance}
              </p>
            </section>

            <section className="mt-3 rounded-xl border border-sidebar-border/70 p-4">
              <div className="flex items-center gap-2">
                <CircleCheck className="h-4 w-4 text-emerald-600 dark:text-emerald-400" />
                <p className="text-sm font-medium">Settlement protected</p>
              </div>
              <p className="mt-2 text-xs leading-5 text-sidebar-foreground/60">
                {wallet.settlement}
              </p>
            </section>

            <dl className="mt-5 space-y-4 px-1">
              <WalletValue
                icon={Radio}
                label="Network"
                value="Sandbox Lightning"
              />
              <WalletValue
                icon={WalletCards}
                label="Wallet account"
                mono
                value={wallet.account}
              />
            </dl>
          </div>
        </div>
      </ProjectHomeColumn>
    </ProjectContextRail>
  );
}

function WalletValue({
  icon: Icon,
  label,
  mono = false,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  mono?: boolean;
  value: string;
}) {
  return (
    <div className="flex items-start gap-3">
      <Icon className="mt-0.5 h-4 w-4 shrink-0 text-sidebar-foreground/50" />
      <div className="min-w-0">
        <dt className="text-xs text-sidebar-foreground/60">{label}</dt>
        <dd className={mono ? "mt-1 font-mono text-xs" : "mt-1 text-sm"}>
          {value}
        </dd>
      </div>
    </div>
  );
}
