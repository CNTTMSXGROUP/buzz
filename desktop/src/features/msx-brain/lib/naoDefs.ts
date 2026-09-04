import { invoke } from "@tauri-apps/api/core";
import type { BrainEntry } from "../types";

export type NaoDef = { id: string; path: string };

/** Parse danh_sach: hỗ trợ cả string cũ ("mkt") và object mới ({id, path}). */
export function normalizeNaoList(raw: unknown): NaoDef[] {
  if (!Array.isArray(raw)) return [];
  const out: NaoDef[] = [];
  for (const item of raw) {
    if (typeof item === "string") {
      out.push({ id: item, path: `Nao Bo Phan/${item}` });
    } else if (item && typeof item === "object") {
      const o = item as { id?: unknown; path?: unknown };
      if (typeof o.id === "string" && typeof o.path === "string") {
        out.push({ id: o.id, path: o.path });
      } else if (typeof o.id === "string") {
        out.push({ id: o.id, path: `Nao Bo Phan/${o.id}` });
      }
    }
  }
  return out;
}

export async function loadNaoDefs(root: string): Promise<NaoDef[]> {
  try {
    const raw = await invoke<string>("brain_read_file", {
      root,
      relPath: "_meta/nguoi-dung.json",
      khu: "*",
    });
    const cfg = JSON.parse(raw) as { nao_con?: { danh_sach?: unknown } };
    const list = normalizeNaoList(cfg.nao_con?.danh_sach);
    if (list.length) return list;
  } catch {
    /* fallback */
  }
  return [
    { id: "chung", path: "Nao Bo Phan/chung" },
    { id: "mkt", path: "Nao Bo Phan/mkt" },
    { id: "tech", path: "Nao Bo Phan/tech" },
    { id: "sale", path: "Nao Bo Phan/sale" },
  ];
}

/** Não được chọn có bao phủ relPath không? */
export function naoCoversPath(def: NaoDef, relPath: string): boolean {
  return relPath === def.path || relPath.startsWith(`${def.path}/`);
}

/** Lọc entries theo não đang chọn:
 * - giữ file/thư mục TRONG não
 * - giữ thư mục TỔ TIÊN dẫn tới não (để điều hướng vào được)
 * - bỏ các não khác (path không liên quan)
 */
export function filterByNao(entries: BrainEntry[], def: NaoDef | null, allPaths: string[]): BrainEntry[] {
  if (!def) return entries;
  const others = allPaths.filter((p) => p && p !== def.path);
  return entries.filter((e) => {
    if (naoCoversPath(def, e.rel_path)) return true;
    // tổ tiên của não: "Du An" là tiền tố của "Du An/X"
    if (e.is_dir && def.path.startsWith(`${e.rel_path}/`)) return true;
    // bỏ nếu thuộc não khác
    if (others.some((p) => e.rel_path === p || e.rel_path.startsWith(`${p}/`))) return false;
    // giữ file chung (1. Thu Thập, MSX Knowledge...) — ngoài mọi não
    return !others.some((p) => e.rel_path.startsWith(`${p.split("/")[0]}/`));
  });
}

/** Prefix quyền cho Rust khu_ok: path thật của não (comma-list khi nhiều). */
export function khuForNao(defs: NaoDef[], ids: string[]): string {
  const paths = defs.filter((d) => ids.includes(d.id)).map((d) => d.path);
  return paths.join(",");
}
