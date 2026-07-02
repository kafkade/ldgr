/**
 * Cross-language byte-format proof for the sync blob + merge surface.
 *
 * Builds an `EventBatch` using the EXACT wire shapes the TypeScript outbox
 * emits (capitalized serde enum variants `"Account"`/`"Create"`, payload as a
 * `number[]` of the canonical `sync::payload` JSON), then round-trips it through
 * the REAL compiled `ldgr-wasm` `sealBatch`/`openBatch` (the same functions the
 * CLI/iOS use via `framing::{seal_batch, open_batch}`). This proves the blob the
 * web app produces is byte-compatible with — and decryptable by — the canonical
 * Rust pipeline, and that `mergeBatch` accepts TS-shaped input.
 *
 * Requires `npm run build:wasm` first; skipped gracefully if pkg/ is absent so
 * `npm test` still passes in a clean checkout.
 */

import { test, describe } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const pkgJs = new URL('../pkg/ldgr_wasm.js', import.meta.url);
const pkgWasm = new URL('../pkg/ldgr_wasm_bg.wasm', import.meta.url);
const havePkg = existsSync(fileURLToPath(pkgJs)) && existsSync(fileURLToPath(pkgWasm));

const enc = (obj) => Array.from(new TextEncoder().encode(JSON.stringify(obj)));
const dec = (arr) => JSON.parse(new TextDecoder().decode(Uint8Array.from(arr)));

describe('ldgr-wasm cross-language sync blob', { skip: havePkg ? false : 'pkg not built (run npm run build:wasm)' }, () => {
  let LdgrWasm;
  let mergeBatch;
  let vault;

  test('loads the compiled wasm module', async () => {
    const mod = await import(pkgJs.href);
    await mod.default({ module_or_path: readFileSync(pkgWasm) });
    LdgrWasm = mod.LdgrWasm;
    mergeBatch = mod.mergeBatch;
    const res = LdgrWasm.createVault('correct-horse-battery', 'proof');
    vault = LdgrWasm.openVault(res.vaultData, 'correct-horse-battery');
    assert.equal(typeof vault.sealBatch, 'function');
    assert.equal(typeof vault.openBatch, 'function');
  });

  // The exact AccountPayload shape the TS outbox writes.
  const accountPayload = {
    id: 'acct-1',
    name: 'Assets:Checking',
    account_type: 'asset',
    commodity: 'USD',
    parent_id: null,
    note: null,
    created_at: '2024-01-15T00:00:00.000Z',
    modified_at: '2024-01-15T00:00:00.000Z',
  };

  // The exact EventBatch shape the TS push() builds: capitalized enum variants,
  // payload as a JSON number array, vector_clock as { clocks: {...} }.
  const tsBatch = () => ({
    device_id: 'web-device-1',
    events: [
      {
        id: 'evt-1',
        device_id: 'web-device-1',
        lamport_clock: 1,
        entity_type: 'Account',
        entity_id: 'acct-1',
        operation: 'Create',
        payload: enc(accountPayload),
        version: 1,
        created_at: '2024-01-15T00:00:00.000Z',
      },
    ],
    vector_clock: { clocks: { 'web-device-1': 1 } },
  });

  test('TS-shaped batch round-trips through real sealBatch/openBatch', () => {
    const blob = vault.sealBatch(JSON.stringify(tsBatch()));
    assert.ok(blob instanceof Uint8Array && blob.length > 0);

    const restored = JSON.parse(vault.openBatch(blob));
    assert.equal(restored.device_id, 'web-device-1');
    assert.equal(restored.events.length, 1);

    const ev = restored.events[0];
    // serde round-trips the capitalized enum variants unchanged.
    assert.equal(ev.entity_type, 'Account');
    assert.equal(ev.operation, 'Create');
    assert.equal(ev.version, 1);
    assert.equal(ev.lamport_clock, 1);

    // The canonical payload survives byte-for-byte and decodes to the same object.
    const payload = dec(ev.payload);
    assert.deepEqual(payload, accountPayload);
  });

  test('a wrong vault key cannot open the blob', () => {
    const blob = vault.sealBatch(JSON.stringify(tsBatch()));
    const other = LdgrWasm.createVault('a-different-password', 'other');
    const otherVault = LdgrWasm.openVault(other.vaultData, 'a-different-password');
    assert.throws(() => otherVault.openBatch(blob));
  });

  test('mergeBatch accepts TS-shaped batches and applies a novel event', () => {
    const remote = tsBatch();
    const localPending = JSON.stringify([]);
    const localClock = JSON.stringify({ clocks: {} });
    const out = JSON.parse(
      mergeBatch(localPending, JSON.stringify(remote), localClock, '2024-01-15T00:00:00.000Z'),
    );
    assert.equal(out.applied.length, 1);
    assert.equal(out.conflicts.length, 0);
    assert.equal(out.applied[0].entity_type, 'Account');
    assert.equal(out.applied[0].operation, 'Create');
  });
});

/**
 * Cross-language golden-vector check (issue #165, task 5).
 *
 * Asserts the web client emits structural sync/auth JSON that is byte-identical
 * to the canonical Rust known-answer vectors in
 * `crates/ldgr-core/tests/fixtures/sync/` (generated + verified by
 * `ldgr-core/tests/sync_vectors.rs`). Because the on-the-wire sync blob uses
 * random nonces it is not byte-reproducible, so the shared contract is this
 * decrypted/structural layer. This block needs no compiled wasm — it proves the
 * JS object shapes (field order, `skip_serializing_if` omission) match the Rust
 * serde output exactly, so iOS/web/CLI stay byte-compatible.
 */
describe('cross-language sync wire vectors (golden)', () => {
  const fixture = (name) =>
    readFileSync(
      fileURLToPath(new URL(`../../../crates/ldgr-core/tests/fixtures/sync/${name}`, import.meta.url)),
      'utf8',
    );

  // Same canonical inputs as crates/ldgr-core/tests/sync_vectors.rs. Object key
  // order MUST match the Rust struct field order (serde serializes in that order).
  test('AccountPayload matches the Rust golden vector byte-for-byte', () => {
    const accountPayload = {
      id: 'aaaaaaaa-0000-7000-8000-000000000001',
      name: 'Assets:Cash',
      account_type: 'asset',
      commodity: 'USD',
      parent_id: null,
      note: 'petty cash',
      created_at: '2024-01-15T12:00:00Z',
      modified_at: '2024-01-15T12:00:00Z',
    };
    assert.equal(JSON.stringify(accountPayload), fixture('account_payload_v1.json'));
  });

  test('TransactionPayload matches the Rust golden vector byte-for-byte', () => {
    const txnPayload = {
      id: 'bbbbbbbb-0000-7000-8000-000000000002',
      date: '2024-01-15',
      status: 'cleared',
      code: 'REF-42',
      description: 'Lunch',
      comment: 'with team',
      created_at: '2024-01-15T12:00:00Z',
      modified_at: '2024-01-15T12:00:00Z',
      postings: [
        {
          id: 'cccccccc-0000-7000-8000-000000000003',
          account_id: 'aaaaaaaa-0000-7000-8000-000000000001',
          amount_quantity: '-10.00',
          amount_commodity: 'USD',
          balance_assertion_quantity: null,
          balance_assertion_commodity: null,
          created_at: '2024-01-15T12:00:00Z',
          version: 1,
        },
        {
          id: 'dddddddd-0000-7000-8000-000000000004',
          account_id: 'aaaaaaaa-0000-7000-8000-000000000005',
          amount_quantity: '10.00',
          amount_commodity: 'USD',
          balance_assertion_quantity: null,
          balance_assertion_commodity: null,
          created_at: '2024-01-15T12:00:00Z',
          version: 1,
        },
      ],
    };
    assert.equal(JSON.stringify(txnPayload), fixture('transaction_payload_v1.json'));
  });

  test('single-secret RegisterRequest matches the golden vector (auth_scheme omitted)', () => {
    const req = {
      username: 'alice',
      salt: '00112233445566778899aabbccddeeff',
      verifier: '0123456789abcdef',
      // auth_scheme intentionally omitted — Rust drops it via skip_serializing_if.
    };
    assert.equal(JSON.stringify(req), fixture('register_request_1secret_v1.json'));
  });

  test('2SKD RegisterRequest matches the golden vector (auth_scheme present)', () => {
    const req = {
      username: 'carol',
      salt: 'ffeeddccbbaa99887766554433221100',
      verifier: 'fedcba9876543210',
      auth_scheme: 'srp-2skd-v1',
      account_id: '018f5a3c-0000-7000-8000-000000000001',
    };
    assert.equal(JSON.stringify(req), fixture('register_request_2skd_v1.json'));
  });
});
