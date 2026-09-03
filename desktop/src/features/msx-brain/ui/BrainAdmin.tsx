import { Pencil, Plus, Save, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useMyPubkey } from "../lib/useMyPubkey";

type BrainUser = {
  ten: string;
  pubkey: string;
  vai_tro: string;
  khu: string;
};

type BrainConfig = {
  vai_tro?: Record<string, unknown>;
  nguoi?: BrainUser[];
  agents?: Array<{ ten: string; doc?: string[] }>;
};

async function readMeta(root: string): Promise<BrainConfig | null> {
  try {
    const raw = await invoke<string>("brain_read_file", {
      root,
      relPath: "_meta/nguoi-dung.json",
      khu: "*",
    });
    return JSON.parse(raw) as BrainConfig;
  } catch {
    return null;
  }
}

async function writeMeta(root: string, cfg: BrainConfig): Promise<void> {
  await invoke("brain_write_meta", { root, content: JSON.stringify(cfg, null, 2) });
}

export function BrainAdmin({ vaultRoot }: { vaultRoot: string }) {
  const myPubkey = useMyPubkey();
  const [cfg, setCfg] = useState<BrainConfig | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState<BrainUser | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [isOwner, setIsOwner] = useState(false);

  useEffect(() => {
    void (async () => {
      const loaded = await readMeta(vaultRoot);
      setCfg(loaded);
      const me = loaded?.nguoi?.find((u) => u.pubkey === myPubkey);
      setIsOwner(me?.vai_tro === "chu");
    })();
  }, [vaultRoot, myPubkey]);

  async function save() {
    if (!cfg) return;
    try {
      await writeMeta(vaultRoot, cfg);
      setStatus("Đã lưu phân quyền vào _meta/nguoi-dung.json.");
    } catch (err) {
      setStatus(`Lỗi: ${String(err)}`);
    }
  }

  if (!cfg) return <div className="p-4 text-sm text-muted-foreground">Không đọc được config.</div>;

  return (
    <div className="p-4 text-sm">
      <div className="mb-3 flex items-center gap-2">
        <span className="font-semibold">Quản trị phân quyền Não</span>
        {status && <span className="text-xs text-muted-foreground">{status}</span>}
        <div className="flex-1" />
        {isOwner && (
          <button
            type="button"
            className="flex items-center gap-1 rounded-md bg-emerald-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-emerald-700"
            onClick={() => void save()}
          >
            <Save className="h-3.5 w-3.5" /> Lưu
          </button>
        )}
      </div>
      {!isOwner && (
        <div className="mb-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs">
          Chỉ chủ não (vai trò "chu") mới được sửa. Xem ở chế độ chỉ đọc.
        </div>
      )}
      <table className="w-full border-collapse">
        <thead>
          <tr className="border-b text-left text-xs text-muted-foreground">
            <th className="py-1.5 pr-2">Tên</th>
            <th className="py-1.5 pr-2">Vai trò</th>
            <th className="py-1.5 pr-2">Khu</th>
            <th className="py-1.5 pr-2">Pubkey</th>
            {isOwner && <th />}
          </tr>
        </thead>
        <tbody>
          {(cfg.nguoi ?? []).map((u, i) => (
            <tr key={u.pubkey} className="border-b">
              {editing === i && draft ? (
                <>
                  <td className="py-1 pr-2">
                    <input
                      className="w-28 rounded border bg-transparent px-1.5 py-0.5"
                      value={draft.ten}
                      onChange={(ev) => setDraft({ ...draft, ten: ev.target.value })}
                    />
                  </td>
                  <td className="py-1 pr-2">
                    <select
                      className="rounded border bg-transparent px-1.5 py-0.5"
                      value={draft.vai_tro}
                      onChange={(ev) => setDraft({ ...draft, vai_tro: ev.target.value })}
                    >
                      <option value="chu">chu</option>
                      <option value="quan-ly">quan-ly</option>
                      <option value="nhan-vien">nhan-vien</option>
                    </select>
                  </td>
                  <td className="py-1 pr-2">
                    <input
                      className="w-16 rounded border bg-transparent px-1.5 py-0.5"
                      value={draft.khu}
                      onChange={(ev) => setDraft({ ...draft, khu: ev.target.value })}
                    />
                  </td>
                  <td className="max-w-[16rem] truncate py-1 pr-2 text-xs text-muted-foreground">
                    {u.pubkey}
                  </td>
                </>
              ) : (
                <>
                  <td className="py-1 pr-2">{u.ten}</td>
                  <td className="py-1 pr-2">{u.vai_tro}</td>
                  <td className="py-1 pr-2">{u.khu}</td>
                  <td className="max-w-[16rem] truncate py-1 pr-2 text-xs text-muted-foreground">
                    {u.pubkey}
                  </td>
                </>
              )}
              {isOwner && (
                <td className="py-1 text-right">
                  {editing === i ? (
                    <button
                      type="button"
                      className="mr-1 rounded border px-1.5 py-0.5 text-xs"
                      onClick={() => {
                        const next = [...(cfg.nguoi ?? [])];
                        next[i] = draft!;
                        setCfg({ ...cfg, nguoi: next });
                        setEditing(null);
                      }}
                    >
                      OK
                    </button>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="mr-1 rounded px-1 py-0.5 hover:bg-accent"
                        onClick={() => {
                          setEditing(i);
                          setDraft({ ...u });
                        }}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button
                        type="button"
                        className="rounded px-1 py-0.5 text-destructive hover:bg-accent"
                        onClick={() => {
                          setCfg({ ...cfg, nguoi: (cfg.nguoi ?? []).filter((_, j) => j !== i) });
                        }}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </>
                  )}
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {isOwner && (
        <button
          type="button"
          className="mt-3 flex items-center gap-1 rounded-md border px-2.5 py-1.5 text-xs hover:bg-accent"
          onClick={() => {
            setCfg({
              ...cfg,
              nguoi: [
                ...(cfg.nguoi ?? []),
                { ten: "Người mới", pubkey: "PASTE_PUBKEY", vai_tro: "nhan-vien", khu: "mkt" },
              ],
            });
            setEditing((cfg.nguoi ?? []).length);
            setDraft({ ten: "Người mới", pubkey: "PASTE_PUBKEY", vai_tro: "nhan-vien", khu: "mkt" });
          }}
        >
          <Plus className="h-3.5 w-3.5" /> Thêm người
        </button>
      )}
    </div>
  );
}
