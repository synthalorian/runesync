//! runesync - deterministic dry-run-first directory synchronization
//!
//! Pure functions for comparing directories and planning sync operations.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A file entry discovered during directory scan
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

/// Difference between source and destination
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diff {
    /// File exists in source but not destination
    MissingInDest { path: PathBuf },
    /// File exists in destination but not source
    MissingInSource { path: PathBuf },
    /// File exists in both but differs
    ContentDiffers { path: PathBuf, src_size: u64, dst_size: u64 },
    /// File is identical
    Identical { path: PathBuf },
}

/// Scan a directory recursively and return all files
pub fn scan_directory(root: &Path) -> std::io::Result<BTreeMap<PathBuf, FileEntry>> {
    let mut entries = BTreeMap::new();
    if !root.exists() {
        return Ok(entries);
    }
    scan_recursive(root, root, &mut entries)?;
    Ok(entries)
}

fn scan_recursive(
    root: &Path,
    current: &Path,
    entries: &mut BTreeMap<PathBuf, FileEntry>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            scan_recursive(root, &path, entries)?;
        } else if metadata.is_file() {
            let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let mtime = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            entries.insert(
                rel_path.clone(),
                FileEntry {
                    path: rel_path,
                    size: metadata.len(),
                    mtime,
                },
            );
        }
    }
    Ok(())
}

/// Compare source and destination scans, return differences
pub fn diff_scans(
    source: &BTreeMap<PathBuf, FileEntry>,
    dest: &BTreeMap<PathBuf, FileEntry>,
) -> Vec<Diff> {
    let mut diffs = Vec::new();

    // Check source files against dest
    for (path, src_entry) in source {
        match dest.get(path) {
            None => diffs.push(Diff::MissingInDest {
                path: path.clone(),
            }),
            Some(dst_entry) => {
                if src_entry.size != dst_entry.size {
                    diffs.push(Diff::ContentDiffers {
                        path: path.clone(),
                        src_size: src_entry.size,
                        dst_size: dst_entry.size,
                    });
                } else {
                    diffs.push(Diff::Identical {
                        path: path.clone(),
                    });
                }
            }
        }
    }

    // Check for files only in dest
    for path in dest.keys() {
        if !source.contains_key(path) {
            diffs.push(Diff::MissingInSource {
                path: path.clone(),
            });
        }
    }

    diffs
}

/// Plan sync operations from diffs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOp {
    Copy { path: PathBuf },
    Delete { path: PathBuf },
    Skip { path: PathBuf, reason: String },
}

pub fn plan_sync(diffs: Vec<Diff>, delete_extraneous: bool) -> Vec<SyncOp> {
    diffs
        .into_iter()
        .map(|d| match d {
            Diff::MissingInDest { path } => SyncOp::Copy { path },
            Diff::ContentDiffers { path, .. } => SyncOp::Copy { path },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_missing_in_dest() {
        let mut src = BTreeMap::new();
        src.insert(
            PathBuf::from("file.txt"),
            FileEntry {
                path: PathBuf::from("file.txt"),
                size: 100,
                mtime: 12345,
            },
        );
        let dst = BTreeMap::new();

        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0], Diff::MissingInDest { .. }));
    }

    #[test]
    fn test_diff_identical() {
        let mut src = BTreeMap::new();
        src.insert(
            PathBuf::from("file.txt"),
            FileEntry {
                path: PathBuf::from("file.txt"),
                size: 100,
                mtime: 12345,
            },
        );
        let mut dst = BTreeMap::new();
        dst.insert(
            PathBuf::from("file.txt"),
            FileEntry {
                path: PathBuf::from("file.txt"),
                size: 100,
                mtime: 12345,
            },
        );

        let diffs = diff_scans(&src, &dst);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0], Diff::Identical { .. }));
    }

    #[test]
    fn test_plan_sync_no_delete() {
        let diffs = vec![
            Diff::MissingInDest {
                path: PathBuf::from("new.txt"),
            },
            Diff::MissingInSource {
                path: PathBuf::from("old.txt"),
            },
        ];
        let ops = plan_sync(diffs, false);
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], SyncOp::Copy { .. }));
        assert!(matches!(ops[1], SyncOp::Skip { .. }));
    }

    #[test]
    fn test_plan_sync_with_delete() {
        let diffs = vec![Diff::MissingInSource {
            path: PathBuf::from("old.txt"),
        }];
        let ops = plan_sync(diffs, true);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SyncOp::Delete { .. }));
    }
}
