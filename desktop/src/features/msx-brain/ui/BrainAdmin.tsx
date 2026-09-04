import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Pencil, Plus, Save, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { loadNaoCon } from "../lib/useBrainTabs";
import { useMyPubkey } from "../lib/useMyPubkey";

type BrainUser = {
  ten: string;
  pubkey: string;
  vai_tro: string;
  khu: string;
  nao?: string[];
};

type BrainConfig = {
  vai_tro?: Record<string, unknown>;
  nguoi?: BrainUser[];
  agents?: Array<{ ten: string; doc?: string[] }>;
  nao_con?: { danh_sach?: string[] };
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
  const [naoList, setNaoList] = useState<string[]>([]);
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState<BrainUser | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [isOwner, setIsOwner] = useState(false);
  const [newNao, setNewNao] = useState("");
  const [newNaoParent, setNewNaoParent] = useState("Nao Bo Phan");
  const [dirChoices, setDirChoices] = useState<string[]>(["Nao Bo Phan"]);

  async function reload() {
    const [loaded, nao] = await Promise.all([readMeta(vaultRoot), loadNaoCon(vaultRoot)]);
    setCfg(loaded);
    setNaoList(nao);
    // thư mục ứng viên đặt não: root + Nao Bo Phan + các thư mục cấp 1
    try {
      const all = await invoke<Array<{ rel_path: string; is_dir: boolean }>>("brain_list_tree", {
        root: vaultRoot,
        khu: "*",
      });
      const dirs = all.filter((e) => e.is_dir && !e.rel_path.includes("/")).map((e) => e.rel_path);
      setDirChoices(["", ...dirs]);
    } catch {
      /* giữ mặc định */
    }
    const me = loaded?.nguoi?.find((u) => u.pubkey === myPubkey);
    setIsOwner(me?.vai_tro === "chu");
  }

  useEffect(() => {
    void reload();
  }, [vaultRoot, myPubkey]);

  async function handleCreateNao() {
    try {
      const created = await invoke<string>("brain_create_nao", {
        root: vaultRoot,
        id: newNao,
        parentRel: newNaoParent,
      });
      setNewNao("");
      await reload();
      setStatus(`Đã tạo não con "${created}" — tick chọn cho người cần xem.`);
      window.dispatchEvent(new CustomEvent("msx-brain-nao-added", { detail: { id: created } }));
    } catch (err) {
      setStatus(`Lỗi: ${String(err)}`);
    }
  }

  async function handlePickFolder() {
    try {
      const picked = await openDialog({ directory: true, multiple: false, title: "Chọn thư mục chứa não con", defaultPath: vaultRoot });
      if (typeof picked !== "string") return;
      // chỉ chấp nhận folder nằm trong vault
      const norm = picked.replace(/\\/g, "/");
      const root = vaultRoot.replace(/\\/g, "/");
      if (norm === root) {
        setNewNaoParent("");
      } else if (norm.startsWith(root + "/")) {
        setNewNaoParent(norm.slice(root.length + 1));
      } else {
        setStatus("Thư mục phải nằm trong vault Não chủ.");
      }
    } catch (err) {
      setStatus(`Lỗi: ${String(err)}`);
    }
  }

  async function save() {
    if (!cfg) return;
    try {
      await writeMeta(vaultRoot, cfg);
      setStatus("Đã lưu phân quyền.");
    } catch (err) {
      setStatus(`Lỗi: ${String(err)}`);
    }
  }

  function toggleNaoFor(draftUser: BrainUser, nao: string): BrainUser {
    const cur = draftUser.nao ?? [];
    return {
      ...draftUser,
      nao: cur.includes(nao) ? cur.filter((n) => n !== nao) : [...cur, nao],
    };
  }

  if (!cfg)
    return <div className="p-4 text-sm text-muted-foreground">Không đọc được config.</div>;

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

      {isOwner && (
        <div className="mb-3 flex items-center gap-2 rounded-md border border-dashed px-3 py-2">
          <span className="text-xs font-medium">Não con mới:</span>
          <input
            className="w-36 rounded border bg-transparent px-1.5 py-1 text-xs"
            placeholder="vd: kho-van"
            value={newNao}
            onChange={(ev) => setNewNao(ev.target.value)}
            onKeyDown={(ev) => {
              if (ev.key === "Enter") void handleCreateNao();
            }}
          />
          <select
            className="max-w-44 rounded border bg-transparent px-1.5 py-1 text-xs"
            value={newNaoParent}
            onChange={(ev) => setNewNaoParent(ev.target.value)}
            title="Thư mục cha đặt não con (trong vault)"
          >
            <option value="">— gốc vault —</option>
            {dirChoices
              .filter((d) => d !== "")
              .map((d) => (
                <option key={d} value={d}>
                  trong {d}
                </option>
              ))}
          </select>
          <button
            type="button"
            className="flex items-center gap-1 rounded-md border px-2 py-1 text-xs hover:bg-accent"
            onClick={() => void handlePickFolder()}
            title="Mở hộp thoại chọn thư mục trong vault"
          >
            <FolderOpen className="h-3.5 w-3.5" /> Chọn folder…
          </button>
          <button
            type="button"
            className="flex items-center gap-1 rounded-md bg-amber-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-amber-700"
            onClick={() => void handleCreateNao()}
          >
            <Plus className="h-3.5 w-3.5" /> Thêm não
          </button>
          <span className="text-[11px] text-muted-foreground">
            Tự tạo thư mục pipeline + đăng ký danh sách.
          </span>
        </div>
      )}

      <table className="w-full border-collapse">
        <thead>
          <tr className="border-b text-left text-xs text-muted-foreground">
            <th className="py-1.5 pr-2">Tên</th>
            <th className="py-1.5 pr-2">Vai trò</th>
            <th className="py-1.5 pr-2">Não con được xem</th>
            <th className="py-1.5 pr-2">Pubkey</th>
            {isOwner && <th />}
          </tr>
        </thead>
        <tbody>
          {(cfg.nguoi ?? []).map((u, i) => (
            <tr key={u.pubkey} className="border-b align-top">
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
                    <div className="flex flex-wrap gap-1.5">
                      {naoList.map((n) => {
                        const on = (draft.nao ?? []).includes(n) || draft.khu === "*";
                        return (
                          <label
                            key={n}
                            className={`cursor-pointer rounded-full px-2 py-0.5 text-xs ${
                              on
                                ? "bg-amber-500/20 text-amber-700 dark:text-amber-400"
                                : "border text-muted-foreground"
                            }`}
                          >
                            <input
                              type="checkbox"
                              className="mr-1 hidden"
                              checked={on}
                              onChange={() => setDraft(toggleNaoFor(draft, n))}
                            />
                            {n}
                          </label>
                        );
                      })}
                    </div>
                  </td>
                  <td className="max-w-[14rem] py-1 pr-2">
                    <input
                      className="w-full rounded border bg-transparent px-1.5 py-0.5 text-xs"
                      value={draft.pubkey}
                      placeholder="Dán pubkey của người đó…"
                      onChange={(ev) => setDraft({ ...draft, pubkey: ev.target.value })}
                    />
                  </td>
                </>
              ) : (
                <>
                  <td className="py-1 pr-2">{u.ten}</td>
                  <td className="py-1 pr-2">{u.vai_tro}</td>
                  <td className="py-1 pr-2">
                    {u.khu === "*" ? (
                      <span className="text-xs">tất cả</span>
                    ) : (
                      <div className="flex flex-wrap gap-1">
                        {(u.nao ?? [u.khu]).map((n) => (
                          <span
                            key={n}
                            className="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-400"
                          >
                            {n}
                          </span>
                        ))}
                      </div>
                    )}
                  </td>
                  <td className="max-w-[14rem] truncate py-1 pr-2 text-xs text-muted-foreground">
                    {u.pubkey}
                  </td>
                </>
              )}
              {isOwner && (
                <td className="whitespace-nowrap py-1 text-right">
                  {editing === i ? (
                    <button
                      type="button"
                      className="mr-1 rounded border px-1.5 py-0.5 text-xs"
                      onClick={() => {
                        const next = [...(cfg.nguoi ?? [])];
                        const d = draft!;
                        // đồng bộ khu = danh sách não (chu = "*")
                        const synced: BrainUser = {
                          ...d,
                          khu: d.vai_tro === "chu" ? "*" : (d.nao ?? []).join(","),
                        };
                        next[i] = synced;
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
                          setDraft({ ...u, nao: u.nao ?? (u.khu === "*" ? ["*"] : u.khu.split(",")) });
                        }}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button
                        type="button"
                        className="rounded px-1 py-0.5 text-destructive hover:bg-accent"
                        onClick={() =>
                          setCfg({ ...cfg, nguoi: (cfg.nguoi ?? []).filter((_, j) => j !== i) })
                        }
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
            const nguoiMoi: BrainUser = {
              ten: "Người mới",
              pubkey: "PASTE_PUBKEY",
              vai_tro: "nhan-vien",
              khu: "chung",
              nao: ["chung"],
            };
            setCfg({ ...cfg, nguoi: [...(cfg.nguoi ?? []), nguoiMoi] });
            setEditing((cfg.nguoi ?? []).length);
            setDraft(nguoiMoi);
          }}
        >
          <Plus className="h-3.5 w-3.5" /> Thêm người
        </button>
      )}
    </div>
  );
}
