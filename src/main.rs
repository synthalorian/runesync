//! runesync CLI — deterministic dry-run-first directory synchronization

use runesync::{OpSummary, SyncOp, diff_scans, plan_sync, scan_directory};
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_usage();
        return ExitCode::FAILURE;
    }

    match args[0].as_str() {
        "plan" => cmd_plan(&args[1..]),
        "sync" => cmd_sync(&args[1..]),
        "check" => cmd_check(&args[1..]),
        "--help" | "-h" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        "--version" | "-V" | "version" => {
            println!("runesync {VERSION}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("error: unknown command '{other}'");
            eprintln!("run 'runesync --help' for usage");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// Usage / help
// ---------------------------------------------------------------------------

fn print_usage() {
    println!("runesync {VERSION} — deterministic dry-run-first directory synchronization");
    println!();
    println!("USAGE:");
    println!("    runesync <COMMAND> <SOURCE> <DEST> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    plan     Show what would be copied, deleted, or skipped (dry-run)");
    println!("    sync     Perform the synchronization (requires --execute)");
    println!("    check    Verify two directories are in sync (exit 0=yes, 1=no)");
    println!();
    println!("OPTIONS:");
    println!("    --delete    Remove files in dest that don't exist in source");
    println!("    --execute   Actually perform the sync (sync command only)");
    println!();
    println!("EXAMPLES:");
    println!("    runesync plan ~/Projects/active /mnt/backup/active");
    println!("    runesync sync ~/Projects/active /mnt/backup/active --execute");
    println!("    runesync sync ~/Projects/active /mnt/backup/active --execute --delete");
    println!("    runesync check ~/Projects/active /mnt/backup/active");
    println!();
    println!("SAFETY:");
    println!("    - plan is always a dry-run (no changes made)");
    println!("    - sync requires --execute to make any changes");
    println!("    - deletions require --delete flag");
    println!("    - symlinks are never followed");
    println!("    - output is deterministic (sorted, no timestamps)");
}

// ---------------------------------------------------------------------------
// Argument parsing helpers
// ---------------------------------------------------------------------------

struct ParsedArgs {
    source: PathBuf,
    dest: PathBuf,
    delete: bool,
    execute: bool,
}

fn parse_sync_args(args: &[String], cmd: &str) -> Result<ParsedArgs, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut delete = false;
    let mut execute = false;

    for arg in args {
        match arg.as_str() {
            "--delete" => delete = true,
            "--execute" => execute = true,
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            s if s.starts_with("--") => {
                return Err(format!("unknown flag '{s}'"));
            }
            s => positional.push(s),
        }
    }

    if positional.len() != 2 {
        return Err(format!(
            "'{cmd}' requires exactly 2 paths: <source> <dest>\n\
             run 'runesync --help' for usage"
        ));
    }

    Ok(ParsedArgs {
        source: PathBuf::from(positional[0]),
        dest: PathBuf::from(positional[1]),
        delete,
        execute,
    })
}

fn validate_source(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("source does not exist: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("source is not a directory: {}", path.display()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_plan(args: &[String]) -> ExitCode {
    let parsed = match parse_sync_args(args, "plan") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = validate_source(&parsed.source) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }

    let ops = match compute_plan(&parsed.source, &parsed.dest, parsed.delete) {
        Ok(ops) => ops,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    print_ops(&ops);
    let summary = OpSummary::from_ops(&ops);
    println!();
    println!("Plan: {summary}");

    if !summary.in_sync() {
        println!();
        println!("Dry-run only. Use 'runesync sync --execute' to apply changes.");
    } else {
        println!();
        println!("Directories are in sync.");
    }

    ExitCode::SUCCESS
}

fn cmd_sync(args: &[String]) -> ExitCode {
    let parsed = match parse_sync_args(args, "sync") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = validate_source(&parsed.source) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }

    if !parsed.execute {
        eprintln!("error: 'sync' requires --execute flag to make changes");
        eprintln!("use 'runesync plan' to preview changes without modifying anything");
        return ExitCode::FAILURE;
    }

    let ops = match compute_plan(&parsed.source, &parsed.dest, parsed.delete) {
        Ok(ops) => ops,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    print_ops(&ops);
    let summary = OpSummary::from_ops(&ops);
    println!();
    println!("Sync: {summary}");

    if summary.in_sync() {
        println!("Directories are already in sync.");
        return ExitCode::SUCCESS;
    }

    // Execute the operations.
    match execute_ops(&ops, &parsed.source, &parsed.dest) {
        Ok(()) => {
            println!("Sync complete.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error during sync: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_check(args: &[String]) -> ExitCode {
    let parsed = match parse_sync_args(args, "check") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = validate_source(&parsed.source) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }

    // For check, we compare raw diffs — not planned ops.
    // Extra files in dest mean out-of-sync regardless of --delete.
    let src_scan = match scan_directory(&parsed.source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot scan source {}: {e}", parsed.source.display());
            return ExitCode::FAILURE;
        }
    };
    let dst_scan = match scan_directory(&parsed.dest) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot scan dest {}: {e}", parsed.dest.display());
            return ExitCode::FAILURE;
        }
    };

    let diffs = diff_scans(&src_scan, &dst_scan);
    let total = diffs.len();
    let identical = diffs
        .iter()
        .filter(|d| matches!(d, runesync::Diff::Identical { .. }))
        .count();

    if identical == total {
        println!("IN SYNC  {identical} files match");
        ExitCode::SUCCESS
    } else {
        for d in &diffs {
            println!("{d}");
        }
        println!();
        println!("OUT OF SYNC  {identical}/{total} files match");
        ExitCode::FAILURE
    }
}

// ---------------------------------------------------------------------------
// Core logic (thin wrappers over lib functions)
// ---------------------------------------------------------------------------

fn compute_plan(source: &Path, dest: &Path, delete: bool) -> Result<Vec<SyncOp>, String> {
    let src_scan = scan_directory(source)
        .map_err(|e| format!("cannot scan source {}: {e}", source.display()))?;
    let dst_scan =
        scan_directory(dest).map_err(|e| format!("cannot scan dest {}: {e}", dest.display()))?;
    let diffs = diff_scans(&src_scan, &dst_scan);
    Ok(plan_sync(diffs, delete))
}

fn print_ops(ops: &[SyncOp]) {
    for op in ops {
        println!("{op}");
    }
}

fn execute_ops(ops: &[SyncOp], source: &Path, dest: &Path) -> Result<(), String> {
    for op in ops {
        match op {
            SyncOp::Copy { path } => {
                let src_path = source.join(path);
                let dst_path = dest.join(path);

                // Safety: verify the destination path is actually under dest.
                if !dst_path.starts_with(dest) {
                    return Err(format!(
                        "refusing to write outside dest: {}",
                        dst_path.display()
                    ));
                }

                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!("cannot create directory {}: {e}", parent.display())
                    })?;
                }
                std::fs::copy(&src_path, &dst_path).map_err(|e| {
                    format!(
                        "cannot copy {} -> {}: {e}",
                        src_path.display(),
                        dst_path.display()
                    )
                })?;
            }
            SyncOp::Delete { path } => {
                let dst_path = dest.join(path);

                // Safety: verify the path is actually under dest.
                if !dst_path.starts_with(dest) {
                    return Err(format!(
                        "refusing to delete outside dest: {}",
                        dst_path.display()
                    ));
                }

                std::fs::remove_file(&dst_path)
                    .map_err(|e| format!("cannot delete {}: {e}", dst_path.display()))?;
            }
            SyncOp::Skip { .. } => {}
        }
    }
    Ok(())
}
