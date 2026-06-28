'use client';

import {
  createContext,
  useContext,
  useState,
  useCallback,
  useEffect,
  useRef,
  type ReactNode,
} from 'react';
import type { Database } from 'sql.js';
import type { LdgrWasm, WasmModule, WasmSyncClient } from '@/lib/wasm';
import { loadWasm } from '@/lib/wasm';
import {
  saveVaultBlob,
  loadVaultBlob,
  listVaultNames,
  createDatabase,
  exportDatabase,
} from '@/lib/storage';
import {
  createAccount as emitCreateAccount,
  createTransaction as emitCreateTransaction,
  deleteTransaction as emitDeleteTransaction,
  runSync,
  loadServerConfig,
  saveServerConfig,
  loadToken,
  saveToken,
  clearToken,
  getOrCreateDeviceId,
  listOpenConflicts,
  resolveConflict,
  makeFetchCallback,
  type ServerConfig,
  type SyncOutcome,
  type ConflictRow,
} from '@/lib/sync';
import type {
  StoredAccount,
  StoredTransaction,
  StoredPosting,
  BalanceReport,
  RegisterReport,
  Transaction,
} from '@/lib/types';

// ── Types ──────────────────────────────────────────────────────────────────────

interface VaultState {
  status: 'loading' | 'locked' | 'unlocked' | 'error';
  vaultName: string | null;
  error: string | null;
  existingVaults: string[];
}

interface VaultData {
  accounts: StoredAccount[];
  transactions: StoredTransaction[];
  postings: StoredPosting[];
}

interface VaultContextValue {
  state: VaultState;
  data: VaultData;
  wasm: WasmModule | null;

  createVault: (password: string, name: string) => Promise<string>;
  unlockVault: (name: string, password: string) => Promise<void>;
  lockVault: () => void;
  refreshVaults: () => Promise<void>;

  addAccount: (name: string, type: string, commodity: string) => void;
  addTransaction: (
    date: string,
    description: string,
    postings: Array<{ accountId: string; amount: string; commodity: string }>,
  ) => void;
  deleteTransaction: (id: string) => void;
  saveVault: () => Promise<void>;

  // ── Sync ──
  serverConfig: ServerConfig | null;
  serverAuthenticated: boolean;
  syncing: boolean;
  deviceId: string | null;
  conflicts: ConflictRow[];
  configureServer: (cfg: ServerConfig) => Promise<void>;
  registerServer: (username: string, password: string) => Promise<void>;
  loginServer: (username: string, password: string) => Promise<void>;
  logoutServer: () => Promise<void>;
  createRemoteVault: () => Promise<void>;
  sync: () => Promise<SyncOutcome>;
  resolveSyncConflict: (id: string, keepRemote: boolean) => Promise<void>;

  getBalanceReport: (accountFilter?: string) => BalanceReport | null;
  getRegisterReport: (accountFilter?: string, begin?: string, end?: string) => RegisterReport | null;
  getTransactionsForWasm: () => Transaction[];
}

const VaultContext = createContext<VaultContextValue | null>(null);

export function useVault(): VaultContextValue {
  const ctx = useContext(VaultContext);
  if (!ctx) throw new Error('useVault must be used within VaultProvider');
  return ctx;
}

// ── Provider ───────────────────────────────────────────────────────────────────

export function VaultProvider({ children }: { children: ReactNode }) {
  const [wasm, setWasm] = useState<WasmModule | null>(null);
  const [vault, setVault] = useState<LdgrWasm | null>(null);
  const [db, setDb] = useState<Database | null>(null);
  const [state, setState] = useState<VaultState>({
    status: 'loading',
    vaultName: null,
    error: null,
    existingVaults: [],
  });
  const [data, setData] = useState<VaultData>({
    accounts: [],
    transactions: [],
    postings: [],
  });

  const clientRef = useRef<WasmSyncClient | null>(null);
  const [serverConfig, setServerConfig] = useState<ServerConfig | null>(null);
  const [serverAuthenticated, setServerAuthenticated] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [conflicts, setConflicts] = useState<ConflictRow[]>([]);

  // Hydrate the in-memory sync state from a freshly opened database.
  const initSyncState = useCallback((database: Database) => {
    clientRef.current = null;
    setServerConfig(loadServerConfig(database));
    setDeviceId(getOrCreateDeviceId(database));
    setConflicts(listOpenConflicts(database));
    setServerAuthenticated(loadToken(database) !== null);
  }, []);

  // Load WASM on mount
  useEffect(() => {
    loadWasm()
      .then((w) => {
        setWasm(w);
        return listVaultNames();
      })
      .then((names) => {
        setState((s) => ({
          ...s,
          status: 'locked',
          existingVaults: names,
        }));
      })
      .catch((err) => {
        setState((s) => ({
          ...s,
          status: 'error',
          error: String(err),
        }));
      });
  }, []);

  const refreshData = useCallback((database: Database) => {
    const accts = database
      .exec('SELECT * FROM accounts WHERE deleted = 0 ORDER BY name')
      .flatMap((r) =>
        r.values.map((row) => ({
          id: row[0] as string,
          name: row[1] as string,
          type: row[2] as StoredAccount['type'],
          commodity: row[3] as string,
          parent_id: row[4] as string | null,
          note: row[5] as string | null,
          created_at: row[6] as string,
          modified_at: row[7] as string,
          version: row[8] as number,
          deleted: row[9] as number,
        })),
      );

    const txns = database
      .exec(
        'SELECT * FROM transactions WHERE deleted = 0 ORDER BY date DESC, created_at DESC',
      )
      .flatMap((r) =>
        r.values.map((row) => ({
          id: row[0] as string,
          date: row[1] as string,
          status: row[2] as string,
          code: row[3] as string | null,
          description: row[4] as string,
          comment: row[5] as string | null,
          created_at: row[6] as string,
          modified_at: row[7] as string,
          version: row[8] as number,
          deleted: row[9] as number,
        })),
      );

    const posts = database
      .exec('SELECT * FROM postings ORDER BY transaction_id, posting_order')
      .flatMap((r) =>
        r.values.map((row) => ({
          id: row[0] as string,
          transaction_id: row[1] as string,
          account_id: row[2] as string,
          amount_quantity: row[3] as string | null,
          amount_commodity: row[4] as string | null,
          posting_order: row[5] as number,
          created_at: row[6] as string,
        })),
      );

    setData({ accounts: accts, transactions: txns, postings: posts });
  }, []);

  const refreshVaults = useCallback(async () => {
    const names = await listVaultNames();
    setState((s) => ({ ...s, existingVaults: names }));
  }, []);

  const createVault = useCallback(
    async (password: string, name: string): Promise<string> => {
      if (!wasm) throw new Error('WASM not loaded');

      const result = wasm.LdgrWasm.createVault(password, name);
      const vaultData = result.vaultData;
      const recoveryKey = result.recoveryKey;

      await saveVaultBlob(name, vaultData);

      const v = wasm.LdgrWasm.openVault(vaultData, password);
      const database = await createDatabase();

      setVault(v);
      setDb(database);
      refreshData(database);
      initSyncState(database);
      setState((s) => ({
        ...s,
        status: 'unlocked',
        vaultName: name,
        error: null,
      }));

      result.free();
      return recoveryKey;
    },
    [wasm, refreshData, initSyncState],
  );

  const unlockVault = useCallback(
    async (name: string, password: string) => {
      if (!wasm) throw new Error('WASM not loaded');

      const blob = await loadVaultBlob(name);
      if (!blob) throw new Error(`Vault "${name}" not found`);

      const v = wasm.LdgrWasm.openVault(blob, password);
      let database: Database;

      if (v.itemCount > 0) {
        const dbBlob = v.getItem(v.itemCount - 1);
        database = await createDatabase(dbBlob);
      } else {
        database = await createDatabase();
      }

      setVault(v);
      setDb(database);
      refreshData(database);
      initSyncState(database);
      setState((s) => ({
        ...s,
        status: 'unlocked',
        vaultName: name,
        error: null,
      }));
    },
    [wasm, refreshData, initSyncState],
  );

  const lockVault = useCallback(() => {
    if (vault) vault.free();
    if (db) db.close();
    clientRef.current = null;
    setVault(null);
    setDb(null);
    setData({ accounts: [], transactions: [], postings: [] });
    setServerConfig(null);
    setServerAuthenticated(false);
    setDeviceId(null);
    setConflicts([]);
    setState((s) => ({ ...s, status: 'locked', vaultName: null }));
  }, [vault, db]);

  const addAccount = useCallback(
    (name: string, type: string, commodity: string) => {
      if (!db) return;
      emitCreateAccount(db, { name, type, commodity });
      refreshData(db);
    },
    [db, refreshData],
  );

  const addTransaction = useCallback(
    (
      date: string,
      description: string,
      postings: Array<{ accountId: string; amount: string; commodity: string }>,
    ) => {
      if (!db) return;
      emitCreateTransaction(db, date, description, postings);
      refreshData(db);
    },
    [db, refreshData],
  );

  const deleteTransaction = useCallback(
    (id: string) => {
      if (!db) return;
      emitDeleteTransaction(db, id);
      refreshData(db);
    },
    [db, refreshData],
  );

  const saveVault = useCallback(async () => {
    if (!vault || !db || !state.vaultName) return;
    const dbBlob = exportDatabase(db);
    if (vault.itemCount > 0) {
      vault.replaceItem(0, dbBlob);
    } else {
      vault.addItem(dbBlob);
    }
    const vaultBlob = vault.serializeVault();
    await saveVaultBlob(state.vaultName, vaultBlob);
  }, [vault, db, state.vaultName]);

  // ── Sync actions ──────────────────────────────────────────────────────────────

  const buildClient = useCallback(
    (cfg: ServerConfig, token?: string | null): WasmSyncClient => {
      if (!wasm) throw new Error('WASM not loaded');
      const callback = makeFetchCallback(cfg.serverUrl);
      return token
        ? wasm.WasmSyncClient.withToken(callback, token)
        : new wasm.WasmSyncClient(callback);
    },
    [wasm],
  );

  const configureServer = useCallback(
    async (cfg: ServerConfig) => {
      if (!db) return;
      saveServerConfig(db, cfg);
      setServerConfig(cfg);
      clientRef.current = null;
      await saveVault();
    },
    [db, saveVault],
  );

  const registerServer = useCallback(
    async (username: string, password: string) => {
      if (!serverConfig) throw new Error('Configure the server first');
      const client = buildClient(serverConfig);
      await client.register(username, password);
    },
    [serverConfig, buildClient],
  );

  const loginServer = useCallback(
    async (username: string, password: string) => {
      if (!db || !serverConfig) throw new Error('Configure the server first');
      const client = buildClient(serverConfig);
      await client.login(username, password);
      clientRef.current = client;
      const token = client.token;
      if (token) saveToken(db, token);
      setServerAuthenticated(client.isAuthenticated());
      await saveVault();
    },
    [db, serverConfig, buildClient, saveVault],
  );

  const logoutServer = useCallback(async () => {
    if (!db) return;
    clientRef.current?.logout();
    clientRef.current = null;
    clearToken(db);
    setServerAuthenticated(false);
    await saveVault();
  }, [db, saveVault]);

  const createRemoteVault = useCallback(async () => {
    if (!db || !serverConfig) throw new Error('Configure the server first');
    const client =
      clientRef.current ?? buildClient(serverConfig, loadToken(db));
    clientRef.current = client;
    await client.createVault(serverConfig.vaultId);
  }, [db, serverConfig, buildClient]);

  const sync = useCallback(async (): Promise<SyncOutcome> => {
    if (!wasm || !vault || !db || !serverConfig)
      throw new Error('Sync is not configured');
    const client =
      clientRef.current ?? buildClient(serverConfig, loadToken(db));
    clientRef.current = client;
    setSyncing(true);
    try {
      const outcome = await runSync(db, wasm, vault, client, serverConfig.vaultId);
      refreshData(db);
      setConflicts(listOpenConflicts(db));
      await saveVault();
      return outcome;
    } finally {
      setSyncing(false);
    }
  }, [wasm, vault, db, serverConfig, buildClient, refreshData, saveVault]);

  const resolveSyncConflict = useCallback(
    async (id: string, keepRemote: boolean) => {
      if (!db) return;
      resolveConflict(db, id, keepRemote);
      refreshData(db);
      setConflicts(listOpenConflicts(db));
      await saveVault();
    },
    [db, refreshData, saveVault],
  );

  const getTransactionsForWasm = useCallback((): Transaction[] => {
    if (!db) return [];
    return data.transactions.map((txn) => {
      const txnPostings = data.postings
        .filter((p) => p.transaction_id === txn.id)
        .map((p) => {
          const acct = data.accounts.find((a) => a.id === p.account_id);
          return {
            account: acct?.name ?? p.account_id,
            amount: p.amount_quantity
              ? { quantity: p.amount_quantity, commodity: p.amount_commodity ?? 'USD' }
              : null,
            balance_assertion: null,
            comment: null,
            tags: {},
            source_line: 0,
          };
        });
      return {
        date: txn.date,
        status: txn.status === 'cleared' ? 'Cleared' : txn.status === 'pending' ? 'Pending' : 'Unmarked',
        code: txn.code,
        description: txn.description,
        comment: txn.comment,
        tags: {},
        postings: txnPostings,
        source_line: 0,
      } as Transaction;
    });
  }, [data, db]);

  const getBalanceReport = useCallback(
    (accountFilter?: string): BalanceReport | null => {
      if (!wasm) return null;
      const txns = getTransactionsForWasm();
      if (txns.length === 0) return null;
      try {
        const json = wasm.computeBalance(
          JSON.stringify(txns),
          accountFilter,
        );
        return JSON.parse(json);
      } catch {
        return null;
      }
    },
    [wasm, getTransactionsForWasm],
  );

  const getRegisterReport = useCallback(
    (accountFilter?: string, begin?: string, end?: string): RegisterReport | null => {
      if (!wasm) return null;
      const txns = getTransactionsForWasm();
      if (txns.length === 0) return null;
      try {
        const json = wasm.computeRegister(
          JSON.stringify(txns),
          accountFilter,
          begin,
          end,
        );
        return JSON.parse(json);
      } catch {
        return null;
      }
    },
    [wasm, getTransactionsForWasm],
  );

  return (
    <VaultContext.Provider
      value={{
        state,
        data,
        wasm,
        createVault,
        unlockVault,
        lockVault,
        refreshVaults,
        addAccount,
        addTransaction,
        deleteTransaction,
        saveVault,
        serverConfig,
        serverAuthenticated,
        syncing,
        deviceId,
        conflicts,
        configureServer,
        registerServer,
        loginServer,
        logoutServer,
        createRemoteVault,
        sync,
        resolveSyncConflict,
        getBalanceReport,
        getRegisterReport,
        getTransactionsForWasm,
      }}
    >
      {children}
    </VaultContext.Provider>
  );
}
