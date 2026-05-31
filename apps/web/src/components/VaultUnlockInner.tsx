'use client';

import { useState, useEffect } from 'react';
import { useRouter } from 'next/navigation';
import { useVault } from '@/contexts/VaultContext';
import Link from 'next/link';

export default function VaultUnlockInner() {
  const { state, unlockVault, refreshVaults } = useVault();
  const router = useRouter();
  const [selectedVault, setSelectedVault] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    refreshVaults();
  }, [refreshVaults]);

  useEffect(() => {
    if (state.existingVaults.length > 0 && !selectedVault) {
      setSelectedVault(state.existingVaults[0]);
    }
  }, [state.existingVaults, selectedVault]);

  const handleUnlock = async () => {
    if (!selectedVault) {
      setError('Select a vault');
      return;
    }
    if (!password) {
      setError('Enter your password');
      return;
    }

    setLoading(true);
    setError(null);
    try {
      await unlockVault(selectedVault, password);
      router.push('/vault');
    } catch (err) {
      setError('Incorrect password or corrupted vault');
    } finally {
      setLoading(false);
    }
  };

  if (state.existingVaults.length === 0) {
    return (
      <div className="flex min-h-screen items-center justify-center px-4">
        <div className="text-center space-y-4">
          <p className="text-[var(--color-text-secondary)]">No vaults found.</p>
          <Link
            href="/create"
            className="inline-block rounded-lg bg-[var(--color-accent)] px-6 py-3 font-semibold text-white hover:opacity-90"
          >
            Create a Vault
          </Link>
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
          <h1 className="mt-2 text-2xl font-bold">Unlock Vault</h1>
          <p className="text-sm text-[var(--color-text-secondary)]">
            Enter your password to access your financial data.
          </p>
        </div>

        {error && (
          <div className="rounded-lg bg-red-50 dark:bg-red-900/20 p-3 text-sm text-[var(--color-danger)]">
            {error}
          </div>
        )}

        <div className="space-y-4">
          {state.existingVaults.length > 1 && (
            <div>
              <label className="block text-sm font-medium mb-1">Vault</label>
              <select
                value={selectedVault}
                onChange={(e) => setSelectedVault(e.target.value)}
                className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-4 py-2.5 text-[var(--color-text)] focus:border-[var(--color-accent)] focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
              >
                {state.existingVaults.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            </div>
          )}

          {state.existingVaults.length === 1 && (
            <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-3">
              <span className="text-sm text-[var(--color-text-secondary)]">Vault: </span>
              <span className="font-medium">{state.existingVaults[0]}</span>
            </div>
          )}

          <div>
            <label className="block text-sm font-medium mb-1">Password</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleUnlock()}
              placeholder="Enter vault password"
              autoFocus
              className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-4 py-2.5 text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)] focus:border-[var(--color-accent)] focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
            />
          </div>
        </div>

        <button
          onClick={handleUnlock}
          disabled={loading}
          className="w-full rounded-lg bg-[var(--color-accent)] px-6 py-3 font-semibold text-white hover:opacity-90 transition-opacity disabled:opacity-50"
        >
          {loading ? 'Unlocking…' : 'Unlock'}
        </button>
      </div>
    </div>
  );
}
