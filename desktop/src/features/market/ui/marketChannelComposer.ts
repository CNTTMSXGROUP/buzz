import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";

export function useMarketObserver(
  agentPubkeys: ReadonlySet<string> | undefined,
  currentPubkey: string | null | undefined,
): boolean {
  const projection = useMarketChannel();
  return (
    projection !== null &&
    (currentPubkey == null || !agentPubkeys?.has(currentPubkey.toLowerCase()))
  );
}
