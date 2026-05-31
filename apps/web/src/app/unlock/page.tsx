'use client';

import dynamic from 'next/dynamic';
import { VaultProvider } from '@/contexts/VaultContext';
import { ThemeProvider } from '@/contexts/ThemeContext';

const VaultUnlockInner = dynamic(
  () => import('@/components/VaultUnlockInner'),
  { ssr: false },
);

export default function UnlockPage() {
  return (
    <ThemeProvider>
      <VaultProvider>
        <VaultUnlockInner />
      </VaultProvider>
    </ThemeProvider>
  );
}
