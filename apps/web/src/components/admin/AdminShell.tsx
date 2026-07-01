'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useAdmin } from '@/contexts/AdminContext';
import { btnSecondary } from './ui';

const NAV = [
  { href: '/admin/users', label: 'Users' },
  { href: '/admin/invites', label: 'Invites' },
  { href: '/admin/settings', label: 'Settings' },
  { href: '/admin/storage', label: 'Storage' },
  { href: '/admin/server', label: 'Server' },
];

/** Authenticated admin chrome: sidebar nav + header, wrapping the active page. */
export function AdminShell({ children }: { children: React.ReactNode }) {
  const { session, signOut } = useAdmin();
  const pathname = usePathname();

  return (
    <div className="min-h-screen">
      <header className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
        <div className="flex items-center gap-3">
          <Link href="/admin" className="text-lg font-bold no-underline">
            <span className="text-[var(--color-accent)]">ldgr</span> admin
          </Link>
          {session && (
            <span className="hidden text-xs text-[var(--color-text-secondary)] sm:inline">
              {session.username} @ {session.serverUrl}
            </span>
          )}
        </div>
        <button type="button" className={btnSecondary} onClick={signOut}>
          Sign out
        </button>
      </header>

      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 md:flex-row">
        <nav className="flex shrink-0 gap-1 overflow-x-auto md:w-44 md:flex-col">
          {NAV.map((item) => {
            const active =
              pathname === item.href || pathname.startsWith(`${item.href}/`);
            return (
              <Link
                key={item.href}
                href={item.href}
                className={`rounded-lg px-3 py-2 text-sm no-underline transition-colors ${
                  active
                    ? 'bg-[var(--color-accent-light)] font-medium text-[var(--color-accent)]'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-surface)]'
                }`}
              >
                {item.label}
              </Link>
            );
          })}
        </nav>

        <main className="min-w-0 flex-1">{children}</main>
      </div>
    </div>
  );
}
