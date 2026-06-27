use std::net::SocketAddr;

/// Account registration policy (ADR-008 Decision 5). Controls who may create an
/// account via the public `register` endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationPolicy {
    /// Anyone can self-register.
    Open,
    /// Registration requires a valid admin-issued invite token (default).
    InviteOnly,
    /// Only an admin may create accounts; public self-registration is refused.
    AdminOnly,
}

impl RegistrationPolicy {
    /// Parse from a config/setting string. Unknown values fall back to the
    /// secure default (`invite-only`).
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "open" => Self::Open,
            "admin-only" | "admin_only" | "adminonly" => Self::AdminOnly,
            // Default + explicit "invite-only" spellings.
            _ => Self::InviteOnly,
        }
    }

    /// Canonical wire/storage spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InviteOnly => "invite-only",
            Self::AdminOnly => "admin-only",
        }
    }
}

/// Server configuration, read from environment variables.
pub struct Config {
    pub bind_addr: SocketAddr,
    pub db_path: String,
    pub session_ttl_hours: u64,
    pub relay_ttl_minutes: u64,
    pub max_blob_bytes: usize,
    pub srp_handshake_ttl_secs: u64,
    /// Who may register (ADR-008 Decision 5). Defaults to `invite-only`.
    pub registration_policy: RegistrationPolicy,
    /// Email seeded as the first admin on first boot (`LDGR_ADMIN_EMAIL`). When
    /// set, the account registering with this email becomes `admin` and bypasses
    /// the registration policy. Preferred for unattended docker-compose deploys.
    pub admin_email: Option<String>,
    /// Default per-user storage quota in bytes, applied when a user's
    /// `storage_quota_bytes` is unset.
    pub default_user_quota_bytes: i64,
    /// Human-readable server name advertised by `GET /api/v1/server/info` and
    /// `/ping` (`LDGR_SERVER_NAME`). Lets operators label their instance; purely
    /// cosmetic and never used for auth. Defaults to `ldgr-server`.
    pub server_name: String,
}

impl Config {
    /// Load configuration from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        let admin_email = std::env::var("LDGR_ADMIN_EMAIL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

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
            registration_policy: RegistrationPolicy::parse(&env_or(
                "LDGR_REGISTRATION",
                "invite-only",
            )),
            admin_email,
            default_user_quota_bytes: env_or("LDGR_DEFAULT_QUOTA_BYTES", "1073741824") // 1 GiB
                .parse()
                .expect("LDGR_DEFAULT_QUOTA_BYTES must be a valid number"),
            server_name: {
                let name = env_or("LDGR_SERVER_NAME", "ldgr-server").trim().to_string();
                if name.is_empty() {
                    "ldgr-server".to_string()
                } else {
                    name
                }
            },
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
