'use client';

import dynamic from 'next/dynamic';

const AdminGate = dynamic(
  () => import('@/components/admin/AdminGate').then((m) => m.AdminGate),
  { ssr: false },
);

export default function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return <AdminGate>{children}</AdminGate>;
}
