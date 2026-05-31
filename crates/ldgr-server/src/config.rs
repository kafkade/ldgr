use std::net::SocketAddr;

/// Server configuration, read from environment variables.
pub struct Config {
    pub bind_addr: SocketAddr,
    pub db_path: String,
    pub session_ttl_hours: u64,
    pub relay_ttl_minutes: u64,
    pub max_blob_bytes: usize,
    pub srp_handshake_ttl_secs: u64,
}

impl Config {
    /// Load configuration from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        Self {
            bind_addr: env_or("LDGR_BIND_ADDR", "127.0.0.1:8080")
                .parse()
                .expect("LDGR_BIND_ADDR must be a valid socket address"),
            db_path: env_or("LDGR_DB_PATH", "ldgr-server.db"),
            session_ttl_hours: env_or("LDGR_SESSION_TTL_HOURS", "720")
                .parse()
                .expect("LDGR_SESSION_TTL_HOURS must be a valid number"),
            relay_ttl_minutes: env_or("LDGR_RELAY_TTL_MINUTES", "10")
                .parse()
                .expect("LDGR_RELAY_TTL_MINUTES must be a valid number"),
            max_blob_bytes: env_or("LDGR_MAX_BLOB_BYTES", "52428800") // 50 MB
                .parse()
                .expect("LDGR_MAX_BLOB_BYTES must be a valid number"),
            srp_handshake_ttl_secs: env_or("LDGR_SRP_HANDSHAKE_TTL_SECS", "120")
                .parse()
                .expect("LDGR_SRP_HANDSHAKE_TTL_SECS must be a valid number"),
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
