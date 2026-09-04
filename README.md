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

```sh
runesync plan ~/Projects/active /mnt/backup/active
```
```sh
runesync apply ~/Projects/active /mnt/backup/active --yes
```

## Architecture

`src/main.rs` contains the complete v0 implementation: parsing, validation, pure core functions, CLI dispatch, and unit tests. The next extraction boundary is a `core` module once the format stabilizes; until then, keeping the tape on one reel makes audits cheap.

## Roadmap

- [ ] Plan and explicit apply
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
Made by [synth](https://github.com/synthalorian) with synthclaw 🎹🦞
