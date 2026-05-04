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
