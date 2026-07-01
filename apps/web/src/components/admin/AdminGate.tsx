'use client';

import { AdminProvider, useAdmin } from '@/contexts/AdminContext';
import { AdminShell } from './AdminShell';
import { AdminSignIn } from './AdminSignIn';

function Gate({ children }: { children: React.ReactNode }) {
  const { ready, session } = useAdmin();

  if (!ready) {
    return (
      <div className="flex min-h-screen items-center justify-center text-[var(--color-text-secondary)]">
        Loading…
      </div>
    );
  }

  if (!session) return <AdminSignIn />;

  return <AdminShell>{children}</AdminShell>;
}

/**
 * Top-level admin boundary: provides auth state and renders either the sign-in
 * screen or the authenticated shell. Loaded client-only (needs WASM +
 * sessionStorage), mirroring the `/vault` shell.
 */
export function AdminGate({ children }: { children: React.ReactNode }) {
  return (
    <AdminProvider>
      <Gate>{children}</Gate>
    </AdminProvider>
  );
}
