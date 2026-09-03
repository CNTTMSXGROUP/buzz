import { Brain, ChevronRight, File, Folder, Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { BrainEntry } from "../types";
import { MSX_VAULT_ROOT_DEFAULT } from "../lib/vaultRoot";
import { canReadPath, loadBrainRole } from "../lib/permissions";
import { useMyPubkey } from "../lib/useMyPubkey";

type Props = {
  disabled?: boolean;
  /** Gọi khi chọn file — Composer sẽ chèn token vào tin nhắn. */
  onPick: (entry: BrainEntry) => void;
};

/** Nút 🧠 trong composer chat: chọn file não → chèn token [nao:rel_path] vào tin nhắn. */
export function BrainAttachButton({ disabled, onPick }: Props) {
  const pubkey = useMyPubkey();
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<BrainEntry[] | null>(null);
  const [cwd, setCwd] = useState("");
  const [loading, setLoading] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onDoc(ev: MouseEvent) {
      if (ref.current && !ref.current.contains(ev.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);

  async function loadDir(dir: string) {
    setLoading(true);
    setCwd(dir);
    try {
      const root = getRoot();
      const role = await loadBrainRole(root, pubkey);
      const khu = role?.khu ?? "__khong_co_quyen__";
      const all = await invoke<BrainEntry[]>("brain_list_tree", { root, khu });
      setEntries(all.filter((e) => parentOf(e.rel_path) === dir));
    } finally {
      setLoading(false);
    }
  }

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

  async function toggle() {
    const next = !open;
    setOpen(next);
    if (next && entries === null) await loadDir("");
  }

  function pick(e: BrainEntry) {
    if (e.is_dir) {
      void loadDir(e.rel_path);
      return;
    }
    onPick(e);
    setOpen(false);
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
      {open && (
        <div className="absolute bottom-full left-0 z-50 mb-2 w-80 rounded-lg border bg-popover shadow-lg">
          <div className="border-b px-3 py-2 text-xs font-semibold">
            {cwd === "" ? "Não MSX — chọn tài liệu" : `Não MSX / ${cwd}`}
            {cwd !== "" && (
              <button
                type="button"
                className="float-right text-muted-foreground hover:text-foreground"
                onClick={() => void loadDir(parentOf(cwd))}
              >
                …quay lại
              </button>
            )}
          </div>
          <div className="max-h-72 overflow-y-auto p-1">
            {entries?.length ? (
              entries.map((e) => (
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
                  <span className="truncate">{e.name}</span>
                  {e.is_dir && <ChevronRight className="ml-auto h-3 w-3 text-muted-foreground" />}
                </button>
              ))
            ) : (
              <div className="px-3 py-4 text-sm text-muted-foreground">Thư mục trống</div>
            )}
          </div>
          <div className="border-t px-3 py-1.5 text-[10px] text-muted-foreground">
            Chọn file .md để đính kèm token xem trong panel Não MSX
          </div>
        </div>
      )}
    </div>
  );
}

export function brainTokenFor(entry: BrainEntry): string {
  void canReadPath; // giữ import để tree-shake ổn định
  return `[nao:${entry.rel_path}]`;
}
