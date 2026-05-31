/**
 * Storage layer: sql.js (in-memory SQLite) + IndexedDB vault persistence.
 *
 * Data flow:
 * 1. Vault blob stored in IndexedDB (encrypted)
 * 2. On unlock, sql.js database initialized from decrypted vault item
 * 3. On save, sql.js exported → encrypted as vault item → vault re-serialized → IndexedDB
 */

import type { Database } from 'sql.js';

// ── IndexedDB Helpers ──────────────────────────────────────────────────────────

const IDB_NAME = 'ldgr-vault';
const IDB_STORE = 'vaults';
const IDB_VERSION = 1;

function openIdb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(IDB_NAME, IDB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(IDB_STORE)) {
        db.createObjectStore(IDB_STORE);
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function saveVaultBlob(
  name: string,
  data: Uint8Array,
): Promise<void> {
  const db = await openIdb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(IDB_STORE, 'readwrite');
    tx.objectStore(IDB_STORE).put(data, name);
    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onerror = () => {
      db.close();
      reject(tx.error);
    };
  });
}

export async function loadVaultBlob(
  name: string,
): Promise<Uint8Array | null> {
  const db = await openIdb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(IDB_STORE, 'readonly');
    const req = tx.objectStore(IDB_STORE).get(name);
    req.onsuccess = () => {
      db.close();
      const result = req.result;
      resolve(result instanceof Uint8Array ? result : null);
    };
    req.onerror = () => {
      db.close();
      reject(req.error);
    };
  });
}

export async function listVaultNames(): Promise<string[]> {
  const db = await openIdb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(IDB_STORE, 'readonly');
    const req = tx.objectStore(IDB_STORE).getAllKeys();
    req.onsuccess = () => {
      db.close();
      resolve((req.result as string[]).filter((k) => typeof k === 'string'));
    };
    req.onerror = () => {
      db.close();
      reject(req.error);
    };
  });
}

export async function deleteVaultBlob(name: string): Promise<void> {
  const db = await openIdb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(IDB_STORE, 'readwrite');
    tx.objectStore(IDB_STORE).delete(name);
    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onerror = () => {
      db.close();
      reject(tx.error);
    };
  });
}

// ── sql.js Initialization ──────────────────────────────────────────────────────

const SCHEMA_SQL = `
  CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    type TEXT NOT NULL CHECK(type IN ('asset','liability','income','expense','equity')),
    commodity TEXT NOT NULL DEFAULT 'USD',
    parent_id TEXT,
    note TEXT,
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
  );

  CREATE TABLE IF NOT EXISTS transactions (
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

  CREATE TABLE IF NOT EXISTS postings (
    id TEXT PRIMARY KEY,
    transaction_id TEXT NOT NULL REFERENCES transactions(id),
    account_id TEXT NOT NULL REFERENCES accounts(id),
    amount_quantity TEXT,
    amount_commodity TEXT,
    posting_order INTEGER NOT NULL,
    created_at TEXT NOT NULL
  );

  CREATE TABLE IF NOT EXISTS sync_events (
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

  CREATE TABLE IF NOT EXISTS sync_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
  );
`;

let sqlPromise: Promise<typeof import('sql.js')> | null = null;

async function getSqlJs() {
  if (!sqlPromise) {
    sqlPromise = import('sql.js').then((m) => m.default);
  }
  const initSqlJs = await sqlPromise;
  return initSqlJs({
    locateFile: (file: string) => `/sql.js/${file}`,
  });
}

export async function createDatabase(data?: Uint8Array): Promise<Database> {
  const SQL = await getSqlJs();
  const db = data ? new SQL.Database(data) : new SQL.Database();
  if (!data) {
    db.run(SCHEMA_SQL);
  }
  return db;
}

export function exportDatabase(db: Database): Uint8Array {
  return db.export();
}
