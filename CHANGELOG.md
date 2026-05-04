# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cargo workspace with three crates: `ldgr-core` (library), `ldgr-cli` (binary), `ldgr-server` (sync server)
- CLI skeleton with `init`, `unlock`, `lock`, and `status` subcommands (stubs)
- Core library with `crypto` and `storage` module placeholders
- Feature flags for WASM bundle control (`core`, `sync`, `import-export`, `market`, `loans`, `budget`, `goals`)
- Key hierarchy: Argon2id password derivation, HKDF domain-separated key derivation, and AES-256-GCM key wrapping with recovery key support
- Per-item envelope encryption with size-bucket padding (512 B / 2 KB / 8 KB / 32 KB)
- CI pipeline: build, test, clippy, formatting, and WASM smoke test on every PR
- Release pipeline: multi-platform binary builds and GitHub Releases on tag push
- Vault binary file format with `LDGR` magic bytes, versioned header, and encrypted metadata
- Vault operations: create, open (unlock with password), serialize (save), and validate
- Recovery key generation at vault creation with Crockford Base32 human-readable display
- Vault recovery flow: unlock with recovery key and set a new password
- Password change for unlocked vaults (re-wraps vault key, items untouched)
- SQLite storage layer with schema migration mechanism (`sqlite` feature flag)
- Account CRUD with hierarchical names, optimistic concurrency, and soft delete
- Transaction and posting CRUD with atomic writes and version tracking
- CLI commands: `ldgr init`, `ldgr unlock`, `ldgr lock`, `ldgr status`
- `--vault <PATH>` flag to specify a custom vault file path
- Session-based vault unlock with configurable timeout (`--timeout`, default 15 min)
- Recovery key displayed in a bordered box during vault creation
- SQLite database auto-initialized alongside the vault on `ldgr init`
