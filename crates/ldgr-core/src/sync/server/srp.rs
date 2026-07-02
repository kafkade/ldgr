//! SRP-6a **client**-side implementation (RFC 5054).
//!
//! This mirrors the server's parameters and formulas exactly
//! (`crates/ldgr-server/src/auth/srp.rs`): the 2048-bit group from RFC 5054
//! Appendix A, SHA-256 as the hash function, and big-endian values left-padded
//! to the length of `N` before hashing.
//!
//! The flow:
//! 1. **Registration** — derive `x` from the username/password/salt, compute the
//!    verifier `v = g^x mod N`, and send `(salt, v)` to the server. The server
//!    never sees the password.
//! 2. **Login init** — generate an ephemeral secret `a` and send `A = g^a mod N`.
//! 3. **Login verify** — given the server's `salt` and `B`, compute the shared
//!    secret `S`, the session key `K = H(S)`, and the client proof `M1`. Send
//!    `M1`; on success the server returns `M2`, which the client verifies.
//!
//! Pure computation — no I/O. Secret material is kept out of `Debug` output.

use std::fmt;

use num_bigint::BigUint;
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::{AuthKey, SecretKey, derive_x_seed};

// ── RFC 5054 2048-bit Group Parameters ────────────────────────────────────────

/// 2048-bit prime N from RFC 5054, Appendix A.
const N_HEX: &str = concat!(
    "AC6BDB41324A9A9BF166DE5E1389582F",
    "AF72B6651987EE07FC3192943DB56050",
    "A37329CBB4A099ED8193E0757767A13D",
    "D52312AB4B03310DCD7F48A9DA04FD50",
    "E8083969EDB767B0CF6095179A163AB3",
    "661A05FBD5FAAAE82918A9962F0B93B8",
    "55F97993EC975EEAA80D740ADBF4FF74",
    "7359D041D5C33EA71D281E446B14773B",
    "CA97B43A23FB801676BD207A436C6481",
    "F1D2B9078717461A5B9D32E688F87748",
    "544523B524B0D57D5EA77A2775D2ECFA",
    "032CFBDBF52FB3786160279004E57AE6",
    "AF874E7303CE53299CCC041C7BC308D8",
    "2A5698F3A8D0C38271AE35F8E9DBFBB6",
    "94B5C803D89F7AE435DE236D525F5475",
    "9B65E372FCD68EF20FA7111F9E4AFF73",
);

/// Generator g = 2.
const G_VALUE: u32 = 2;

/// Byte length of N (2048 bits = 256 bytes).
const N_LEN: usize = 256;

/// Byte length of the client ephemeral secret `a` (256 bits).
const EPHEMERAL_LEN: usize = 32;

/// Default salt length for new registrations (bytes). The server requires at
/// least 16 bytes.
const SALT_LEN: usize = 16;

fn get_n() -> BigUint {
    BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("hardcoded N is valid hex")
}

fn get_g() -> BigUint {
    BigUint::from(G_VALUE)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Left-pad a `BigUint` to `N_LEN` bytes (matches the server's `pad`).
fn pad(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    if bytes.len() >= N_LEN {
        return bytes;
    }
    let mut padded = vec![0u8; N_LEN - bytes.len()];
    padded.extend_from_slice(&bytes);
    padded
}

/// SHA-256 hash of concatenated inputs.
fn hash(slices: &[&[u8]]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for s in slices {
        hasher.update(s);
    }
    hasher.finalize().to_vec()
}

/// Compute k = H(pad(N) || pad(g)) — SRP multiplier.
fn compute_k() -> BigUint {
    let n = get_n();
    let g = get_g();
    let h = hash(&[&pad(&n), &pad(&g)]);
    BigUint::from_bytes_be(&h)
}

/// Compute u = H(pad(A) || pad(B)).
fn compute_u(a: &BigUint, b: &BigUint) -> BigUint {
    let h = hash(&[&pad(a), &pad(b)]);
    BigUint::from_bytes_be(&h)
}

/// Compute the SRP private key x = H(salt || H(username || ":" || password)).
///
/// This derivation is a client-internal convention. The server only stores and
/// uses the resulting verifier, so the only requirement is that registration
/// and login derive `x` identically.
fn compute_x(username: &str, password: &[u8], salt: &[u8]) -> BigUint {
    let inner = hash(&[username.as_bytes(), b":", password]);
    let h = hash(&[salt, &inner]);
    BigUint::from_bytes_be(&h)
}

/// Compute the SRP private key `x` from the two-secret derivation (ADR-008).
///
/// `x = OS2IP(x_seed) mod N`, where `x_seed` mixes the master auth key
/// (`MK_auth`) and the account [`SecretKey`], bound to `account_id` and the
/// per-account `srp_salt`. The server only ever sees the resulting verifier.
fn compute_x_2skd(
    account_id: &Uuid,
    mk_auth: &AuthKey,
    secret_key: &SecretKey,
    salt: &[u8],
) -> BigUint {
    let x_seed = derive_x_seed(mk_auth, secret_key, account_id.as_bytes(), salt)
        .expect("HKDF expansion to 32 bytes is infallible");
    BigUint::from_bytes_be(x_seed.as_slice()) % get_n()
}

/// Compute the session key K = H(pad(S)).
fn compute_session_key(s: &BigUint) -> Vec<u8> {
    hash(&[&pad(s)])
}

/// Compute M1 = H(H(N) XOR H(g) || H(username) || salt || pad(A) || pad(B) || K).
fn compute_m1(username: &str, salt: &[u8], a: &BigUint, b_pub: &BigUint, key: &[u8]) -> Vec<u8> {
    let n = get_n();
    let g = get_g();

    let h_n = hash(&[&pad(&n)]);
    let h_g = hash(&[&pad(&g)]);

    let xor: Vec<u8> = h_n.iter().zip(h_g.iter()).map(|(x, y)| x ^ y).collect();
    let h_user = hash(&[username.as_bytes()]);

    hash(&[&xor, &h_user, salt, &pad(a), &pad(b_pub), key])
}

/// Compute M2 = H(pad(A) || M1 || K).
fn compute_m2(a: &BigUint, m1: &[u8], key: &[u8]) -> Vec<u8> {
    hash(&[&pad(a), m1, key])
}

/// Constant-time byte comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ── Errors ─────────────────────────────────────────────────────────────────────

/// Errors raised during the SRP-6a client handshake.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SrpError {
    /// The server's public value B is invalid (B ≡ 0 mod N).
    #[error("invalid server public value")]
    InvalidServerPublic,
    /// The scrambling parameter u computed to zero.
    #[error("invalid u value")]
    InvalidU,
    /// A two-secret (2SKD) login was finished without an account id — the
    /// server did not return one at `login/init` (e.g. a legacy/1-secret
    /// account or an outdated server).
    #[error("missing account id for two-secret login")]
    MissingAccountId,
}

// ── Registration ───────────────────────────────────────────────────────────────

/// The output of SRP-6a registration. Send `salt` and `verifier` (hex-encoded)
/// to the server's `/register` endpoint. Neither value reveals the password.
#[derive(Clone, PartialEq, Eq)]
pub struct RegistrationVerifier {
    /// Random per-user salt (big-endian bytes).
    pub salt: Vec<u8>,
    /// SRP verifier v = g^x mod N (big-endian bytes).
    pub verifier: Vec<u8>,
}

impl fmt::Debug for RegistrationVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistrationVerifier")
            .field("salt_len", &self.salt.len())
            .field("verifier", &"[REDACTED]")
            .finish()
    }
}

/// Generate a registration verifier with a freshly generated random salt.
#[must_use]
pub fn register(username: &str, password: &[u8]) -> RegistrationVerifier {
    let mut salt = vec![0u8; SALT_LEN];
    rand::rng().fill_bytes(&mut salt);
    register_with_salt(username, password, salt)
}

/// Generate a registration verifier with a caller-supplied salt.
///
/// Primarily useful for deterministic tests; production code should prefer
/// [`register`], which generates a random salt.
#[must_use]
pub fn register_with_salt(username: &str, password: &[u8], salt: Vec<u8>) -> RegistrationVerifier {
    let n = get_n();
    let g = get_g();
    let x = compute_x(username, password, &salt);
    let v = g.modpow(&x, &n);
    RegistrationVerifier {
        salt,
        verifier: v.to_bytes_be(),
    }
}

/// Generate a **two-secret (2SKD)** registration verifier with a fresh random
/// salt (ADR-008, Decision 1).
///
/// The verifier is derived from both the master auth key (`MK_auth`, the
/// existing [`AuthKey`]) and the account [`SecretKey`]. The server stores only
/// `(salt, verifier)` and never receives either secret.
#[must_use]
pub fn register_2skd(
    account_id: &Uuid,
    mk_auth: &AuthKey,
    secret_key: &SecretKey,
) -> RegistrationVerifier {
    let mut salt = vec![0u8; SALT_LEN];
    rand::rng().fill_bytes(&mut salt);
    register_2skd_with_salt(account_id, mk_auth, secret_key, salt)
}

/// Generate a two-secret (2SKD) registration verifier with a caller-supplied
/// salt. Primarily for deterministic tests; production code should prefer
/// [`register_2skd`].
#[must_use]
pub fn register_2skd_with_salt(
    account_id: &Uuid,
    mk_auth: &AuthKey,
    secret_key: &SecretKey,
    salt: Vec<u8>,
) -> RegistrationVerifier {
    let n = get_n();
    let g = get_g();
    let x = compute_x_2skd(account_id, mk_auth, secret_key, &salt);
    let v = g.modpow(&x, &n);
    RegistrationVerifier {
        salt,
        verifier: v.to_bytes_be(),
    }
}

// ── Login ──────────────────────────────────────────────────────────────────────

/// The secret a client uses to derive the SRP private key `x` on login.
enum Credential {
    /// Legacy single-secret: `x = H(salt || H(username || ":" || password))`.
    Password(Zeroizing<Vec<u8>>),
    /// Two-secret (2SKD): `x = OS2IP(x_seed) mod N` (ADR-008).
    ///
    /// `account_id` is resolved from the server's `login/init` response
    /// ([`ClientLogin::set_account_id`]) before [`ClientLogin::finish`], so it
    /// starts unset.
    TwoSecret {
        account_id: Option<Uuid>,
        mk_auth: AuthKey,
        secret_key: SecretKey,
    },
}

/// Secret client state between login init and verify.
///
/// Holds the ephemeral secret `a`, the public `A`, and the credentials needed
/// to finish the handshake. Consumed by [`ClientLogin::finish`].
pub struct ClientLogin {
    username: String,
    credential: Credential,
    a: BigUint,
    a_pub: BigUint,
}

impl fmt::Debug for ClientLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientLogin")
            .field("username", &self.username)
            .field("a", &"[REDACTED]")
            .field("credential", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl ClientLogin {
    /// Begin a login: generate the ephemeral secret `a` and public `A = g^a mod N`.
    ///
    /// Returns the state to retain plus the big-endian bytes of `A` to send to
    /// the server's `/login/init` endpoint.
    #[must_use]
    pub fn start(username: &str, password: &[u8]) -> (Self, Vec<u8>) {
        Self::start_with_credential(
            username,
            Credential::Password(Zeroizing::new(password.to_vec())),
        )
    }

    /// Begin a **two-secret (2SKD)** login (ADR-008).
    ///
    /// `mk_auth` is the existing [`AuthKey`] derived from the master password;
    /// `secret_key` is the account [`SecretKey`]. Both are required to
    /// reproduce the verifier's `x` — the password or Secret Key alone cannot.
    ///
    /// The account id is **not** required here: it is learned from the server's
    /// `login/init` response and injected via [`set_account_id`] before
    /// [`finish`]. Calling `finish` without it yields
    /// [`SrpError::MissingAccountId`].
    ///
    /// [`set_account_id`]: ClientLogin::set_account_id
    /// [`finish`]: ClientLogin::finish
    #[must_use]
    pub fn start_2skd(username: &str, mk_auth: AuthKey, secret_key: SecretKey) -> (Self, Vec<u8>) {
        Self::start_with_credential(
            username,
            Credential::TwoSecret {
                account_id: None,
                mk_auth,
                secret_key,
            },
        )
    }

    /// Inject the account id learned from the server's `login/init` response.
    ///
    /// No-op for a single-secret login. Must be called before [`finish`] on a
    /// two-secret login.
    ///
    /// [`finish`]: ClientLogin::finish
    pub fn set_account_id(&mut self, account_id: Uuid) {
        if let Credential::TwoSecret {
            account_id: slot, ..
        } = &mut self.credential
        {
            *slot = Some(account_id);
        }
    }

    fn start_with_credential(username: &str, credential: Credential) -> (Self, Vec<u8>) {
        let n = get_n();
        let g = get_g();

        let mut a_bytes = [0u8; EPHEMERAL_LEN];
        rand::rng().fill_bytes(&mut a_bytes);
        let a = BigUint::from_bytes_be(&a_bytes);
        let a_pub = g.modpow(&a, &n);

        let a_pub_bytes = a_pub.to_bytes_be();
        let state = Self {
            username: username.to_string(),
            credential,
            a,
            a_pub,
        };
        (state, a_pub_bytes)
    }

    /// The big-endian bytes of the client public value `A`.
    #[must_use]
    pub fn public(&self) -> Vec<u8> {
        self.a_pub.to_bytes_be()
    }

    /// Finish the handshake given the server's `salt` and public value `B`.
    ///
    /// Computes the shared secret, session key, and client proof `M1`. The
    /// returned [`ClientSession`] carries `M1` (to send to `/login/verify`) and
    /// can verify the server's `M2`.
    ///
    /// # Errors
    ///
    /// Returns [`SrpError::InvalidServerPublic`] if `B ≡ 0 (mod N)` and
    /// [`SrpError::InvalidU`] if the scrambling parameter is zero.
    #[allow(clippy::many_single_char_names)] // SRP variable names match RFC 5054 spec
    pub fn finish(self, salt: &[u8], server_public: &[u8]) -> Result<ClientSession, SrpError> {
        let n = get_n();
        let g = get_g();

        let b_pub = BigUint::from_bytes_be(server_public);
        if &b_pub % &n == BigUint::from(0u32) {
            return Err(SrpError::InvalidServerPublic);
        }

        let u = compute_u(&self.a_pub, &b_pub);
        if u == BigUint::from(0u32) {
            return Err(SrpError::InvalidU);
        }

        let k = compute_k();
        let x = match &self.credential {
            Credential::Password(password) => compute_x(&self.username, password, salt),
            Credential::TwoSecret {
                account_id,
                mk_auth,
                secret_key,
            } => {
                let account_id = (*account_id).ok_or(SrpError::MissingAccountId)?;
                compute_x_2skd(&account_id, mk_auth, secret_key, salt)
            }
        };

        // S = (B - k * g^x) ^ (a + u * x) mod N
        let gx = g.modpow(&x, &n);
        let kgx = (&k * &gx) % &n;
        // Add N before subtracting to stay non-negative (B, kgx are both < N).
        let base = (&b_pub + &n - kgx) % &n;
        let exp = &self.a + &u * &x;
        let s = base.modpow(&exp, &n);

        let key = Zeroizing::new(compute_session_key(&s));
        let m1 = compute_m1(&self.username, salt, &self.a_pub, &b_pub, &key);

        Ok(ClientSession {
            a_pub: self.a_pub,
            m1,
            key,
        })
    }
}

/// A completed client-side SRP handshake.
///
/// Carries the client proof `M1` to submit to `/login/verify`, the shared
/// session key `K`, and the ability to verify the server's proof `M2`.
pub struct ClientSession {
    a_pub: BigUint,
    m1: Vec<u8>,
    key: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for ClientSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientSession")
            .field("key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl ClientSession {
    /// The client proof `M1` to send to the server.
    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.m1
    }

    /// The shared session key `K = H(S)`. Treat as secret key material.
    #[must_use]
    pub fn session_key(&self) -> &[u8] {
        &self.key
    }

    /// Verify the server's proof `M2 = H(pad(A) || M1 || K)` in constant time.
    #[must_use]
    pub fn verify_server_proof(&self, server_proof: &[u8]) -> bool {
        let expected = compute_m2(&self.a_pub, &self.m1, &self.key);
        constant_time_eq(server_proof, &expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A faithful re-implementation of the server's SRP-6a math, used to verify
    /// that the client interoperates with the server's verifier logic.
    ///
    /// Mirrors `crates/ldgr-server/src/auth/srp.rs`.
    struct ReferenceServer {
        username: String,
        salt: Vec<u8>,
        verifier: BigUint,
        a: BigUint,
        b_secret: BigUint,
        b_pub: BigUint,
    }

    impl ReferenceServer {
        #[allow(clippy::many_single_char_names)] // SRP variable names match RFC 5054 spec
        fn generate_server_public(v: &BigUint, b: &BigUint) -> BigUint {
            let n = get_n();
            let g = get_g();
            let k = compute_k();
            let gb = g.modpow(b, &n);
            let kv = (&k * v) % &n;
            (&kv + &gb) % &n
        }

        fn initiate(username: &str, salt: &[u8], verifier: &[u8], client_public: &[u8]) -> Self {
            use rand::Rng as _;
            let v = BigUint::from_bytes_be(verifier);
            let a = BigUint::from_bytes_be(client_public);
            // Deterministic-ish server ephemeral for tests.
            let mut b_bytes = [0u8; 32];
            rand::rng().fill_bytes(&mut b_bytes);
            let b_secret = BigUint::from_bytes_be(&b_bytes);
            let b_pub = Self::generate_server_public(&v, &b_secret);
            Self {
                username: username.to_string(),
                salt: salt.to_vec(),
                verifier: v,
                a,
                b_secret,
                b_pub,
            }
        }

        fn server_public_bytes(&self) -> Vec<u8> {
            self.b_pub.to_bytes_be()
        }

        /// Verify the client proof and return the server proof M2 on success.
        fn verify(&self, client_proof: &[u8]) -> Option<Vec<u8>> {
            let n = get_n();
            let u = compute_u(&self.a, &self.b_pub);
            if u == BigUint::from(0u32) {
                return None;
            }
            // S = (A · v^u)^b mod N
            let v_u = self.verifier.modpow(&u, &n);
            let a_vu = (&self.a * &v_u) % &n;
            let s = a_vu.modpow(&self.b_secret, &n);
            let key = compute_session_key(&s);
            let expected_m1 = compute_m1(&self.username, &self.salt, &self.a, &self.b_pub, &key);
            if !constant_time_eq(client_proof, &expected_m1) {
                return None;
            }
            Some(compute_m2(&self.a, client_proof, &key))
        }
    }

    fn run_handshake(username: &str, password: &[u8]) -> bool {
        let reg = register(username, password);

        let (login, a_pub) = ClientLogin::start(username, password);
        let server = ReferenceServer::initiate(username, &reg.salt, &reg.verifier, &a_pub);

        let session = login
            .finish(&reg.salt, &server.server_public_bytes())
            .expect("client finish");

        let Some(m2) = server.verify(session.proof()) else {
            return false;
        };
        session.verify_server_proof(&m2)
    }

    #[test]
    fn n_parses_to_expected_length() {
        assert_eq!(get_n().to_bytes_be().len(), N_LEN);
    }

    #[test]
    fn successful_handshake() {
        assert!(run_handshake("alice", b"correct horse battery staple"));
    }

    #[test]
    fn wrong_password_fails() {
        let username = "bob";
        let reg = register(username, b"hunter2");

        let (login, a_pub) = ClientLogin::start(username, b"wrong-password");
        let server = ReferenceServer::initiate(username, &reg.salt, &reg.verifier, &a_pub);
        let session = login
            .finish(&reg.salt, &server.server_public_bytes())
            .expect("client finish");

        // Server rejects the mismatched proof.
        assert!(server.verify(session.proof()).is_none());
    }

    #[test]
    fn rejects_zero_server_public() {
        let (login, _a) = ClientLogin::start("alice", b"pw");
        let n = get_n();
        // B = N ≡ 0 (mod N)
        let err = login.finish(&[0u8; 16], &n.to_bytes_be()).unwrap_err();
        assert_eq!(err, SrpError::InvalidServerPublic);
    }

    #[test]
    fn session_keys_match_between_client_and_server() {
        let username = "carol";
        let password = b"s3cr3t";
        let reg = register(username, password);
        let (login, a_pub) = ClientLogin::start(username, password);
        let server = ReferenceServer::initiate(username, &reg.salt, &reg.verifier, &a_pub);
        let session = login
            .finish(&reg.salt, &server.server_public_bytes())
            .expect("client finish");
        let m2 = server.verify(session.proof()).expect("server accepts");
        assert!(session.verify_server_proof(&m2));
    }

    #[test]
    fn debug_redacts_secrets() {
        let reg = register("dave", b"pw");
        assert!(format!("{reg:?}").contains("[REDACTED]"));

        let (login, _a) = ClientLogin::start("dave", b"pw");
        assert!(format!("{login:?}").contains("[REDACTED]"));
    }

    #[test]
    fn distinct_salts_produce_distinct_verifiers() {
        let r1 = register("u", b"pw");
        let r2 = register("u", b"pw");
        // Salts are random, so verifiers should differ with overwhelming probability.
        assert_ne!(r1.salt, r2.salt);
        assert_ne!(r1.verifier, r2.verifier);
    }

    // ── Two-secret key derivation (2SKD, ADR-008) ───────────────────────────

    use crate::crypto::{Argon2Params, SecretKey, derive_auth_key, derive_master_key};
    use uuid::Uuid;

    fn auth_key(password: &[u8]) -> AuthKey {
        let mk = derive_master_key(password, b"argon-salt-16byte", &Argon2Params::test()).unwrap();
        derive_auth_key(&mk).unwrap()
    }

    /// A full 2SKD registration + login handshake against the server's verifier
    /// logic, using a fixed salt so registration and login derive the same `x`.
    fn run_2skd_handshake(
        username: &str,
        account_id: Uuid,
        mk_auth: &AuthKey,
        secret_key: &SecretKey,
    ) -> bool {
        let salt = vec![9u8; SALT_LEN];
        let reg = register_2skd_with_salt(&account_id, mk_auth, secret_key, salt);

        let (mut login, a_pub) =
            ClientLogin::start_2skd(username, mk_auth.clone(), secret_key.clone());
        login.set_account_id(account_id);
        let server = ReferenceServer::initiate(username, &reg.salt, &reg.verifier, &a_pub);

        let session = login
            .finish(&reg.salt, &server.server_public_bytes())
            .expect("client finish");

        let Some(m2) = server.verify(session.proof()) else {
            return false;
        };
        session.verify_server_proof(&m2)
    }

    #[test]
    fn two_secret_handshake_succeeds() {
        let account_id = Uuid::from_bytes([3u8; 16]);
        let mk_auth = auth_key(b"correct horse battery staple");
        let secret_key = SecretKey::generate(account_id);
        assert!(run_2skd_handshake(
            "alice",
            account_id,
            &mk_auth,
            &secret_key
        ));
    }

    #[test]
    fn two_secret_verifier_is_deterministic() {
        let account_id = Uuid::from_bytes([4u8; 16]);
        let mk_auth = auth_key(b"password");
        let secret_key = SecretKey::generate(account_id);
        let salt = vec![1u8; SALT_LEN];
        let v1 = register_2skd_with_salt(&account_id, &mk_auth, &secret_key, salt.clone());
        let v2 = register_2skd_with_salt(&account_id, &mk_auth, &secret_key, salt);
        assert_eq!(v1.verifier, v2.verifier);
    }

    #[test]
    fn wrong_secret_key_produces_different_verifier_and_fails_login() {
        let account_id = Uuid::from_bytes([5u8; 16]);
        let mk_auth = auth_key(b"password");
        let salt = vec![7u8; SALT_LEN];

        let registered = SecretKey::generate(account_id);
        let attacker = SecretKey::generate(account_id); // correct password, wrong Secret Key

        let reg = register_2skd_with_salt(&account_id, &mk_auth, &registered, salt.clone());
        let wrong = register_2skd_with_salt(&account_id, &mk_auth, &attacker, salt);
        assert_ne!(
            reg.verifier, wrong.verifier,
            "a different Secret Key must yield a different verifier"
        );

        // Login with the wrong Secret Key (but correct password) is rejected.
        let (mut login, a_pub) = ClientLogin::start_2skd("eve", mk_auth.clone(), attacker.clone());
        login.set_account_id(account_id);
        let server = ReferenceServer::initiate("eve", &reg.salt, &reg.verifier, &a_pub);
        let session = login
            .finish(&reg.salt, &server.server_public_bytes())
            .expect("client finish");
        assert!(server.verify(session.proof()).is_none());
    }

    #[test]
    fn wrong_password_with_correct_secret_key_fails_login() {
        let account_id = Uuid::from_bytes([6u8; 16]);
        let secret_key = SecretKey::generate(account_id);
        let salt = vec![8u8; SALT_LEN];

        let reg =
            register_2skd_with_salt(&account_id, &auth_key(b"right-password"), &secret_key, salt);

        let (mut login, a_pub) =
            ClientLogin::start_2skd("frank", auth_key(b"wrong-password"), secret_key.clone());
        login.set_account_id(account_id);
        let server = ReferenceServer::initiate("frank", &reg.salt, &reg.verifier, &a_pub);
        let session = login
            .finish(&reg.salt, &server.server_public_bytes())
            .expect("client finish");
        assert!(server.verify(session.proof()).is_none());
    }

    #[test]
    fn two_secret_verifier_differs_from_legacy() {
        // The 2SKD verifier must not coincide with the legacy password-only one.
        let account_id = Uuid::from_bytes([2u8; 16]);
        let mk_auth = auth_key(b"password");
        let secret_key = SecretKey::generate(account_id);
        let salt = vec![5u8; SALT_LEN];

        let two = register_2skd_with_salt(&account_id, &mk_auth, &secret_key, salt.clone());
        let legacy = register_with_salt("grace", b"password", salt);
        assert_ne!(two.verifier, legacy.verifier);
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// For any username/password, a registration followed by a login
        /// handshake against the server's verifier logic must succeed and the
        /// server proof must verify.
        #[test]
        fn prop_handshake_succeeds(
            username in "[a-zA-Z0-9_]{1,32}",
            password in proptest::collection::vec(any::<u8>(), 1..48),
        ) {
            prop_assert!(run_handshake(&username, &password));
        }

        /// A login with a different password than registration must be rejected
        /// by the server.
        #[test]
        fn prop_wrong_password_rejected(
            username in "[a-zA-Z0-9_]{1,32}",
            password in proptest::collection::vec(any::<u8>(), 1..48),
            other in proptest::collection::vec(any::<u8>(), 1..48),
        ) {
            prop_assume!(password != other);
            let reg = register(&username, &password);
            let (login, a_pub) = ClientLogin::start(&username, &other);
            let server = ReferenceServer::initiate(&username, &reg.salt, &reg.verifier, &a_pub);
            let session = login
                .finish(&reg.salt, &server.server_public_bytes())
                .expect("client finish");
            prop_assert!(server.verify(session.proof()).is_none());
        }

        /// For any password/account/Secret Key, a 2SKD registration followed by
        /// a 2SKD login handshake must complete against the server's verifier
        /// logic and the server proof must verify.
        #[test]
        fn prop_2skd_handshake_succeeds(
            username in "[a-zA-Z0-9_]{1,32}",
            password in proptest::collection::vec(any::<u8>(), 1..48),
            account_bytes in proptest::array::uniform16(any::<u8>()),
        ) {
            let account_id = Uuid::from_bytes(account_bytes);
            let mk_auth = auth_key(&password);
            let secret_key = SecretKey::generate(account_id);
            prop_assert!(run_2skd_handshake(&username, account_id, &mk_auth, &secret_key));
        }

        /// A 2SKD login with the correct password but a different Secret Key is
        /// always rejected by the server.
        #[test]
        fn prop_2skd_wrong_secret_key_rejected(
            username in "[a-zA-Z0-9_]{1,32}",
            password in proptest::collection::vec(any::<u8>(), 1..48),
            account_bytes in proptest::array::uniform16(any::<u8>()),
        ) {
            let account_id = Uuid::from_bytes(account_bytes);
            let mk_auth = auth_key(&password);
            let registered = SecretKey::generate(account_id);
            let attacker = SecretKey::generate(account_id);
            let salt = vec![3u8; SALT_LEN];

            let reg = register_2skd_with_salt(&account_id, &mk_auth, &registered, salt);
            let (mut login, a_pub) =
                ClientLogin::start_2skd(&username, mk_auth.clone(), attacker);
            login.set_account_id(account_id);
            let server = ReferenceServer::initiate(&username, &reg.salt, &reg.verifier, &a_pub);
            let session = login
                .finish(&reg.salt, &server.server_public_bytes())
                .expect("client finish");
            prop_assert!(server.verify(session.proof()).is_none());
        }
    }
}
