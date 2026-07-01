# Vault format diagrams

Polished, standalone visual assets for the [vault format documentation](../). Each
diagram is self-contained — you can understand it without reading the surrounding
prose — and every diagram uses the same visual language so colors and shapes mean
the same thing across the whole set.

They support the three tiers of vault-format docs:

- [How is my data protected?](../vault-overview.md) — non-technical overview
- [How the vault container works](../vault-format-guide.md) — intermediate guide
- [Vault format — Expert specification](../vault-format-spec.md) — byte-precise spec

## Visual language

| Color | Meaning |
| --- | --- |
| 🟩 **Green** | Plaintext at rest — stored unencrypted (e.g. the fixed header, item counts) |
| 🟥 **Red** | Encrypted at rest — ciphertext stored in the file or sent over sync |
| 🟨 **Yellow** | Key material — a cryptographic key |
| ⬜ **Grey** | An operation (a KDF, a wrap/seal, a serialization step) |
| 🟦 **Blue** | An actor — you, or your password |

Border style encodes lifetime:

- **Dashed border** — a key or value that exists **only in memory** (ephemeral); it
  is never written to disk.
- **Solid border** — a value that is **persisted** (on disk or in transit).

## Source format

These are **hand-authored SVGs**, matching the style of
[`../assets/vault-overview-flow.svg`](../assets/vault-overview-flow.svg). The SVG is
the editable source: it renders natively in GitHub's markdown preview, stays small,
and can be edited directly (no separate binary source or export step). Content is
kept in sync with the [expert specification](../vault-format-spec.md) (§3 key
hierarchy, §4 binary layout, §13 flows).

---

## 1. Key hierarchy

![Vault key hierarchy: the password and a header salt run through Argon2id to
produce the Master Key, which HKDF splits into an AuthKey (server auth, not stored)
and the Master Encryption Key. The MEK and an independent recovery key each wrap the
same Vault Key; the Vault Key wraps a per-item key that seals each encrypted item.
Dashed boxes are keys held only in memory; red boxes are encrypted data stored in
the file.](./key-hierarchy.svg)

How every key is derived and what wraps what. The **Vault Key (VK)** is the pivot: it
is wrapped twice in the header — once by the password-derived MEK, once by the
recovery key — and both wraps protect the *same* VK. The Master Key is never stored.

Supports: guide, specification (§3).

## 2. Vault binary file structure

![Vault binary layout, top to bottom: a 51-byte plaintext fixed header (magic,
versions, salt, Argon2 parameters), a variable header holding wrapped_vk,
recovery_wrapped_vk and the encrypted metadata, then the body — a plaintext item
count followed by one encrypted sealed envelope per item. A side panel expands a
sealed envelope into version, wrapped item key, nonce, ciphertext length and
ciphertext.](./vault-structure.svg)

The on-disk byte layout, with plaintext regions in green and encrypted regions in
red. The zoom panel expands one `SealedEnvelope`.

Supports: specification (§4), guide.

## 3. Create vault

![Create-vault sequence: generate a salt and KDF parameters, derive the Master Key
with Argon2id, derive the MEK and AuthKey with HKDF, generate the Vault Key and
recovery key, wrap the Vault Key with the MEK and again with the recovery key, seal
the metadata, serialize the container, then write the encrypted file to disk and
show the recovery key once.](./create-vault-sequence.svg)

Password → encrypted file on disk. Note that the password, MK, MEK, VK and recovery
key never touch the disk; only encrypted bytes are written.

Supports: guide, specification (§13.2).

## 4. Open vault

![Open-vault sequence: read and validate the fixed header, read the salt and KDF
parameters, derive the Master Key and MEK, unwrap the Vault Key, decrypt the
metadata, and decrypt items on demand. A wrong password fails as an authentication
error at the unwrap step.](./open-vault-sequence.svg)

Encrypted file + password → plaintext data in memory. A wrong password surfaces as an
AES-GCM authentication failure when unwrapping the Vault Key.

Supports: guide, specification (§13.1).

## 5. Recovery

![Recovery sequence: parse the header, unwrap the Vault Key with the recovery key
(recovering the same Vault Key), generate a new salt, derive a new Master Key and
MEK, re-wrap the Vault Key, and re-seal the metadata. The recovery wrap and every
item envelope stay untouched, so item data is not re-encrypted.](./recovery-sequence.svg)

Recovery key + a new password → a re-keyed vault. Because the Vault Key itself never
changes, no item data is re-encrypted — only the password-derived path is rebuilt.

Supports: guide.

## 6. Password change

![Password-change diagram: a four-step flow (unwrap the Vault Key with the old
password path, derive a new salt / Master Key / MEK, re-wrap the Vault Key, re-seal
the metadata) above a split panel. The "changes" column lists the salt, Master Key,
MEK, wrapped_vk and encrypted metadata; the "stays the same" column lists the Vault
Key, the recovery wrap, all item keys, all item ciphertext and the recovery
key.](./password-change-sequence.svg)

What a password change rewrites (salt, MK, MEK, `wrapped_vk`, metadata nonce) versus
what stays identical (VK, the recovery wrap, every item key and all item data).

Supports: guide.

## 7. Zero-knowledge overview

![Zero-knowledge split view. On your device: your password, the keys in memory, your
plaintext data, and an encrypt-before-send step — the client sees everything in the
clear. On the server / sync side: encrypted vault blobs, encrypted sync events, and
only coarse metadata (padded sizes and counts); it never sees your password, any
key, or any plaintext. Only encrypted bytes cross the trust
boundary.](./zero-knowledge-overview.svg)

What your device can see (plaintext data and keys in memory) versus what the server
and sync transport can see (only encrypted blobs, never any key). Only encrypted
bytes cross the trust boundary.

Supports: overview, guide.
