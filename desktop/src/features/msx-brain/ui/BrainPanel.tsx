import { useState } from "react";
import { useBrainTree } from "../lib/useBrainTree";
import { sendApprove } from "../lib/approve";
import { listShareableChannels, shareToChannel } from "../lib/shareToChannel";
import { FileTree } from "./FileTree";
import { MarkdownPreview } from "./MarkdownPreview";

export function BrainPanel({
  vaultRoot,
  myPubkey,
}: {
  vaultRoot: string;
  myPubkey: string;
}) {
  const { entries, selected, content, error, open, refresh } = useBrainTree(vaultRoot, myPubkey);
  const [status, setStatus] = useState<string | null>(null);

  const isInThuThap = selected?.rel_path.startsWith("1. Thu Thập") ?? false;
  const isOwner = myPubkey === OWNER_FALLBACK || true; // quyền duyệt quyết bởi bridge; nút hiện cho file Thu Thập
  void isOwner;

  async function handleApprove() {
    if (!selected) return;
    try {
      await sendApprove(selected.name);
      setStatus("Đã gửi lệnh duyệt — bridge xử lý trong ~1 phút.");
    } catch (err) {
      setStatus(String(err));
    }
  }

  async function handleShare() {
    if (!selected) return;
    try {
      const channels = await listShareableChannels();
      const pick = window.prompt(
        "Gửi tới kênh nào? Gõ tên kênh:\n" + channels.map((c) => `• ${c.name}`).join("\n"),
        "marketing",
      );
      if (!pick) return;
      const ch = channels.find((c) => c.name === pick.trim()) ?? channels.find((c) => c.name === "marketing");
      if (!ch) throw new Error("Không có kênh đó");
      await shareToChannel(ch.id, selected.name, content);
      setStatus(`Đã gửi **${selected.name}** vào #${ch.name}.`);
    } catch (err) {
      setStatus(String(err));
    }
  }

  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <span className="text-sm font-semibold">Não MSX</span>
        <div className="flex-1" />
        {status && <span className="text-xs text-muted-foreground">{status}</span>}
        {isInThuThap && (
          <button
            type="button"
            className="rounded bg-emerald-600 px-2 py-1 text-xs font-medium text-white hover:bg-emerald-700"
            onClick={handleApprove}
          >
            ✅ Duyệt
          </button>
        )}
        {selected && !selected.is_dir && (
          <button
            type="button"
            className="rounded border px-2 py-1 text-xs hover:bg-accent"
            onClick={handleShare}
          >
            📤 Gửi ra kênh
          </button>
        )}
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="w-80 shrink-0 overflow-y-auto border-r">
          <FileTree entries={entries} selectedPath={selected?.rel_path ?? null} onOpen={open} />
        </div>
        <div className="min-w-0 flex-1 overflow-y-auto">
          {error && <div className="p-4 text-destructive">{error}</div>}
          {selected ? (
            <MarkdownPreview content={content} />
          ) : (
            <div className="flex h-full items-center justify-center text-muted-foreground">
              Chọn một tài liệu để xem
            </div>
          )}
        </div>
      </div>
      <button type="button" className="hidden" onClick={refresh} aria-hidden="true" />
    </div>
  );
}

const OWNER_FALLBACK = "";
