'use client';

import { useState } from 'react';
import { useRouter } from 'next/navigation';
import { useVault } from '@/contexts/VaultContext';
import Link from 'next/link';

export default function VaultCreateInner() {
  const { createVault } = useVault();
  const router = useRouter();
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const handleCreate = async () => {
    if (!name.trim()) {
      setError('Vault name is required');
      return;
    }
    if (password.length < 8) {
      setError('Password must be at least 8 characters');
      return;
    }
    if (password !== confirm) {
      setError('Passwords do not match');
      return;
    }

    setLoading(true);
    setError(null);
    try {
      const key = await createVault(password, name.trim());
      setRecoveryKey(key);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  if (recoveryKey) {
    return (
      <div className="flex min-h-screen items-center justify-center px-4">
        <div className="w-full max-w-md space-y-6">
          <h1 className="text-2xl font-bold text-center">Vault Created ✓</h1>
          <div className="rounded-lg border-2 border-[var(--color-warning)] bg-[var(--color-surface)] p-4">
            <p className="text-sm font-semibold text-[var(--color-warning)] mb-2">
              ⚠️ Save Your Recovery Key
            </p>
            <p className="text-xs text-[var(--color-text-secondary)] mb-3">
              This key is the only way to recover your vault if you forget your
              password. Store it somewhere safe — it will not be shown again.
            </p>
            <code className="block break-all rounded bg-[var(--color-bg)] p-3 text-sm font-mono border border-[var(--color-border)]">
              {recoveryKey}
            </code>
            <button
              onClick={() => navigator.clipboard.writeText(recoveryKey)}
              className="mt-3 text-sm text-[var(--color-accent)] hover:underline"
            >
              Copy to clipboard
            </button>
          </div>
          <button
            onClick={() => router.push('/vault')}
            className="w-full rounded-lg bg-[var(--color-accent)] px-6 py-3 font-semibold text-white hover:opacity-90 transition-opacity"
          >
            Continue to Vault
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center px-4">
      <div className="w-full max-w-md space-y-6">
        <div>
          <Link href="/" className="text-sm text-[var(--color-accent)] hover:underline">
            ← Back
          </Link>
          <h1 className="mt-2 text-2xl font-bold">Create Vault</h1>
          <p className="text-sm text-[var(--color-text-secondary)]">
            Your vault is encrypted with AES-256-GCM. Choose a strong password.
          </p>
        </div>

        {error && (
          <div className="rounded-lg bg-red-50 dark:bg-red-900/20 p-3 text-sm text-[var(--color-danger)]">
            {error}
          </div>
        )}

        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium mb-1">Vault Name</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="My Finances"
              className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-4 py-2.5 text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)] focus:border-[var(--color-accent)] focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
            />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">Password</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="At least 8 characters"
              className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-4 py-2.5 text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)] focus:border-[var(--color-accent)] focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
            />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">
              Confirm Password
            </label>
            <input
              type="password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              placeholder="Repeat your password"
              className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-4 py-2.5 text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)] focus:border-[var(--color-accent)] focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
            />
          </div>
        </div>

        <button
          onClick={handleCreate}
          disabled={loading}
          className="w-full rounded-lg bg-[var(--color-accent)] px-6 py-3 font-semibold text-white hover:opacity-90 transition-opacity disabled:opacity-50"
        >
          {loading ? 'Creating vault…' : 'Create Vault'}
        </button>
      </div>
    </div>
  );
}
