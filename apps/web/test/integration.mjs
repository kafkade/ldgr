/**
 * Integration test: sql.js (SQLite in WASM) with the ldgr schema.
 *
 * Proves that:
 * 1. sql.js initializes successfully in Node.js
 * 2. The ldgr schema (accounts, transactions, postings) can be created
 * 3. Data can be inserted and queried
 * 4. The database can be exported/imported (simulating IndexedDB persistence)
 *
 * This test runs without ldgr-wasm — it validates the storage layer only.
 * Full integration with ldgr-wasm crypto requires a browser environment
 * or wasm-pack test.
 */

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import initSqlJs from "sql.js";

// Subset of the ldgr schema (migration v1 + v2) for web use
const SCHEMA_SQL = `
  CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    type TEXT NOT NULL CHECK(type IN ('asset','liability','income','expense','equity')),
    commodity TEXT,
    parent_id TEXT,
    note TEXT,
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
  );

  CREATE TABLE transactions (
    id TEXT PRIMARY KEY,
    date TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'unmarked',
    code TEXT,
    description TEXT NOT NULL,
    comment TEXT,
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
  );

  CREATE TABLE postings (
    id TEXT PRIMARY KEY,
    transaction_id TEXT NOT NULL REFERENCES transactions(id),
    account_id TEXT NOT NULL REFERENCES accounts(id),
    amount_quantity TEXT,
    amount_commodity TEXT,
    posting_order INTEGER NOT NULL,
    created_at TEXT NOT NULL
  );

  CREATE TABLE sync_events (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    payload BLOB NOT NULL,
    lamport_clock INTEGER NOT NULL,
    version INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0
  );

  CREATE TABLE sync_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
  );
`;

describe("sql.js with ldgr schema", () => {
  let SQL;

  test("initializes sql.js", async () => {
    SQL = await initSqlJs();
    assert.ok(SQL, "sql.js should initialize");
  });

  test("creates ldgr schema", async () => {
    const db = new SQL.Database();
    db.run(SCHEMA_SQL);

    // Verify tables exist
    const tables = db
      .exec("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
      .flatMap((r) => r.values.map((v) => v[0]));

    assert.ok(tables.includes("accounts"));
    assert.ok(tables.includes("transactions"));
    assert.ok(tables.includes("postings"));
    assert.ok(tables.includes("sync_events"));
    assert.ok(tables.includes("sync_state"));

    db.close();
  });

  test("inserts and queries accounts", async () => {
    const db = new SQL.Database();
    db.run(SCHEMA_SQL);

    const now = new Date().toISOString();
    db.run(
      `INSERT INTO accounts (id, name, type, commodity, created_at, modified_at)
       VALUES ('a1', 'Assets:Checking', 'asset', 'USD', ?, ?)`,
      [now, now],
    );
    db.run(
      `INSERT INTO accounts (id, name, type, commodity, created_at, modified_at)
       VALUES ('a2', 'Expenses:Food', 'expense', 'USD', ?, ?)`,
      [now, now],
    );

    const result = db.exec(
      "SELECT id, name, type FROM accounts WHERE deleted = 0 ORDER BY name",
    );
    assert.equal(result[0].values.length, 2);
    assert.equal(result[0].values[0][1], "Assets:Checking");
    assert.equal(result[0].values[1][1], "Expenses:Food");

    db.close();
  });

  test("inserts transactions with postings", async () => {
    const db = new SQL.Database();
    db.run(SCHEMA_SQL);

    const now = new Date().toISOString();
    db.run(
      `INSERT INTO accounts (id, name, type, commodity, created_at, modified_at)
       VALUES ('a1', 'Assets:Checking', 'asset', 'USD', ?, ?)`,
      [now, now],
    );
    db.run(
      `INSERT INTO accounts (id, name, type, commodity, created_at, modified_at)
       VALUES ('a2', 'Expenses:Food', 'expense', 'USD', ?, ?)`,
      [now, now],
    );

    db.run(
      `INSERT INTO transactions (id, date, status, description, created_at, modified_at)
       VALUES ('t1', '2024-01-15', 'cleared', 'Grocery store', ?, ?)`,
      [now, now],
    );
    db.run(
      `INSERT INTO postings (id, transaction_id, account_id, amount_quantity, amount_commodity, posting_order, created_at)
       VALUES ('p1', 't1', 'a1', '-50.00', 'USD', 0, ?)`,
      [now],
    );
    db.run(
      `INSERT INTO postings (id, transaction_id, account_id, amount_quantity, amount_commodity, posting_order, created_at)
       VALUES ('p2', 't1', 'a2', '50.00', 'USD', 1, ?)`,
      [now],
    );

    // Verify transaction with postings
    const txns = db.exec(
      "SELECT t.id, t.description, COUNT(p.id) as posting_count FROM transactions t JOIN postings p ON p.transaction_id = t.id GROUP BY t.id",
    );
    assert.equal(txns[0].values[0][1], "Grocery store");
    assert.equal(txns[0].values[0][2], 2);

    db.close();
  });

  test("exports and imports database (persistence simulation)", async () => {
    const db = new SQL.Database();
    db.run(SCHEMA_SQL);

    const now = new Date().toISOString();
    db.run(
      `INSERT INTO accounts (id, name, type, commodity, created_at, modified_at)
       VALUES ('a1', 'Assets:Cash', 'asset', 'USD', ?, ?)`,
      [now, now],
    );

    // Export to binary (simulates saving to IndexedDB)
    const exported = db.export();
    assert.ok(exported instanceof Uint8Array);
    assert.ok(exported.length > 0);
    db.close();

    // Import from binary (simulates loading from IndexedDB)
    const db2 = new SQL.Database(exported);
    const result = db2.exec("SELECT name FROM accounts");
    assert.equal(result[0].values[0][0], "Assets:Cash");
    db2.close();
  });

  test("sync tables work correctly", async () => {
    const db = new SQL.Database();
    db.run(SCHEMA_SQL);

    db.run(
      `INSERT INTO sync_state (key, value) VALUES ('device_id', 'web-browser-001')`,
    );
    db.run(
      `INSERT INTO sync_state (key, value) VALUES ('lamport_clock', '0')`,
    );

    const deviceId = db.exec(
      "SELECT value FROM sync_state WHERE key = 'device_id'",
    );
    assert.equal(deviceId[0].values[0][0], "web-browser-001");

    // Simulate recording a sync event
    db.run(
      `INSERT INTO sync_events (id, device_id, entity_type, entity_id, operation, payload, lamport_clock, version, created_at)
       VALUES ('e1', 'web-browser-001', 'account', 'a1', 'create', X'7B7D', 1, 1, ?)`,
      [new Date().toISOString()],
    );

    const pending = db.exec(
      "SELECT COUNT(*) FROM sync_events WHERE synced = 0",
    );
    assert.equal(pending[0].values[0][0], 1);

    db.close();
  });
});
