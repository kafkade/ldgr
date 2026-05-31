'use client';

import dynamic from 'next/dynamic';

const VaultShell = dynamic(() => import('@/components/VaultShell'), {
  ssr: false,
});

export default function VaultLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return <VaultShell>{children}</VaultShell>;
}
