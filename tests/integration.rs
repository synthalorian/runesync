//! Integration tests for runesync using real temporary directories.

use runesync::{OpSummary, SyncOp, diff_scans, plan_sync, scan_directory};
use std::fs;
use std::path::{Path, PathBuf};

/// Create a temporary directory structure for testing.
/// Returns (temp_dir_path, src_path, dst_path).
fn setup_test_dirs(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("runesync-test-{name}"));
    let src = base.join("src");
    let dst = base.join("dst");

    // Clean up from previous runs.
    let _ = fs::remove_dir_all(&base);

    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&dst).unwrap();

    (base, src, dst)
}

fn cleanup(base: &Path) {
    let _ = fs::remove_dir_all(base);
}

fn write_file(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

// ---------------------------------------------------------------------------
// scan_directory
// ---------------------------------------------------------------------------

#[test]
fn scan_empty_directory() {
    let (base, src, _) = setup_test_dirs("scan-empty");
    let scan = scan_directory(&src).unwrap();
    assert!(scan.is_empty());
    cleanup(&base);
}

#[test]
fn scan_finds_files() {
    let (base, src, _) = setup_test_dirs("scan-finds");
    write_file(&src, "a.txt", "hello");
    write_file(&src, "sub/b.txt", "world");

    let scan = scan_directory(&src).unwrap();
    assert_eq!(scan.len(), 2);
    assert!(scan.contains_key(&PathBuf::from("a.txt")));
    assert!(scan.contains_key(&PathBuf::from("sub/b.txt")));
    cleanup(&base);
}

#[test]
fn scan_nonexistent_returns_empty() {
    let scan = scan_directory(Path::new("/nonexistent/path/that/does/not/exist")).unwrap();
    assert!(scan.is_empty());
}

#[test]
fn scan_file_returns_error() {
    let (base, src, _) = setup_test_dirs("scan-file");
    let file_path = src.join("not-a-dir.txt");
    fs::write(&file_path, "hello").unwrap();

    let result = scan_directory(&file_path);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not a directory"), "unexpected error: {err}");
    cleanup(&base);
}

#[test]
fn scan_skips_symlinks() {
    let (base, src, _) = setup_test_dirs("scan-symlink");
    write_file(&src, "real.txt", "real content");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src.join("real.txt"), src.join("link.txt")).unwrap();
        let scan = scan_directory(&src).unwrap();
        assert_eq!(scan.len(), 1);
        assert!(scan.contains_key(&PathBuf::from("real.txt")));
        assert!(!scan.contains_key(&PathBuf::from("link.txt")));
    }

    cleanup(&base);
}

#[test]
fn scan_deterministic_order() {
    let (base, src, _) = setup_test_dirs("scan-order");
    write_file(&src, "c.txt", "c");
    write_file(&src, "a.txt", "a");
    write_file(&src, "b.txt", "b");

    let scan1 = scan_directory(&src).unwrap();
    let scan2 = scan_directory(&src).unwrap();

    let keys1: Vec<_> = scan1.keys().collect();
    let keys2: Vec<_> = scan2.keys().collect();
    assert_eq!(keys1, keys2);

    // Verify sorted order.
    assert_eq!(keys1[0], &PathBuf::from("a.txt"));
    assert_eq!(keys1[1], &PathBuf::from("b.txt"));
    assert_eq!(keys1[2], &PathBuf::from("c.txt"));
    cleanup(&base);
}

// ---------------------------------------------------------------------------
// diff_scans with real files
// ---------------------------------------------------------------------------

#[test]
fn diff_real_directories() {
    let (base, src, dst) = setup_test_dirs("diff-real");

    write_file(&src, "same.txt", "same content");
    write_file(&src, "new.txt", "new file");
    write_file(&src, "changed.txt", "version 2");
    write_file(&dst, "same.txt", "same content");
    write_file(&dst, "changed.txt", "version 1");
    write_file(&dst, "extra.txt", "only in dst");

    let src_scan = scan_directory(&src).unwrap();
    let dst_scan = scan_directory(&dst).unwrap();
    let diffs = diff_scans(&src_scan, &dst_scan);

    assert_eq!(diffs.len(), 4);

    // Sorted by path: changed.txt, extra.txt, new.txt, same.txt
    assert!(
        matches!(&diffs[0], runesync::Diff::ContentDiffers { path } if path == &PathBuf::from("changed.txt"))
    );
    assert!(
        matches!(&diffs[1], runesync::Diff::MissingInSource { path } if path == &PathBuf::from("extra.txt"))
    );
    assert!(
        matches!(&diffs[2], runesync::Diff::MissingInDest { path } if path == &PathBuf::from("new.txt"))
    );
    assert!(
        matches!(&diffs[3], runesync::Diff::Identical { path } if path == &PathBuf::from("same.txt"))
    );

    cleanup(&base);
}

// ---------------------------------------------------------------------------
// plan_sync with real files
// ---------------------------------------------------------------------------

#[test]
fn plan_no_delete() {
    let (base, src, dst) = setup_test_dirs("plan-no-del");

    write_file(&src, "new.txt", "new");
    write_file(&dst, "extra.txt", "extra");

    let src_scan = scan_directory(&src).unwrap();
    let dst_scan = scan_directory(&dst).unwrap();
    let diffs = diff_scans(&src_scan, &dst_scan);
    let ops = plan_sync(diffs, false);

    assert_eq!(ops.len(), 2);
    assert!(matches!(&ops[0], SyncOp::Skip { path, .. } if path == &PathBuf::from("extra.txt")));
    assert!(matches!(&ops[1], SyncOp::Copy { path } if path == &PathBuf::from("new.txt")));

    cleanup(&base);
}

#[test]
fn plan_with_delete() {
    let (base, src, dst) = setup_test_dirs("plan-del");

    write_file(&src, "new.txt", "new");
    write_file(&dst, "extra.txt", "extra");

    let src_scan = scan_directory(&src).unwrap();
    let dst_scan = scan_directory(&dst).unwrap();
    let diffs = diff_scans(&src_scan, &dst_scan);
    let ops = plan_sync(diffs, true);

    assert_eq!(ops.len(), 2);
    assert!(matches!(&ops[0], SyncOp::Delete { path } if path == &PathBuf::from("extra.txt")));
    assert!(matches!(&ops[1], SyncOp::Copy { path } if path == &PathBuf::from("new.txt")));

    cleanup(&base);
}

// ---------------------------------------------------------------------------
// OpSummary with real scenario
// ---------------------------------------------------------------------------

#[test]
fn summary_real_scenario() {
    let (base, src, dst) = setup_test_dirs("summary-real");

    write_file(&src, "same.txt", "same");
    write_file(&src, "new.txt", "new");
    write_file(&dst, "same.txt", "same");
    write_file(&dst, "extra.txt", "extra");

    let src_scan = scan_directory(&src).unwrap();
    let dst_scan = scan_directory(&dst).unwrap();
    let diffs = diff_scans(&src_scan, &dst_scan);
    let ops = plan_sync(diffs, false);
    let summary = OpSummary::from_ops(&ops);

    assert_eq!(summary.copies, 1);
    assert_eq!(summary.deletes, 0);
    assert_eq!(summary.skips, 2); // same.txt (identical) + extra.txt (no --delete)
    assert!(!summary.in_sync());

    cleanup(&base);
}

#[test]
fn summary_in_sync_scenario() {
    let (base, src, dst) = setup_test_dirs("summary-sync");

    write_file(&src, "a.txt", "aaa");
    write_file(&dst, "a.txt", "aaa");

    let src_scan = scan_directory(&src).unwrap();
    let dst_scan = scan_directory(&dst).unwrap();
    let diffs = diff_scans(&src_scan, &dst_scan);
    let ops = plan_sync(diffs, false);
    let summary = OpSummary::from_ops(&ops);

    assert!(summary.in_sync());
    assert_eq!(summary.copies, 0);
    assert_eq!(summary.deletes, 0);
    assert_eq!(summary.skips, 1);

    cleanup(&base);
}

// ---------------------------------------------------------------------------
// Content hash with real files
// ---------------------------------------------------------------------------

#[test]
fn content_hash_detects_changes() {
    let (base, src, _) = setup_test_dirs("hash-detect");

    write_file(&src, "file.txt", "version 1");
    let scan1 = scan_directory(&src).unwrap();
    let hash1 = scan1[&PathBuf::from("file.txt")].content_hash;

    write_file(&src, "file.txt", "version 2");
    let scan2 = scan_directory(&src).unwrap();
    let hash2 = scan2[&PathBuf::from("file.txt")].content_hash;

    assert_ne!(hash1, hash2);
    cleanup(&base);
}

#[test]
fn content_hash_same_for_same_content() {
    let (base, src, _) = setup_test_dirs("hash-same");

    write_file(&src, "a.txt", "identical content");
    write_file(&src, "b.txt", "identical content");

    let scan = scan_directory(&src).unwrap();
    let hash_a = scan[&PathBuf::from("a.txt")].content_hash;
    let hash_b = scan[&PathBuf::from("b.txt")].content_hash;

    assert_eq!(hash_a, hash_b);
    cleanup(&base);
}
