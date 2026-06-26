# ldgr vault format — test vectors (v1)

> **Status:** these vectors target vault **format version 1** (`FORMAT_VERSION =
> 1`, `KDF_VERSION = 1`). They are the canonical known-answer vectors for
> third-party implementations and security audits. The companion formal
> specification is [`vault-format-spec.md`](./vault-format-spec.md); where this
> document and the spec disagree, the spec — and ultimately the reference
> implementation — wins.

## What these vectors are for

A third-party implementation of the ldgr vault format can verify byte-for-byte
compatibility by reproducing every vector below from its inputs. Each vector
lists **all inputs**, the relevant **intermediate values** (so you can debug
step by step), and the **expected output**.

Every vector has a matching binary (or text) fixture under
[`tests/fixtures/vault/`](../../tests/fixtures/vault/), and the reference
implementation re-derives and checks all of them in CI
(`crates/ldgr-core/tests/vault_vectors.rs`).

### Conventions

- All byte strings are shown in **lowercase hex**; all human-readable strings
  are **UTF-8**.
- Multi-byte integers in the binary format are **little-endian** unless stated
  otherwise (the size-bucket length prefix is the one big-endian field).
- **Nonces are fixed, not random.** Production code generates a fresh random
  nonce for every encryption; these vectors pin explicit nonces purely for
  reproducibility. **Never reuse a `(key, nonce)` pair in real use** — it breaks
  AES-256-GCM. The deterministic helpers that accept explicit nonces live behind
  the test-only `test-vectors` cargo feature and are `#[doc(hidden)]`.

### Cryptographic primitives (recap)

| Step | Algorithm | Domain separation |
| --- | --- | --- |
| Password → MK | Argon2id v0x13, 32-byte output | salt + params |
| MK → MEK | HKDF-SHA256, info `ldgr-enc-v1` | (auth key uses `ldgr-auth-v1`) |
| MEK → VK wrap | AES-256-GCM | AAD `ldgr-vault-wrap-v1` |
| RK → VK wrap | AES-256-GCM | AAD `ldgr-recovery-wrap-v1` |
| VK → IK wrap | AES-256-GCM | AAD `ldgr-item-wrap-v1` |
| IK → item seal | AES-256-GCM over padded payload | AAD `ldgr-item-seal-v1` |
| Recovery key text | Crockford Base32, dash groups of 4 | — |

### Argon2id parameters used by all vectors

To keep the vectors instantly reproducible, every vector uses the minimal
parameter set (this is `Argon2Params::test()` in the reference implementation):

| Parameter | Value |
| --- | --- |
| Algorithm | Argon2id, version `0x13` |
| Memory cost | `64` KiB |
| Iterations (time cost) | `1` |
| Parallelism (lanes) | `1` |
| Output length | `32` bytes |

> Production vaults use much stronger parameters (e.g. desktop: 256 MiB / 3 / 4).
> The KDF output is fully determined by the parameters recorded in the vault
> header, so pinning these values is sufficient for interoperability.

---

## 1. KDF vectors (password → MK → MEK)

For each vector: `MK = Argon2id(password, salt, params)` and
`MEK = HKDF-SHA256-Expand(ikm = MK, salt = none, info = "ldgr-enc-v1", L = 32)`.

Fixtures `kdf-{1,2,3}.bin` each contain `MK || MEK` (64 bytes).

### Vector kdf-1

| Field | Value |
| --- | --- |
| password (UTF-8) | `correct horse battery staple` |
| password (hex) | `636f727265637420686f727365206261747465727920737461706c65` |
| salt (hex) | `000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f` |
| **MK** (hex) | `da1aebe764f4e4a2614b8c4a8488b223e546dec13ffc77a693ac917cf47410f5` |
| **MEK** (hex) | `94328c1306a37e742e2d8f75ed32cc1bdac730639bdba8b62855aab86041d0e0` |

### Vector kdf-2

| Field | Value |
| --- | --- |
| password (UTF-8) | `Tr0ub4dour&3` |
| password (hex) | `547230756234646f75722633` |
| salt (hex) | `a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf` |
| **MK** (hex) | `53548b8c2d2c67f1023ddef17f7964d57e791256aaa4ef4b51802d906219f68a` |
| **MEK** (hex) | `96b96ace39a6d1aead35dea1932cbec9211bdd0252f22b32ea0ec1dca51e7210` |

### Vector kdf-3 (empty password)

| Field | Value |
| --- | --- |
| password (UTF-8) | *(empty string)* |
| password (hex) | *(empty)* |
| salt (hex) | `4242424242424242424242424242424242424242424242424242424242424242` |
| **MK** (hex) | `af1d56a40ea00996024aa2ea7ba13793207fb0be01c7731d00bb369b0c5e8e30` |
| **MEK** (hex) | `4d85b03d5c93bb65070b346b5295d279a13b457ae184f9be6ed106972fb25efc` |

---

## 2. Key wrapping vectors

`WrappedKey = AES-256-GCM-Encrypt(key = wrapping_key, nonce = nonce, aad = tag,
plaintext = wrapped_key)`. The 32-byte key plus the 16-byte GCM tag give a
48-byte `ciphertext`.

The binary sub-format (fixtures `wrap-*.bin`, 65 bytes each) is:

```
version : u8        = 0x01
nonce   : [u8; 12]
ct_len  : u32 LE    = 0x00000030 (48)
ciphertext : [u8; ct_len]
```

### vault-wrap (MEK → VK), AAD `ldgr-vault-wrap-v1`

| Field | Value |
| --- | --- |
| MEK (hex) | `0101…01` (32 bytes of `0x01`) |
| VK (hex) | `0202…02` (32 bytes of `0x02`) |
| nonce (hex) | `000102030405060708090a0b` |
| **ciphertext** (hex) | `b9a4972422fc047bfc68e5c6226bcaa1acc46908f793470273c83a82a3853fb05334198ab6c01782cd89bb58ad793fd5` |

### recovery-wrap (RK → VK), AAD `ldgr-recovery-wrap-v1`

| Field | Value |
| --- | --- |
| RK (hex) | `0303…03` (32 bytes of `0x03`) |
| VK (hex) | `0404…04` (32 bytes of `0x04`) |
| nonce (hex) | `101010101010101010101010` |
| **ciphertext** (hex) | `3e62020eb140a67b05c656c636b854ac8508dcd22189a86d6695c93c55d35120662254ce18d56391128ce0d8169353f0` |

### item-wrap (VK → IK), AAD `ldgr-item-wrap-v1`

| Field | Value |
| --- | --- |
| VK (hex) | `0505…05` (32 bytes of `0x05`) |
| IK (hex) | `0606…06` (32 bytes of `0x06`) |
| nonce (hex) | `202020202020202020202020` |
| **ciphertext** (hex) | `c11848a8bda3479ac05315c39f24e77303592f72f58b36542ae2d124d14608ee495336ea372b6546341ade7b2b9d9059` |

> **Domain separation check:** the same 32-byte plaintext key wrapped under the
> same wrapping key but a different AAD tag produces different ciphertext, and a
> blob wrapped with one tag must fail to unwrap under another. Implementations
> should verify they reject cross-tag unwrapping.

---

## 3. Recovery key encoding vectors

`encode_recovery_key` maps a 256-bit key to a 52-character Crockford Base32
string (alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, excluding `I L O U`),
emitted in dash-separated groups of 4 (13 groups). The 256 bits do not divide
evenly into 5-bit symbols, so the final symbol carries the last bit padded with
4 zero bits.

Fixtures `recovery-*.txt` contain the exact encoded string (no trailing
newline).

| Vector | Key (hex) | Encoded |
| --- | --- | --- |
| recovery-zeros | `00…00` | `0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000` |
| recovery-ff | `ff…ff` | `ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZG` |
| recovery-aa | `aa…aa` (`1010…`) | `NANA-NANA-NANA-NANA-NANA-NANA-NANA-NANA-NANA-NANA-NANA-NANA-NAN0` |
| recovery-55 | `55…55` (`0101…`) | `ANAN-ANAN-ANAN-ANAN-ANAN-ANAN-ANAN-ANAN-ANAN-ANAN-ANAN-ANAN-ANAG` |
| recovery-seq | `000102…1e1f` | `000G-40R4-0M30-E209-185G-R38E-1W81-24GK-2GAH-C5RR-34D1-P70X-3RFG` |

Notes:

- The trailing symbol differs between `ff…ff` (`…ZZZG`) and `aa…aa` (`…NAN0`)
  because of the 4-bit zero padding on the final 5-bit group — a good check that
  an implementation pads (rather than truncates) correctly.
- **Decoding** is case-insensitive, ignores dashes/whitespace, and normalizes
  confusable characters `O → 0` and `I` / `L → 1`. Decoding must reject input
  whose 4 padding bits are non-zero.

---

## 4. Envelope encryption vectors

Per-item sealing pads the plaintext, then encrypts it with the item key, and
wraps the item key with the vault key:

1. `padded = u32_BE(len(plaintext)) || plaintext || zero-pad` to the smallest
   bucket in `{512, 2048, 8192, 32768}` (then multiples of 32768) that fits
   `4 + len(plaintext)`.
2. `payload_ct = AES-256-GCM(key = IK, nonce = seal_nonce, aad =
   "ldgr-item-seal-v1", msg = padded)` — `payload_ct_len = bucket + 16` (tag).
3. `wrapped_ik = AES-256-GCM(key = VK, nonce = ik_wrap_nonce, aad =
   "ldgr-item-wrap-v1", msg = IK)`.

The binary sub-format (fixtures `envelope-*.bin`) is:

```
version    : u8 = 0x01
wrapped_ik : WrappedKey  (version u8 || nonce[12] || ct_len u32 LE || ct)
nonce      : [u8; 12]    (the seal nonce)
ct_len     : u32 LE      (the padded-payload ciphertext length)
ciphertext : [u8; ct_len]
```

All three vectors below share `VK = 0707…07` (32 bytes of `0x07`).

| Vector | IK (hex) | plaintext | bucket | seal nonce | ik-wrap nonce | payload ct len |
| --- | --- | --- | --- | --- | --- | --- |
| envelope-512 | `0808…08` | `ldgr envelope vector: small payload` (35 B) | 512 | `303030…30` | `313131…31` | 528 |
| envelope-2k | `0909…09` | 600 × `0xAB` | 2048 | `323232…32` | `333333…33` | 2064 |
| envelope-8k | `0a0a…0a` | 3000 × `0xCD` | 8192 | `343434…34` | `353535…35` | 8208 |

The full ciphertext bytes are in the fixtures; `payload_ct_len = bucket + 16`
confirms the size-bucket padding is applied before encryption.

---

## 5. Complete vault file vector

Fixture: [`complete-vault.bin`](../../tests/fixtures/vault/complete-vault.bin)
(**2015 bytes**). It is created with known inputs and contains two items. The
reference implementation's CI both **reproduces these exact bytes** and
**opens** the file with `open_vault()` and `recover_vault()`.

### Inputs

| Field | Value |
| --- | --- |
| password (UTF-8) | `correct horse battery staple` |
| salt (hex) | `000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f` |
| Argon2 params | 64 KiB / 1 / 1 (as above) |
| VK (hex) | `1111…11` (32 bytes of `0x11`) |
| RK (hex) | `2222…22` (32 bytes of `0x22`) |
| MEK→VK wrap nonce | `424242424242424242424242` |
| RK→VK wrap nonce | `434343434343434343434343` |
| metadata IK (hex) | `1212…12` |
| metadata seal nonce | `404040404040404040404040` |
| metadata IK-wrap nonce | `414141414141414141414141` |
| item0 IK / seal / wrap nonce | `1313…13` / `444444…44` / `454545…45` |
| item1 IK / seal / wrap nonce | `1414…14` / `464646…46` / `474747…47` |

### Known plaintext (after unlock)

- **metadata JSON:** `{"name":"Test Vault","created_at":"2024-01-01T00:00:00Z"}`
- **item[0]:**
  ```
  2024-01-15 Groceries
    Expenses:Food  42.50 USD
    Assets:Checking
  ```
- **item[1]:**
  ```
  2024-01-16 Salary
    Assets:Checking  3000.00 USD
    Income:Salary
  ```

### Annotated byte layout

Offsets are `[start..end)`. Long ciphertext payloads are shown truncated; the
full bytes are in the fixture.

```
=== FIXED HEADER (51 bytes) ===
[   0..  4) magic              = 4c444752  ("LDGR")
[   4..  6) format_version u16 = 0100      (1)
[   6..  7) kdf_version u8     = 01        (1 = Argon2id v0x13)
[   7.. 39) salt[32]           = 000102…1e1f
[  39.. 43) argon2 mem KiB u32 = 40000000  (64)
[  43.. 47) argon2 iters  u32  = 01000000  (1)
[  47.. 51) argon2 lanes  u32  = 01000000  (1)

=== VARIABLE HEADER ===
-- WrappedKey: MEK-wrapped vault key --
[  51.. 52) version            = 01
[  52.. 64) nonce              = 424242424242424242424242
[  64.. 68) ct_len u32 LE      = 30000000  (48)
[  68..116) ciphertext         = cfaaf8725c206eb0fb619457e0e8c3a2…8344001c

-- WrappedKey: recovery-wrapped vault key --
[ 116..117) version            = 01
[ 117..129) nonce              = 434343434343434343434343
[ 129..133) ct_len u32 LE      = 30000000  (48)
[ 133..181) ciphertext         = a7ba821fb1abeb8ed97b8235eb88acbe…d68becd7

-- SealedEnvelope: encrypted metadata --
[ 181..182) version            = 01
[ 182..183)   wrapped_ik.version = 01
[ 183..195)   wrapped_ik.nonce   = 414141414141414141414141
[ 195..199)   wrapped_ik.ct_len  = 30000000  (48)
[ 199..247)   wrapped_ik.ct      = 4ea8e2092e117084a2ae5d9d005d4da3…e9c572bb
[ 247..259) payload nonce      = 404040404040404040404040
[ 259..263) payload ct_len     = 10020000  (528 = 512 bucket + 16 tag)
[ 263..791) payload ciphertext = e9227f77d4a7108c3fa426d908df053a… (528 bytes)

=== BODY ===
[ 791..795) item_count u32 LE  = 02000000  (2)

-- SealedEnvelope: item[0] --
[ 795..796) version            = 01
[ 796..797)   wrapped_ik.version = 01
[ 797..809)   wrapped_ik.nonce   = 454545454545454545454545
[ 809..813)   wrapped_ik.ct_len  = 30000000  (48)
[ 813..861)   wrapped_ik.ct      = 9c4e913977278eee12ab93ac5e9bb14d…1513548e
[ 861..873) payload nonce      = 444444444444444444444444
[ 873..877) payload ct_len     = 10020000  (528)
[ 877..1405) payload ciphertext = 8213d9e4c43be2180626b5ad53482592… (528 bytes)

-- SealedEnvelope: item[1] --
[1405..1406) version            = 01
[1406..1407)   wrapped_ik.version = 01
[1407..1419)   wrapped_ik.nonce   = 474747474747474747474747
[1419..1423)   wrapped_ik.ct_len  = 30000000  (48)
[1423..1471)   wrapped_ik.ct      = 2e99252b605f2448f24c50f50282da0f…f143a069
[1471..1483) payload nonce      = 464646464646464646464646
[1483..1487) payload ct_len     = 10020000  (528)
[1487..2015) payload ciphertext = 0abbe05e886a2331e910d505713f8473… (528 bytes)

=== total: 2015 bytes ===
```

To unlock: derive `MK` from `password + salt + params`, derive `MEK` (HKDF info
`ldgr-enc-v1`), unwrap the MEK-wrapped vault key (AAD `ldgr-vault-wrap-v1`),
unwrap the metadata item key (AAD `ldgr-item-wrap-v1`), decrypt + unpad the
metadata payload (AAD `ldgr-item-seal-v1`), then repeat for each item.

---

## 6. Negative vectors

Each fixture below is a mutation of `complete-vault.bin` that a conformant
implementation must reject. The reference implementation asserts the exact error
class for each in CI.

| Fixture | Mutation | Expected result |
| --- | --- | --- |
| `negative-bad-magic.bin` | byte `0` set to `0x00` (magic ≠ `LDGR`) | reject — invalid vault (bad magic) |
| `negative-bad-version.bin` | bytes `4..6` set to `99` LE | reject — unsupported version `99` |
| `negative-truncated.bin` | file cut to 30 bytes (inside the 51-byte fixed header) | reject — invalid vault (too short) |
| `negative-excessive-argon2.bin` | memory cost (bytes `39..43`) set to `0xFFFFFFFF` (> 4 GiB max) | reject — invalid vault (param bounds) |
| `negative-corrupted-metadata.bin` | first byte of the metadata payload ciphertext XOR `0xFF` | reject on open — GCM auth failure (decryption failed) |
| `negative-corrupted-item.bin` | final byte (item[1] GCM tag) XOR `0xFF` | opens, but reading item[1] fails — decryption failed |
| `negative-corrupted-recovery-wrap.bin` | first byte of the recovery-wrapped VK ciphertext XOR `0xFF` | password unlock still works; recovery-key unlock fails — unwrap failed |

These cover one case per error class: bad magic, unsupported version, truncated
header, out-of-bounds KDF parameters, and corrupted ciphertext (both
authenticated-header data and item data), plus a corrupted recovery wrap.

---

## Reproducing and regenerating

The reference implementation verifies every vector here against the committed
fixtures on each CI run via `cargo test --workspace --all-features` (the
`test-vectors` feature is enabled by `--all-features`).

If the format ever changes intentionally, regenerate the fixtures with:

```sh
LDGR_REGENERATE_VECTORS=1 cargo test -p ldgr-core --features test-vectors \
    --test vault_vectors
```

and update the hex values in this document from the refreshed
`tests/fixtures/vault/manifest.json`.

## References

- Reference implementation:
  `crates/ldgr-core/src/crypto/{kdf,keys,wrap,envelope,recovery,vault}.rs`
- Vector generator / CI verifier: `crates/ldgr-core/tests/vault_vectors.rs`
- Formal specification: [`vault-format-spec.md`](./vault-format-spec.md)
- Plain-language overview: [`vault-overview.md`](./vault-overview.md)
