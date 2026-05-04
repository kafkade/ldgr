# Contributing to ldgr

Thank you for your interest in contributing to ldgr! This document covers how to
build the project, our development workflow, and contribution requirements.

## Prerequisites

- **Rust 1.85+** — install via [rustup](https://rustup.rs/)
- **wasm-pack** (optional) — for WASM builds: `cargo install wasm-pack`

## Building from Source

```sh
# Clone the repository
git clone https://github.com/kafkade/ldgr.git
cd ldgr

# Build all crates
cargo build --workspace

# Build with all features (includes SQLite storage)
cargo build -p ldgr-core --features full

# Build the CLI
cargo build -p ldgr-cli

# Build WASM (requires wasm-pack)
wasm-pack build crates/ldgr-core --target web --features core
```

## Running Tests

```sh
# Run all tests
cargo test --workspace

# Run tests for a specific crate
cargo test -p ldgr-core

# Run tests with SQLite feature
cargo test -p ldgr-core --features sqlite

# Run a single test by name
cargo test -p ldgr-core test_name

# Run tests for a specific module
cargo test -p ldgr-core crypto::
```

## Code Quality

All code must pass these checks before merging:

```sh
# Formatting (rustfmt)
cargo fmt --check
# Fix formatting issues:
cargo fmt

# Linting (clippy)
cargo clippy --workspace --all-targets -- -D warnings

# All checks run in CI on every pull request
```

## Development Workflow

1. **Fork and clone** the repository.
2. **Create a feature branch** from `main`:

   ```sh
   git checkout -b feat/my-feature
   ```

3. **Make your changes** and ensure all checks pass.
4. **Sign off your commits** (DCO requirement — see below):

   ```sh
   git commit -s -m "feat: add my feature"
   ```

5. **Open a pull request** against `main`.

### Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` — new feature
- `fix:` — bug fix
- `docs:` — documentation changes
- `test:` — adding or updating tests
- `refactor:` — code restructuring without behavior change
- `chore:` — maintenance tasks (CI, dependencies, etc.)

For multi-component changes, include the component:
`feat(crypto): add vault key rotation`

### Pull Request Checklist

- [ ] Tests pass (`cargo test --workspace`)
- [ ] Clippy passes (`cargo clippy --workspace -- -D warnings`)
- [ ] Formatting passes (`cargo fmt --check`)
- [ ] Commits are signed off (DCO)
- [ ] PR description follows the template

## Architecture Guidelines

### ldgr-core Must Have Zero I/O

The core library (`crates/ldgr-core/`) must not depend on networking, file system
access, or platform-specific APIs. All I/O happens in platform-specific code
(CLI, iOS, web). This keeps the core testable and compilable to WASM.

### Decimal Arithmetic

All monetary amounts use `rust_decimal::Decimal`, stored as TEXT in SQLite.
Never use floating-point for financial calculations.

### Error Handling

- Use `thiserror` for library errors in `ldgr-core`.
- Use `anyhow` only in binary crates (`ldgr-cli`, `ldgr-server`).
- Crypto failures must never expose key material in error messages.
- Import errors must include file/line context.

### Encryption

- All encryption uses audited [RustCrypto](https://github.com/RustCrypto) crates.
- No custom cryptographic primitives.
- Key material must implement `Zeroize` and `ZeroizeOnDrop`.
- `Debug` implementations must redact secret values.

## Developer Certificate of Origin (DCO)

All contributions to this project must be signed off under the
[Developer Certificate of Origin](DCO) (DCO). By signing off your commits, you
certify that you wrote the code or have the right to submit it under the
project's license.

Add the sign-off to your commits with `git commit -s` or manually:

```text
Signed-off-by: Your Name <your.email@example.com>
```

This is a lightweight alternative to a CLA (Contributor License Agreement),
used by projects like the Linux kernel and many CNCF projects.

## License

- All code except the sync server is licensed under [Apache-2.0](LICENSE).
- The sync server (`crates/ldgr-server/`) is licensed under [AGPL-3.0](crates/ldgr-server/LICENSE).

By contributing, you agree that your contributions will be licensed under the
same license as the component you are contributing to.
