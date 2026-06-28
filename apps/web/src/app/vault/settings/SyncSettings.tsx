'use client';

import { useEffect, useState } from 'react';
import { useVault } from '@/contexts/VaultContext';

const cardClass =
  'rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5 space-y-3';
const inputClass =
  'w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm';
const btnPrimary =
  'rounded-lg border border-[var(--color-accent)] bg-[var(--color-accent-light)] px-3 py-2 text-sm font-medium text-[var(--color-accent)] disabled:opacity-50 transition-colors';
const btnSecondary =
  'rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm hover:bg-[var(--color-bg)] disabled:opacity-50 transition-colors';

export default function SyncSettings() {
  const {
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
  } = useVault();

  const [serverUrl, setServerUrl] = useState('');
  const [vaultId, setVaultId] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (serverConfig) {
      setServerUrl(serverConfig.serverUrl);
      setVaultId(serverConfig.vaultId);
      setUsername(serverConfig.username);
    }
  }, [serverConfig]);

  const run = async (label: string, fn: () => Promise<void>) => {
    setBusy(label);
    setError(null);
    setMessage(null);
    try {
      await fn();
    } catch (err) {
      setError(String(err instanceof Error ? err.message : err));
    } finally {
      setBusy(null);
    }
  };

  const handleSave = () =>
    run('save', async () => {
      await configureServer({ serverUrl, username, vaultId });
      setMessage('Server settings saved.');
    });

  const handleRegister = () =>
    run('register', async () => {
      await configureServer({ serverUrl, username, vaultId });
      await registerServer(username, password);
      setPassword('');
      setMessage('Account registered. You can now log in.');
    });

  const handleLogin = () =>
    run('login', async () => {
      await configureServer({ serverUrl, username, vaultId });
      await loginServer(username, password);
      setPassword('');
      setMessage('Logged in.');
    });

  const handleCreateVault = () =>
    run('create-vault', async () => {
      await createRemoteVault();
      setMessage('Remote vault ready.');
    });

  const handleSync = () =>
    run('sync', async () => {
      const outcome = await sync();
      setMessage(
        `Synced: pushed ${outcome.pushed}, applied ${outcome.applied}, ` +
          `conflicts ${outcome.conflicts}, skipped ${outcome.skipped}.`,
      );
    });

  const handleLogout = () =>
    run('logout', async () => {
      await logoutServer();
      setMessage('Logged out.');
    });

  return (
    <div className={cardClass}>
      <h2 className="text-sm font-semibold text-[var(--color-text-secondary)]">
        Sync (ldgr-server)
      </h2>
      <p className="text-xs text-[var(--color-text-secondary)]">
        Optional. Sync only ever transmits encrypted blobs — the server never
        sees your financial data or your password (SRP-6a). Your password is
        never stored.
      </p>

      <div className="space-y-2">
        <label className="block text-xs text-[var(--color-text-secondary)]">
          Server URL
          <input
            className={inputClass}
            type="url"
            placeholder="https://sync.example.com"
            value={serverUrl}
            onChange={(e) => setServerUrl(e.target.value)}
          />
        </label>
        <label className="block text-xs text-[var(--color-text-secondary)]">
          Vault ID
          <input
            className={inputClass}
            type="text"
            placeholder="my-vault"
            value={vaultId}
            onChange={(e) => setVaultId(e.target.value)}
          />
        </label>
        <label className="block text-xs text-[var(--color-text-secondary)]">
          Username
          <input
            className={inputClass}
            type="text"
            autoComplete="username"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
          />
        </label>
        <label className="block text-xs text-[var(--color-text-secondary)]">
          Password{' '}
          <span className="opacity-60">(used only to authenticate)</span>
          <input
            className={inputClass}
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
      </div>

      <div className="flex flex-wrap gap-2">
        <button
          className={btnSecondary}
          onClick={handleSave}
          disabled={busy !== null || !serverUrl || !vaultId}
        >
          Save
        </button>
        <button
          className={btnSecondary}
          onClick={handleRegister}
          disabled={busy !== null || !serverUrl || !username || !password}
        >
          {busy === 'register' ? 'Registering…' : 'Register'}
        </button>
        <button
          className={btnPrimary}
          onClick={handleLogin}
          disabled={busy !== null || !serverUrl || !username || !password}
        >
          {busy === 'login' ? 'Logging in…' : 'Log in'}
        </button>
      </div>

      <div className="flex items-center justify-between pt-1">
        <span className="text-xs text-[var(--color-text-secondary)]">
          {serverAuthenticated ? '🟢 Authenticated' : '⚪ Not authenticated'}
          {deviceId ? ` · device ${deviceId.slice(0, 8)}` : ''}
        </span>
        {serverAuthenticated && (
          <button
            className={btnSecondary}
            onClick={handleLogout}
            disabled={busy !== null}
          >
            Log out
          </button>
        )}
      </div>

      {serverAuthenticated && (
        <div className="flex flex-wrap gap-2 border-t border-[var(--color-border)] pt-3">
          <button
            className={btnSecondary}
            onClick={handleCreateVault}
            disabled={busy !== null || !vaultId}
          >
            {busy === 'create-vault' ? 'Creating…' : 'Create remote vault'}
          </button>
          <button
            className={btnPrimary}
            onClick={handleSync}
            disabled={busy !== null || syncing || !vaultId}
          >
            {syncing || busy === 'sync' ? 'Syncing…' : '🔄 Sync now'}
          </button>
        </div>
      )}

      {message && (
        <p className="text-xs text-[var(--color-accent)]">{message}</p>
      )}
      {error && (
        <p className="text-xs text-[var(--color-danger)] break-words">{error}</p>
      )}

      {conflicts.length > 0 && (
        <div className="space-y-2 border-t border-[var(--color-border)] pt-3">
          <h3 className="text-xs font-semibold text-[var(--color-danger)]">
            Conflicts to review ({conflicts.length})
          </h3>
          {conflicts.map((c) => (
            <div
              key={c.id}
              className="rounded-lg border border-[var(--color-border)] p-3 space-y-2"
            >
              <div className="text-xs">
                <span className="font-medium capitalize">{c.entityType}</span>{' '}
                <span className="opacity-60">{c.entityId.slice(0, 8)}</span>
              </div>
              <div className="grid grid-cols-2 gap-2 text-xs">
                <div>
                  <div className="opacity-60">This device</div>
                  <div className="break-words">{c.localSummary}</div>
                </div>
                <div>
                  <div className="opacity-60">Remote</div>
                  <div className="break-words">{c.remoteSummary}</div>
                </div>
              </div>
              <div className="flex gap-2">
                <button
                  className={btnSecondary}
                  onClick={() =>
                    run('resolve', () => resolveSyncConflict(c.id, false))
                  }
                  disabled={busy !== null}
                >
                  Keep mine
                </button>
                <button
                  className={btnSecondary}
                  onClick={() =>
                    run('resolve', () => resolveSyncConflict(c.id, true))
                  }
                  disabled={busy !== null}
                >
                  Keep remote
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
