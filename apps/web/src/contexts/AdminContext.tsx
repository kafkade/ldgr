'use client';

/**
 * Admin panel auth + client context.
 *
 * Sign-in reuses the shared WASM SRP client (`WasmSyncClient.login`) over the
 * existing `makeFetchCallback` transport, so the password never leaves the
 * browser — only the resulting bearer session token is kept. That token drives
 * the typed {@link AdminClient} the panel screens call.
 *
 * After a successful SRP login we probe `GET /admin/stats`; a 401/403 means the
 * account is not an active admin, so sign-in is rejected (this is what keeps
 * non-admins out of the admin screens, #179).
 *
 * The token lives in memory and `sessionStorage` only — never the vault, never
 * `localStorage`. It is cleared on sign-out and when the server rejects it.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react';
import { AdminApiError, AdminClient } from '@/lib/admin';
import { loadWasm } from '@/lib/wasm';
import { makeFetchCallback } from '@/lib/sync';

const STORAGE_KEY = 'ldgr:admin:session';

interface AdminSession {
  serverUrl: string;
  username: string;
  token: string;
}

export interface AdminContextValue {
  ready: boolean;
  session: AdminSession | null;
  client: AdminClient | null;
  signIn: (
    serverUrl: string,
    username: string,
    password: string,
  ) => Promise<void>;
  signOut: () => void;
  /** Drop the session in response to a server 401/403 during normal use. */
  handleAuthError: (err: unknown) => boolean;
}

const AdminContext = createContext<AdminContextValue | null>(null);

export function useAdmin(): AdminContextValue {
  const ctx = useContext(AdminContext);
  if (!ctx) throw new Error('useAdmin must be used within AdminProvider');
  return ctx;
}

function loadStoredSession(): AdminSession | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<AdminSession>;
    if (parsed.serverUrl && parsed.username && parsed.token) {
      return parsed as AdminSession;
    }
  } catch {
    // Corrupt entry — ignore and start fresh.
  }
  return null;
}

function storeSession(session: AdminSession | null): void {
  if (typeof window === 'undefined') return;
  if (session) {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(session));
  } else {
    window.sessionStorage.removeItem(STORAGE_KEY);
  }
}

export function AdminProvider({ children }: { children: React.ReactNode }) {
  const [ready, setReady] = useState(false);
  const [session, setSession] = useState<AdminSession | null>(null);

  // Restore and validate any persisted session on mount.
  useEffect(() => {
    let cancelled = false;
    const stored = loadStoredSession();
    if (!stored) {
      setReady(true);
      return;
    }
    const client = new AdminClient(stored.serverUrl, stored.token);
    client
      .getStats()
      .then(() => {
        if (!cancelled) setSession(stored);
      })
      .catch(() => {
        // Expired/invalid token, or unreachable server — require a fresh sign-in.
        storeSession(null);
      })
      .finally(() => {
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const signIn = useCallback(
    async (serverUrl: string, username: string, password: string) => {
      const url = serverUrl.trim();
      const user = username.trim();
      if (!url) throw new Error('Enter the server URL.');
      if (!user) throw new Error('Enter your username.');
      if (!password) throw new Error('Enter your password.');

      const wasm = await loadWasm();
      const client = new wasm.WasmSyncClient(makeFetchCallback(url));
      await client.login(user, password);
      const token = client.token;
      client.free();
      if (!token) throw new Error('Sign-in failed: no session token returned.');

      // Confirm the account is actually an active admin before granting access.
      try {
        await new AdminClient(url, token).getStats();
      } catch (err) {
        if (err instanceof AdminApiError && err.isAuthError) {
          throw new Error(
            'This account is not an administrator on this server.',
          );
        }
        throw err;
      }

      const next: AdminSession = { serverUrl: url, username: user, token };
      storeSession(next);
      setSession(next);
    },
    [],
  );

  const signOut = useCallback(() => {
    storeSession(null);
    setSession(null);
  }, []);

  const handleAuthError = useCallback(
    (err: unknown): boolean => {
      if (err instanceof AdminApiError && err.isAuthError) {
        storeSession(null);
        setSession(null);
        return true;
      }
      return false;
    },
    [],
  );

  const client = useMemo(
    () => (session ? new AdminClient(session.serverUrl, session.token) : null),
    [session],
  );

  const value = useMemo<AdminContextValue>(
    () => ({ ready, session, client, signIn, signOut, handleAuthError }),
    [ready, session, client, signIn, signOut, handleAuthError],
  );

  return (
    <AdminContext.Provider value={value}>{children}</AdminContext.Provider>
  );
}
