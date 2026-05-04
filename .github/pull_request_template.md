## Description

<!-- What does this PR do? Provide a brief summary of the changes. -->

## Related Issues

<!-- Link related issues: "Closes #123" or "Relates to #456" -->

## Type of Change

- [ ] Bug fix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Documentation update
- [ ] Refactoring (no functional changes)
- [ ] CI / infrastructure
- [ ] Other (describe below)

## Component

<!-- Which part of the monorepo does this touch? -->

- [ ] `crates/ldgr-core/` — Core library (crypto, accounting, storage)
- [ ] `crates/ldgr-cli/` — CLI tool
- [ ] `crates/ldgr-server/` — Sync server
- [ ] `bindings/swift/` — UniFFI Swift bindings
- [ ] `apps/ios/` — iOS / iPadOS / watchOS app
- [ ] `apps/web/` — Web app (Next.js + WASM)
- [ ] `docs/` — Documentation

## Privacy Checklist

<!-- All changes must uphold zero-knowledge principles -->

- [ ] No plaintext financial data is sent to or stored on the server
- [ ] No new metadata exposure introduced (or documented if unavoidable)
- [ ] Any new external service interaction is documented in the trust boundary model
- [ ] Crypto changes use audited RustCrypto crates — no custom primitives
- [ ] Key material is never exposed in error messages, logs, or Debug output

## Checklist

- [ ] I have read [CONTRIBUTING.md](CONTRIBUTING.md)
- [ ] Tests pass (`cargo test --workspace`)
- [ ] Clippy passes (`cargo clippy --workspace -- -D warnings`)
- [ ] Formatting passes (`cargo fmt --check`)
- [ ] I have updated documentation (if applicable)
