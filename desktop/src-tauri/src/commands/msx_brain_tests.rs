//! Unit tests for `commands/msx_brain.rs`.

use super::{brain_read_file, can_read, filter_entries};
use std::fs;

fn fixture_vault(tmp: &Path) {
    fs::create_dir_all(tmp.join("1. Thu Thập")).unwrap();
    fs::write(tmp.join("1. Thu Thập/a.md"), "A").unwrap();
    fs::create_dir_all(tmp.join("2. Tinh Lọc/Kiến Thức Nguồn")).unwrap();
    fs::write(tmp.join("2. Tinh Lọc/Kiến Thức Nguồn/b.md"), "B").unwrap();
    fs::create_dir_all(tmp.join("_mat")).unwrap();
    fs::write(tmp.join("_mat/secret.md"), "S").unwrap();
    fs::create_dir_all(tmp.join(".git")).unwrap();
    fs::write(tmp.join(".git/config"), "G").unwrap();
}

use std::path::Path;

#[test]
fn test_mat_blocked_for_everyone() {
    assert!(!can_read("_mat/secret.md"));
    assert!(!can_read("_mat"));
}

#[test]
fn test_owner_thay_tat_ca_khu_chung() {
    assert!(can_read("2. Tinh Lọc/x.md"));
    assert!(can_read("MSX Knowledge/a.md"));
}

#[test]
fn test_filter_entries_loai_bo_thu_muc_cam() {
    let tmp = tempfile::tempdir().unwrap();
    fixture_vault(tmp.path());
    let entries = filter_entries(tmp.path(), "*");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"1. Thu Thập"));
    assert!(names.contains(&"2. Tinh Lọc"));
    assert!(!names.contains(&"_mat"));
    assert!(!names.contains(&".git"));
}

#[test]
fn test_nhan_vien_khu_mkt_khong_thay_khu_khac() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("mkt")).unwrap();
    fs::create_dir_all(tmp.path().join("1. Thu Thập")).unwrap();
    fs::write(tmp.path().join("mkt/note.md"), "M").unwrap();
    fs::create_dir_all(tmp.path().join("tech")).unwrap();
    fs::write(tmp.path().join("tech/runbook.md"), "T").unwrap();
    let entries = filter_entries(tmp.path(), "mkt");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"mkt"));
    assert!(names.contains(&"1. Thu Thập")); // khu chung vẫn thấy
    assert!(!names.iter().any(|n| *n == "tech"));
}

#[test]
fn test_read_file_chong_path_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    fixture_vault(tmp.path());
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("leak.md"), "LEAK").unwrap();
    let r = brain_read_file(
        tmp.path().to_string_lossy().to_string(),
        format!("../{}/leak.md", outside.path().file_name().unwrap().to_string_lossy()),
        "*".to_string(),
    );
    assert!(r.is_err());
}

#[test]
fn test_write_meta_chi_ghi_dung_file_va_validate_json() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("_meta")).unwrap();
    // JSON thiếu mảng "nguoi" -> từ chối
    let bad = super::brain_write_meta(
        tmp.path().to_string_lossy().to_string(),
        r#"{"vai_tro":{}}"#.to_string(),
    );
    assert!(bad.is_err());
    // JSON hợp lệ -> ghi OK
    let ok = super::brain_write_meta(
        tmp.path().to_string_lossy().to_string(),
        r#"{"nguoi":[]}"#.to_string(),
    );
    assert!(ok.is_ok());
    assert!(tmp.path().join("_meta/nguoi-dung.json").exists());
}

#[test]
fn test_create_nao_validate_va_tao_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("_meta")).unwrap();
    fs::create_dir_all(tmp.path().join("Nao Bo Phan")).unwrap();
    fs::write(
        tmp.path().join("_meta/nguoi-dung.json"),
        r#"{"nguoi":[], "nao_con": {"danh_sach": ["chung"]}}"#,
    )
    .unwrap();
    let root = tmp.path().to_string_lossy().to_string();
    // tên rỗng -> từ chối
    assert!(super::brain_create_nao(root.clone(), "!!!".into(), "".into()).is_err());
    // "Kho Vận" -> slug ASCII "khovn", tạo trong Nao Bo Phan
    assert_eq!(
        super::brain_create_nao(root.clone(), "Kho Vận".into(), "Nao Bo Phan".into()).unwrap(),
        "khovn"
    );
    assert!(tmp.path().join("Nao Bo Phan/khovn/1. Thu Thập").is_dir());
    assert!(tmp.path().join("Nao Bo Phan/khovn/2. Tinh Lọc/Kiến Thức Nguồn").is_dir());
    assert!(tmp.path().join("Nao Bo Phan/khovn/README.md").exists());
    // tạo trùng -> từ chối
    assert!(super::brain_create_nao(root.clone(), "khovn".into(), "Nao Bo Phan".into()).is_err());
    // parent ngoài root -> từ chối (traversal)
    assert!(super::brain_create_nao(root.clone(), "x".into(), "../ngoai".into()).is_err());
    // config đã thêm {id, path}
    let raw = fs::read_to_string(tmp.path().join("_meta/nguoi-dung.json")).unwrap();
    assert!(raw.contains("Nao Bo Phan/khovn"));
}

#[test]
fn test_create_ghinhanh_chi_trong_thu_thap() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("Nao Bo Phan/mkt/1. Thu Thập")).unwrap();
    let root = tmp.path().to_string_lossy().to_string();
    // ok
    let rel = super::brain_create_ghinhanh(root.clone(), "Nao Bo Phan/mkt".into(), "Kế hoạch A".into(), "nội dung".into()).unwrap();
    assert!(rel.ends_with("GHI NHANH — Kế hoạch A.md"));
    // trùng -> từ chối
    assert!(super::brain_create_ghinhanh(root.clone(), "Nao Bo Phan/mkt".into(), "Kế hoạch A".into(), "x".into()).is_err());
    // traversal -> từ chối
    assert!(super::brain_create_ghinhanh(root, "../ngoai".into(), "T".into(), "x".into()).is_err());
}

#[test]
fn test_search_tim_noi_dung_ton_trong_quyen() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("a.md"), "kế hoạch livestream cuối tuần").unwrap();
    fs::write(tmp.path().join("b.md"), "không liên quan").unwrap();
    fs::create_dir_all(tmp.path().join("_mat")).unwrap();
    fs::write(tmp.path().join("_mat/s.md"), "livestream bí mật").unwrap();
    let root = tmp.path().to_string_lossy().to_string();
    let hits = super::brain_search(root.clone(), "*".into(), "livestream".into()).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].rel_path.ends_with("a.md"));
    assert!(hits[0].snippet.contains("livestream"));
    // query ngắn -> rỗng
    assert!(super::brain_search(root, "*".into(), "a".into()).unwrap().is_empty());
}

#[test]
#[ignore] // chạy manual: cargo test verify_real_vault -- --ignored (cần vault thật trên máy)
fn verify_real_vault() {
    let root = "/Users/qthang/Library/CloudStorage/GoogleDrive-aios.msxgroup@gmail.com/Drive của tôi/MSXGROUP_AIOS_BRAIN";
    if !std::path::Path::new(root).exists() {
        eprintln!("vault không có trên máy này — bỏ qua");
        return;
    }
    let entries = filter_entries(std::path::Path::new(root), "*");
    let dirs: Vec<_> = entries.iter().filter(|e| e.is_dir).map(|e| e.name.clone()).collect();
    println!("tổng entries: {}", entries.len());
    println!("thư mục (12 đầu): {:?}", &dirs[..dirs.len().min(12)]);
    assert!(!dirs.iter().any(|d| d == "_mat" || d == ".git" || d == ".obsidian"), "_mat/.git/.obsidian lộ!");
    let r = super::brain_read_file(root.to_string(), "_meta/nguoi-dung.json".to_string(), "*".to_string());
    assert!(r.is_ok(), "không đọc được nguoi-dung.json: {:?}", r.err());
}
