'use client';

import { useState } from 'react';
import { useAdmin } from '@/contexts/AdminContext';
import { fetchServerInfo, type ServerInfo } from '@/lib/admin';
import {
  btnPrimary,
  btnSecondary,
  errorMessage,
  inputClass,
  labelClass,
} from './ui';

/**
 * Admin sign-in screen. Collects the server URL and admin credentials, runs the
 * SRP handshake via the shared WASM client (through {@link useAdmin}), and — on
 * success — the provider swaps in the authenticated shell.
 */
export function AdminSignIn() {
  const { signIn } = useAdmin();
  const [serverUrl, setServerUrl] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [probing, setProbing] = useState(false);

  const probe = async () => {
    const url = serverUrl.trim();
    if (!url) return;
    setProbing(true);
    setInfo(null);
    setError(null);
    try {
      setInfo(await fetchServerInfo(url));
    } catch {
      // Non-fatal: the sign-in attempt will surface a clearer error.
    } finally {
      setProbing(false);
    }
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await signIn(serverUrl, username, password);
      setPassword('');
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center px-4">
      <form onSubmit={submit} className="w-full max-w-sm space-y-5">
        <div className="text-center">
          <h1 className="text-3xl font-bold tracking-tight">
            <span className="text-[var(--color-accent)]">ldgr</span> admin
          </h1>
          <p className="mt-2 text-sm text-[var(--color-text-secondary)]">
            Sign in to manage users and server settings.
          </p>
        </div>

        <div>
          <label className={labelClass} htmlFor="admin-server-url">
            Server URL
          </label>
          <input
            id="admin-server-url"
            className={inputClass}
            type="url"
            inputMode="url"
            placeholder="https://sync.example.com"
            value={serverUrl}
            onChange={(e) => setServerUrl(e.target.value)}
            onBlur={probe}
            autoComplete="off"
            required
          />
          {probing && (
            <p className="mt-1 text-xs text-[var(--color-text-secondary)]">
              Checking server…
            </p>
          )}
          {info && (
            <p className="mt-1 text-xs text-[var(--color-text-secondary)]">
              {info.name} · v{info.version} · policy: {info.registration_policy}
            </p>
          )}
        </div>

        <div>
          <label className={labelClass} htmlFor="admin-username">
            Username
          </label>
          <input
            id="admin-username"
            className={inputClass}
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="username"
            required
          />
        </div>

        <div>
          <label className={labelClass} htmlFor="admin-password">
            Password
          </label>
          <input
            id="admin-password"
            className={inputClass}
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="current-password"
            required
          />
        </div>

        {error && (
          <p className="text-sm text-[var(--color-danger)]" role="alert">
            {error}
          </p>
        )}

        <button type="submit" className={`${btnPrimary} w-full`} disabled={busy}>
          {busy ? 'Signing in…' : 'Sign in'}
        </button>

        <a
          href="/"
          className={`${btnSecondary} block w-full text-center no-underline`}
        >
          Back to app
        </a>

        <p className="text-center text-xs text-[var(--color-text-secondary)]">
          Your password is processed locally; only a proof is sent to the server.
        </p>
      </form>
    </div>
  );
}
