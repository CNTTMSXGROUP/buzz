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
