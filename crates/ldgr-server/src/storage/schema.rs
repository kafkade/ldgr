/// SQL statements to initialize the server database.
///
/// Fresh databases get the full multi-user `users` schema (ADR-008). Existing
/// databases created before ADR-008 are upgraded in place by [`migrate`] (see
/// `storage/mod.rs`), which adds the new columns with the same defaults used
/// here so the two paths converge.
///
/// [`migrate`]: super::ServerDb
pub const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS users (
    id                  TEXT PRIMARY KEY,
    username            TEXT UNIQUE NOT NULL,
    salt                BLOB NOT NULL,
    verifier            BLOB NOT NULL,
    created_at          TEXT NOT NULL,
    email               TEXT,
    role                TEXT NOT NULL DEFAULT 'user',
    status              TEXT NOT NULL DEFAULT 'active',
    storage_quota_bytes INTEGER,
    invited_by          TEXT,
    updated_at          TEXT,
    auth_scheme         TEXT NOT NULL DEFAULT 'srp-1secret',
    secret_key_version  INTEGER,
    account_id          TEXT
);

-- Minimal invite mechanism for the `invite-only` registration policy. Issuing
-- invites (the admin API) lands in #176; this is the redemption-side seam.
CREATE TABLE IF NOT EXISTS invites (
    token_hash  TEXT PRIMARY KEY,
    email       TEXT,
    role        TEXT NOT NULL DEFAULT 'user',
    created_by  TEXT,
    created_at  TEXT NOT NULL,
    expires_at  TEXT,
    redeemed_at TEXT,
    redeemed_by TEXT
);

-- Runtime-updatable server settings (ADR-008). Env vars provide the bootstrap
-- defaults; the admin API (#176) persists overrides here, which then win over
-- env on subsequent reads. Absent keys fall back to the env/config default.
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token_hash  TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user    ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);

CREATE TABLE IF NOT EXISTS vaults (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vaults_user ON vaults(user_id);

CREATE TABLE IF NOT EXISTS blobs (
    path          TEXT PRIMARY KEY,
    vault_id      TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    data          BLOB NOT NULL,
    size          INTEGER NOT NULL,
    content_hash  TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_blobs_vault   ON blobs(vault_id);
CREATE INDEX IF NOT EXISTS idx_blobs_created ON blobs(vault_id, created_at);

CREATE TABLE IF NOT EXISTS devices (
    id              TEXT NOT NULL,
    vault_id        TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    encrypted_info  BLOB NOT NULL,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (vault_id, id)
);

CREATE TABLE IF NOT EXISTS relay_offers (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    offer_data      BLOB NOT NULL,
    response_data   BLOB,
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_relay_user    ON relay_offers(user_id);
CREATE INDEX IF NOT EXISTS idx_relay_expires ON relay_offers(expires_at);
";
