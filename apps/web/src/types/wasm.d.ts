/**
 * Type declarations for the wasm-pack generated module.
 * This file is only used when the WASM package has not been built yet
 * (pkg/ directory is gitignored and built on demand).
 */

/* eslint-disable @typescript-eslint/no-explicit-any */
declare module '*pkg/ldgr_wasm' {
  export default function init(): Promise<void>;

  export class LdgrWasm {
    static createVault(password: string, name: string): CreateVaultResult;
    static openVault(vaultData: Uint8Array, password: string): LdgrWasm;
    get vaultName(): string;
    addItem(plaintext: Uint8Array): void;
    getItem(index: number): Uint8Array;
    replaceItem(index: number, plaintext: Uint8Array): void;
    clearItems(): void;
    get itemCount(): number;
    serializeVault(): Uint8Array;
    kdfParams(): string;
    sealBatch(eventBatchJson: string): Uint8Array;
    openBatch(ciphertext: Uint8Array): string;
    free(): void;
  }

  export class WasmSyncClient {
    constructor(sendCallback: (request: any) => Promise<any>);
    static withToken(
      sendCallback: (request: any) => Promise<any>,
      token: string,
    ): WasmSyncClient;
    get token(): string | undefined;
    isAuthenticated(): boolean;
    logout(): void;
    register(username: string, password: string): Promise<void>;
    login(username: string, password: string): Promise<void>;
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
    login2skd(
      username: string,
      password: string,
      secretKey: string,
      argon2Salt: Uint8Array,
      memoryCostKib: number,
      iterations: number,
      parallelism: number,
    ): Promise<void>;
    serverInfo(): Promise<string>;
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

  export class CreateVaultResult {
    get vaultData(): Uint8Array;
    get recoveryKey(): string;
    free(): void;
  }

  export function parseJournal(text: string): string;
  export function generateSecretKey(): string;
  export function buildEmergencyKit(
    address: string,
    email: string,
    secretKey: string,
    recoveryKey?: string | null,
  ): string;
  export function mergeBatch(
    localPendingJson: string,
    remoteBatchJson: string,
    localClockJson: string,
    now: string,
  ): string;
  export function computeBalance(
    transactionsJson: string,
    accountFilter?: string,
    beginDate?: string,
    endDate?: string,
  ): string;
  export function computeRegister(
    transactionsJson: string,
    accountFilter?: string,
    beginDate?: string,
    endDate?: string,
  ): string;
}
