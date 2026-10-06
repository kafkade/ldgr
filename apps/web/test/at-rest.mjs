/**
 * At-rest encryption regression test for the web working store (issue #315).
 *
 * The web app never persists a plaintext SQLite database: `sql.js` runs purely
 * in memory, and `VaultContext.saveVault()` seals the exported db bytes into the
 * Rust core's AES-256-GCM vault container (`serializeVault()`) BEFORE writing the
 * result to IndexedDB (`saveVaultBlob`). This test locks that guarantee in:
 *
 *   Part 1 (crypto) — reproduces the exact saveVault pipeline against the REAL
 *   compiled `ldgr-wasm` (addItem → serializeVault) and asserts the persisted
 *   blob is ciphertext: no `SQLite format 3` magic header and none of the known
 *   plaintext tokens (account name, sync token, 2SKD secret key) survive. Also
 *   proves the sealed item round-trips back out and that a wrong password cannot
 *   open it. Requires `npm run build:wasm`; skipped gracefully if pkg/ is absent.
 *
 *   Part 2 (static audit) — scans the web source and asserts the ONLY keys ever
 *   written to localStorage / sessionStorage are the documented non-secret ones
 *   (theme preference; ephemeral admin-panel bearer token), so no vault secret
 *   can leak to Web Storage, and that saveVault seals before it persists.
 */

import { test, describe } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const pkgJs = new URL('../pkg/ldgr_wasm.js', import.meta.url);
const pkgWasm = new URL('../pkg/ldgr_wasm_bg.wasm', import.meta.url);
const havePkg = existsSync(fileURLToPath(pkgJs)) && existsSync(fileURLToPath(pkgWasm));

const SQLITE_MAGIC = Buffer.from('SQLite format 3\0', 'latin1');

// The known plaintext secrets a broken saveVault would leak. These live in the
// sql.js DB (account names + the sync_state table's token / 2SKD secret key), so
// they must all be sealed inside the vault container, never persisted in clear.
const ACCOUNT_NAME = 'Assets:SecretBankAccount';
const SYNC_TOKEN = 'sync-token-DEADBEEF-must-not-leak';
const SECRET_KEY = 'A3-SECRETKEY-MUST-NOT-LEAK-0000';

/** Build a fake sql.js `db.export()` blob: a real SQLite header + secret tokens. */
function fakeSqlJsExport() {
  const body = Buffer.from(
    `\x00tables...${ACCOUNT_NAME}...sync_state:${SYNC_TOKEN}...secret:${SECRET_KEY}...`,
    'latin1',
  );
  return new Uint8Array(Buffer.concat([SQLITE_MAGIC, body]));
}

const contains = (haystack, needle) =>
  Buffer.from(haystack).includes(Buffer.from(needle, 'latin1'));

describe('web working store is encrypted at rest', {
  skip: havePkg ? false : 'pkg not built (run npm run build:wasm)',
}, () => {
  let LdgrWasm;
  let persisted; // the blob saveVault() would write to IndexedDB

  test('loads the compiled wasm module', async () => {
    const mod = await import(pkgJs.href);
    await mod.default({ module_or_path: readFileSync(pkgWasm) });
    LdgrWasm = mod.LdgrWasm;
    assert.equal(typeof LdgrWasm.createVault, 'function');
  });

  test('the blob persisted to IndexedDB is ciphertext, not plaintext SQLite', () => {
    // Mirror VaultContext.saveVault(): seal the exported db bytes as vault item 0,
    // then serialize the sealed container — this is exactly what saveVaultBlob
    // receives.
    const created = LdgrWasm.createVault('correct-horse-battery', 'web');
    const vault = LdgrWasm.openVault(created.vaultData, 'correct-horse-battery');

    const dbBlob = fakeSqlJsExport();
    if (vault.itemCount() > 0) {
      vault.replaceItem(0, dbBlob);
    } else {
      vault.addItem(dbBlob);
    }
    persisted = vault.serializeVault();

    assert.ok(persisted instanceof Uint8Array && persisted.length > 0);

    // Not a plaintext SQLite database.
    assert.ok(
      !Buffer.from(persisted.subarray(0, 16)).equals(SQLITE_MAGIC),
      'persisted vault blob must not start with the SQLite magic header',
    );
    // None of the plaintext secrets survive in the sealed blob.
    assert.ok(!contains(persisted, ACCOUNT_NAME), 'account name leaked to disk');
    assert.ok(!contains(persisted, SYNC_TOKEN), 'sync token leaked to disk');
    assert.ok(!contains(persisted, SECRET_KEY), 'secret key leaked to disk');
    assert.ok(!contains(persisted, 'SQLite format 3'), 'SQLite header leaked to disk');
  });

  test('the sealed db blob round-trips back out with the right password', () => {
    const reopened = LdgrWasm.openVault(persisted, 'correct-horse-battery');
    assert.equal(reopened.itemCount(), 1);
    const restored = reopened.getItem(reopened.itemCount() - 1);
    assert.ok(
      Buffer.from(restored).equals(Buffer.from(fakeSqlJsExport())),
      'sealed db bytes must decrypt back to the original export',
    );
  });

  test('a wrong password cannot open the persisted blob', () => {
    assert.throws(() => LdgrWasm.openVault(persisted, 'wrong-password'));
  });
});

describe('web Web Storage carries no vault secrets (static audit)', () => {
  const webSrc = fileURLToPath(new URL('../src', import.meta.url));

  // The complete allowlist of keys the web app may write to localStorage /
  // sessionStorage. Both are NON-secret: the theme preference, and the ephemeral
  // admin-panel bearer token (sessionStorage only — cleared on tab close, never a
  // vault secret). Any new key must be reviewed for at-rest leakage before being
  // added here.
  const ALLOWED_KEYS = new Set(['ldgr-theme', 'ldgr:admin:session']);

  function walk(dir) {
    const files = [];
    for (const entry of readdirSync(dir)) {
      const p = `${dir}/${entry}`;
      if (statSync(p).isDirectory()) files.push(...walk(p));
      else if (/\.(ts|tsx)$/.test(entry)) files.push(p);
    }
    return files;
  }

  test('every localStorage/sessionStorage key written is a known non-secret', () => {
    const setItem = /\b(?:local|session)Storage\.setItem\(\s*['"`]([^'"`]+)['"`]/g;
    const found = new Set();
    for (const file of walk(webSrc)) {
      const src = readFileSync(file, 'utf8');
      for (const m of src.matchAll(setItem)) found.add(m[1]);
    }
    assert.ok(found.size > 0, 'expected to find at least one setItem call to audit');
    for (const key of found) {
      assert.ok(
        ALLOWED_KEYS.has(key),
        `unexpected Web Storage key "${key}" — audit it for at-rest secret leakage`,
      );
    }
  });

  test('saveVault seals via serializeVault before persisting to IndexedDB', () => {
    const src = readFileSync(`${webSrc}/contexts/VaultContext.tsx`, 'utf8');
    const seal = src.indexOf('serializeVault()');
    const persist = src.indexOf('saveVaultBlob(', seal);
    assert.ok(seal !== -1, 'saveVault must call serializeVault()');
    assert.ok(
      persist !== -1 && persist > seal,
      'saveVaultBlob must be called with the sealed serializeVault() output',
    );
  });
});
