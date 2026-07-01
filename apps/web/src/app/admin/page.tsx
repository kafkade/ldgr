'use client';

import { useEffect } from 'react';
import { useRouter } from 'next/navigation';

/** `/admin` redirects to the Users screen (the default admin landing). */
export default function AdminIndexPage() {
  const router = useRouter();
  useEffect(() => {
    router.replace('/admin/users');
  }, [router]);
  return (
    <p className="text-sm text-[var(--color-text-secondary)]">Loading…</p>
  );
}
