use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const BLOCKED_DIRS: &[&str] = &[
    "_mat",
    ".git",
    ".obsidian",
    ".claude",
    ".agents",
    ".codex",
    ".trash",
    "node_modules",
];

#[derive(Serialize)]
pub struct BrainEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub area: String,
}

/// Thư mục ẩn khỏi cây (không phải cấm đọc — `_meta` cần cho config/app).
const HIDDEN_DIRS: &[&str] = &["_meta"];

/// Lớp chặn cứng: thư mục hệ thống và `_mat` không bao giờ đọc được, mọi vai trò.
pub fn can_read(rel_path: &str) -> bool {
    let first = rel_path.split('/').next().unwrap_or("");
    if BLOCKED_DIRS.contains(&first) {
        return false;
    }
    if rel_path.contains("/_mat") {
        return false;
    }
    true
}

/// Khu: `"*"` thấy hết; vai khác thấy khu chung MindMirror/PARA + khu riêng của mình.
pub fn khu_ok(rel_path: &str, khu: &str) -> bool {
    if khu == "*" {
        return true;
    }
    // config phân quyền: mọi người đều phải đọc được để biết quyền của mình
    if rel_path == "_meta/nguoi-dung.json" {
        return true;
    }
    let first = rel_path.split('/').next().unwrap_or("");
    matches!(
        first,
        "0. Bắt Đầu"
            | "1. Thu Thập"
            | "2. Tinh Lọc"
            | "3. Chuyển Hoá"
            | "4. Kiến Tạo"
            | "5. Hộp Công Cụ"
            | "5. Công Cụ"
            | "MSX Knowledge"
    ) || rel_path.starts_with(khu)
}

pub fn filter_entries(root: &Path, khu: &str) -> Vec<BrainEntry> {
    let mut out = Vec::new();
    walk(root, root, khu, &mut out);
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    out
}

fn walk(root: &Path, dir: &Path, khu: &str, out: &mut Vec<BrainEntry>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if BLOCKED_DIRS.contains(&name.as_str()) || HIDDEN_DIRS.contains(&name.as_str()) {
            continue;
        }
        let p: PathBuf = entry.path();
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        let is_dir = p.is_dir();
        if !khu_ok(&rel, khu) {
            continue;
        }
        out.push(BrainEntry {
            name: name.clone(),
            rel_path: rel.clone(),
            is_dir,
            area: first_area(&rel),
        });
        if is_dir {
            walk(root, &p, khu, out);
        }
    }
}

fn first_area(rel_path: &str) -> String {
    rel_path.split('/').next().unwrap_or("").to_string()
}

#[tauri::command]
pub fn brain_list_tree(root: String, khu: String) -> Vec<BrainEntry> {
    filter_entries(Path::new(&root), &khu)
}

#[tauri::command]
pub fn brain_read_file(root: String, rel_path: String, khu: String) -> Result<String, String> {
    if !khu_ok(&rel_path, &khu) || !can_read(&rel_path) {
        return Err("forbidden".into());
    }
    let full = Path::new(&root).join(&rel_path);
    // chống path traversal: đường dẫn tuyệt đối phải nằm trong root
    let root_canon = Path::new(&root).canonicalize().unwrap_or_default();
    match full.canonicalize() {
        Ok(canon) if canon.starts_with(&root_canon) => {
            fs::read_to_string(&full).map_err(|e| e.to_string())
        }
        _ => Err("forbidden".into()),
    }
}


#[tauri::command]
pub fn brain_write_meta(root: String, content: String) -> Result<String, String> {
    // chỉ cho phép ghi đúng 1 file: _meta/nguoi-dung.json
    let full = Path::new(&root).join("_meta/nguoi-dung.json");
    let root_canon = Path::new(&root).canonicalize().unwrap_or_default();
    // file có thể chưa tồn tại — kiểm tra qua thư mục cha
    let parent_canon = full
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_default();
    if !parent_canon.starts_with(&root_canon) {
        return Err("forbidden".into());
    }
    // validate JSON trước khi ghi
    let cfg: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("JSON lỗi: {e}"))?;
    if cfg.get("nguoi").and_then(|v| v.as_array()).is_none() {
        return Err("config thiếu mảng 'nguoi'".into());
    }
    fs::write(&full, &content).map_err(|e| e.to_string())?;
    Ok(full.to_string_lossy().to_string())
}
#[path = "msx_brain_tests.rs"]
mod tests;
