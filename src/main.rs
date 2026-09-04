//! runesync CLI - deterministic dry-run-first directory synchronization

use runesync::{diff_scans, plan_sync, scan_directory, SyncOp};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: {} <source> <dest> [--delete] [--execute]", args[0]);
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --delete   Remove files in dest that don't exist in source");
        eprintln!("  --execute  Actually perform the sync (default is dry-run)");
        return ExitCode::FAILURE;
    }

    let source = PathBuf::from(&args[1]);
    let dest = PathBuf::from(&args[2]);
    let delete = args.iter().any(|a| a == "--delete");
    let execute = args.iter().any(|a| a == "--execute");

    // Scan both directories
    let src_scan = match scan_directory(&source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error scanning source {}: {}", source.display(), e);
            return ExitCode::FAILURE;
        }
    };

    let dst_scan = match scan_directory(&dest) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error scanning dest {}: {}", dest.display(), e);
            return ExitCode::FAILURE;
        }
    };

    // Compute diff and plan
    let diffs = diff_scans(&src_scan, &dst_scan);
    let ops = plan_sync(diffs, delete);

    // Report
    let mut copies = 0;
    let mut deletes = 0;
    let mut skips = 0;

    for op in &ops {
        match op {
            SyncOp::Copy { path } => {
                println!("COPY: {}", path.display());
                copies += 1;
            }
            SyncOp::Delete { path } => {
                println!("DELETE: {}", path.display());
                deletes += 1;
            }
            SyncOp::Skip { path, reason } => {
                println!("SKIP: {} ({})", path.display(), reason);
                skips += 1;
            }
        }
    }

    println!();
    println!(
        "Summary: {} to copy, {} to delete, {} skipped",
        copies, deletes, skips
    );

    if !execute {
        println!();
        println!("Dry-run mode. Use --execute to perform the sync.");
        return ExitCode::SUCCESS;
    }

    // Execute the sync
    for op in ops {
        match op {
            SyncOp::Copy { path } => {
                let src_path = source.join(&path);
                let dst_path = dest.join(&path);
                if let Some(parent) = dst_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        eprintln!("Error creating directory {}: {}", parent.display(), e);
                        return ExitCode::FAILURE;
                    }
                }
                if let Err(e) = std::fs::copy(&src_path, &dst_path) {
                    eprintln!(
                        "Error copying {} -> {}: {}",
                        src_path.display(),
                        dst_path.display(),
                        e
                    );
                    return ExitCode::FAILURE;
                }
            }
            SyncOp::Delete { path } => {
                let dst_path = dest.join(&path);
                if let Err(e) = std::fs::remove_file(&dst_path) {
                    eprintln!("Error deleting {}: {}", dst_path.display(), e);
                    return ExitCode::FAILURE;
                }
            }
            SyncOp::Skip { .. } => {}
        }
    }

    println!("Sync complete.");
    ExitCode::SUCCESS
}
