# runesync

Deterministic dry-run-first directory synchronization.

## Why this exists

The current project fleet already covers agent frameworks, music software, games, privacy, sync, mobile, and archival tooling. `runesync` fills a narrower gap: a small local-first utility that can be audited in one sitting and composed with OpenShark, OpenShield, shell scripts, or other agents.

## v0 scope

- No network access.
- No external Rust dependencies.
- Deterministic output where the filesystem allows it.
- Plain text formats that can be reviewed in Git.
- Real unit tests, not placeholder stubs.

## Commands

### `plan` — dry-run preview (default safe mode)

Show what would be copied, deleted, or skipped. Never modifies anything.

```sh
runesync plan ~/Projects/active /mnt/backup/active
runesync plan ~/Projects/active /mnt/backup/active --delete
```

### `sync` — perform the synchronization

Requires `--execute` flag. Without it, sync refuses to run.

```sh
runesync sync ~/Projects/active /mnt/backup/active --execute
runesync sync ~/Projects/active /mnt/backup/active --execute --delete
```

### `check` — verify directories are in sync

Exit code 0 if identical, 1 if different. Useful in scripts and CI.

```sh
runesync check ~/Projects/active /mnt/backup/active
```

## Options

| Flag        | Effect                                              |
|-------------|-----------------------------------------------------|
| `--delete`  | Remove files in dest that don't exist in source     |
| `--execute` | Actually perform the sync (required by `sync`)      |

## Safety

- `plan` is always a dry-run — no changes are ever made.
- `sync` requires `--execute` to make any changes.
- Deletions require `--delete` flag.
- Symlinks are never followed.
- Output is deterministic (sorted, no timestamps).
- Files are compared by content hash, not just size or mtime.

## Architecture

- `src/lib.rs` — pure functions: scanning, hashing, diffing, planning. No side effects.
- `src/main.rs` — CLI parsing, validation, dispatch, and execution. Thin wrapper over lib.
- `tests/integration.rs` — integration tests using real temporary directories.

Content comparison uses FNV-1a 64-bit hashing — simple, deterministic, and dependency-free.

## Roadmap

- [x] Plan and explicit apply
- [ ] Ignore files
- [ ] Rename detection
- [ ] SSH transport behind a separate adapter

## Development

```sh
cargo fmt --check
cargo test
cargo run -- --help
```

## Safety

Local commits only. Never push or create remotes without explicit instruction. Do not weaken validation to make a failing test pass.

---
Made by [synth](https://github.com/synthalorian) with blackclaw ⚫🦞
