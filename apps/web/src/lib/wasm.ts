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
  free(): void;
}

export interface LdgrWasmStatic {
  createVault(password: string, name: string): CreateVaultResult;
  openVault(vaultData: Uint8Array, password: string): LdgrWasm;
}

export interface WasmModule {
  LdgrWasm: LdgrWasmStatic;
  parseJournal(text: string): string;
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
