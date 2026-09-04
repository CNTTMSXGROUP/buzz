import { Brain, ChevronRight, File, Folder, Loader2, Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { BrainEntry } from "../types";
import { MSX_VAULT_ROOT_DEFAULT } from "../lib/vaultRoot";
import { loadBrainRole } from "../lib/permissions";
import { useMyPubkey } from "../lib/useMyPubkey";

type Props = {
  disabled?: boolean;
  /** Gọi khi chọn file — Composer chèn NaoFile node gọn. */
  onPick: (entry: BrainEntry) => void;
};

const MENU_W = 320;
const MARGIN = 8;

function getRoot(): string {
  try {
    return localStorage.getItem("msx-brain-vault-root") ?? MSX_VAULT_ROOT_DEFAULT;
  } catch {
    return MSX_VAULT_ROOT_DEFAULT;
  }
}

function parentOf(rel: string): string {
  return rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "";
}

/** Picker chọn tài liệu não trong composer: search + breadcrumb + điều hướng thư mục. */
export function BrainAttachButton({ disabled, onPick }: Props) {
  const pubkey = useMyPubkey();
  const [open, setOpen] = useState(false);
  const [all, setAll] = useState<BrainEntry[] | null>(null);
  const [cwd, setCwd] = useState("");
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [menuPos, setMenuPos] = useState<{ left: number; bottom: number } | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    function onDoc(ev: MouseEvent) {
      if (ref.current && !ref.current.contains(ev.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);

  async function ensureLoaded() {
    if (all !== null) return;
    setLoading(true);
    try {
      const root = getRoot();
      const role = await loadBrainRole(root, pubkey);
      const khu = role?.khu ?? "__khong_co_quyen__";
      setAll(await invoke<BrainEntry[]>("brain_list_tree", { root, khu }));
    } finally {
      setLoading(false);
    }
  }

  async function toggle() {
    const next = !open;
    setOpen(next);
    if (next) {
      const rect = ref.current?.getBoundingClientRect();
      if (rect) {
        const left = Math.max(MARGIN, Math.min(rect.left, window.innerWidth - MENU_W - MARGIN));
        setMenuPos({ left, bottom: window.innerHeight - rect.top + 6 });
      }
      setQuery("");
      setCwd("");
      await ensureLoaded();
      setTimeout(() => searchRef.current?.focus(), 50);
    }
  }

  const crumbs = cwd ? cwd.split("/") : [];

  const shown = useMemo(() => {
    if (!all) return [];
    const q = query.trim().toLowerCase();
    if (q) {
      // search toàn não theo tên file
      return all
        .filter((e) => !e.is_dir && e.name.toLowerCase().includes(q))
        .slice(0, 30);
    }
    return all.filter((e) => parentOf(e.rel_path) === cwd);
  }, [all, cwd, query]);

  function pick(e: BrainEntry) {
    if (e.is_dir) {
      setCwd(e.rel_path);
      setQuery("");
      return;
    }
    onPick(e);
    setOpen(false);
  }

  function goCrumb(i: number) {
    setCwd(i < 0 ? "" : crumbs.slice(0, i + 1).join("/"));
    setQuery("");
  }

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        aria-label="Đính kèm tài liệu từ Não MSX"
        disabled={disabled}
        className="inline-flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:bg-accent disabled:opacity-50"
        onClick={() => void toggle()}
      >
        {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Brain className="h-4 w-4" />}
      </button>
      {open && menuPos && (
        <div
          className="fixed z-[100] flex max-h-[60vh] flex-col overflow-hidden rounded-xl border bg-popover shadow-xl"
          style={{ left: menuPos.left, bottom: menuPos.bottom, width: MENU_W }}
        >
          <div className="border-b px-3 pb-2 pt-2.5">
            <div className="relative mb-1.5">
              <Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
              <input
                ref={searchRef}
                value={query}
                onChange={(ev) => setQuery(ev.target.value)}
                placeholder="Tìm tài liệu…"
                className="w-full rounded-md border bg-transparent py-1.5 pl-8 pr-7 text-sm outline-none focus:ring-1 focus:ring-ring"
              />
              {query && (
                <button
                  type="button"
                  aria-label="Xoá tìm kiếm"
                  className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 hover:bg-accent"
                  onClick={() => setQuery("")}
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
            <div className="flex items-center gap-1 overflow-x-auto text-xs text-muted-foreground">
              <button
                type="button"
                className={`shrink-0 rounded px-1.5 py-0.5 hover:bg-accent ${cwd === "" && !query ? "font-medium text-foreground" : ""}`}
                onClick={() => goCrumb(-1)}
              >
                Não MSX
              </button>
              {crumbs.map((c, i) => (
                <span key={`${i}-${c}`} className="flex shrink-0 items-center gap-1">
                  <span>/</span>
                  <button
                    type="button"
                    className={`rounded px-1.5 py-0.5 hover:bg-accent ${i === crumbs.length - 1 ? "font-medium text-foreground" : ""}`}
                    onClick={() => goCrumb(i)}
                  >
                    {c}
                  </button>
                </span>
              ))}
              {query && <span className="shrink-0 italic">· tìm "{query}"</span>}
            </div>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto p-1">
            {shown.length ? (
              shown.map((e) => (
                <button
                  key={e.rel_path}
                  type="button"
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent"
                  onClick={() => pick(e)}
                >
                  {e.is_dir ? (
                    <Folder className="h-4 w-4 shrink-0 text-amber-500" />
                  ) : (
                    <File className="h-4 w-4 shrink-0 text-sky-500" />
                  )}
                  <span className="min-w-0 flex-1 truncate" title={e.rel_path}>
                    {e.name}
                  </span>
                  {e.is_dir ? (
                    <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  ) : (
                    query && (
                      <span className="shrink-0 truncate text-[11px] text-muted-foreground">
                        {parentOf(e.rel_path)}
                      </span>
                    )
                  )}
                </button>
              ))
            ) : (
              <div className="px-3 py-6 text-center text-sm text-muted-foreground">
                {query ? `Không thấy "${query}"` : "Thư mục trống"}
              </div>
            )}
          </div>
          <div className="border-t px-3 py-1.5 text-[11px] text-muted-foreground">
            Chọn file → chèn chip gọn vào tin nhắn
          </div>
        </div>
      )}
    </div>
  );
}
