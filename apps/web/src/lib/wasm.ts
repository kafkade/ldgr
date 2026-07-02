/**
 * Dynamic WASM module loader for ldgr-wasm.
 *
 * Lazily loads the wasm-pack output and caches the initialized module.
 * Must only be called from client components (browser environment).
 */

// Type definitions for the wasm-bindgen generated module.
// These mirror the Rust exports in crates/ldgr-wasm/src/lib.rs.

export interface CreateVaultResult {
  readonly vaultData: Uint8Array;
  readonly recoveryKey: string;
  free(): void;
}

export interface LdgrWasm {
  readonly vaultName: string;
  addItem(plaintext: Uint8Array): void;
  getItem(index: number): Uint8Array;
  replaceItem(index: number, plaintext: Uint8Array): void;
  clearItems(): void;
  readonly itemCount: number;
  serializeVault(): Uint8Array;
  /**
   * The vault's Argon2id KDF salt and parameters as a JSON string
   * (see {@link KdfParams}). Two-secret sign-in derives `MK_auth` from the
   * master password using exactly these values (ADR-008).
   */
  kdfParams(): string;
  /** Seal an EventBatch JSON string into the canonical encrypted blob. */
  sealBatch(eventBatchJson: string): Uint8Array;
  /** Decrypt a canonical blob back into an EventBatch JSON string. */
  openBatch(ciphertext: Uint8Array): string;
  free(): void;
}

export interface LdgrWasmStatic {
  createVault(password: string, name: string): CreateVaultResult;
  openVault(vaultData: Uint8Array, password: string): LdgrWasm;
}

/** A raw HTTP request the JS `fetch` callback must execute for sync. */
export interface SyncRawRequest {
  method: string;
  path: string;
  query: Array<[string, string]>;
  headers: Array<[string, string]>;
  body: Uint8Array;
}

/** The response shape the JS `fetch` callback must resolve with. */
export interface SyncRawResponse {
  status: number;
  body: Uint8Array;
}

export type SyncSendCallback = (
  request: SyncRawRequest,
) => Promise<SyncRawResponse>;

export interface WasmSyncClient {
  readonly token: string | undefined;
  isAuthenticated(): boolean;
  logout(): void;
  register(username: string, password: string): Promise<void>;
  login(username: string, password: string): Promise<void>;
  /**
   * Register a two-secret (2SKD) account. Returns the assigned user id.
   * `accountId` comes from {@link WasmModule.generateSecretKey}.
   */
  register2skd(
    username: string,
    accountId: string,
    password: string,
    secretKey: string,
    argon2Salt: Uint8Array,
    memoryCostKib: number,
    iterations: number,
    parallelism: number,
  ): Promise<string>;
  /** Two-secret (2SKD) login; the account id is supplied by the server. */
  login2skd(
    username: string,
    password: string,
    secretKey: string,
    argon2Salt: Uint8Array,
    memoryCostKib: number,
    iterations: number,
    parallelism: number,
  ): Promise<void>;
  /** Fetch `GET /server/info`; returns a JSON string (see {@link ServerInfo}). */
  serverInfo(): Promise<string>;
  /** Liveness probe (`GET /server/ping`); returns a JSON string. */
  ping(): Promise<string>;
  createVault(vaultId: string): Promise<void>;
  putBatch(
    vaultId: string,
    deviceId: string,
    batchId: string,
    ciphertext: Uint8Array,
  ): Promise<void>;
  getBatch(
    vaultId: string,
    deviceId: string,
    batchId: string,
  ): Promise<Uint8Array>;
  listBatches(
    vaultId: string,
    since?: string | null,
    deviceId?: string | null,
    limit?: number | null,
  ): Promise<string>;
  free(): void;
}

export interface WasmSyncClientStatic {
  new (sendCallback: SyncSendCallback): WasmSyncClient;
  withToken(sendCallback: SyncSendCallback, token: string): WasmSyncClient;
}

/** Parsed shape of {@link LdgrWasm.kdfParams}'s JSON result. */
export interface KdfParams {
  /** Argon2id salt bytes (the vault header salt). */
  salt: number[];
  memoryCostKib: number;
  iterations: number;
  parallelism: number;
}

/** Parsed shape of {@link WasmModule.generateSecretKey}'s JSON result. */
export interface SecretKeyMaterial {
  accountId: string;
  secretKey: string;
  accountHint: string;
}

/** Parsed shape of {@link WasmModule.buildEmergencyKit}'s JSON result. */
export interface EmergencyKit {
  version: number;
  address: string;
  email: string;
  accountHint: string;
  secretKey: string;
  recoveryKey: string | null;
  qrPayload: string;
}

/** Parsed shape of {@link WasmSyncClient.serverInfo}'s JSON result. */
export interface ServerInfo {
  name: string;
  version: string;
  protocol_version: number;
  min_protocol_version: number;
  max_protocol_version: number;
  registration_policy: string;
  public_registration: boolean;
  two_secret_auth: boolean;
}

export interface WasmModule {
  LdgrWasm: LdgrWasmStatic;
  WasmSyncClient: WasmSyncClientStatic;
  parseJournal(text: string): string;
  /** Generate a fresh account id + Secret Key; returns JSON (see {@link SecretKeyMaterial}). */
  generateSecretKey(): string;
  /** Build an Emergency Kit; returns JSON (see {@link EmergencyKit}). */
  buildEmergencyKit(
    address: string,
    email: string,
    secretKey: string,
    recoveryKey?: string | null,
  ): string;
  computeBalance(
    transactionsJson: string,
    accountFilter?: string,
    beginDate?: string,
    endDate?: string,
  ): string;
  computeRegister(
    transactionsJson: string,
    accountFilter?: string,
    beginDate?: string,
    endDate?: string,
  ): string;
  /**
   * Three-way merge a decrypted remote batch against local pending events.
   * Returns JSON `{ applied, conflicts, skipped }`.
   */
  mergeBatch(
    localPendingJson: string,
    remoteBatchJson: string,
    localClockJson: string,
    now: string,
  ): string;
}

let cached: WasmModule | null = null;
let loading: Promise<WasmModule> | null = null;

export async function loadWasm(): Promise<WasmModule> {
  if (cached) return cached;
  if (loading) return loading;

  loading = (async () => {
    try {
      // Dynamic import of wasm-pack output (built to apps/web/pkg/)
      const wasm = await import('../../pkg/ldgr_wasm');
      await wasm.default();
      cached = wasm as unknown as WasmModule;
      return cached;
    } catch (err) {
      loading = null;
      throw new Error(
        `Failed to load ldgr WASM module. Run "npm run build:wasm" first. ${err}`,
      );
    }
  })();

  return loading;
}

export function getWasm(): WasmModule | null {
  return cached;
}
