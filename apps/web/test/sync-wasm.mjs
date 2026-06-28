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
