# ldgr Vault Format — Expert Specification (v1)

> **Status:** Normative specification for vault format version **1**.
> **Audience:** Implementers of compatible parsers/serializers, and security
> auditors. This document is byte-precise: a correct implementation can be
> written from this document alone.
> **Scope:** The on-disk/at-rest binary container produced by
> `crates/ldgr-core/src/crypto/`. It does **not** cover sync wire formats,
> the journal subset, or platform storage policy.

## Related documents

- **[How is my data protected?](./vault-overview.md)** — non-technical overview.
- **[How the vault container works](./vault-format-guide.md)** — intermediate
  conceptual guide (key hierarchy, flows, comparisons). Read this first if you
  want the "why" before the "what".
- **[Vault format test vectors](./test-vectors.md)** — known-answer test
  vectors (KDF, wrapping, envelope, recovery encoding, full-file hex dumps,
  negative cases) for verifying an implementation against this spec.
- **[ldgr Architecture](../ldgr-architecture.md)** — full system design.

This document is the **canonical reference** for the byte layout. Where prose
elsewhere disagrees with this document, this document wins for format details;
where this document disagrees with the Rust source, the source
(`crates/ldgr-core/src/crypto/`) wins, and the discrepancy is a spec bug to be
filed.

---

## 1. Notation and conventions

### 1.1 Byte ranges and types

- `[a..b]` denotes the half-open byte range starting at offset `a` (inclusive)
  and ending at offset `b` (exclusive), i.e. `b - a` bytes. Offsets are
  zero-based from the start of the relevant structure.
- `u8`, `u16`, `u32` are unsigned integers of 1, 2, and 4 bytes.
- `[u8; N]` is a fixed-length array of `N` bytes, stored verbatim with no
  length prefix.
- `bytes(N)` is `N` raw bytes.

### 1.2 Endianness — read this carefully

> **All multi-byte integers in the container are little-endian (LE), with
> exactly one exception: the payload padding length-prefix inside an item
> envelope is big-endian (BE).** See §9. This is the single most common interop
> mistake — implement the BE prefix deliberately.

| Field class | Endianness |
| --- | --- |
| Format version, KDF parameters, `ct_len`, item count | **little-endian** |
| Envelope padding length-prefix (§9) | **big-endian** |

### 1.3 AEAD ciphertext and tags

All encryption uses **AES-256-GCM** with a 96-bit (12-byte) nonce and a 128-bit
(16-byte) authentication tag. Throughout this document, a stored "ciphertext"
field is the AES-GCM output **with the 16-byte tag appended** (the convention of
the RustCrypto `aes-gcm` crate). Therefore:

```
ct_len == len(plaintext_input_to_GCM) + 16
```

The Additional Authenticated Data (AAD) for each encryption is a fixed ASCII
domain-separation tag (no trailing NUL); the exact bytes are given per use site.
AAD is authenticated but not stored — a verifier must supply the identical AAD
bytes to decrypt.

### 1.4 Randomness

Every nonce and every generated key (vault key, item keys, recovery key, salt)
MUST come from a cryptographically secure RNG. Nonces are generated fresh per
encryption operation; a 96-bit random nonce per fresh random key is safe under
AES-GCM because each key is used for a single message.

---

## 2. Cryptographic primitives

| Purpose | Algorithm | Parameters |
| --- | --- | --- |
| Password hashing (KDF) | **Argon2id**, version `0x13` (v1.3) | memory/iterations/parallelism from header; output **32 bytes**; no secret/AD inputs |
| Subkey derivation | **HKDF-SHA256** | salt = none (all-zero block); per-use `info` string; output **32 bytes** |
| Key wrapping | **AES-256-GCM** | 12-byte random nonce, 16-byte tag, per-use AAD |
| Item/metadata encryption | **AES-256-GCM** | 12-byte random nonce, 16-byte tag, AAD `ldgr-item-seal-v1` |
| Recovery key encoding | **Crockford Base32** | 256-bit key → 52 chars (§11) |

All keys are 256-bit (32-byte). Source: `crypto/keys.rs` (`KEY_LEN = 32`).

---

## 3. Key hierarchy

```
            password (user secret)            salt (32 B, in header)
                 │                                   │
                 └──────────── Argon2id ─────────────┘
                                  │
                                  ▼
                          MK  (Master Key, 32 B)         ── never stored ──
                            │
              ┌─────────────┴──────────────┐
        HKDF-SHA256                    HKDF-SHA256
      info=ldgr-auth-v1              info=ldgr-enc-v1
              │                            │
              ▼                            ▼
        AuthKey (32 B)              MEK (Master Encryption Key, 32 B)
   (server auth; NOT in vault file)        │
                                           │  AES-256-GCM wrap
                                           │  AAD=ldgr-vault-wrap-v1
                                           ▼
                                    ┌──► VK (Vault Key, 32 B) ◄──┐
                                    │                            │
   recovery key (RK, 32 B) ─────────┘     AES-256-GCM wrap        │
   AES-256-GCM wrap                       AAD=ldgr-item-wrap-v1   │
   AAD=ldgr-recovery-wrap-v1                   │                  │
   (alternate path to the SAME VK)             ▼                  │
                                         IK (per item, 32 B)      │
                                               │                  │
                                AES-256-GCM seal, AAD=ldgr-item-seal-v1
                                               │
                                               ▼
                                  encrypted item payload
```

Notes:

- **MK is never persisted.** It exists only in memory while the vault is open.
- The **VK** is the pivot: it is wrapped twice in the header (once by MEK, once
  by RK), and it wraps every per-item key (IK). Both wraps protect the *same*
  VK bytes.
- **AuthKey** is part of the hierarchy (HKDF `info=ldgr-auth-v1`) but is used for
  server authentication and is **not present in the vault file**.
- The recovery key provides an alternate route to the VK so the password can be
  reset without re-encrypting item data.

Source: `crypto/kdf.rs` (HKDF info strings), `crypto/wrap.rs` (AAD tags),
`crypto/vault.rs` (`create_vault`).

---

## 4. Top-level binary layout

A vault file is: **fixed header** (51 bytes) + **variable header** + **body**.

```
┌──────────────────────────────────────────────────────────────────┐
│ FIXED HEADER (51 bytes)                                           │
├──────────────────────────────────────────────────────────────────┤
│ VARIABLE HEADER                                                   │
│   WrappedKey      wrapped_vk           (MEK-wrapped vault key)    │
│   WrappedKey      recovery_wrapped_vk  (RK-wrapped vault key)     │
│   SealedEnvelope  encrypted_metadata   (vault name + created_at)  │
├──────────────────────────────────────────────────────────────────┤
│ BODY                                                             │
│   u32 LE          item_count                                     │
│   SealedEnvelope  items[0]                                       │
│   SealedEnvelope  items[1]                                       │
│   ...             items[item_count-1]                            │
└──────────────────────────────────────────────────────────────────┘
```

### 4.1 Fixed header (offsets are absolute from start of file)

| Offset | Len | Field | Type | Value / notes |
| --- | --- | --- | --- | --- |
| `[0..4]` | 4 | Magic | `[u8; 4]` | `b"LDGR"` = `4C 44 47 52` |
| `[4..6]` | 2 | Format version | `u16` LE | `1` |
| `[6..7]` | 1 | KDF version | `u8` | `1` → Argon2id v`0x13` |
| `[7..39]` | 32 | Salt | `[u8; 32]` | Argon2id salt (random per vault) |
| `[39..43]` | 4 | Argon2 memory cost (KiB) | `u32` LE | see §6 |
| `[43..47]` | 4 | Argon2 iterations | `u32` LE | see §6 |
| `[47..51]` | 4 | Argon2 parallelism | `u32` LE | see §6 |

Total fixed header length = `4 + 2 + 1 + 32 + 4 + 4 + 4 = 51` bytes
(`FIXED_HEADER_LEN`).

### 4.2 Variable header

Immediately after offset 51, three binary-serialized crypto blobs appear in
this exact order:

1. `wrapped_vk` — a `WrappedKey` (§7): the vault key wrapped by the MEK,
   AAD `ldgr-vault-wrap-v1`.
2. `recovery_wrapped_vk` — a `WrappedKey` (§7): the vault key wrapped by the
   recovery key, AAD `ldgr-recovery-wrap-v1`.
3. `encrypted_metadata` — a `SealedEnvelope` (§8) sealing the metadata JSON
   (§10) under the vault key.

These are variable-length in general (each carries its own `ct_len`), so a
parser MUST consume them sequentially using their internal length fields rather
than seeking to fixed offsets. In format v1 each `WrappedKey` ciphertext is
exactly 48 bytes (§7.2), but parsers MUST still honor the encoded `ct_len`.

### 4.3 Body

| Field | Type | Notes |
| --- | --- | --- |
| `item_count` | `u32` LE | Number of items; MUST be `≤ 1_000_000` (§12) |
| `items[i]` | `SealedEnvelope` | `item_count` envelopes, each sealing one item payload under the vault key |

The body ends after the last item envelope. Trailing bytes after the final
envelope are not produced by a conforming serializer; a parser MAY treat
trailing bytes as an error or ignore them, but MUST NOT misinterpret them as
items (the `item_count` is authoritative).

---

## 5. Object size reference (v1)

For format version 1 (32-byte keys, AES-GCM 16-byte tag), the fixed-size pieces
have these byte lengths:

| Object | Composition | Size |
| --- | --- | --- |
| Fixed header | §4.1 | **51 B** |
| `WrappedKey` | `1 + 12 + 4 + 48` | **65 B** |
| `SealedEnvelope` overhead | `1 + 65 (wrapped_ik) + 12 + 4` | **82 B** + ciphertext |
| Metadata envelope (≤512-byte bucket) | `82 + (512 + 16)` | **610 B** |
| Empty-vault body | `item_count` only | **4 B** |

**Minimal vault (no items, metadata fits the 512-byte bucket):**
`51 (fixed) + 65 (wrapped_vk) + 65 (recovery_wrapped_vk) + 610 (metadata) +
4 (item_count) = 795 bytes`.

Each stored item adds `82 + (bucket + 16)` bytes, where `bucket` is the padded
size from §9.

---

## 6. KDF specification (Argon2id + HKDF)

### 6.1 Argon2id

- **Algorithm:** Argon2id, **version `0x13`** (Argon2 v1.3). The on-disk KDF
  version byte (`[6..7]`) is `1` and maps to exactly this algorithm/version.
- **Inputs:** `password` (raw bytes, not pre-hashed), `salt` = header `[7..39]`
  (32 bytes).
- **Parameters:** `memory_cost_kib`, `iterations` (time cost `t`), and
  `parallelism` (lanes `p`) are read from the fixed header (§4.1). The output
  ("tag") length is **32 bytes**. No secret key and no associated data are
  supplied to Argon2.
- **Output:** the 32-byte Master Key (`MK`).

Reference parameter presets used by the implementation (informative — actual
values are whatever the header stores):

| Preset | memory (KiB) | iterations | parallelism |
| --- | --- | --- | --- |
| Desktop | `262144` (256 MiB) | `3` | `4` |
| Mobile | `65536` (64 MiB) | `4` | `2` |
| WASM | `65536` (64 MiB) | `3` | `1` |

On derivation, the implementation also enforces lower bounds: `memory_cost_kib ≥
8`, `iterations ≥ 1`, `parallelism ≥ 1`. Upper bounds for untrusted input are in
§12.

### 6.2 HKDF-SHA256 subkey derivation

Both subkeys are derived from `MK` using HKDF-SHA256 with **no salt** (the HKDF
salt is the all-zero block of the hash length) and a 32-byte output:

| Subkey | HKDF `info` (ASCII, no NUL) | Use |
| --- | --- | --- |
| `AuthKey` | `ldgr-auth-v1` | Server authentication (not stored in vault) |
| `MEK` | `ldgr-enc-v1` | Wrapping/unwrapping the vault key |

```
MK  = Argon2id(password, salt, params)               // 32 bytes
AuthKey = HKDF-SHA256(ikm=MK, salt=∅, info="ldgr-auth-v1", L=32)
MEK     = HKDF-SHA256(ikm=MK, salt=∅, info="ldgr-enc-v1",  L=32)
```

Source: `crypto/kdf.rs`.

---

## 7. Key wrapping — `WrappedKey` sub-format

A `WrappedKey` holds a 256-bit key encrypted under another 256-bit key using
AES-256-GCM with a domain-separation AAD.

### 7.1 Binary layout

| Offset (within object) | Len | Field | Type | Notes |
| --- | --- | --- | --- | --- |
| `[0..1]` | 1 | `version` | `u8` | `1` for this format |
| `[1..13]` | 12 | `nonce` | `[u8; 12]` | AES-GCM nonce |
| `[13..17]` | 4 | `ct_len` | `u32` LE | length of `ciphertext`; MUST be `≤ 1 MiB` (§12) |
| `[17..17+ct_len]` | `ct_len` | `ciphertext` | `bytes(ct_len)` | AES-GCM output incl. 16-byte tag |

### 7.2 Plaintext, ciphertext length, and AAD

The GCM plaintext is the 32-byte wrapped key, so in v1 `ct_len` is always
`32 + 16 = 48`, and the whole `WrappedKey` object is `1 + 12 + 4 + 48 = 65`
bytes. A parser MUST still read `ct_len` and not assume 48.

The AAD selects the wrapping relationship (domain separation). A `WrappedKey`
produced with one AAD will fail authentication if decrypted with another:

| Relationship | Wrapping key | Wrapped key | AAD (ASCII) |
| --- | --- | --- | --- |
| MEK → VK (`wrapped_vk`) | MEK | VK | `ldgr-vault-wrap-v1` |
| RK → VK (`recovery_wrapped_vk`) | Recovery Key | VK | `ldgr-recovery-wrap-v1` |
| VK → IK (inside each envelope) | VK | IK | `ldgr-item-wrap-v1` |

To unwrap: verify `version == 1`, then
`plaintext = AES256-GCM-Decrypt(key=wrapping_key, nonce, ciphertext, aad)`; the
resulting 32 bytes are the unwrapped key. Authentication failure (wrong key,
wrong AAD, or tampering) MUST be reported as an error and MUST NOT yield key
material.

Source: `crypto/wrap.rs`, serialized by `write_wrapped_key`/`read_wrapped_key`
in `crypto/vault.rs`.

---

## 8. Envelope encryption — `SealedEnvelope` sub-format

A `SealedEnvelope` binds a wrapped per-item key to an encrypted, padded payload.

### 8.1 Binary layout

| Offset (within object) | Len | Field | Type | Notes |
| --- | --- | --- | --- | --- |
| `[0..1]` | 1 | `version` | `u8` | `1` for this format |
| `[1..N]` | 65 (v1) | `wrapped_ik` | `WrappedKey` | inline §7; item key wrapped by VK, AAD `ldgr-item-wrap-v1` |
| `[N..N+12]` | 12 | `nonce` | `[u8; 12]` | AES-GCM nonce for the payload |
| `[N+12..N+16]` | 4 | `ct_len` | `u32` LE | length of `ciphertext`; MUST be `≤ 1 MiB` (§12) |
| `[N+16..]` | `ct_len` | `ciphertext` | `bytes(ct_len)` | AES-GCM output of the padded payload, incl. 16-byte tag |

`N = 1 + len(wrapped_ik)`. With a v1 `wrapped_ik` of 65 bytes, the fixed
overhead before `ciphertext` is `1 + 65 + 12 + 4 = 82` bytes.

### 8.2 Sealing and opening

**Seal** a plaintext payload `P` under vault key `VK`:

1. Generate a random item key `IK`.
2. `padded = pad_to_bucket(P)` (§9).
3. `ciphertext = AES256-GCM-Encrypt(key=IK, nonce=random12, msg=padded,
   aad="ldgr-item-seal-v1")` (tag appended).
4. `wrapped_ik = WrappedKey(VK wraps IK, aad="ldgr-item-wrap-v1")` (§7).
5. Emit `SealedEnvelope { version=1, wrapped_ik, nonce, ciphertext }`.

So `ct_len == bucket + 16`, where `bucket` is the padded length from §9.

**Open** a `SealedEnvelope` with `VK`:

1. Verify `version == 1` (else error: unsupported envelope version).
2. `IK = unwrap(wrapped_ik, VK, aad="ldgr-item-wrap-v1")` (§7).
3. `padded = AES256-GCM-Decrypt(key=IK, nonce, ciphertext,
   aad="ldgr-item-seal-v1")` (auth failure → error).
4. `P = unpad(padded)` (§9.2).

The AAD for payload encryption is **`ldgr-item-seal-v1`** — distinct from the
item-key wrapping AAD `ldgr-item-wrap-v1`. Both appear within a single envelope.

Source: `crypto/envelope.rs`, serialized by
`write_sealed_envelope`/`read_sealed_envelope` in `crypto/vault.rs`.

---

## 9. Size-bucket padding

To avoid leaking exact plaintext lengths, payloads are padded to a fixed set of
size buckets before encryption. Padding is applied to the GCM *plaintext*; the
ciphertext is then `bucket + 16` bytes.

### 9.1 Padding algorithm (encode)

Constants: `LENGTH_PREFIX_LEN = 4`, buckets `[512, 2048, 8192, 32768]`,
`LARGEST_BUCKET = 32768`.

```
function padded_size(payload_len):           # returns the target bucket size
    total = LENGTH_PREFIX_LEN + payload_len  # 4-byte prefix counts toward bucket
    for bucket in [512, 2048, 8192, 32768]:
        if total <= bucket:
            return bucket
    return ceil(total / 32768) * 32768        # round up to next 32 KiB multiple

function pad_to_bucket(payload):
    target = padded_size(len(payload))
    out = u32_BE(len(payload))                # 4-byte BIG-ENDIAN length prefix
    out ||= payload
    out ||= zero_bytes(target - len(out))     # zero padding to the bucket size
    return out                                # len(out) == target
```

> **Endianness reminder:** the 4-byte length prefix is **big-endian**, unlike
> every other integer in the container (which is little-endian). `len(payload)`
> MUST be `< 4 GiB` (it fits in a `u32`).

### 9.2 Unpadding algorithm (decode)

```
function unpad(padded):
    if len(padded) < 4: error "too short for length prefix"
    len = u32_BE(padded[0..4])
    if 4 + len > len(padded): error "length prefix exceeds padded data"
    return padded[4 .. 4+len]
```

The trailing zero bytes are discarded; only the declared `len` bytes after the
prefix are the original payload.

### 9.3 Worked examples

| `payload_len` | `total = 4 + len` | Bucket (`padded_size`) | `ct_len` (`bucket + 16`) |
| --- | --- | --- | --- |
| 0 | 4 | 512 | 528 |
| 1 | 5 | 512 | 528 |
| 508 | 512 | 512 (exact) | 528 |
| 509 | 513 | 2048 | 2064 |
| 2044 | 2048 | 2048 (exact) | 2064 |
| 2045 | 2049 | 8192 | 8208 |
| 8188 | 8192 | 8192 (exact) | 8208 |
| 8189 | 8193 | 32768 | 32784 |
| 32764 | 32768 | 32768 (exact) | 32784 |
| 32765 | 32769 | 65536 | 65552 |
| 98300 | 98304 | 98304 (= 3 × 32768, exact) | 98320 |

Source: `crypto/envelope.rs` (`padded_size`, `pad_to_bucket`, `unpad`, and the
`padded_size_buckets` test).

---

## 10. Vault metadata encoding

The `encrypted_metadata` envelope (§4.2) seals a UTF-8 JSON object of this shape
under the vault key:

```json
{
  "name": "Personal Finance",
  "created_at": "2024-01-15T09:30:00Z"
}
```

| Field | JSON type | Notes |
| --- | --- | --- |
| `name` | string | Human-readable vault name |
| `created_at` | string | RFC 3339 / ISO 8601 UTC timestamp (serde encoding of a `chrono` `DateTime<Utc>`) |

The JSON bytes are the payload `P` fed into the §8 seal procedure with the §9
padding. Implementations SHOULD treat unknown JSON fields leniently on read (for
forward compatibility) but a v1 serializer emits exactly these two fields.

Source: `crypto/vault.rs` (`VaultMetadata`, `create_vault`, `open_vault`).

---

## 11. Recovery key encoding (Crockford Base32)

The recovery key is a 256-bit random key. Its *bytes* are what wrap the VK
(§7, AAD `ldgr-recovery-wrap-v1`); its *human-readable form* is a Crockford
Base32 string shown to the user as an emergency kit. The encoding is purely a
presentation/transcription layer — only the 32 raw bytes participate in
cryptography.

### 11.1 Alphabet

```
0123456789ABCDEFGHJKMNPQRSTVWXYZ
```

(32 symbols; excludes `I`, `L`, `O`, `U` to reduce transcription errors.)
Index 0 = `0` … index 31 = `Z`.

### 11.2 Encode (32 bytes → 52 chars → grouped display)

Process the 256 bits most-significant-first, emitting one alphabet symbol per
5-bit group:

```
buffer = 0; bits = 0; out = ""
for byte in key_bytes (32 of them):
    buffer = (buffer << 8) | byte;  bits += 8
    while bits >= 5:
        bits -= 5
        out += ALPHABET[(buffer >> bits) & 0x1F]
# 256 mod 5 == 1 leftover bit: left-shift by (5-1)=4 to pad with 4 zero bits
if bits > 0:
    out += ALPHABET[(buffer << (5 - bits)) & 0x1F]
# out has exactly 52 characters
```

For **display**, split the 52 characters into groups of 4 joined by `-`,
producing 13 groups and 12 dashes (64 display characters total):

```
XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX
```

### 11.3 Decode (string → 32 bytes)

1. **Sanitize:** remove all whitespace and `-` characters. The remaining string
   MUST be exactly **52 characters**, else error.
2. **Per-character value** with Crockford normalization (case-insensitive):
   - `0 O o` → `0`
   - `1 I i L l` → `1`
   - `2..9` → their digit value
   - `A..Z` (excluding `I L O U`, case-insensitive) → `10..31` per the alphabet
   - `U`/`u` and any other character → error
3. **Accumulate** 5 bits per symbol, MSB-first, emitting a byte whenever ≥ 8
   bits are buffered:

```
buffer = 0; bits = 0; out = []
for ch in sanitized (52 chars):
    v = value(ch)                 # 0..31
    buffer = (buffer << 5) | v;  bits += 5
    while bits >= 8:
        bits -= 8
        out.push((buffer >> bits) & 0xFF)
# 52*5 = 260 bits = 32 bytes (256) + 4 leftover bits
if bits > 0:
    if (buffer & ((1 << bits) - 1)) != 0: error "invalid padding"  # pad bits MUST be zero
assert len(out) == 32
```

> **Padding rule:** the 4 trailing bits after the 32nd byte MUST be zero. A
> decoder MUST reject non-zero padding bits. This makes the 52-char ↔ 32-byte
> mapping injective.

Source: `crypto/recovery.rs`.

---

## 12. Validation bounds for untrusted input

A parser handling a file from an untrusted source MUST enforce these bounds
*before* acting on the values (e.g. before invoking Argon2 or allocating
buffers). Violations MUST be rejected as an invalid-vault error.

| Field | Bound | Constant | Rationale |
| --- | --- | --- | --- |
| Magic | `== b"LDGR"` | `MAGIC` | Reject non-vault files early |
| Format version | `== 1` | `FORMAT_VERSION` | Unknown versions are not parseable by a v1 reader (§14) |
| KDF version | `== 1` | `KDF_VERSION` | Only Argon2id v`0x13` is defined |
| File length | `≥ 51` before reading fixed header | `FIXED_HEADER_LEN` | Avoid out-of-bounds reads |
| Argon2 memory cost | `≤ 4 GiB` (`4 * 1024 * 1024` KiB) | `MAX_MEMORY_COST_KIB` | Prevent memory-exhaustion DoS via crafted header |
| Argon2 iterations | `≤ 100` | `MAX_ITERATIONS` | Prevent CPU-exhaustion DoS |
| Argon2 parallelism | `≤ 16` | `MAX_PARALLELISM` | Bound thread/lane allocation |
| Any `ct_len` (wrap or envelope) | `≤ 1 MiB` (`1024 * 1024`) | `MAX_BLOB_LEN` | Bound per-blob allocation |
| `item_count` | `≤ 1_000_000` | `MAX_ITEM_COUNT` | Bound total work / allocation |
| Any read | MUST be within remaining bytes | — | Reject truncated files (the reader checks `pos + n ≤ len`) |

These are *upper* bounds for parsing untrusted data. The KDF additionally
enforces *lower* bounds at derivation time (memory ≥ 8 KiB, iterations ≥ 1,
parallelism ≥ 1; §6.1). Crucially, the parser validates the Argon2 parameter
bounds **before** any password-derivation work is attempted.

Source: `crypto/vault.rs` (`validate_vault`, `validate_param_bounds`,
`parse_vault`, `read_wrapped_key`, `read_sealed_envelope`, `BinaryReader`).

---

## 13. Parsing and serialization algorithms

### 13.1 Parse (`data → header + items`)

```
1. validate_vault(data):
     require len(data) >= 51
     require data[0..4] == "LDGR"
     require u16_LE(data[4..6]) == 1            # else UnsupportedVersion
2. read fixed header:
     skip 4 (magic, already checked)
     format_version = u16_LE
     kdf_version    = u8;  require == 1
     salt           = bytes(32)
     mem            = u32_LE
     iters          = u32_LE
     par            = u32_LE
     validate_param_bounds(mem, iters, par)     # §12 upper bounds
3. read variable header (sequential, length-driven):
     wrapped_vk          = read_WrappedKey()    # §7 (enforce ct_len <= 1 MiB)
     recovery_wrapped_vk = read_WrappedKey()
     encrypted_metadata  = read_SealedEnvelope() # §8 (enforce ct_len <= 1 MiB)
4. read body:
     item_count = u32_LE;  require <= 1_000_000
     items = [ read_SealedEnvelope() for _ in 0..item_count ]
5. return (header, items)
```

Every read goes through a bounds-checked reader: requesting `n` bytes when fewer
than `n` remain is an error (no wrap-around, no partial reads).

Unlocking adds: derive `MK = Argon2id(password, salt, params)`,
`MEK = HKDF(MK, "ldgr-enc-v1")`, `VK = unwrap(wrapped_vk, MEK,
"ldgr-vault-wrap-v1")`, then decrypt `encrypted_metadata` with `VK` and parse
its JSON (§10). A wrong password manifests as an unwrap authentication failure.

Recovery unlocking instead computes `VK = unwrap(recovery_wrapped_vk, RK,
"ldgr-recovery-wrap-v1")`.

### 13.2 Serialize (`header + items → data`)

```
write "LDGR"
write u16_LE(format_version=1)
write u8(kdf_version=1)
write bytes(salt[32])
write u32_LE(mem); write u32_LE(iters); write u32_LE(par)
write_WrappedKey(wrapped_vk)
write_WrappedKey(recovery_wrapped_vk)
write_SealedEnvelope(encrypted_metadata)
write u32_LE(item_count = len(items))     # error if > u32::MAX
for item in items: write_SealedEnvelope(item)
```

`write_WrappedKey`: `u8(version)`, `bytes(nonce[12])`, `u32_LE(ct_len)`,
`bytes(ciphertext)`. `write_SealedEnvelope`: `u8(version)`,
`write_WrappedKey(wrapped_ik)`, `bytes(nonce[12])`, `u32_LE(ct_len)`,
`bytes(ciphertext)`.

### 13.3 Determinism note

Serialization is **not** byte-deterministic across runs: nonces, the vault key,
item keys, and the recovery key are randomly generated, and re-wrapping/
re-sealing (e.g. password change) produces fresh nonces. The *structure* is
fully deterministic; the *bytes* are not. Reproducible test vectors therefore
require injecting fixed nonces/keys (see [test-vectors.md](./test-vectors.md)).

---

## 14. Versioning contract

The format carries two independent version fields plus per-object versions:

- **Format version** (`[4..6]`, currently `1`) governs the overall container
  layout (§4).
- **KDF version** (`[6..7]`, currently `1`) governs the password-hashing scheme
  (Argon2id v`0x13`).
- **Per-object versions** (`WrappedKey.version`, `SealedEnvelope.version`, both
  `1`) allow the wrapping/envelope encodings to evolve independently of the
  container.

### 14.1 v1 reader behavior

A v1 reader rejects:

- any **format version ≠ 1** with an "unsupported version" error (it does not
  attempt a best-effort parse);
- any **KDF version ≠ 1** with an invalid-vault error;
- any per-object `version ≠ 1` (`WrappedKey`/`SealedEnvelope`) with an error.

### 14.2 Compatible vs breaking changes

**Backward-compatible (does not require a format-version bump):**

- Adding new fields to the metadata JSON (§10) — readers ignore unknown keys.
- Tightening or relaxing the numeric *defaults* of Argon2 parameters (they are
  stored per-vault in the header; old files keep their own parameters).
- New optional sibling documents/tooling that do not change the byte layout.

**Breaking (requires bumping the relevant version field and is rejected by older
readers):**

- Any change to the fixed-header layout, field order, sizes, or endianness.
- Changing the magic bytes, salt length, nonce length, tag size, or key length.
- Changing an AAD domain-separation tag, an HKDF `info` string, the KDF
  algorithm/version, or the set/semantics of size buckets or the padding
  prefix.
- Reordering or repurposing the variable-header blobs or the body framing.

### 14.3 Evolution strategy

Future versions SHOULD:

- Reuse the magic + version-field framing so a reader can detect and cleanly
  reject (or dispatch on) versions it does not implement.
- Introduce new behavior under a **new** format/KDF/object version rather than
  redefining v1 fields, so existing files remain readable by their original
  code path.
- Keep the validation-bounds discipline (§12): every length and count read from
  an untrusted file must be bounded before use.

---

## 15. Conformance checklist

An implementation is conformant with format v1 if it:

- [ ] Emits/accepts the 51-byte fixed header exactly as in §4.1, little-endian.
- [ ] Treats the padding length-prefix as **big-endian** (§9) while all other
      integers are little-endian.
- [ ] Uses Argon2id v`0x13` with a 32-byte output and HKDF-SHA256 `info`
      strings `ldgr-auth-v1` / `ldgr-enc-v1` (§6).
- [ ] Applies the exact AAD tags `ldgr-vault-wrap-v1`, `ldgr-recovery-wrap-v1`,
      `ldgr-item-wrap-v1`, `ldgr-item-seal-v1` (§7–§8).
- [ ] Implements the bucket set `512 / 2048 / 8192 / 32768 / 32768·k` with the
      4-byte prefix counted toward the bucket (§9).
- [ ] Encodes/decodes recovery keys per Crockford Base32 with the normalization
      and zero-padding rules (§11).
- [ ] Enforces all validation bounds in §12 before doing work.
- [ ] Rejects unknown format/KDF/object versions (§14).
- [ ] Passes every case in [test-vectors.md](./test-vectors.md).

---

## 16. References

Rust implementation (the authoritative source; this spec tracks it):

- `crates/ldgr-core/src/crypto/vault.rs` — container layout, header/body,
  binary reader/writer, validation bounds, parse/serialize.
- `crates/ldgr-core/src/crypto/wrap.rs` — `WrappedKey`, key-wrap AAD tags.
- `crates/ldgr-core/src/crypto/envelope.rs` — `SealedEnvelope`, item-seal AAD,
  size-bucket padding.
- `crates/ldgr-core/src/crypto/kdf.rs` — Argon2id parameters, HKDF info strings.
- `crates/ldgr-core/src/crypto/recovery.rs` — Crockford Base32 recovery encoding.
- `crates/ldgr-core/src/crypto/keys.rs` — 256-bit key types (`Zeroize`).

Companion documents: [vault-overview.md](./vault-overview.md),
[vault-format-guide.md](./vault-format-guide.md),
[test-vectors.md](./test-vectors.md).
