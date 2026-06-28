/**
 * Web sync engine: outbox emitter + apply path + ldgr-server push/pull.
 *
 * The canonical compose/apply pipeline in ldgr-core is sqlite-gated (rusqlite)
 * and cannot run in the browser, so this module reimplements the DB-facing half
 * over sql.js while staying faithful to the Rust semantics:
 *
 * - Outbox: the three vault mutations also write a `sync_events` row carrying
 *   the canonical `sync::payload` JSON, a lamport clock, and an entity version.
 * - Framing + conflict policy stay in Rust: sealing/opening the encrypted blob
 *   (`vault.sealBatch`/`openBatch`) and the three-way merge (`wasm.mergeBatch`).
 * - Apply mirrors `apply_remote_account` / `apply_remote_transaction`:
 *   version-gated upsert by explicit id, wholesale posting replace, soft delete,
 *   idempotent.
 *
 * Entity/operation enums cross the wasm boundary using serde's variant names
 * (`"Account"`, `"Create"`, …); the `sync_events` columns store the lowercase
 * convention used by the Rust outbox.
 */

import type { Database, SqlValue } from 'sql.js';
import type { LdgrWasm, WasmModule, WasmSyncClient } from './wasm';

// ── sync_state keys (must match ldgr-core where shared) ──────────────────────────

const DEVICE_ID_KEY = 'sync:device_id';
const LAMPORT_KEY = 'lamport_clock'; // matches ldgr-core storage::sync
const VECTOR_CLOCK_KEY = 'sync:vector_clock'; // matches ldgr-core pipeline
const APPLIED_BATCHES_KEY = 'sync:applied_batches';

const CFG_SERVER_URL = 'sync:server_url';
const CFG_USERNAME = 'sync:username';
const CFG_VAULT_ID = 'sync:vault_id';
const CFG_TOKEN = 'sync:token';

// ── Enum wire mapping (serde variant names) ──────────────────────────────────────

const ENTITY_TO_WIRE: Record<string, string> = {
  account: 'Account',
  transaction: 'Transaction',
  price: 'Price',
  budget: 'Budget',
  goal: 'Goal',
};
const OP_TO_WIRE: Record<string, string> = {
  create: 'Create',
  update: 'Update',
  delete: 'Delete',
};

// ── Types ────────────────────────────────────────────────────────────────────────

export interface ServerConfig {
  serverUrl: string;
  username: string;
  vaultId: string;
}

export interface SyncOutcome {
  pushed: number;
  applied: number;
  conflicts: number;
  skipped: number;
}

export interface ConflictRow {
  id: string;
  entityType: string;
  entityId: string;
  detectedAt: string;
  localSummary: string;
  remoteSummary: string;
}

interface WireEvent {
  id: string;
  device_id: string;
  lamport_clock: number;
  entity_type: string;
  entity_id: string;
  operation: string;
  payload: number[];
  version: number;
  created_at: string;
}

interface MergeOutput {
  applied: WireEvent[];
  conflicts: WireConflict[];
  skipped: number;
}

interface WireConflict {
  id: string;
  entity_type: string;
  entity_id: string;
  local_event: WireEvent;
  remote_event: WireEvent;
  detected_at: string;
  resolved: boolean;
  resolution: string | null;
}

// ── Low-level helpers ────────────────────────────────────────────────────────────

const now = (): string => new Date().toISOString();
const uid = (): string => crypto.randomUUID();
const enc = new TextEncoder();
const dec = new TextDecoder();

function toBytes(obj: unknown): Uint8Array {
  return enc.encode(JSON.stringify(obj));
}

function bytesToObj<T>(bytes: Uint8Array | number[]): T {
  const arr = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
  return JSON.parse(dec.decode(arr)) as T;
}

function getState(db: Database, key: string): string | null {
  const res = db.exec('SELECT value FROM sync_state WHERE key = ?', [key]);
  if (res.length === 0 || res[0].values.length === 0) return null;
  return res[0].values[0][0] as string;
}

function setState(db: Database, key: string, value: string): void {
  db.run(
    `INSERT INTO sync_state (key, value) VALUES (?, ?)
     ON CONFLICT(key) DO UPDATE SET value = excluded.value`,
    [key, value],
  );
}

function scalar(
  db: Database,
  sql: string,
  params: SqlValue[],
): SqlValue | null {
  const res = db.exec(sql, params);
  if (res.length === 0 || res[0].values.length === 0) return null;
  return res[0].values[0][0];
}

// ── Device id + clocks ───────────────────────────────────────────────────────────

export function getOrCreateDeviceId(db: Database): string {
  let id = getState(db, DEVICE_ID_KEY);
  if (!id) {
    id = uid();
    setState(db, DEVICE_ID_KEY, id);
  }
  return id;
}

function lamport(db: Database): number {
  return Number(getState(db, LAMPORT_KEY) ?? '0');
}

function tickLamport(db: Database): number {
  const next = lamport(db) + 1;
  setState(db, LAMPORT_KEY, String(next));
  return next;
}

function observeLamport(db: Database, observed: number): void {
  if (observed > lamport(db)) setState(db, LAMPORT_KEY, String(observed));
}

interface VectorClock {
  clocks: Record<string, number>;
}

function loadLocalClock(db: Database, deviceId: string): VectorClock {
  const raw = getState(db, VECTOR_CLOCK_KEY);
  const clock: VectorClock = raw
    ? (JSON.parse(raw) as VectorClock)
    : { clocks: {} };
  if (!clock.clocks) clock.clocks = {};
  const own = Number(
    scalar(db, 'SELECT COUNT(*) FROM sync_events WHERE device_id = ?', [
      deviceId,
    ]) ?? 0,
  );
  clock.clocks[deviceId] = Math.max(clock.clocks[deviceId] ?? 0, own);
  return clock;
}

function persistLocalClock(db: Database, clock: VectorClock): void {
  setState(db, VECTOR_CLOCK_KEY, JSON.stringify(clock));
}

function mergeClock(into: VectorClock, other: VectorClock): void {
  for (const [device, value] of Object.entries(other.clocks ?? {})) {
    into.clocks[device] = Math.max(into.clocks[device] ?? 0, value);
  }
}

// ── Outbox (record one event) ────────────────────────────────────────────────────

function recordEvent(
  db: Database,
  deviceId: string,
  entityType: 'account' | 'transaction',
  entityId: string,
  operation: 'create' | 'update' | 'delete',
  payload: Uint8Array,
  version: number,
): void {
  db.run(
    `INSERT INTO sync_events
       (id, device_id, entity_type, entity_id, operation, payload, lamport_clock, version, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    [
      uid(),
      deviceId,
      entityType,
      entityId,
      operation,
      payload,
      tickLamport(db),
      version,
      now(),
    ],
  );
}

// ── Mutations + outbox (called by VaultContext) ──────────────────────────────────

export function createAccount(
  db: Database,
  fields: { name: string; type: string; commodity: string },
): void {
  const deviceId = getOrCreateDeviceId(db);
  const id = uid();
  const ts = now();
  db.run(
    `INSERT INTO accounts (id, name, type, commodity, created_at, modified_at, version)
     VALUES (?, ?, ?, ?, ?, ?, 1)`,
    [id, fields.name, fields.type, fields.commodity, ts, ts],
  );
  const payload = toBytes({
    id,
    name: fields.name,
    account_type: fields.type,
    commodity: fields.commodity,
    parent_id: null,
    note: null,
    created_at: ts,
    modified_at: ts,
  });
  recordEvent(db, deviceId, 'account', id, 'create', payload, 1);
}

export function createTransaction(
  db: Database,
  date: string,
  description: string,
  postings: Array<{ accountId: string; amount: string; commodity: string }>,
): void {
  const deviceId = getOrCreateDeviceId(db);
  const ts = now();
  const txId = uid();
  db.run(
    `INSERT INTO transactions (id, date, status, description, created_at, modified_at, version)
     VALUES (?, ?, 'unmarked', ?, ?, ?, 1)`,
    [txId, date, description, ts, ts],
  );
  const wirePostings = postings.map((p, i) => {
    const pid = uid();
    db.run(
      `INSERT INTO postings
         (id, transaction_id, account_id, amount_quantity, amount_commodity, posting_order, created_at, version)
       VALUES (?, ?, ?, ?, ?, ?, ?, 1)`,
      [pid, txId, p.accountId, p.amount, p.commodity, i, ts],
    );
    return {
      id: pid,
      account_id: p.accountId,
      amount_quantity: p.amount,
      amount_commodity: p.commodity,
      balance_assertion_quantity: null,
      balance_assertion_commodity: null,
      created_at: ts,
      version: 1,
    };
  });
  const payload = toBytes({
    id: txId,
    date,
    status: 'unmarked',
    code: null,
    description,
    comment: null,
    created_at: ts,
    modified_at: ts,
    postings: wirePostings,
  });
  recordEvent(db, deviceId, 'transaction', txId, 'create', payload, 1);
}

export function deleteTransaction(db: Database, id: string): void {
  const deviceId = getOrCreateDeviceId(db);
  const current = Number(
    scalar(db, 'SELECT version FROM transactions WHERE id = ?', [id]) ?? 0,
  );
  const version = current + 1;
  db.run(
    'UPDATE transactions SET deleted = 1, version = ?, modified_at = ? WHERE id = ?',
    [version, now(), id],
  );
  const payload = toBytes({ id });
  recordEvent(db, deviceId, 'transaction', id, 'delete', payload, version);
}

// ── Pending batch composition ────────────────────────────────────────────────────

function pendingWireEvents(db: Database): WireEvent[] {
  const res = db.exec(
    `SELECT id, device_id, lamport_clock, entity_type, entity_id, operation, payload, version, created_at
     FROM sync_events WHERE synced = 0 ORDER BY lamport_clock ASC`,
  );
  if (res.length === 0) return [];
  return res[0].values.map((row) => ({
    id: row[0] as string,
    device_id: row[1] as string,
    lamport_clock: row[2] as number,
    entity_type: ENTITY_TO_WIRE[row[3] as string] ?? (row[3] as string),
    entity_id: row[4] as string,
    operation: OP_TO_WIRE[row[5] as string] ?? (row[5] as string),
    payload: Array.from(row[6] as Uint8Array),
    version: row[7] as number,
    created_at: row[8] as string,
  }));
}

function pendingEventIds(db: Database): string[] {
  const res = db.exec('SELECT id FROM sync_events WHERE synced = 0');
  if (res.length === 0) return [];
  return res[0].values.map((row) => row[0] as string);
}

function markSynced(db: Database, ids: string[]): void {
  if (ids.length === 0) return;
  const placeholders = ids.map(() => '?').join(',');
  db.run(
    `UPDATE sync_events SET synced = 1 WHERE id IN (${placeholders})`,
    ids as SqlValue[],
  );
}

// ── Apply (faithful to ldgr-core apply_remote_*) ─────────────────────────────────

function currentVersion(
  db: Database,
  table: 'accounts' | 'transactions',
  id: string,
): number | null {
  const v = scalar(db, `SELECT version FROM ${table} WHERE id = ?`, [id]);
  return v === null ? null : Number(v);
}

interface AccountPayload {
  id: string;
  name: string;
  account_type: string;
  commodity: string | null;
  parent_id: string | null;
  note: string | null;
  created_at: string;
  modified_at: string;
}

interface PostingPayload {
  id: string;
  account_id: string;
  amount_quantity: string | null;
  amount_commodity: string | null;
  balance_assertion_quantity: string | null;
  balance_assertion_commodity: string | null;
  created_at: string;
  version: number;
}

interface TransactionPayload {
  id: string;
  date: string;
  status: string;
  code: string | null;
  description: string;
  comment: string | null;
  created_at: string;
  modified_at: string;
  postings: PostingPayload[];
}

function applyAccount(db: Database, p: AccountPayload, version: number): boolean {
  const local = currentVersion(db, 'accounts', p.id);
  if (local !== null && local >= version) return false;
  db.run(
    `INSERT INTO accounts (id, name, type, commodity, parent_id, note, created_at, modified_at, version, deleted)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0)
     ON CONFLICT(id) DO UPDATE SET
       name = excluded.name, type = excluded.type, commodity = excluded.commodity,
       parent_id = excluded.parent_id, note = excluded.note,
       created_at = excluded.created_at, modified_at = excluded.modified_at,
       version = excluded.version, deleted = 0`,
    [
      p.id,
      p.name,
      p.account_type,
      p.commodity ?? 'USD',
      p.parent_id,
      p.note,
      p.created_at,
      p.modified_at,
      version,
    ],
  );
  return true;
}

function applyTransaction(
  db: Database,
  p: TransactionPayload,
  version: number,
): boolean {
  const local = currentVersion(db, 'transactions', p.id);
  if (local !== null && local >= version) return false;
  db.run(
    `INSERT INTO transactions (id, date, status, code, description, comment, created_at, modified_at, version, deleted)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0)
     ON CONFLICT(id) DO UPDATE SET
       date = excluded.date, status = excluded.status, code = excluded.code,
       description = excluded.description, comment = excluded.comment,
       created_at = excluded.created_at, modified_at = excluded.modified_at,
       version = excluded.version, deleted = 0`,
    [
      p.id,
      p.date,
      p.status,
      p.code,
      p.description,
      p.comment,
      p.created_at,
      p.modified_at,
      version,
    ],
  );
  db.run('DELETE FROM postings WHERE transaction_id = ?', [p.id]);
  p.postings.forEach((pp, i) => {
    db.run(
      `INSERT INTO postings
         (id, transaction_id, account_id, amount_quantity, amount_commodity,
          balance_assertion_quantity, balance_assertion_commodity, posting_order, created_at, version)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        pp.id,
        p.id,
        pp.account_id,
        pp.amount_quantity,
        pp.amount_commodity,
        pp.balance_assertion_quantity,
        pp.balance_assertion_commodity,
        i,
        pp.created_at,
        pp.version,
      ],
    );
  });
  return true;
}

function applyDelete(
  db: Database,
  table: 'accounts' | 'transactions',
  id: string,
  version: number,
): boolean {
  const local = currentVersion(db, table, id);
  if (local !== null && local < version) {
    db.run(
      `UPDATE ${table} SET deleted = 1, version = ?, modified_at = ? WHERE id = ?`,
      [version, now(), id],
    );
    return true;
  }
  return false;
}

// Deterministic total order: lamport → id → device (matches events::total_order).
function totalOrder(a: WireEvent, b: WireEvent): number {
  if (a.lamport_clock !== b.lamport_clock)
    return a.lamport_clock - b.lamport_clock;
  if (a.id !== b.id) return a.id < b.id ? -1 : 1;
  if (a.device_id !== b.device_id) return a.device_id < b.device_id ? -1 : 1;
  return 0;
}

function applyEvent(db: Database, ev: WireEvent): boolean {
  const entity = ev.entity_type.toLowerCase();
  const op = ev.operation.toLowerCase();
  if (entity === 'account') {
    if (op === 'delete') {
      const p = bytesToObj<{ id: string }>(ev.payload);
      return applyDelete(db, 'accounts', p.id, ev.version);
    }
    return applyAccount(db, bytesToObj<AccountPayload>(ev.payload), ev.version);
  }
  if (entity === 'transaction') {
    if (op === 'delete') {
      const p = bytesToObj<{ id: string }>(ev.payload);
      return applyDelete(db, 'transactions', p.id, ev.version);
    }
    return applyTransaction(
      db,
      bytesToObj<TransactionPayload>(ev.payload),
      ev.version,
    );
  }
  // Price/Budget/Goal have no web storage — skip rather than corrupt.
  return false;
}

function persistConflicts(db: Database, conflicts: WireConflict[]): void {
  for (const c of conflicts) {
    db.run(
      `INSERT OR REPLACE INTO sync_conflicts
         (id, entity_type, entity_id, local_event_id, remote_event_id,
          local_payload, remote_payload, detected_at, resolved, resolution)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, NULL)`,
      [
        c.id,
        c.entity_type,
        c.entity_id,
        c.local_event.id,
        c.remote_event.id,
        Uint8Array.from(c.local_event.payload),
        Uint8Array.from(c.remote_event.payload),
        c.detected_at,
      ],
    );
  }
}

// ── Push / Pull orchestration ────────────────────────────────────────────────────

async function push(
  db: Database,
  vault: LdgrWasm,
  client: WasmSyncClient,
  deviceId: string,
  vaultId: string,
): Promise<number> {
  const events = pendingWireEvents(db);
  if (events.length === 0) return 0;

  const clock = loadLocalClock(db, deviceId);
  persistLocalClock(db, clock);

  const batch = { device_id: deviceId, events, vector_clock: clock };
  const ciphertext = vault.sealBatch(JSON.stringify(batch));
  const batchId = uid();

  await client.putBatch(vaultId, deviceId, batchId, ciphertext);

  markSynced(db, pendingEventIds(db));
  // Our own batch — record it as applied so we never reprocess it.
  addAppliedBatch(db, batchId);
  return events.length;
}

function getAppliedBatches(db: Database): Set<string> {
  const raw = getState(db, APPLIED_BATCHES_KEY);
  return new Set<string>(raw ? (JSON.parse(raw) as string[]) : []);
}

function addAppliedBatch(db: Database, batchId: string): void {
  const set = getAppliedBatches(db);
  set.add(batchId);
  setState(db, APPLIED_BATCHES_KEY, JSON.stringify([...set]));
}

interface RemoteBatchMeta {
  batch_id: string;
  device_id: string;
}

async function pull(
  db: Database,
  wasm: WasmModule,
  vault: LdgrWasm,
  client: WasmSyncClient,
  deviceId: string,
  vaultId: string,
): Promise<{ applied: number; conflicts: number; skipped: number }> {
  const metasJson = await client.listBatches(vaultId);
  const metas = JSON.parse(metasJson) as RemoteBatchMeta[];
  const seen = getAppliedBatches(db);

  let applied = 0;
  let conflicts = 0;
  let skipped = 0;

  for (const meta of metas) {
    if (meta.device_id === deviceId || seen.has(meta.batch_id)) continue;

    const ciphertext = await client.getBatch(
      vaultId,
      meta.device_id,
      meta.batch_id,
    );
    const batchJson = vault.openBatch(ciphertext);
    const remoteBatch = JSON.parse(batchJson) as {
      events: WireEvent[];
      vector_clock: VectorClock;
    };

    const localPending = JSON.stringify(pendingWireEvents(db));
    const localClock = loadLocalClock(db, deviceId);
    const mergeJson = wasm.mergeBatch(
      localPending,
      batchJson,
      JSON.stringify(localClock),
      now(),
    );
    const merge = JSON.parse(mergeJson) as MergeOutput;

    const sorted = [...merge.applied].sort(totalOrder);
    for (const ev of sorted) {
      if (applyEvent(db, ev)) applied += 1;
      else skipped += 1;
    }

    persistConflicts(db, merge.conflicts);
    conflicts += merge.conflicts.length;
    skipped += merge.skipped;

    mergeClock(localClock, remoteBatch.vector_clock);
    persistLocalClock(db, localClock);
    const maxRemote = remoteBatch.events.reduce(
      (m, e) => Math.max(m, e.lamport_clock),
      0,
    );
    observeLamport(db, maxRemote);

    addAppliedBatch(db, meta.batch_id);
  }

  return { applied, conflicts, skipped };
}

/**
 * Run a full sync cycle. Pull + apply remote batches first so that incoming
 * events are merged against the current local *pending* set (this is what makes
 * concurrent-edit conflict detection work); then push the still-pending local
 * events. The caller is responsible for persisting the vault afterwards.
 */
export async function runSync(
  db: Database,
  wasm: WasmModule,
  vault: LdgrWasm,
  client: WasmSyncClient,
  vaultId: string,
): Promise<SyncOutcome> {
  const deviceId = getOrCreateDeviceId(db);
  const { applied, conflicts, skipped } = await pull(
    db,
    wasm,
    vault,
    client,
    deviceId,
    vaultId,
  );
  const pushed = await push(db, vault, client, deviceId, vaultId);
  return { pushed, applied, conflicts, skipped };
}

// ── Server config persistence (non-secret) ───────────────────────────────────────

export function loadServerConfig(db: Database): ServerConfig | null {
  const serverUrl = getState(db, CFG_SERVER_URL);
  const vaultId = getState(db, CFG_VAULT_ID);
  if (!serverUrl || !vaultId) return null;
  return {
    serverUrl,
    username: getState(db, CFG_USERNAME) ?? '',
    vaultId,
  };
}

export function saveServerConfig(db: Database, cfg: ServerConfig): void {
  setState(db, CFG_SERVER_URL, cfg.serverUrl);
  setState(db, CFG_USERNAME, cfg.username);
  setState(db, CFG_VAULT_ID, cfg.vaultId);
}

/**
 * Session token persistence. The token lives inside the encrypted vault DB
 * (sync_state) so it survives reloads; the SRP password is never stored.
 */
export function loadToken(db: Database): string | null {
  return getState(db, CFG_TOKEN);
}

export function saveToken(db: Database, token: string): void {
  setState(db, CFG_TOKEN, token);
}

export function clearToken(db: Database): void {
  db.run('DELETE FROM sync_state WHERE key = ?', [CFG_TOKEN]);
}

// ── Conflicts ────────────────────────────────────────────────────────────────────

export function listOpenConflicts(db: Database): ConflictRow[] {
  const res = db.exec(
    `SELECT id, entity_type, entity_id, local_payload, remote_payload, detected_at
     FROM sync_conflicts WHERE resolved = 0 ORDER BY detected_at DESC`,
  );
  if (res.length === 0) return [];
  return res[0].values.map((row) => ({
    id: row[0] as string,
    entityType: row[1] as string,
    entityId: row[2] as string,
    localSummary: summarize(row[3] as Uint8Array),
    remoteSummary: summarize(row[4] as Uint8Array),
    detectedAt: row[5] as string,
  }));
}

function summarize(payload: Uint8Array): string {
  try {
    const obj = bytesToObj<Record<string, unknown>>(payload);
    if (typeof obj.description === 'string') return obj.description;
    if (typeof obj.name === 'string') return obj.name;
    return JSON.stringify(obj).slice(0, 80);
  } catch {
    return '(unreadable)';
  }
}

/**
 * Resolve a conflict. `keepRemote` applies the remote event's payload to the
 * canonical tables (bypassing the version gate) and drops the conflicting local
 * pending event so it won't be re-pushed. `keepLocal` leaves local state intact.
 */
export function resolveConflict(
  db: Database,
  conflictId: string,
  keepRemote: boolean,
): void {
  const res = db.exec(
    `SELECT entity_type, entity_id, local_event_id, remote_payload
     FROM sync_conflicts WHERE id = ?`,
    [conflictId],
  );
  if (res.length === 0 || res[0].values.length === 0) return;
  const row = res[0].values[0];
  const entityType = (row[0] as string).toLowerCase();
  const entityId = row[1] as string;
  const localEventId = row[2] as string;
  const remotePayload = row[3] as Uint8Array;

  if (keepRemote) {
    const localVersion =
      currentVersion(
        db,
        entityType === 'account' ? 'accounts' : 'transactions',
        entityId,
      ) ?? 0;
    const forcedVersion = localVersion + 1;
    if (entityType === 'account') {
      applyAccount(db, bytesToObj<AccountPayload>(remotePayload), forcedVersion);
    } else {
      applyTransaction(
        db,
        bytesToObj<TransactionPayload>(remotePayload),
        forcedVersion,
      );
    }
    // Drop the superseded local pending event.
    db.run('UPDATE sync_events SET synced = 1 WHERE id = ?', [localEventId]);
  }

  db.run(
    'UPDATE sync_conflicts SET resolved = 1, resolution = ? WHERE id = ?',
    [keepRemote ? 'KeepRemote' : 'KeepLocal', conflictId],
  );
}

// ── JS fetch callback factory for WasmSyncClient ─────────────────────────────────

/**
 * Build the `(request) => Promise<{status, body}>` callback the
 * `WasmSyncClient` injects for HTTP. All networking lives here, in JS.
 */
export function makeFetchCallback(baseUrl: string) {
  const base = baseUrl.replace(/\/+$/, '');
  return async (request: {
    method: string;
    path: string;
    query: Array<[string, string]>;
    headers: Array<[string, string]>;
    body: Uint8Array;
  }): Promise<{ status: number; body: Uint8Array }> => {
    const url = new URL(base + request.path);
    for (const [k, v] of request.query) url.searchParams.append(k, v);

    const init: RequestInit = {
      method: request.method,
      headers: request.headers,
    };
    if (request.body && request.body.length > 0 && request.method !== 'GET') {
      init.body = request.body as BodyInit;
    }

    const resp = await fetch(url.toString(), init);
    const buf = new Uint8Array(await resp.arrayBuffer());
    return { status: resp.status, body: buf };
  };
}
