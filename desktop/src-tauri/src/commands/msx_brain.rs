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
    pub mtime: u64,
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
    let common = matches!(
        first,
        "0. Bắt Đầu"
            | "1. Thu Thập"
            | "2. Tinh Lọc"
            | "3. Chuyển Hoá"
            | "4. Kiến Tạo"
            | "5. Hộp Công Cụ"
            | "5. Công Cụ"
            | "MSX Knowledge"
    );
    if common {
        return true;
    }
    // nhiều não / khu: "chung,mkt" hoặc prefix thật "4. Kiến Tạo/..."
    for k in khu.split(',') {
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        if rel_path == k || rel_path.starts_with(&format!("{k}/")) {
            return true;
        }
        // back-compat id não cũ -> Nao Bo Phan/<id>/
        if rel_path.starts_with(&format!("Nao Bo Phan/{k}/")) {
            return true;
        }
    }
    false
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
        let mtime = p
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(BrainEntry {
            name: name.clone(),
            rel_path: rel.clone(),
            is_dir,
            area: first_area(&rel),
            mtime,
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

#[tauri::command]
pub fn brain_read_bytes(root: String, rel_path: String, khu: String) -> Result<String, String> {
    if !khu_ok(&rel_path, &khu) || !can_read(&rel_path) {
        return Err("forbidden".into());
    }
    let full = Path::new(&root).join(&rel_path);
    let root_canon = Path::new(&root).canonicalize().unwrap_or_default();
    match full.canonicalize() {
        Ok(canon) if canon.starts_with(&root_canon) => {
            let bytes = fs::read(&full).map_err(|e| e.to_string())?;
            use base64::Engine as _;
            Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
            as Result<String, String>
        }
        _ => Err("forbidden".into()),
    }
}

#[tauri::command]
pub fn brain_stat(root: String, rel_path: String, khu: String) -> Result<BrainStat, String> {
    if !khu_ok(&rel_path, &khu) || !can_read(&rel_path) {
        return Err("forbidden".into());
    }
    let full = Path::new(&root).join(&rel_path);
    let root_canon = Path::new(&root).canonicalize().unwrap_or_default();
    match full.canonicalize() {
        Ok(canon) if canon.starts_with(&root_canon) => {
            let meta = fs::metadata(&full).map_err(|e| e.to_string())?;
            Ok(BrainStat {
                size: meta.len(),
                is_dir: meta.is_dir(),
            })
        }
        _ => Err("forbidden".into()),
    }
}

#[derive(Serialize)]
pub struct BrainStat {
    pub size: u64,
    pub is_dir: bool,
}


/// Tạo não con mới: chỉ cho id `a-z0-9-` ≤ 20 ký tự, tạo đúng 3 file mẫu
/// (README + .keep hai thư mục pipeline), update `danh_sach` trong config.
#[tauri::command]
pub fn brain_create_nao(root: String, id: String, parent_rel: String) -> Result<String, String> {
    let clean: String = id
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .chars()
        .take(20)
        .collect();
    if clean.is_empty() {
        return Err("Tên não phải có ít nhất 1 chữ cái/số (a-z, 0-9, -)".into());
    }
    let root_canon = Path::new(&root).canonicalize().map_err(|e| e.to_string())?;
    // parent phải là thư mục hợp lệ NẰM TRONG root (chống traversal)
    let parent_canon = root_canon.join(&parent_rel).canonicalize().map_err(|_| "Thư mục cha không hợp lệ".to_string())?;
    if !parent_canon.starts_with(&root_canon) || !parent_canon.is_dir() {
        return Err("Thư mục cha không hợp lệ".to_string());
    }
    let rel_parent = parent_canon.strip_prefix(&root_canon).unwrap().to_string_lossy().replace('\\', "/");
    let nao_dir = parent_canon.join(&clean);
    if nao_dir.exists() {
        return Err(format!("Não \"{clean}\" đã có"));
    }
    for sub in ["1. Thu Thập", "2. Tinh Lọc/Kiến Thức Nguồn"].iter() {
        fs::create_dir_all(nao_dir.join(sub)).map_err(|e| e.to_string())?;
    }
    fs::write(
        nao_dir.join("README.md"),
        format!(
            "# Não {clean}\n\nNão con của bộ phận {clean} trong Não chủ MSXGROUP.\n\nPipeline: `1. Thu Thập` (!dex từ Buzz) → `2. Tinh Lọc/Kiến Thức Nguồn` (sau !duyet).\n\nQuyền đọc cấu hình trong panel **⚙ Quản trị** của app (mục nao_con).\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    // update danh_sach trong config
    let meta_path = root_canon.join("_meta/nguoi-dung.json");
    let raw = fs::read_to_string(&meta_path).map_err(|e| e.to_string())?;
    let mut cfg: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("JSON lỗi: {e}"))?;
    let list = cfg
        .pointer_mut("/nao_con/danh_sach")
        .and_then(|v| v.as_array_mut())
        .ok_or("config thiếu nao_con.danh_sach".to_string())?;
    let rel_path = if rel_parent.is_empty() {
        clean.clone()
    } else {
        format!("{rel_parent}/{clean}")
    };
    let exists = list.iter().any(|v| {
        v.as_str() == Some(clean.as_str())
            || v.get("id").and_then(|x| x.as_str()) == Some(clean.as_str())
    });
    if !exists {
        let mut m = serde_json::Map::new();
        m.insert("id".into(), serde_json::Value::String(clean.clone()));
        m.insert("path".into(), serde_json::Value::String(rel_path));
        list.push(serde_json::Value::Object(m));
    }
    fs::write(&meta_path, serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(clean)
}


/// Tạo GHI NHANH trong `*/1. Thu Thập/` của não (đường dẫn tương đối từ root).
/// Chỉ cho phép ghi đúng mẫu `GHI NHANH — <slug>.md`, slug ASCII-hoá an toàn.
#[tauri::command]
pub fn brain_create_ghinhanh(root: String, nao_rel: String, title: String, body: String) -> Result<String, String> {
    let root_canon = Path::new(&root).canonicalize().map_err(|e| e.to_string())?;
    let dir_canon = root_canon.join(&nao_rel).join("1. Thu Thập").canonicalize().unwrap_or_else(|_| root_canon.join(&nao_rel).join("1. Thu Thập"));
    if !dir_canon.starts_with(&root_canon) {
        return Err("forbidden".into());
    }
    fs::create_dir_all(&dir_canon).map_err(|e| e.to_string())?;
    let slug: String = title
        .trim()
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .chars()
        .take(60)
        .collect();
    if slug.is_empty() {
        return Err("Tiêu đề trống".into());
    }
    let now = chrono::Local::now();
    let fname = format!("GHI NHANH — {slug}.md");
    let full = dir_canon.join(&fname);
    if full.exists() {
        return Err(format!("Đã có \"{fname}\""));
    }
    let content = format!(
        "---\ncreated: {}\nloai: ghi-nhanh\ntrang-thai: chua-xu-ly\ntags: [thu-thap, tu-app]\n---\n# {}\n\n{}\n",
        now.format("%Y-%m-%d"),
        title.trim(),
        body.trim()
    );
    fs::write(&full, content).map_err(|e| e.to_string())?;
    Ok(full
        .strip_prefix(&root_canon)
        .unwrap_or(&full)
        .to_string_lossy()
        .replace('\\', "/"))
}

#[path = "msx_brain_tests.rs"]
mod tests;
