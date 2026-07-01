'use client';

import { useEffect, useState } from 'react';
import Link from 'next/link';
import { listVaultNames } from '@/lib/storage';

export default function LandingPage() {
  const [vaults, setVaults] = useState<string[]>([]);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    listVaultNames().then((names) => {
      setVaults(names);
      setReady(true);
    });
  }, []);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-4">
      <div className="w-full max-w-md space-y-8 text-center">
        {/* Logo */}
        <div>
          <h1 className="text-5xl font-bold tracking-tight">
            <span className="text-[var(--color-accent)]">ldgr</span>
          </h1>
          <p className="mt-3 text-lg text-[var(--color-text-secondary)]">
            Zero-knowledge personal finance.
            <br />
            Your data never leaves your device.
          </p>
        </div>

        {/* Features */}
        <div className="grid grid-cols-2 gap-3 text-sm text-[var(--color-text-secondary)]">
          <div className="rounded-lg border border-[var(--color-border)] p-3">
            🔒 AES-256-GCM encryption
          </div>
          <div className="rounded-lg border border-[var(--color-border)] p-3">
            📊 Double-entry bookkeeping
          </div>
          <div className="rounded-lg border border-[var(--color-border)] p-3">
            📴 Works offline
          </div>
          <div className="rounded-lg border border-[var(--color-border)] p-3">
            🌐 hledger compatible
          </div>
        </div>

        {/* Actions */}
        {ready && (
          <div className="space-y-3">
            {vaults.length > 0 ? (
              <>
                <Link
                  href="/unlock"
                  className="block w-full rounded-lg bg-[var(--color-accent)] px-6 py-3 text-center font-semibold text-white hover:opacity-90 transition-opacity"
                >
                  Unlock Vault
                </Link>
                <Link
                  href="/create"
                  className="block w-full rounded-lg border border-[var(--color-border)] px-6 py-3 text-center font-semibold text-[var(--color-text)] hover:bg-[var(--color-surface)] transition-colors"
                >
                  Create New Vault
                </Link>
              </>
            ) : (
              <Link
                href="/create"
                className="block w-full rounded-lg bg-[var(--color-accent)] px-6 py-3 text-center font-semibold text-white hover:opacity-90 transition-opacity"
              >
                Create Your First Vault
              </Link>
            )}
          </div>
        )}

        {!ready && (
          <div className="py-4 text-[var(--color-text-secondary)]">
            Loading…
          </div>
        )}

        <p className="text-xs text-[var(--color-text-secondary)]">
          All encryption happens in your browser. No data is ever sent to a
          server.
        </p>

        <p className="text-xs text-[var(--color-text-secondary)]">
          <Link
            href="/admin"
            className="underline hover:text-[var(--color-text)]"
          >
            Server admin panel
          </Link>
        </p>
      </div>
    </div>
  );
}
