import { invoke } from "@tauri-apps/api/core";

const BLOCKED = ["_mat", "_meta", ".git", ".obsidian", ".claude", ".agents", ".codex", ".trash", "node_modules"];
const COMMON_AREAS = [
  "0. Bắt Đầu",
  "1. Thu Thập",
  "2. Tinh Lọc",
  "3. Chuyển Hoá",
  "4. Kiến Tạo",
  "5. Hộp Công Cụ",
  "5. Công Cụ",
  "MSX Knowledge",
];

export type BrainRole = { ten: string; vaiTro: "chu" | "quan-ly" | "nhan-vien"; khu: string };

export function canReadPath(role: BrainRole, relPath: string): boolean {
  const first = relPath.split("/")[0] ?? "";
  if (BLOCKED.some((b) => first === b || relPath.includes(`/${b}/`) || relPath.startsWith(`${b}/`)))
    return false;
  if (role.khu === "*") return true;
  return COMMON_AREAS.includes(first) || relPath.startsWith(`${role.khu}/`);
}

const roleCache = new Map<string, BrainRole | null>();

interface BrainConfigUser {
  ten: string;
  pubkey: string;
  vai_tro: string;
  khu: string;
}

export async function loadBrainRole(root: string, pubkey: string): Promise<BrainRole | null> {
  if (roleCache.has(pubkey)) return roleCache.get(pubkey) ?? null;
  let role: BrainRole | null = null;
  try {
    const raw = await invoke<string>("brain_read_file", {
      root,
      relPath: "_meta/nguoi-dung.json",
      khu: "*",
    });
    const cfg = JSON.parse(raw) as { nguoi?: BrainConfigUser[] };
    const u = cfg.nguoi?.find((x) => x.pubkey === pubkey);
    if (u) role = { ten: u.ten, vaiTro: u.vai_tro as BrainRole["vaiTro"], khu: u.khu };
  } catch {
    role = null;
  }
  roleCache.set(pubkey, role);
  return role;
}
