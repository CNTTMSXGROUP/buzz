import { Brain, Check, ChevronDown, FolderOpen, Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { sendApprove } from "../lib/approve";
import { listShareableChannels, shareToChannel } from "../lib/shareToChannel";
import { useBrainTree } from "../lib/useBrainTree";
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
  const shareRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onDocClick(ev: MouseEvent) {
      if (shareRef.current && !shareRef.current.contains(ev.target as Node)) setShareOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, []);

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
        <span className="text-sm font-semibold">Não MSX</span>
        <div className="flex-1" />
        {status && <span className="truncate text-xs text-muted-foreground">{status}</span>}
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
              <div className="absolute right-0 top-full z-50 mt-1 max-h-64 w-56 overflow-y-auto rounded-md border bg-popover shadow-md">
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
      <div className="flex min-h-0 flex-1">
        <div className="w-80 shrink-0 overflow-y-auto border-r">
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
    </div>
  );
}
