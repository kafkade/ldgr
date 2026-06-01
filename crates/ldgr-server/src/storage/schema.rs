/// SQL statements to initialize the server database.
pub const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS users (
    id          TEXT PRIMARY KEY,
    username    TEXT UNIQUE NOT NULL,
    salt        BLOB NOT NULL,
    verifier    BLOB NOT NULL,
    created_at  TEXT NOT NULL
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
