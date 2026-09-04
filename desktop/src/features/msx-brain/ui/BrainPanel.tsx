import { Brain, Check, ChevronDown, File as FileIcon, FolderOpen, Plus, RefreshCw, Send, Settings, X } from "lucide-react";
import React, { useEffect, useRef, useState } from "react";
import { sendApprove } from "../lib/approve";
import { listShareableChannels, shareToChannel } from "../lib/shareToChannel";
import { useBrainTabs } from "../lib/useBrainTabs";
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
  const { entries, tabs, activePath, error, open, close, openByName, setActivePath, naoDefs, naoChon, setNaoChon, refresh, addNaoDef, khu } =
    useBrainTabs(vaultRoot, myPubkey);
  const openRef = React.useRef(open);
  openRef.current = open;
  const [status, setStatus] = useState<string | null>(null);
  const [shareOpen, setShareOpen] = useState(false);
  const [shareNote, setShareNote] = useState("");
  const [channels, setChannels] = useState<Array<{ id: string; name: string }>>([]);
  const [adminOpen, setAdminOpen] = useState(false);
  const [label, setLabel] = useState<string>(() => {
    try {
      return localStorage.getItem("msx-brain-label") ?? "Não MSX";
    } catch {
      return "Não MSX";
    }
  });
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [quickOpen, setQuickOpen] = useState(false);
  const [quickTitle, setQuickTitle] = useState("");
  const [quickBody, setQuickBody] = useState("");
  const shareRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onDocClick(ev: MouseEvent) {
      if (shareRef.current && !shareRef.current.contains(ev.target as Node)) setShareOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, []);

  // chip [nao:...] / [[wiki]] từ tin nhắn hoặc preview → mở tab
  useEffect(() => {
    function onNaoAdded(ev: Event) {
      const detail = (ev as CustomEvent<{ id: string; path: string }>).detail;
      if (detail?.id) addNaoDef(detail.id, detail.path || detail.id);
      void refresh();
    }
    window.addEventListener("msx-brain-nao-added", onNaoAdded);
    async function onOpenFile(ev: Event) {
      const rel = (ev as CustomEvent<{ rel: string }>).detail?.rel;
      if (rel) {
        setAdminOpen(false);
        await open(rel);
      }
    }
    async function onOpenWiki(ev: Event) {
      const name = (ev as CustomEvent<{ name: string }>).detail?.name;
      if (name) openByName(name);
    }
    window.addEventListener("msx-brain-open-file", onOpenFile);
    window.addEventListener("msx-brain-open-wiki", onOpenWiki);
    return () => {
      window.removeEventListener("msx-brain-open-file", onOpenFile);
      window.removeEventListener("msx-brain-open-wiki", onOpenWiki);
    };
  }, [open, openByName]);

  const active = tabs.find((t) => t.relPath === activePath) ?? null;
  const isInThuThap = active?.relPath.startsWith("1. Thu Thập") ?? false;

  async function handleApprove() {
    if (!active) return;
    try {
      await sendApprove(active.name);
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

  const [previewShare, setPreviewShare] = useState<{ channelId: string; channelName: string } | null>(null);

  async function handleShare(channelId: string, channelName: string) {
    if (!active) return;
    setShareOpen(false);
    setPreviewShare({ channelId, channelName }); // xem trước trước khi gửi
  }

  async function confirmShare() {
    if (!active || !previewShare) return;
    try {
      await shareToChannel(previewShare.channelId, active.name, active.content, { note: shareNote });
      setStatus(`Đã gửi "${active.name}" vào #${previewShare.channelName}.`);
      setShareNote("");
      setPreviewShare(null);
    } catch (err) {
      setStatus(`Lỗi: ${String(err)}`);
    }
  }

  return (
    <div className="flex h-full w-full flex-col overflow-hidden">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <Brain className="h-4 w-4 text-amber-500" />
        {renaming ? (
          <form
            onSubmit={(ev) => {
              ev.preventDefault();
              const v = renameValue.trim();
              if (v) {
                setLabel(v);
                try {
                  localStorage.setItem("msx-brain-label", v);
                } catch {
                  /* ignore */
                }
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
            className="select-none text-sm font-semibold"
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
          aria-label="Tải lại cây thư mục"
          data-testid="msx-refresh-button"
          className="rounded-md p-1.5 transition-colors hover:bg-accent"
          onClick={() => void refresh()}
        >
          <RefreshCw className="h-4 w-4" />
        </button>
        <button
          type="button"
          data-testid="msx-quick-button"
          className="flex items-center gap-1 rounded-md border px-2.5 py-1.5 text-xs font-medium transition-colors hover:bg-accent"
          onClick={() => setQuickOpen(true)}
        >
          <Plus className="h-3.5 w-3.5" />
          Ghi nhanh
        </button>
        <button
          type="button"
          aria-label="Quản trị phân quyền Não"
          data-testid="msx-admin-button"
          className={`rounded-md p-1.5 transition-colors hover:bg-accent ${adminOpen ? "bg-accent" : ""}`}
          onClick={() => setAdminOpen((v) => !v)}
        >
          <Settings className="h-4 w-4" />
        </button>
        {active && active.kind === "md" && (
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
        {active && active.kind === "md" && isInThuThap && (
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
        <>
        <div className="flex items-center gap-1.5 border-b px-3 py-1.5">
          <span className="text-xs text-muted-foreground">Não con:</span>
          {naoDefs.map((d) => (
            <button
              key={d.id}
              type="button"
              title={d.path}
              className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors ${
                naoChon?.id === d.id
                  ? "bg-amber-500/20 text-amber-700 dark:text-amber-400"
                  : "text-muted-foreground hover:bg-accent"
              }`}
              onClick={() => setNaoChon(d)}
            >
              🧠 {d.id}
            </button>
          ))}
        </div>
        <div className="flex min-h-0 flex-1">
          <div className="w-80 min-w-0 max-w-[20rem] shrink-0 overflow-hidden border-r">
            <FileTree
              entries={entries}
              selectedPath={activePath}
              onOpen={(e) => void open(e.rel_path)}
              naoChon={naoChon}
              allPaths={naoDefs.map((d) => d.path)}
              vaultRoot={vaultRoot}
              khu={khu}
            />
          </div>
          <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
            {tabs.length > 0 && (
              <div className="flex items-center gap-1 overflow-x-auto border-b bg-muted/30 px-1 py-1">
                {tabs.map((t) => (
                  <div
                    key={t.relPath}
                    className={`group flex shrink-0 items-center gap-1 rounded-md border px-2 py-1 text-xs ${
                      activePath === t.relPath
                        ? "bg-background font-medium"
                        : "bg-transparent text-muted-foreground hover:bg-accent"
                    }`}
                  >
                    <button
                      type="button"
                      className="max-w-48 truncate"
                      onClick={() => setActivePath(t.relPath)}
                    >
                      {t.name}
                      {t.kind === "other" ? " •" : ""}
                  </button>
                  <button
                    type="button"
                    aria-label={`Đóng ${t.name}`}
                    className="rounded p-0.5 opacity-0 transition-opacity hover:bg-accent group-hover:opacity-100"
                    onClick={() => close(t.relPath)}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
              </div>
            )}
            <div className="min-h-0 flex-1 overflow-hidden">
              {error && <div className="p-4 text-sm text-destructive">{error}</div>}
              {active ? (
                active.kind === "md" ? (
                  <MarkdownPreview
                    content={active.content}
                    onOpenFile={(rel) => void open(rel)}
                    onOpenWiki={(n) => openByName(n)}
                  />
                ) : active.kind === "image" ? (
                  <div className="flex h-full items-center justify-center p-4">
                    <img src={active.content} alt={active.name} className="max-h-full max-w-full object-contain" />
                  </div>
                ) : active.kind === "text" ? (
                  <pre className="h-full overflow-auto p-4 font-mono text-xs leading-relaxed">
                    {active.content.slice(0, 20000)}
                    {active.content.length > 20000 ? "\n… (cắt bớt — mở bằng Obsidian để xem đầy đủ)" : null}
                  </pre>
                ) : (
                  <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
                    <FileIcon className="h-8 w-8" />
                    <span>"{active.name}" — định dạng chưa xem trước được ({(active.size / 1024).toFixed(0)} KB).</span>
                    <span>Hãy mở bằng Obsidian / Google Drive.</span>
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
        </>
      )}
      {previewShare && active && (
        <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50">
          <div className="flex max-h-[80vh] w-[32rem] max-w-[92vw] flex-col rounded-xl border bg-popover shadow-xl">
            <div className="border-b px-4 py-2.5">
              <div className="text-sm font-semibold">Xem trước khi gửi</div>
              <div className="text-xs text-muted-foreground">
                "{active.name}" → #{previewShare.channelName}
              </div>
            </div>
            <div className="msx-md min-h-0 flex-1 overflow-y-auto px-5 py-4">
              <div className="mx-auto max-w-2xl text-sm">{active.content.slice(0, 3000)}</div>
              {shareNote.trim() && (
                <div className="mx-auto mt-2 max-w-2xl rounded-md bg-amber-500/10 px-3 py-2 text-sm">
                  💬 {shareNote}
                </div>
              )}
            </div>
            <div className="flex justify-end gap-2 border-t px-4 py-2.5">
              <button
                type="button"
                className="rounded-md border px-3 py-1.5 text-xs hover:bg-accent"
                onClick={() => setPreviewShare(null)}
              >
                Huỷ
              </button>
              <button
                type="button"
                data-testid="msx-share-confirm"
                className="rounded-md bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-700"
                onClick={() => void confirmShare()}
              >
                Xác nhận gửi
              </button>
            </div>
          </div>
        </div>
      )}
      {quickOpen && (
        <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50">
          <div className="w-[28rem] max-w-[90vw] rounded-xl border bg-popover p-4 shadow-xl">
            <div className="mb-2 text-sm font-semibold">Ghi nhanh vào {naoChon ? `não "${naoChon.id}"` : "não"}</div>
            <input
              autoFocus
              value={quickTitle}
              onChange={(ev) => setQuickTitle(ev.target.value)}
              placeholder="Tiêu đề…"
              className="mb-2 w-full rounded-md border bg-transparent px-2.5 py-1.5 text-sm outline-none focus:ring-1 focus:ring-ring"
            />
            <textarea
              value={quickBody}
              onChange={(ev) => setQuickBody(ev.target.value)}
              placeholder="Nội dung…"
              rows={5}
              className="mb-3 w-full rounded-md border bg-transparent px-2.5 py-1.5 text-sm outline-none focus:ring-1 focus:ring-ring"
            />
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="rounded-md border px-3 py-1.5 text-xs hover:bg-accent"
                onClick={() => setQuickOpen(false)}
              >
                Huỷ
              </button>
              <button
                type="button"
                data-testid="msx-quick-save"
                className="rounded-md bg-amber-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-amber-700"
                onClick={() => void (async () => {
                  try {
                    const { invoke } = await import("@tauri-apps/api/core");
                    const naoPath = naoChon?.path ?? "";
                    const rel = await invoke<string>("brain_create_ghinhanh", {
                      root: vaultRoot,
                      naoRel: naoPath,
                      title: quickTitle,
                      body: quickBody,
                    });
                    setQuickOpen(false);
                    setStatus(`Đã ghi nhanh vào 1. Thu Thập (${rel}).`);
                    setQuickTitle("");
                    setQuickBody("");
                    await refresh();
                    // báo cho sếp biết để duyệt
                    try {
                      const { listShareableChannels } = await import("../lib/shareToChannel");
                      const { sendChannelMessage } = await import("@/shared/api/tauriMessages");
                      const channels = await listShareableChannels();
                      const dh = channels.find((c) => c.name === "dieu-hanh");
                      if (dh) {
                        const who = myPubkey ? myPubkey.slice(0, 8) : "app";
                        await sendChannelMessage(
                          dh.id,
                          `📥 Ghi nhanh mới từ app (bởi ${who}): **${quickTitle}** — vào não "${naoChon?.id ?? ""}". Sếp gõ \`!duyet ${quickTitle}\` để duyệt.`,
                        );
                      }
                    } catch {
                      /* báo kênh là phụ — file đã lưu là chính */
                    }
                  } catch (err) {
                    setStatus(`Lỗi: ${String(err)}`);
                  }
                })()}
              >
                Lưu vào Thu Thập
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
