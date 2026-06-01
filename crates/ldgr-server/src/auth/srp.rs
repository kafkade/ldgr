//! SRP-6a server-side implementation (RFC 5054).
//!
//! Uses SHA-256 as the hash function and the 2048-bit group from RFC 5054.
//! The server stores (salt, verifier) per user and never sees the password.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use num_bigint::BigUint;
use rand::RngCore;
use sha2::{Digest, Sha256};

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

fn get_n() -> BigUint {
    BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("hardcoded N is valid hex")
}

fn get_g() -> BigUint {
    BigUint::from(G_VALUE)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Left-pad a `BigUint` to `N_LEN` bytes.
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

/// Compute k = H(N || pad(g)) — SRP multiplier.
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

/// Compute server-side session secret: S = (A · v^u)^b mod N.
#[allow(clippy::many_single_char_names)] // SRP variable names match RFC 5054 spec
fn compute_server_s(a: &BigUint, v: &BigUint, u: &BigUint, b: &BigUint) -> BigUint {
    let n = get_n();
    let v_u = v.modpow(u, &n);
    let a_vu = (a * &v_u) % &n;
    a_vu.modpow(b, &n)
}

/// Compute session key K = H(S).
fn compute_session_key(s: &BigUint) -> Vec<u8> {
    hash(&[&pad(s)])
}

/// Compute M1 = H(H(N) XOR H(g) || H(username) || salt || A || B || K).
fn compute_m1(username: &str, salt: &[u8], a: &BigUint, b_pub: &BigUint, key: &[u8]) -> Vec<u8> {
    let n = get_n();
    let g = get_g();

    let h_n = hash(&[&pad(&n)]);
    let h_g = hash(&[&pad(&g)]);

    let xor: Vec<u8> = h_n.iter().zip(h_g.iter()).map(|(x, y)| x ^ y).collect();
    let h_user = hash(&[username.as_bytes()]);

    hash(&[&xor, &h_user, salt, &pad(a), &pad(b_pub), key])
}

/// Compute M2 = H(A || M1 || K).
fn compute_m2(a: &BigUint, m1: &[u8], key: &[u8]) -> Vec<u8> {
    hash(&[&pad(a), m1, key])
}

/// Generate B = (k·v + g^b) mod N.
#[allow(clippy::many_single_char_names)] // SRP variable names match RFC 5054 spec
fn generate_server_public(v: &BigUint, b: &BigUint) -> BigUint {
    let n = get_n();
    let g = get_g();
    let k = compute_k();

    let gb = g.modpow(b, &n);
    let kv = (&k * v) % &n;
    (&kv + &gb) % &n
}

/// Constant-time comparison for proof verification.
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

// ── Handshake Store ───────────────────────────────────────────────────────────

struct SrpHandshakeState {
    username: String,
    a: BigUint,
    b_pub: BigUint,
    b_secret: BigUint,
    verifier: BigUint,
    salt: Vec<u8>,
    created_at: Instant,
}

/// In-memory store for pending SRP-6a handshakes.
pub struct SrpHandshakeStore {
    handshakes: Mutex<HashMap<String, SrpHandshakeState>>,
    ttl: Duration,
    max_pending: usize,
}

impl SrpHandshakeStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            handshakes: Mutex::new(HashMap::new()),
            ttl,
            max_pending: 100,
        }
    }

    /// Start a handshake. Returns server public value B.
    ///
    /// # Errors
    ///
    /// Returns an error if A is invalid (A % N == 0), if too many handshakes
    /// are pending, or if the lock is poisoned.
    pub fn initiate(
        &self,
        handshake_id: String,
        username: String,
        client_public: BigUint,
        salt: Vec<u8>,
        verifier: BigUint,
    ) -> Result<BigUint, String> {
        let n = get_n();

        // SRP-6a safety: reject A ≡ 0 (mod N)
        if &client_public % &n == BigUint::from(0u32) {
            return Err("invalid client public value".into());
        }

        // Generate server ephemeral
        let mut b_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut b_bytes);
        let b = BigUint::from_bytes_be(&b_bytes);

        let b_pub = generate_server_public(&verifier, &b);

        let state = SrpHandshakeState {
            username,
            a: client_public,
            b_pub: b_pub.clone(),
            b_secret: b,
            verifier,
            salt,
            created_at: Instant::now(),
        };

        let mut hs = self.handshakes.lock().map_err(|e| e.to_string())?;

        // Clean expired entries
        let ttl = self.ttl;
        hs.retain(|_, v| v.created_at.elapsed() < ttl);

        if hs.len() >= self.max_pending {
            return Err("too many pending handshakes".into());
        }

        hs.insert(handshake_id, state);
        Ok(b_pub)
    }

    /// Verify client proof M1 and complete the handshake.
    ///
    /// Returns `(server_proof_M2, user_id)` on success. The handshake is
    /// consumed (removed from the store) regardless of outcome.
    pub fn verify(
        &self,
        handshake_id: &str,
        client_proof: &[u8],
    ) -> Result<(Vec<u8>, String), String> {
        let state = {
            let mut hs = self.handshakes.lock().map_err(|e| e.to_string())?;
            hs.remove(handshake_id)
                .ok_or("handshake not found or expired")?
        };

        if state.created_at.elapsed() >= self.ttl {
            return Err("handshake expired".into());
        }

        let u = compute_u(&state.a, &state.b_pub);
        if u == BigUint::from(0u32) {
            return Err("invalid u value".into());
        }

        let s = compute_server_s(&state.a, &state.verifier, &u, &state.b_secret);
        let key = compute_session_key(&s);

        let expected_m1 = compute_m1(&state.username, &state.salt, &state.a, &state.b_pub, &key);

        if !constant_time_eq(client_proof, &expected_m1) {
            return Err("invalid client proof".into());
        }

        let m2 = compute_m2(&state.a, client_proof, &key);

        Ok((m2, state.username))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n_parses_correctly() {
        let n = get_n();
        // 2048 bits = 256 bytes; the big-endian encoding should be exactly 256 bytes
        assert_eq!(n.to_bytes_be().len(), N_LEN);
    }

    #[test]
    fn k_is_nonzero() {
        let k = compute_k();
        assert!(k > BigUint::from(0u32));
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
    }

    #[test]
    fn server_public_is_nonzero() {
        let v = BigUint::from(42u32);
        let b = BigUint::from(7u32);
        let pub_b = generate_server_public(&v, &b);
        assert!(pub_b > BigUint::from(0u32));
    }

    #[test]
    fn reject_zero_client_public() {
        let store = SrpHandshakeStore::new(Duration::from_secs(60));
        let n = get_n();
        // A = N, which is ≡ 0 mod N
        let result = store.initiate(
            "hs1".into(),
            "alice".into(),
            n,
            vec![0u8; 16],
            BigUint::from(42u32),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid client public value"));
    }

    #[test]
    fn handshake_store_caps_pending() {
        let store = SrpHandshakeStore {
            handshakes: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(60),
            max_pending: 2,
        };
        let v = BigUint::from(42u32);
        let a = BigUint::from(7u32);
        let salt = vec![0u8; 16];

        assert!(
            store
                .initiate("h1".into(), "u1".into(), a.clone(), salt.clone(), v.clone())
                .is_ok()
        );
        assert!(
            store
                .initiate("h2".into(), "u2".into(), a.clone(), salt.clone(), v.clone())
                .is_ok()
        );
        assert!(
            store
                .initiate("h3".into(), "u3".into(), a, salt, v)
                .is_err()
        );
    }
}
