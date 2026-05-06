# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.x     | :white_check_mark: |

Only the latest release is supported with security updates.

## Reporting a Vulnerability

**Please do not open a public issue for security vulnerabilities.**

If you discover a security vulnerability in ldgr, please report it through
GitHub's private vulnerability reporting feature:

1. Go to the [Security tab](https://github.com/kafkade/ldgr/security) of this repository
2. Click **"Report a vulnerability"**
3. Fill out the form with details about the vulnerability

### What to expect

- **Acknowledgment**: We will acknowledge your report within **48 hours**
- **Assessment**: We will assess the severity and impact within **7 days**
- **Resolution**: We aim to release a fix within **90 days** of the initial report, depending on complexity
- **Disclosure**: We will coordinate disclosure timing with you

### What to include

- A description of the vulnerability
- Steps to reproduce the issue
- The potential impact
- Any suggested fixes (if you have them)

### Scope

Security issues in ldgr may include:

- Cryptographic weaknesses in the vault format or key hierarchy
- Key material exposure in error messages, logs, or Debug output
- Vault key leakage through session files or memory
- Bypass of encryption (plaintext data reaching the server/sync layer)
- Argon2id parameter downgrade attacks
- SQLite injection in the storage layer
- Path traversal in import/export file handling
- Memory safety issues (though `unsafe` code is forbidden)

### Out of scope

- Third-party forks or unofficial builds
- Issues in upstream dependencies (report those to the dependency maintainers)
- Social engineering attacks (phishing for the master password)
- Physical access to an unlocked device

## Cryptographic Details

ldgr uses audited [RustCrypto](https://github.com/RustCrypto) crates exclusively:

- **AES-256-GCM** for symmetric encryption (aes-gcm crate)
- **Argon2id** for password-based key derivation (argon2 crate)
- **HKDF-SHA256** for domain-separated key derivation (hkdf crate)
- **X25519** for key exchange (x25519-dalek crate)

No custom cryptographic primitives are used. All key types implement `Zeroize`
and `ZeroizeOnDrop` for memory safety.

## Thank You

We appreciate responsible disclosure and will credit reporters (with their
permission) in our release notes and changelog.
