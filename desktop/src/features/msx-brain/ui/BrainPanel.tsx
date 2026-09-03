import { Brain, Check, ChevronDown, FolderOpen, Send, Settings } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { sendApprove } from "../lib/approve";
import { listShareableChannels, shareToChannel } from "../lib/shareToChannel";
import { useBrainTree } from "../lib/useBrainTree";
import { BrainAdmin } from "./BrainAdmin";
import { FileTree } from "./FileTree";
import { MarkdownPreview } from "./MarkdownPreview";

export function BrainPanel({
  vaultRoot,
  myPubkey,
}: {
  vaultRoot: string;
  myPubkey: string;
}) {
  const { entries, selected, content, error, open } = useBrainTree(vaultRoot, myPubkey);
  const [status, setStatus] = useState<string | null>(null);
  const [shareOpen, setShareOpen] = useState(false);
  const [channels, setChannels] = useState<Array<{ id: string; name: string }>>([]);
  const [shareNote, setShareNote] = useState("");
  const [label, setLabel] = useState<string>(() => {
    try {
      return localStorage.getItem("msx-brain-label") ?? "Não MSX";
    } catch {
      return "Não MSX";
    }
  });
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [adminOpen, setAdminOpen] = useState(false);
  const shareRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onDocClick(ev: MouseEvent) {
      if (shareRef.current && !shareRef.current.contains(ev.target as Node)) setShareOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, []);

  // mở file từ hyperlink [nao:...] trong tin nhắn
  useEffect(() => {
    async function onOpenFile(ev: Event) {
      const rel = (ev as CustomEvent<{ rel: string }>).detail?.rel;
      if (!rel) return;
      setAdminOpen(false);
      await open({ name: rel.split("/").pop() ?? rel, rel_path: rel, is_dir: false, area: rel.split("/")[0] ?? "" });
    }
    window.addEventListener("msx-brain-open-file", onOpenFile);
    return () => window.removeEventListener("msx-brain-open-file", onOpenFile);
  }, [open]);

  const isInThuThap = selected?.rel_path.startsWith("1. Thu Thập") ?? false;
  const isMd = selected?.name.toLowerCase().endsWith(".md") ?? false;

  async function handleApprove() {
    if (!selected) return;
    try {
      await sendApprove(selected.name);
      setStatus("Đã gửi lệnh duyệt — bridge xử lý trong ~1 phút.");
    } catch (err) {
      setStatus(`Lỗi: ${String(err)}`);
    }
  }

  async function openShareMenu() {
    setShareOpen((v) => !v);
    if (!shareOpen) {
      try {
        setChannels(await listShareableChannels());
      } catch (err) {
        setStatus(`Lỗi tải kênh: ${String(err)}`);
      }
    }
  }

  async function handleShare(channelId: string, channelName: string) {
    if (!selected) return;
    setShareOpen(false);
    try {
      await shareToChannel(channelId, selected.name, content);
      setStatus(`Đã gửi "${selected.name}" vào #${channelName}.`);
    } catch (err) {
      setStatus(`Lỗi: ${String(err)}`);
    }
  }

  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <Brain className="h-4 w-4 text-amber-500" />
        {renaming ? (
          <form
            onSubmit={(ev) => {
              ev.preventDefault();
              const v = renameValue.trim();
              if (v) {
                setLabel(v);
                try { localStorage.setItem("msx-brain-label", v); } catch { /* ignore */ }
              }
              setRenaming(false);
            }}
          >
            <input
              autoFocus
              value={renameValue}
              onChange={(ev) => setRenameValue(ev.target.value)}
              onBlur={() => setRenaming(false)}
              className="w-40 rounded border bg-transparent px-1.5 py-0.5 text-sm outline-none focus:ring-1 focus:ring-ring"
            />
          </form>
        ) : (
          <span
            className="text-sm font-semibold select-none"
            onContextMenu={(ev) => {
              ev.preventDefault();
              setRenameValue(label);
              setRenaming(true);
              }}
          >
            {label}
          </span>
        )}
        <div className="flex-1" />
        {status && <span className="truncate text-xs text-muted-foreground">{status}</span>}
        <button
          type="button"
          aria-label="Quản trị phân quyền Não"
          data-testid="msx-admin-button"
          className={`rounded-md p-1.5 transition-colors hover:bg-accent ${adminOpen ? "bg-accent" : ""}`}
          onClick={() => setAdminOpen((v) => !v)}
        >
          <Settings className="h-4 w-4" />
        </button>
        {selected && !selected.is_dir && isMd && (
          <div className="relative" ref={shareRef}>
            <button
              type="button"
              data-testid="msx-share-button"
              className="flex items-center gap-1 rounded-md border px-2.5 py-1.5 text-xs font-medium transition-colors hover:bg-accent"
              onClick={openShareMenu}
            >
              <Send className="h-3.5 w-3.5" />
              Gửi ra kênh
              <ChevronDown className="h-3 w-3" />
            </button>
            {shareOpen && (
              <div className="absolute right-0 top-full z-50 mt-1 max-h-64 w-64 overflow-y-auto rounded-md border bg-popover p-1 shadow-md">
                <input
                  value={shareNote}
                  onChange={(ev) => setShareNote(ev.target.value)}
                  placeholder="Ghi chú kèm tài liệu (tuỳ chọn)…"
                  className="mb-1 w-full rounded border bg-transparent px-2 py-1.5 text-xs outline-none focus:ring-1 focus:ring-ring"
                />
                {channels.map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className="block w-full truncate px-3 py-2 text-left text-sm hover:bg-accent"
                    onClick={() => handleShare(c.id, c.name)}
                  >
                    #{c.name}
                  </button>
                ))}
                {channels.length === 0 && (
                  <div className="px-3 py-2 text-sm text-muted-foreground">Không có kênh nào</div>
                )}
              </div>
            )}
          </div>
        )}
        {selected && !selected.is_dir && isMd && isInThuThap && (
          <button
            type="button"
            data-testid="msx-approve-button"
            className="flex items-center gap-1 rounded-md bg-emerald-600 px-2.5 py-1.5 text-xs font-medium text-white transition-colors hover:bg-emerald-700"
            onClick={handleApprove}
          >
            <Check className="h-3.5 w-3.5" />
            Duyệt
          </button>
        )}
      </div>
      {adminOpen ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <BrainAdmin vaultRoot={vaultRoot} />
        </div>
      ) : (
      <div className="flex min-h-0 flex-1">
        <div className="w-80 min-w-0 max-w-[20rem] shrink-0 overflow-hidden border-r">
          <FileTree entries={entries} selectedPath={selected?.rel_path ?? null} onOpen={open} />
        </div>
        <div className="min-w-0 flex-1 overflow-y-auto">
          {error && <div className="p-4 text-sm text-destructive">{error}</div>}
          {selected ? (
            isMd ? (
              <MarkdownPreview content={content} />
            ) : (
              <div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
                Chỉ xem trước được file .md.
                <br />
                File "{selected.name}" hãy mở bằng ứng dụng khác (Obsidian/Drive).
              </div>
            )
          ) : (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
              <FolderOpen className="h-8 w-8" />
              <span className="text-sm">Chọn một tài liệu để xem</span>
            </div>
          )}
        </div>
      </div>
      )}
    </div>
  );
}
