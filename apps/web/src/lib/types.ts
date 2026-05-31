/**
 * TypeScript types matching the Rust JSON serialization output.
 *
 * These types correspond to structs in:
 * - crates/ldgr-core/src/accounting/types.rs
 * - crates/ldgr-core/src/accounting/reports.rs
 */

// ── Accounting Types ───────────────────────────────────────────────────────────

export type TransactionStatus = 'Unmarked' | 'Pending' | 'Cleared';

export interface Amount {
  quantity: string;
  commodity: string;
}

export interface Posting {
  account: string;
  amount: Amount | null;
  balance_assertion: Amount | null;
  comment: string | null;
  tags: Record<string, string>;
  source_line: number;
}

export interface Transaction {
  date: string;
  status: TransactionStatus;
  code: string | null;
  description: string;
  comment: string | null;
  tags: Record<string, string>;
  postings: Posting[];
  source_line: number;
}

// ── Report Types ───────────────────────────────────────────────────────────────

export interface AccountBalance {
  account: string;
  balances: Record<string, string>;
  depth: number;
}

export interface BalanceReport {
  accounts: AccountBalance[];
  totals: Record<string, string>;
}

export interface RegisterEntry {
  date: string;
  description: string;
  account: string;
  amount: string;
  commodity: string;
  running_balance: string;
}

export interface RegisterReport {
  entries: RegisterEntry[];
}

// ── Storage Types (sql.js rows) ────────────────────────────────────────────────

export type AccountType = 'asset' | 'liability' | 'income' | 'expense' | 'equity';

export interface StoredAccount {
  id: string;
  name: string;
  type: AccountType;
  commodity: string;
  parent_id: string | null;
  note: string | null;
  created_at: string;
  modified_at: string;
  version: number;
  deleted: number;
}

export interface StoredTransaction {
  id: string;
  date: string;
  status: string;
  code: string | null;
  description: string;
  comment: string | null;
  created_at: string;
  modified_at: string;
  version: number;
  deleted: number;
}

export interface StoredPosting {
  id: string;
  transaction_id: string;
  account_id: string;
  amount_quantity: string | null;
  amount_commodity: string | null;
  posting_order: number;
  created_at: string;
}
