import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { MSX_VAULT_ROOT_DEFAULT } from "@/features/msx-brain/lib/vaultRoot";
import { useMyPubkey } from "@/features/msx-brain/lib/useMyPubkey";

const BrainPanel = React.lazy(async () => {
  const module = await import("@/features/msx-brain/ui/BrainPanel");
  return { default: module.BrainPanel };
});

export const Route = createFileRoute("/msx-brain")({
  component: MsxBrainRouteComponent,
});

function MsxBrainRouteComponent() {
  const pubkey = useMyPubkey();
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <BrainPanel vaultRoot={MSX_VAULT_ROOT_DEFAULT} myPubkey={pubkey} />
    </React.Suspense>
  );
}
