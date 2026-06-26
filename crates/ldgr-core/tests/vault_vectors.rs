//! Known-answer test vectors for the ldgr vault format (v1).
//!
//! This suite is the machine-checked counterpart to
//! `docs/security/test-vectors.md`. It reconstructs every published vector
//! from its canonical inputs using deterministic-nonce crypto helpers (the
//! `test-vectors` feature) and asserts the reference implementation reproduces
//! the committed binary fixtures in `tests/fixtures/vault/` byte-for-byte.
//!
//! Regenerate the fixtures (after an intentional format change) with:
//!
//! ```sh
//! LDGR_REGENERATE_VECTORS=1 cargo test -p ldgr-core --features test-vectors \
//!     --test vault_vectors
//! ```
//!
//! The fixtures are otherwise treated as golden files: any drift fails CI.
#![cfg(feature = "test-vectors")]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use ldgr_core::crypto::{
    Argon2Params, CryptoError, ItemKey, MasterEncryptionKey, RecoveryKey, SealedEnvelope,
    VaultHeader, VaultKey, VaultMetadata, WrappedKey, derive_encryption_key, derive_master_key,
    encode_recovery_key, encrypt_item_with, open_vault, recover_vault, serialize_parts,
    serialize_sealed_envelope, serialize_wrapped_key, wrap_item_key_with_nonce,
    wrap_vault_key_with_nonce, wrap_vault_key_with_recovery_with_nonce,
};

const FIXTURES_DIR: &str = "../../tests/fixtures/vault";

/// Format version these vectors target.
const FORMAT_VERSION: u16 = 1;
/// KDF version (1 = Argon2id v0x13).
const KDF_VERSION: u8 = 1;

fn regenerating() -> bool {
    std::env::var_os("LDGR_REGENERATE_VECTORS").is_some()
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES_DIR)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Canonical Argon2id parameters used by all vectors.
///
/// Deliberately the minimal `Argon2Params::test()` (64 KiB / 1 iteration /
/// 1 lane) so third parties can reproduce the vectors instantly. Production
/// vaults use far stronger parameters; the KDF output only depends on the
/// parameters that are recorded in the vault header, which these vectors pin.
fn vector_params() -> Argon2Params {
    Argon2Params::test()
}

/// Build a 32-byte array `start, start+1, ...` (wrapping).
fn seq32(start: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    let mut v = start;
    for b in &mut a {
        *b = v;
        v = v.wrapping_add(1);
    }
    a
}

/// A fixture record: collected for the manifest, checked against disk.
struct Manifest {
    entries: BTreeMap<String, String>,
}

impl Manifest {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    fn note(&mut self, key: &str, value: String) {
        self.entries.insert(key.to_string(), value);
    }

    /// Verify `bytes` equals the committed fixture `name` (or write it when
    /// regenerating). Also records the bytes (hex) in the manifest.
    fn check_bytes(&mut self, name: &str, bytes: &[u8]) {
        let path = fixtures_dir().join(name);
        if regenerating() {
            fs::create_dir_all(fixtures_dir()).expect("create fixtures dir");
            fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {name}: {e}"));
        } else {
            let on_disk = fs::read(&path).unwrap_or_else(|e| {
                panic!("read fixture {name}: {e} (run with LDGR_REGENERATE_VECTORS=1 to create it)")
            });
            assert_eq!(
                hex(&on_disk),
                hex(bytes),
                "fixture {name} does not match the reference implementation"
            );
        }
        self.note(&format!("fixture/{name}"), hex(bytes));
    }

    /// Verify `text` equals the committed fixture `name` (or write it).
    fn check_text(&mut self, name: &str, text: &str) {
        let path = fixtures_dir().join(name);
        if regenerating() {
            fs::create_dir_all(fixtures_dir()).expect("create fixtures dir");
            fs::write(&path, text).unwrap_or_else(|e| panic!("write {name}: {e}"));
        } else {
            let on_disk =
                fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {name}: {e}"));
            assert_eq!(on_disk, text, "fixture {name} does not match");
        }
        self.note(&format!("fixture/{name}"), text.to_string());
    }

    fn finish(self) {
        if !regenerating() {
            return;
        }
        let mut json = String::from("{\n");
        let n = self.entries.len();
        for (i, (k, v)) in self.entries.iter().enumerate() {
            let comma = if i + 1 < n { "," } else { "" };
            let _ = writeln!(json, "  {k:?}: {v:?}{comma}");
        }
        json.push_str("}\n");
        fs::write(fixtures_dir().join("manifest.json"), json).expect("write manifest");
    }
}

// ── 1. KDF vectors ──────────────────────────────────────────────────────────────

fn kdf_vectors(m: &mut Manifest) {
    let params = vector_params();
    let cases: &[(&str, &[u8], [u8; 32])] = &[
        ("kdf-1", b"correct horse battery staple", seq32(0x00)),
        ("kdf-2", b"Tr0ub4dour&3", seq32(0xA0)),
        ("kdf-3", b"", [0x42u8; 32]),
    ];

    for (name, password, salt) in cases {
        let mk = derive_master_key(password, salt, &params).unwrap();
        let mek = derive_encryption_key(&mk).unwrap();

        m.note(
            &format!("{name}/password.utf8"),
            String::from_utf8_lossy(password).into_owned(),
        );
        m.note(&format!("{name}/password.hex"), hex(password));
        m.note(&format!("{name}/salt.hex"), hex(salt));
        m.note(&format!("{name}/mk.hex"), hex(&mk.to_test_bytes()));
        m.note(&format!("{name}/mek.hex"), hex(&mek.to_test_bytes()));

        // Fixture = MK || MEK (64 bytes).
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&mk.to_test_bytes());
        out.extend_from_slice(&mek.to_test_bytes());
        m.check_bytes(&format!("{name}.bin"), &out);
    }
}

// ── 2. Key wrapping vectors ──────────────────────────────────────────────────────

fn wrap_vectors(m: &mut Manifest) {
    // vault-wrap: MEK → VK
    {
        let mek = MasterEncryptionKey::from_test_bytes([0x01u8; 32]);
        let vk = VaultKey::from_test_bytes([0x02u8; 32]);
        let nonce: [u8; 12] = seq32(0x00)[..12].try_into().unwrap();
        let wrapped = wrap_vault_key_with_nonce(&mek, &vk, &nonce).unwrap();
        record_wrap(m, "wrap-vault", &nonce, &wrapped);
    }
    // recovery-wrap: RK → VK
    {
        let rk = RecoveryKey::from_test_bytes([0x03u8; 32]);
        let vk = VaultKey::from_test_bytes([0x04u8; 32]);
        let nonce = [0x10u8; 12];
        let wrapped = wrap_vault_key_with_recovery_with_nonce(&rk, &vk, &nonce).unwrap();
        record_wrap(m, "wrap-recovery", &nonce, &wrapped);
    }
    // item-wrap: VK → IK
    {
        let vk = VaultKey::from_test_bytes([0x05u8; 32]);
        let ik = ItemKey::from_test_bytes([0x06u8; 32]);
        let nonce = [0x20u8; 12];
        let wrapped = wrap_item_key_with_nonce(&vk, &ik, &nonce).unwrap();
        record_wrap(m, "wrap-item", &nonce, &wrapped);
    }
}

fn record_wrap(m: &mut Manifest, name: &str, nonce: &[u8; 12], wrapped: &WrappedKey) {
    assert_eq!(&wrapped.nonce, nonce, "{name} nonce must be deterministic");
    m.note(&format!("{name}/nonce.hex"), hex(nonce));
    m.note(&format!("{name}/ciphertext.hex"), hex(&wrapped.ciphertext));
    let bytes = serialize_wrapped_key(wrapped).unwrap();
    m.check_bytes(&format!("{name}.bin"), &bytes);
}

// ── 3. Recovery key encoding vectors ─────────────────────────────────────────────

fn recovery_encoding_vectors(m: &mut Manifest) {
    let cases: &[(&str, [u8; 32])] = &[
        ("recovery-zeros", [0x00u8; 32]),
        ("recovery-ff", [0xFFu8; 32]),
        ("recovery-aa", [0xAAu8; 32]), // 1010_1010 alternating bits
        ("recovery-55", [0x55u8; 32]), // 0101_0101 alternating bits
        ("recovery-seq", seq32(0x00)),
    ];
    for (name, bytes) in cases {
        let key = RecoveryKey::from_test_bytes(*bytes);
        let encoded = encode_recovery_key(&key);
        m.note(&format!("{name}/key.hex"), hex(bytes));
        m.note(&format!("{name}/encoded"), encoded.clone());
        m.check_text(&format!("{name}.txt"), &encoded);
    }
}

// ── 4. Envelope encryption vectors ───────────────────────────────────────────────

fn envelope_vectors(m: &mut Manifest) {
    struct EnvCase {
        name: &'static str,
        ik: [u8; 32],
        plaintext: Vec<u8>,
        seal_nonce: [u8; 12],
        ik_wrap_nonce: [u8; 12],
    }

    let vk = VaultKey::from_test_bytes([0x07u8; 32]);

    let cases = [
        EnvCase {
            name: "envelope-512",
            ik: [0x08u8; 32],
            plaintext: b"ldgr envelope vector: small payload".to_vec(),
            seal_nonce: [0x30u8; 12],
            ik_wrap_nonce: [0x31u8; 12],
        },
        EnvCase {
            name: "envelope-2k",
            ik: [0x09u8; 32],
            plaintext: vec![0xABu8; 600],
            seal_nonce: [0x32u8; 12],
            ik_wrap_nonce: [0x33u8; 12],
        },
        EnvCase {
            name: "envelope-8k",
            ik: [0x0Au8; 32],
            plaintext: vec![0xCDu8; 3000],
            seal_nonce: [0x34u8; 12],
            ik_wrap_nonce: [0x35u8; 12],
        },
    ];

    for c in &cases {
        let name = c.name;
        let ik = ItemKey::from_test_bytes(c.ik);
        let env =
            encrypt_item_with(&vk, &ik, &c.plaintext, &c.seal_nonce, &c.ik_wrap_nonce).unwrap();
        m.note(
            &format!("{name}/plaintext_len"),
            c.plaintext.len().to_string(),
        );
        m.note(&format!("{name}/seal_nonce.hex"), hex(&c.seal_nonce));
        m.note(&format!("{name}/ik_wrap_nonce.hex"), hex(&c.ik_wrap_nonce));
        m.note(
            &format!("{name}/payload_ct_len"),
            env.ciphertext.len().to_string(),
        );
        let bytes = serialize_sealed_envelope(&env).unwrap();
        m.check_bytes(&format!("{name}.bin"), &bytes);
    }
}

// ── 5. Complete vault vector ─────────────────────────────────────────────────────

/// Canonical inputs for the complete 2-item vault vector.
struct CompleteVault {
    password: &'static [u8],
    salt: [u8; 32],
    params: Argon2Params,
    vk: [u8; 32],
    rk: [u8; 32],
    vault_wrap_nonce: [u8; 12],
    recovery_wrap_nonce: [u8; 12],
    metadata_name: &'static str,
    metadata_ik: [u8; 32],
    metadata_seal_nonce: [u8; 12],
    metadata_ik_wrap_nonce: [u8; 12],
    items: Vec<CompleteItem>,
}

struct CompleteItem {
    plaintext: Vec<u8>,
    ik: [u8; 32],
    seal_nonce: [u8; 12],
    ik_wrap_nonce: [u8; 12],
}

fn complete_vault_inputs() -> CompleteVault {
    CompleteVault {
        password: b"correct horse battery staple",
        salt: seq32(0x00),
        params: vector_params(),
        vk: [0x11u8; 32],
        rk: [0x22u8; 32],
        vault_wrap_nonce: [0x42u8; 12],
        recovery_wrap_nonce: [0x43u8; 12],
        metadata_name: "Test Vault",
        metadata_ik: [0x12u8; 32],
        metadata_seal_nonce: [0x40u8; 12],
        metadata_ik_wrap_nonce: [0x41u8; 12],
        items: vec![
            CompleteItem {
                plaintext: b"2024-01-15 Groceries\n  Expenses:Food  42.50 USD\n  Assets:Checking"
                    .to_vec(),
                ik: [0x13u8; 32],
                seal_nonce: [0x44u8; 12],
                ik_wrap_nonce: [0x45u8; 12],
            },
            CompleteItem {
                plaintext: b"2024-01-16 Salary\n  Assets:Checking  3000.00 USD\n  Income:Salary"
                    .to_vec(),
                ik: [0x14u8; 32],
                seal_nonce: [0x46u8; 12],
                ik_wrap_nonce: [0x47u8; 12],
            },
        ],
    }
}

/// Build the complete vault deterministically, returning the serialized bytes
/// and the header (so negative vectors can compute byte offsets).
fn build_complete_vault(cv: &CompleteVault) -> (Vec<u8>, VaultHeader, Vec<SealedEnvelope>) {
    let mk = derive_master_key(cv.password, &cv.salt, &cv.params).unwrap();
    let mek = derive_encryption_key(&mk).unwrap();
    let vk = VaultKey::from_test_bytes(cv.vk);
    let rk = RecoveryKey::from_test_bytes(cv.rk);

    let wrapped_vk = wrap_vault_key_with_nonce(&mek, &vk, &cv.vault_wrap_nonce).unwrap();
    let recovery_wrapped_vk =
        wrap_vault_key_with_recovery_with_nonce(&rk, &vk, &cv.recovery_wrap_nonce).unwrap();

    // Fixed, deterministic creation timestamp.
    let created_at = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let metadata = VaultMetadata {
        name: cv.metadata_name.to_string(),
        created_at,
    };
    let metadata_json = serde_json::to_vec(&metadata).unwrap();
    let metadata_ik = ItemKey::from_test_bytes(cv.metadata_ik);
    let encrypted_metadata = encrypt_item_with(
        &vk,
        &metadata_ik,
        &metadata_json,
        &cv.metadata_seal_nonce,
        &cv.metadata_ik_wrap_nonce,
    )
    .unwrap();

    let header = VaultHeader {
        format_version: FORMAT_VERSION,
        kdf_version: KDF_VERSION,
        salt: cv.salt,
        argon2_params: cv.params.clone(),
        wrapped_vk,
        recovery_wrapped_vk,
        encrypted_metadata,
    };

    let items: Vec<SealedEnvelope> = cv
        .items
        .iter()
        .map(|it| {
            let ik = ItemKey::from_test_bytes(it.ik);
            encrypt_item_with(&vk, &ik, &it.plaintext, &it.seal_nonce, &it.ik_wrap_nonce).unwrap()
        })
        .collect();

    let bytes = serialize_parts(&header, &items).unwrap();
    (bytes, header, items)
}

fn complete_vault_vector(m: &mut Manifest) -> (Vec<u8>, VaultHeader) {
    let cv = complete_vault_inputs();
    let (bytes, header, _items) = build_complete_vault(&cv);

    m.note(
        "complete-vault/password.utf8",
        String::from_utf8_lossy(cv.password).into_owned(),
    );
    m.note("complete-vault/salt.hex", hex(&cv.salt));
    m.note("complete-vault/vk.hex", hex(&cv.vk));
    m.note("complete-vault/rk.hex", hex(&cv.rk));
    m.note("complete-vault/total_len", bytes.len().to_string());

    // Record the decrypted metadata + item plaintexts so the doc can show the
    // known plaintext that the vault encrypts.
    {
        let created_at = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let metadata = VaultMetadata {
            name: cv.metadata_name.to_string(),
            created_at,
        };
        let metadata_json = serde_json::to_vec(&metadata).unwrap();
        m.note(
            "complete-vault/metadata.json",
            String::from_utf8_lossy(&metadata_json).into_owned(),
        );
        for (i, it) in cv.items.iter().enumerate() {
            m.note(
                &format!("complete-vault/item{i}.plaintext"),
                String::from_utf8_lossy(&it.plaintext).into_owned(),
            );
        }
    }

    m.check_bytes("complete-vault.bin", &bytes);

    // The reference implementation must open the vector it produced.
    let opened =
        open_vault(&bytes, cv.password).expect("open_vault must accept the complete vault vector");
    assert_eq!(opened.metadata().name, cv.metadata_name);
    assert_eq!(opened.item_count(), cv.items.len());
    for (i, it) in cv.items.iter().enumerate() {
        assert_eq!(
            opened.get_item(i).unwrap(),
            it.plaintext,
            "item {i} must round-trip through open_vault"
        );
    }

    // The recovery key must also unlock it.
    let recovered = recover_vault(
        &bytes,
        &RecoveryKey::from_test_bytes(cv.rk),
        b"new-password",
        &cv.params,
    )
    .expect("recovery key must unlock the complete vault vector");
    assert_eq!(recovered.metadata().name, cv.metadata_name);

    (bytes, header)
}

// ── 6. Negative vectors ──────────────────────────────────────────────────────────

fn wk_encoded_len(wk: &WrappedKey) -> usize {
    1 /* version */ + 12 /* nonce */ + 4 /* ct_len */ + wk.ciphertext.len()
}

fn negative_vectors(m: &mut Manifest, valid: &[u8], header: &VaultHeader) {
    let password = complete_vault_inputs().password;

    // (a) Corrupted magic bytes → InvalidVault.
    {
        let mut bad = valid.to_vec();
        bad[0] = 0x00;
        m.check_bytes("negative-bad-magic.bin", &bad);
        let err = open_vault(&bad, password).unwrap_err();
        assert!(
            matches!(err, CryptoError::InvalidVault(_)),
            "bad magic: {err:?}"
        );
    }

    // (b) Unsupported format version → UnsupportedVersion.
    {
        let mut bad = valid.to_vec();
        bad[4..6].copy_from_slice(&99u16.to_le_bytes());
        m.check_bytes("negative-bad-version.bin", &bad);
        let err = open_vault(&bad, password).unwrap_err();
        assert!(
            matches!(err, CryptoError::UnsupportedVersion(99)),
            "bad version: {err:?}"
        );
    }

    // (c) Truncated header (cut inside the fixed 51-byte header) → InvalidVault.
    {
        let bad = valid[..30].to_vec();
        m.check_bytes("negative-truncated.bin", &bad);
        let err = open_vault(&bad, password).unwrap_err();
        assert!(
            matches!(err, CryptoError::InvalidVault(_)),
            "truncated: {err:?}"
        );
    }

    // (d) Excessive Argon2 memory cost (> 4 GiB max) → InvalidVault.
    {
        let mut bad = valid.to_vec();
        bad[39..43].copy_from_slice(&u32::MAX.to_le_bytes());
        m.check_bytes("negative-excessive-argon2.bin", &bad);
        let err = open_vault(&bad, password).unwrap_err();
        assert!(
            matches!(err, CryptoError::InvalidVault(_)),
            "excessive argon2: {err:?}"
        );
    }

    // (e) Corrupted metadata ciphertext → DecryptionFailed (open_vault fails).
    {
        // Offset of the first byte of the metadata payload ciphertext.
        let meta_ct_off = 51
            + wk_encoded_len(&header.wrapped_vk)
            + wk_encoded_len(&header.recovery_wrapped_vk)
            + 1 // envelope version
            + wk_encoded_len(&header.encrypted_metadata.wrapped_ik)
            + 12 // payload nonce
            + 4; // payload ct_len
        let mut bad = valid.to_vec();
        bad[meta_ct_off] ^= 0xFF;
        m.check_bytes("negative-corrupted-metadata.bin", &bad);
        let err = open_vault(&bad, password).unwrap_err();
        assert!(
            matches!(err, CryptoError::DecryptionFailed(_)),
            "corrupted metadata: {err:?}"
        );
    }

    // (f) Corrupted item ciphertext → opens, but reading the item fails.
    {
        let mut bad = valid.to_vec();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF; // final item's GCM tag
        m.check_bytes("negative-corrupted-item.bin", &bad);
        let opened = open_vault(&bad, password).expect("metadata still intact");
        let err = opened.get_item(opened.item_count() - 1).unwrap_err();
        assert!(
            matches!(err, CryptoError::DecryptionFailed(_)),
            "corrupted item: {err:?}"
        );
    }

    // (g) Corrupted recovery-wrapped vault key → password opens, recovery fails.
    {
        let rec_ct_off = 51 + wk_encoded_len(&header.wrapped_vk) + 1 + 12 + 4;
        let mut bad = valid.to_vec();
        bad[rec_ct_off] ^= 0xFF;
        m.check_bytes("negative-corrupted-recovery-wrap.bin", &bad);
        // Password path is unaffected.
        open_vault(&bad, password).expect("password unlock unaffected by recovery-wrap corruption");
        // Recovery path fails.
        let cv = complete_vault_inputs();
        let err = recover_vault(&bad, &RecoveryKey::from_test_bytes(cv.rk), b"x", &cv.params)
            .unwrap_err();
        assert!(
            matches!(err, CryptoError::UnwrapFailed),
            "corrupted recovery wrap: {err:?}"
        );
    }
}

// ── Orchestrating test ───────────────────────────────────────────────────────────

#[test]
fn vault_format_test_vectors() {
    let mut m = Manifest::new();

    kdf_vectors(&mut m);
    wrap_vectors(&mut m);
    recovery_encoding_vectors(&mut m);
    envelope_vectors(&mut m);
    let (complete, header) = complete_vault_vector(&mut m);
    negative_vectors(&mut m, &complete, &header);

    m.finish();
}
