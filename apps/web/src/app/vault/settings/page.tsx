'use client';

import { useVault } from '@/contexts/VaultContext';
import { useTheme } from '@/contexts/ThemeContext';
import { useRouter } from 'next/navigation';

export default function SettingsPage() {
  const { state, saveVault, lockVault } = useVault();
  const { theme, toggleTheme } = useTheme();
  const router = useRouter();

  const handleLock = async () => {
    await saveVault();
    lockVault();
    router.push('/');
  };

  return (
    <div className="space-y-6 max-w-lg">
      <h1 className="text-2xl font-bold">Settings</h1>

      {/* Vault Info */}
      <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5 space-y-3">
        <h2 className="text-sm font-semibold text-[var(--color-text-secondary)]">
          Vault
        </h2>
        <div className="flex items-center justify-between">
          <span className="text-sm">Name</span>
          <span className="text-sm font-medium">{state.vaultName}</span>
        </div>
        <div className="flex items-center justify-between">
          <span className="text-sm">Status</span>
          <span className="text-sm font-medium text-[var(--color-accent)]">
            🔓 Unlocked
          </span>
        </div>
      </div>

      {/* Theme */}
      <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5 space-y-3">
        <h2 className="text-sm font-semibold text-[var(--color-text-secondary)]">
          Appearance
        </h2>
        <div className="flex items-center justify-between">
          <span className="text-sm">Theme</span>
          <button
            onClick={toggleTheme}
            className="rounded-lg border border-[var(--color-border)] px-3 py-1.5 text-sm hover:bg-[var(--color-bg)] transition-colors"
          >
            {theme === 'light' ? '🌙 Dark' : '☀️ Light'}
          </button>
        </div>
      </div>

      {/* Security */}
      <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5 space-y-3">
        <h2 className="text-sm font-semibold text-[var(--color-text-secondary)]">
          Security
        </h2>
        <p className="text-xs text-[var(--color-text-secondary)]">
          All data is encrypted with AES-256-GCM using keys derived from your
          password via Argon2id. No data is ever sent to a server.
        </p>
        <button
          onClick={handleLock}
          className="w-full rounded-lg border border-[var(--color-danger)] px-4 py-2.5 text-sm font-semibold text-[var(--color-danger)] hover:bg-red-50 dark:hover:bg-red-900/20 transition-colors"
        >
          🔒 Lock Vault
        </button>
      </div>

      {/* About */}
      <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5 space-y-2">
        <h2 className="text-sm font-semibold text-[var(--color-text-secondary)]">
          About
        </h2>
        <p className="text-xs text-[var(--color-text-secondary)]">
          ldgr — Zero-knowledge personal finance. Built with Rust (WASM),
          Next.js, and sql.js. Apache-2.0 license.
        </p>
      </div>
    </div>
  );
}
