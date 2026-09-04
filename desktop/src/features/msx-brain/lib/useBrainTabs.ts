import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { BrainEntry } from "../types";
import { loadBrainRole } from "./permissions";
import { loadNaoDefs, type NaoDef } from "./naoDefs";

export type BrainTab = {
  relPath: string;
  name: string;
  kind: "md" | "text" | "image" | "other";
  content: string;
  size: number;
};

const TEXT_EXT = new Set([
  "txt", "json", "yaml", "yml", "toml", "csv", "log", "canvas", "base",
  "ts", "tsx", "js", "mjs", "py", "rs", "sh", "html", "css", "svg", "excalidraw",
]);
const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp"]);

export function kindOf(name: string): BrainTab["kind"] {
  const ext = name.toLowerCase().split(".").pop() ?? "";
  if (ext === "md" || ext === "markdown") return "md";
  if (IMAGE_EXT.has(ext)) return "image";
  if (TEXT_EXT.has(ext)) return "text";
  return "other";
}

/** Danh sách não từ config — giữ export cũ cho tương thích. */
export async function loadNaoCon(root: string): Promise<string[]> {
  const defs = await loadNaoDefs(root);
  return defs.map((d) => d.id);
}

export function useBrainTabs(vaultRoot: string, myPubkey: string) {
  const [entries, setEntries] = useState<BrainEntry[]>([]);
  const [tabs, setTabs] = useState<BrainTab[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [naoDefs, setNaoDefs] = useState<NaoDef[]>([]);
  const [naoChon, setNaoChon] = useState<NaoDef | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [role, defs] = await Promise.all([
        loadBrainRole(vaultRoot, myPubkey),
        loadNaoDefs(vaultRoot),
      ]);
      setNaoDefs(defs);
      setNaoChon((prev) => prev ?? defs[0] ?? null);
      const khu = role?.khu ?? "__khong_co_quyen__";
      setEntries(await invoke<BrainEntry[]>("brain_list_tree", { root: vaultRoot, khu }));
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }, [vaultRoot, myPubkey]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const open = useCallback(
    async (relPath: string) => {
      const existing = tabs.find((t) => t.relPath === relPath);
      if (existing) {
        setActivePath(relPath);
        return;
      }
      const role = await loadBrainRole(vaultRoot, myPubkey);
      const khu = role?.khu ?? "__khong_co_quyen__";
      const name = relPath.split("/").pop() ?? relPath;
      const kind = kindOf(name);
      try {
        let content = "";
        let size = 0;
        if (kind === "image") {
          const b64 = await invoke<string>("brain_read_bytes", { root: vaultRoot, relPath, khu });
          const ext = name.toLowerCase().split(".").pop() ?? "png";
          content = `data:image/${ext === "jpg" ? "jpeg" : ext};base64,${b64}`;
        } else if (kind === "md" || kind === "text") {
          content = await invoke<string>("brain_read_file", { root: vaultRoot, relPath, khu });
        }
        try {
          const st = await invoke<{ size: number; is_dir: boolean }>("brain_stat", {
            root: vaultRoot,
            relPath,
            khu,
          });
          size = st.size;
        } catch {
          /* size phụ */
        }
        setTabs((prev) => [...prev, { relPath, name, kind, content, size }]);
        setActivePath(relPath);
        setError(null);
      } catch (err) {
        setError(`Không mở được "${name}": ${String(err)}`);
      }
    },
    [tabs, vaultRoot, myPubkey],
  );

  const addNaoDef = useCallback((id: string, path: string) => {
    setNaoDefs((prev) => (prev.some((d) => d.id === id) ? prev : [...prev, { id, path }]));
    setNaoChon({ id, path });
  }, []);

  const close = useCallback(
    (relPath: string) => {
      setTabs((prev) => {
        const idx = prev.findIndex((t) => t.relPath === relPath);
        const next = prev.filter((t) => t.relPath !== relPath);
        if (activePath === relPath) {
          setActivePath(next.length ? next[Math.max(0, idx - 1)].relPath : null);
        }
        return next;
      });
    },
    [activePath],
  );

  const openByName = useCallback(
    (name: string) => {
      const clean = name.replace(/\.md$/, "").trim().toLowerCase();
      const hit = entries.find((e) => {
        const base = e.name.replace(/\.md$/i, "").toLowerCase();
        return !e.is_dir && (base === clean || base.includes(clean));
      });
      if (hit) void open(hit.rel_path);
      else setError(`Không tìm thấy tài liệu "[[${name}]]" trong não.`);
    },
    [entries, open],
  );

  return {
    entries,
    tabs,
    activePath,
    error,
    open,
    close,
    openByName,
    refresh,
    setActivePath,
    naoCon: defs0(defsSafe()),
    naoDefs,
    naoChon,
    setNaoChon,
    addNaoDef,
  };

  function defsSafe(): NaoDef[] {
    return naoDefs;
  }
  function defs0(d: NaoDef[]): string[] {
    return d.map((x) => x.id);
  }
}
