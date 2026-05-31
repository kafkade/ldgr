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
    free(): void;
  }

  export class CreateVaultResult {
    get vaultData(): Uint8Array;
    get recoveryKey(): string;
    free(): void;
  }

  export function parseJournal(text: string): string;
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
