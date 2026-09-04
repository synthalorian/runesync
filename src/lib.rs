//! runesync - deterministic dry-run-first directory synchronization
//!
//! Pure functions for scanning directories, comparing state, and planning
//! sync operations. No side effects in this module.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A file entry discovered during directory scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative path from scan root.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Content hash (FNV-1a 64-bit) for change detection.
    pub content_hash: u64,
}

/// Difference between source and destination for a single path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diff {
    /// File exists in source but not destination.
    MissingInDest { path: PathBuf },
    /// File exists in destination but not source.
    MissingInSource { path: PathBuf },
    /// File exists in both but content differs.
    ContentDiffers { path: PathBuf },
    /// File is identical in both.
    Identical { path: PathBuf },
}

impl fmt::Display for Diff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Diff::MissingInDest { path } => write!(f, "NEW      {}", path.display()),
            Diff::MissingInSource { path } => write!(f, "EXTRA    {}", path.display()),
            Diff::ContentDiffers { path } => write!(f, "CHANGED  {}", path.display()),
            Diff::Identical { path } => write!(f, "SAME     {}", path.display()),
        }
    }
}

/// A planned sync operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOp {
    /// Copy file from source to destination.
    Copy { path: PathBuf },
    /// Delete file from destination.
    Delete { path: PathBuf },
    /// Skip file with a reason.
    Skip { path: PathBuf, reason: String },
}

impl fmt::Display for SyncOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncOp::Copy { path } => write!(f, "COPY     {}", path.display()),
            SyncOp::Delete { path } => write!(f, "DELETE   {}", path.display()),
            SyncOp::Skip { path, reason } => {
                write!(f, "SKIP     {} ({})", path.display(), reason)
            }
        }
    }
}

/// Summary counts for a set of sync operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpSummary {
    pub copies: usize,
    pub deletes: usize,
    pub skips: usize,
}

impl OpSummary {
    pub fn from_ops(ops: &[SyncOp]) -> Self {
        let mut copies = 0;
        let mut deletes = 0;
        let mut skips = 0;
        for op in ops {
            match op {
                SyncOp::Copy { .. } => copies += 1,
                SyncOp::Delete { .. } => deletes += 1,
                SyncOp::Skip { .. } => skips += 1,
            }
        }
        OpSummary {
            copies,
            deletes,
            skips,
        }
    }

    /// True if no changes are needed (all skips).
    pub fn in_sync(&self) -> bool {
        self.copies == 0 && self.deletes == 0
    }
}

impl fmt::Display for OpSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} to copy, {} to delete, {} skipped",
            self.copies, self.deletes, self.skips
        )
    }
}

// ---------------------------------------------------------------------------
// Content hashing (FNV-1a 64-bit — simple, deterministic, no dependencies)
// ---------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Compute FNV-1a 64-bit hash of a byte slice.
pub fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Compute FNV-1a 64-bit hash of a file's contents.
pub fn hash_file(path: &Path) -> std::io::Result<u64> {
    let data = std::fs::read(path)?;
    Ok(fnv1a_hash(&data))
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Scan a directory recursively and return all regular files.
///
/// - Does NOT follow symlinks (symlinks are skipped with an entry in `warnings`).
/// - Returns entries sorted by relative path (BTreeMap guarantees order).
/// - If root does not exist, returns an empty map (not an error).
pub fn scan_directory(root: &Path) -> std::io::Result<BTreeMap<PathBuf, FileEntry>> {
    let mut entries = BTreeMap::new();
    if !root.exists() {
        return Ok(entries);
    }
    if !root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a directory: {}", root.display()),
        ));
    }
    scan_recursive(root, root, &mut entries)?;
    Ok(entries)
}

fn scan_recursive(
    root: &Path,
    current: &Path,
    entries: &mut BTreeMap<PathBuf, FileEntry>,
) -> std::io::Result<()> {
    let mut children: Vec<_> = std::fs::read_dir(current)?.collect::<Result<_, _>>()?;
    // Sort by file name for deterministic traversal order.
    children.sort_by_key(|e| e.file_name());

    for entry in children {
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            // Do not follow symlinks — skip silently.
            continue;
        }

        if file_type.is_dir() {
            scan_recursive(root, &path, entries)?;
        } else if file_type.is_file() {
            let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let metadata = entry.metadata()?;
            let content_hash = hash_file(&path)?;
            entries.insert(
                rel_path.clone(),
                FileEntry {
                    path: rel_path,
                    size: metadata.len(),
                    content_hash,
                },
            );
        }
        // Other file types (sockets, devices, etc.) are silently skipped.
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Diffing
// ---------------------------------------------------------------------------

/// Compare source and destination scans, return differences.
///
/// Output is deterministic: sorted by path (BTreeMap iteration order).
pub fn diff_scans(
    source: &BTreeMap<PathBuf, FileEntry>,
    dest: &BTreeMap<PathBuf, FileEntry>,
) -> Vec<Diff> {
    let mut diffs = Vec::new();

    // Check source files against dest.
    for (path, src_entry) in source {
        match dest.get(path) {
            None => diffs.push(Diff::MissingInDest { path: path.clone() }),
            Some(dst_entry) => {
                if src_entry.size != dst_entry.size
                    || src_entry.content_hash != dst_entry.content_hash
                {
                    diffs.push(Diff::ContentDiffers { path: path.clone() });
                } else {
                    diffs.push(Diff::Identical { path: path.clone() });
                }
            }
        }
    }

    // Check for files only in dest.
    for path in dest.keys() {
        if !source.contains_key(path) {
            diffs.push(Diff::MissingInSource { path: path.clone() });
        }
    }

    // Sort by path for fully deterministic output.
    diffs.sort_by(|a, b| {
        let pa = match a {
            Diff::MissingInDest { path }
            | Diff::MissingInSource { path }
            | Diff::ContentDiffers { path }
            | Diff::Identical { path } => path,
        };
        let pb = match b {
            Diff::MissingInDest { path }
            | Diff::MissingInSource { path }
            | Diff::ContentDiffers { path }
            | Diff::Identical { path } => path,
        };
        pa.cmp(pb)
    });

    diffs
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Plan sync operations from diffs.
///
/// - `delete_extraneous`: if true, files only in dest are marked for deletion.
///   If false, they are skipped with a reason.
pub fn plan_sync(diffs: Vec<Diff>, delete_extraneous: bool) -> Vec<SyncOp> {
    diffs
        .into_iter()
        .map(|d| match d {
            Diff::MissingInDest { path } => SyncOp::Copy { path },
            Diff::ContentDiffers { path } => SyncOp::Copy { path },
            Diff::MissingInSource { path } => {
                if delete_extraneous {
                    SyncOp::Delete { path }
                } else {
                    SyncOp::Skip {
                        path,
                        reason: "exists in dest only (use --delete to remove)".to_string(),
                    }
                }
            }
            Diff::Identical { path } => SyncOp::Skip {
                path,
                reason: "identical".to_string(),
            },
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64, hash: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            size,
            content_hash: hash,
        }
    }

    fn make_scan(entries: Vec<(&str, u64, u64)>) -> BTreeMap<PathBuf, FileEntry> {
        entries
            .into_iter()
            .map(|(p, s, h)| (PathBuf::from(p), entry(p, s, h)))
            .collect()
    }

    // -- FNV-1a hash tests ---------------------------------------------------

    #[test]
    fn fnv1a_empty() {
        assert_eq!(fnv1a_hash(b""), FNV_OFFSET);
    }

    #[test]
    fn fnv1a_deterministic() {
        let h1 = fnv1a_hash(b"hello");
        let h2 = fnv1a_hash(b"hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn fnv1a_different_inputs() {
        assert_ne!(fnv1a_hash(b"hello"), fnv1a_hash(b"world"));
    }

    // -- diff_scans tests ----------------------------------------------------

    #[test]
    fn diff_missing_in_dest() {
        let src = make_scan(vec![("file.txt", 100, 111)]);
        let dst = make_scan(vec![]);
        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0], Diff::MissingInDest { .. }));
    }

    #[test]
    fn diff_missing_in_source() {
        let src = make_scan(vec![]);
        let dst = make_scan(vec![("old.txt", 50, 222)]);
        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0], Diff::MissingInSource { .. }));
    }

    #[test]
    fn diff_identical() {
        let src = make_scan(vec![("file.txt", 100, 111)]);
        let dst = make_scan(vec![("file.txt", 100, 111)]);
        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0], Diff::Identical { .. }));
    }

    #[test]
    fn diff_content_differs_by_hash() {
        let src = make_scan(vec![("file.txt", 100, 111)]);
        let dst = make_scan(vec![("file.txt", 100, 999)]);
        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0], Diff::ContentDiffers { .. }));
    }

    #[test]
    fn diff_content_differs_by_size() {
        let src = make_scan(vec![("file.txt", 100, 111)]);
        let dst = make_scan(vec![("file.txt", 200, 111)]);
        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0], Diff::ContentDiffers { .. }));
    }

    #[test]
    fn diff_sorted_output() {
        let src = make_scan(vec![("b.txt", 1, 1), ("a.txt", 1, 1)]);
        let dst = make_scan(vec![]);
        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 2);
        // Should be sorted by path.
        if let Diff::MissingInDest { path } = &diffs[0] {
            assert_eq!(path, &PathBuf::from("a.txt"));
        } else {
            panic!("expected MissingInDest");
        }
    }

    // -- plan_sync tests -----------------------------------------------------

    #[test]
    fn plan_copy_for_new() {
        let diffs = vec![Diff::MissingInDest {
            path: PathBuf::from("new.txt"),
        }];
        let ops = plan_sync(diffs, false);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SyncOp::Copy { .. }));
    }

    #[test]
    fn plan_copy_for_changed() {
        let diffs = vec![Diff::ContentDiffers {
            path: PathBuf::from("changed.txt"),
        }];
        let ops = plan_sync(diffs, false);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SyncOp::Copy { .. }));
    }

    #[test]
    fn plan_skip_for_identical() {
        let diffs = vec![Diff::Identical {
            path: PathBuf::from("same.txt"),
        }];
        let ops = plan_sync(diffs, false);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SyncOp::Skip { .. }));
    }

    #[test]
    fn plan_skip_for_extra_without_delete() {
        let diffs = vec![Diff::MissingInSource {
            path: PathBuf::from("old.txt"),
        }];
        let ops = plan_sync(diffs, false);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SyncOp::Skip { .. }));
    }

    #[test]
    fn plan_delete_for_extra_with_delete() {
        let diffs = vec![Diff::MissingInSource {
            path: PathBuf::from("old.txt"),
        }];
        let ops = plan_sync(diffs, true);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SyncOp::Delete { .. }));
    }

    // -- OpSummary tests -----------------------------------------------------

    #[test]
    fn summary_counts() {
        let ops = vec![
            SyncOp::Copy {
                path: PathBuf::from("a"),
            },
            SyncOp::Copy {
                path: PathBuf::from("b"),
            },
            SyncOp::Delete {
                path: PathBuf::from("c"),
            },
            SyncOp::Skip {
                path: PathBuf::from("d"),
                reason: "identical".into(),
            },
        ];
        let s = OpSummary::from_ops(&ops);
        assert_eq!(s.copies, 2);
        assert_eq!(s.deletes, 1);
        assert_eq!(s.skips, 1);
        assert!(!s.in_sync());
    }

    #[test]
    fn summary_in_sync() {
        let ops = vec![SyncOp::Skip {
            path: PathBuf::from("a"),
            reason: "identical".into(),
        }];
        let s = OpSummary::from_ops(&ops);
        assert!(s.in_sync());
    }

    // -- Display tests -------------------------------------------------------

    #[test]
    fn diff_display() {
        let d = Diff::MissingInDest {
            path: PathBuf::from("foo/bar.txt"),
        };
        assert_eq!(format!("{d}"), "NEW      foo/bar.txt");
    }

    #[test]
    fn syncop_display() {
        let op = SyncOp::Copy {
            path: PathBuf::from("a.txt"),
        };
        assert_eq!(format!("{op}"), "COPY     a.txt");
    }

    #[test]
    fn summary_display() {
        let s = OpSummary {
            copies: 3,
            deletes: 1,
            skips: 5,
        };
        assert_eq!(format!("{s}"), "3 to copy, 1 to delete, 5 skipped");
    }
}
