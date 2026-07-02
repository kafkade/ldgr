'use client';

import { useEffect, useState } from 'react';
import { useVault } from '@/contexts/VaultContext';
import EmergencyKitView from '@/components/EmergencyKitView';
import type { EmergencyKit, ServerInfo } from '@/lib/wasm';

const cardClass =
  'rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5 space-y-3';
const inputClass =
  'w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm';
const btnPrimary =
  'rounded-lg border border-[var(--color-accent)] bg-[var(--color-accent-light)] px-3 py-2 text-sm font-medium text-[var(--color-accent)] disabled:opacity-50 transition-colors';
const btnSecondary =
  'rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm hover:bg-[var(--color-bg)] disabled:opacity-50 transition-colors';

type AuthMode = 'signin' | 'signup';

export default function SyncSettings() {
  const {
    serverConfig,
    serverAuthenticated,
    hasSecretKey,
    syncing,
    deviceId,
    conflicts,
    configureServer,
    checkServer,
    registerServer,
    loginServer,
    signUpServer,
    signInServer,
    logoutServer,
    createRemoteVault,
    sync,
    resolveSyncConflict,
  } = useVault();

  const [serverUrl, setServerUrl] = useState('');
  const [vaultId, setVaultId] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [secretKey, setSecretKey] = useState('');
  const [serverInfo, setServerInfo] = useState<ServerInfo | null>(null);
  const [mode, setMode] = useState<AuthMode>('signin');
  const [kit, setKit] = useState<EmergencyKit | null>(null);
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

  const handleCheck = () =>
    run('check', async () => {
      const info = await checkServer(serverUrl);
      setServerInfo(info);
      setMode(info.two_secret_auth && !hasSecretKey ? 'signup' : 'signin');
      setMessage(`Connected to ${info.name} (v${info.version}).`);
    });

  // ── Two-secret handlers ──
  const handleSignUp = () =>
    run('signup', async () => {
      await configureServer({ serverUrl, username, vaultId });
      const emergencyKit = await signUpServer(username, password);
      setPassword('');
      setKit(emergencyKit);
      setMessage(null);
    });

  const handleSignIn = () =>
    run('signin', async () => {
      await configureServer({ serverUrl, username, vaultId });
      await signInServer(username, password, secretKey.trim() || null);
      setPassword('');
      setSecretKey('');
      setMessage('Signed in.');
    });

  // ── Single-secret (legacy) handlers ──
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
      setServerInfo(null);
      setMessage('Logged out.');
    });

  const twoSecret = serverInfo?.two_secret_auth ?? false;
  const registrationClosed = serverInfo?.registration_policy === 'admin-only';

  return (
    <div className={cardClass}>
      <h2 className="text-sm font-semibold text-[var(--color-text-secondary)]">
        Sync (ldgr-server)
      </h2>
      <p className="text-xs text-[var(--color-text-secondary)]">
        Optional. Sync only ever transmits encrypted blobs — the server never
        sees your financial data or your password. Your password is never
        stored.
      </p>

      {/* Emergency Kit takes over the panel once generated at sign-up. */}
      {kit ? (
        <EmergencyKitView
          kit={kit}
          onDone={() => {
            setKit(null);
            setMessage('Signed in. Emergency Kit saved.');
          }}
        />
      ) : (
        <>
          {/* Server + vault */}
          <div className="space-y-2">
            <label className="block text-xs text-[var(--color-text-secondary)]">
              Server URL
              <input
                className={inputClass}
                type="url"
                placeholder="https://sync.example.com"
                value={serverUrl}
                onChange={(e) => {
                  setServerUrl(e.target.value);
                  setServerInfo(null);
                }}
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
          </div>

          {!serverInfo && (
            <button
              className={btnPrimary}
              onClick={handleCheck}
              disabled={busy !== null || !serverUrl}
            >
              {busy === 'check' ? 'Connecting…' : 'Connect'}
            </button>
          )}

          {serverInfo && (
            <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-xs space-y-0.5">
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">
                  Server
                </span>
                <span className="font-medium">
                  {serverInfo.name} · v{serverInfo.version}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">
                  Authentication
                </span>
                <span className="font-medium">
                  {twoSecret ? 'Two-secret (Secret Key)' : 'Single-secret'}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">
                  Registration
                </span>
                <span className="font-medium">
                  {serverInfo.registration_policy}
                </span>
              </div>
            </div>
          )}

          {/* Auth forms, shown after a successful Connect and while signed out */}
          {serverInfo && !serverAuthenticated && (
            <>
              {twoSecret && (
                <div className="flex gap-1 pt-1">
                  <button
                    className={mode === 'signup' ? btnPrimary : btnSecondary}
                    onClick={() => setMode('signup')}
                    disabled={busy !== null || registrationClosed}
                  >
                    Sign up
                  </button>
                  <button
                    className={mode === 'signin' ? btnPrimary : btnSecondary}
                    onClick={() => setMode('signin')}
                    disabled={busy !== null}
                  >
                    Sign in
                  </button>
                </div>
              )}

              <div className="space-y-2">
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
                  {twoSecret ? 'Master password' : 'Password'}{' '}
                  <span className="opacity-60">(used only to authenticate)</span>
                  <input
                    className={inputClass}
                    type="password"
                    autoComplete={
                      mode === 'signup' ? 'new-password' : 'current-password'
                    }
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                  />
                </label>

                {/* New-device sign-in needs the Secret Key. */}
                {twoSecret && mode === 'signin' && !hasSecretKey && (
                  <label className="block text-xs text-[var(--color-text-secondary)]">
                    Secret Key{' '}
                    <span className="opacity-60">(from your Emergency Kit)</span>
                    <input
                      className={`${inputClass} font-mono`}
                      type="text"
                      placeholder="A1-XXXXXX-XXXXXX-…"
                      value={secretKey}
                      onChange={(e) => setSecretKey(e.target.value)}
                    />
                  </label>
                )}
                {twoSecret && mode === 'signin' && hasSecretKey && (
                  <p className="text-xs text-[var(--color-text-secondary)]">
                    Using the Secret Key stored on this device.
                  </p>
                )}
              </div>

              <div className="flex flex-wrap gap-2">
                {twoSecret ? (
                  mode === 'signup' ? (
                    <button
                      className={btnPrimary}
                      onClick={handleSignUp}
                      disabled={
                        busy !== null ||
                        !username ||
                        !password ||
                        registrationClosed
                      }
                    >
                      {busy === 'signup'
                        ? 'Creating account…'
                        : 'Create account'}
                    </button>
                  ) : (
                    <button
                      className={btnPrimary}
                      onClick={handleSignIn}
                      disabled={
                        busy !== null ||
                        !username ||
                        !password ||
                        (!hasSecretKey && !secretKey.trim())
                      }
                    >
                      {busy === 'signin' ? 'Signing in…' : 'Sign in'}
                    </button>
                  )
                ) : (
                  <>
                    <button
                      className={btnSecondary}
                      onClick={handleRegister}
                      disabled={busy !== null || !username || !password}
                    >
                      {busy === 'register' ? 'Registering…' : 'Register'}
                    </button>
                    <button
                      className={btnPrimary}
                      onClick={handleLogin}
                      disabled={busy !== null || !username || !password}
                    >
                      {busy === 'login' ? 'Logging in…' : 'Log in'}
                    </button>
                  </>
                )}
              </div>

              {registrationClosed && mode === 'signup' && (
                <p className="text-xs text-[var(--color-text-secondary)]">
                  This server does not allow self-registration. Ask an
                  administrator for an account, then sign in with your Secret
                  Key.
                </p>
              )}
            </>
          )}

          <div className="flex items-center justify-between pt-1">
            <span className="text-xs text-[var(--color-text-secondary)]">
              {serverAuthenticated
                ? '🟢 Authenticated'
                : '⚪ Not authenticated'}
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
        </>
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
