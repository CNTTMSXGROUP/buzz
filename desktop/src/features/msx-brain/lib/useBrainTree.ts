import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { BrainEntry } from "../types";
import { loadBrainRole } from "./permissions";

export function useBrainTree(vaultRoot: string, myPubkey: string) {
  const [entries, setEntries] = useState<BrainEntry[]>([]);
  const [selected, setSelected] = useState<BrainEntry | null>(null);
  const [content, setContent] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const role = await loadBrainRole(vaultRoot, myPubkey);
      const khu = role?.khu ?? "__khong_co_quyen__";
      const list = await invoke<BrainEntry[]>("brain_list_tree", { root: vaultRoot, khu });
      setEntries(list);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }, [vaultRoot, myPubkey]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const open = useCallback(
    async (e: BrainEntry) => {
      if (e.is_dir) return;
      setSelected(e);
      try {
        const role = await loadBrainRole(vaultRoot, myPubkey);
        const khu = role?.khu ?? "__khong_co_quyen__";
        const txt = await invoke<string>("brain_read_file", {
          root: vaultRoot,
          rel_path: e.rel_path,
          khu,
        });
        setContent(txt);
        setError(null);
      } catch (err) {
        setError(String(err));
      }
    },
    [vaultRoot, myPubkey],
  );

  return { entries, selected, content, error, open, refresh };
}
