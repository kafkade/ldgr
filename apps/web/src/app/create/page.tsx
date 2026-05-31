'use client';

import dynamic from 'next/dynamic';
import { VaultProvider } from '@/contexts/VaultContext';
import { ThemeProvider } from '@/contexts/ThemeContext';

const VaultCreateInner = dynamic(() => import('@/components/VaultCreateInner'), {
  ssr: false,
});

export default function CreatePage() {
  return (
    <ThemeProvider>
      <VaultProvider>
        <VaultCreateInner />
      </VaultProvider>
    </ThemeProvider>
  );
}
