'use client';

import { useEffect } from 'react';
import { useRouter, usePathname } from 'next/navigation';
import Link from 'next/link';
import { VaultProvider, useVault } from '@/contexts/VaultContext';
import { ThemeProvider, useTheme } from '@/contexts/ThemeContext';

const NAV_ITEMS = [
  { href: '/vault', label: 'Dashboard', icon: '📊' },
  { href: '/vault/transactions', label: 'Transactions', icon: '📋' },
  { href: '/vault/accounts', label: 'Accounts', icon: '🏦' },
  { href: '/vault/investments', label: 'Investments', icon: '📈' },
  { href: '/vault/budget', label: 'Budget', icon: '💰' },
  { href: '/vault/settings', label: 'Settings', icon: '⚙️' },
];

function ThemeToggle() {
  const { theme, toggleTheme } = useTheme();
  return (
    <button
      onClick={toggleTheme}
      className="rounded-lg p-2 hover:bg-[var(--color-surface)] transition-colors"
      title={`Switch to ${theme === 'light' ? 'dark' : 'light'} mode`}
    >
      {theme === 'light' ? '🌙' : '☀️'}
    </button>
  );
}

function VaultGuard({ children }: { children: React.ReactNode }) {
  const { state, lockVault, saveVault } = useVault();
  const router = useRouter();
  const pathname = usePathname();

  useEffect(() => {
    if (state.status === 'locked') {
      router.replace('/unlock');
    }
  }, [state.status, router]);

  if (state.status === 'loading') {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <p className="text-[var(--color-text-secondary)]">Loading WASM…</p>
      </div>
    );
  }

  if (state.status === 'error') {
    return (
      <div className="flex min-h-screen items-center justify-center px-4">
        <div className="text-center space-y-4 max-w-md">
          <p className="text-[var(--color-danger)] font-semibold">Failed to load</p>
          <p className="text-sm text-[var(--color-text-secondary)]">{state.error}</p>
          <p className="text-xs text-[var(--color-text-secondary)]">
            Make sure to build the WASM module first: <code>npm run build:wasm</code>
          </p>
        </div>
      </div>
    );
  }

  if (state.status !== 'unlocked') return null;

  return (
    <div className="flex min-h-screen">
      {/* Sidebar */}
      <aside className="hidden md:flex md:w-60 flex-col border-r border-[var(--color-border)] bg-[var(--color-surface)]">
        <div className="flex items-center gap-2 px-4 py-4 border-b border-[var(--color-border)]">
          <span className="text-xl font-bold text-[var(--color-accent)]">ldgr</span>
          <span className="text-xs text-[var(--color-text-secondary)] truncate">
            {state.vaultName}
          </span>
        </div>

        <nav className="flex-1 py-2">
          {NAV_ITEMS.map((item) => {
            const active = pathname === item.href;
            return (
              <Link
                key={item.href}
                href={item.href}
                className={`flex items-center gap-3 px-4 py-2.5 text-sm transition-colors ${
                  active
                    ? 'bg-[var(--color-accent-light)] text-[var(--color-accent)] font-medium'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg)] hover:text-[var(--color-text)]'
                }`}
              >
                <span>{item.icon}</span>
                {item.label}
              </Link>
            );
          })}
        </nav>

        <div className="border-t border-[var(--color-border)] p-3 space-y-2">
          <button
            onClick={async () => {
              await saveVault();
              lockVault();
              router.push('/');
            }}
            className="w-full rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-text-secondary)] hover:bg-[var(--color-bg)] transition-colors"
          >
            🔒 Lock Vault
          </button>
          <div className="flex justify-center">
            <ThemeToggle />
          </div>
        </div>
      </aside>

      {/* Mobile header */}
      <div className="md:hidden fixed top-0 left-0 right-0 z-10 border-b border-[var(--color-border)] bg-[var(--color-bg)]">
        <div className="flex items-center justify-between px-4 py-3">
          <span className="font-bold text-[var(--color-accent)]">ldgr</span>
          <div className="flex items-center gap-2">
            <ThemeToggle />
            <button
              onClick={async () => {
                await saveVault();
                lockVault();
                router.push('/');
              }}
              className="text-sm text-[var(--color-text-secondary)]"
            >
              🔒
            </button>
          </div>
        </div>
        {/* Mobile nav */}
        <nav className="flex overflow-x-auto border-t border-[var(--color-border)] px-2">
          {NAV_ITEMS.filter((i) => i.label !== 'Settings').map((item) => {
            const active = pathname === item.href;
            return (
              <Link
                key={item.href}
                href={item.href}
                className={`flex-shrink-0 px-3 py-2.5 text-xs transition-colors ${
                  active
                    ? 'text-[var(--color-accent)] border-b-2 border-[var(--color-accent)] font-medium'
                    : 'text-[var(--color-text-secondary)]'
                }`}
              >
                {item.icon} {item.label}
              </Link>
            );
          })}
        </nav>
      </div>

      {/* Main content */}
      <main className="flex-1 overflow-y-auto md:p-6 p-4 pt-28 md:pt-6">
        {children}
      </main>
    </div>
  );
}

export default function VaultShell({ children }: { children: React.ReactNode }) {
  return (
    <ThemeProvider>
      <VaultProvider>
        <VaultGuard>{children}</VaultGuard>
      </VaultProvider>
    </ThemeProvider>
  );
}
