import type { Metadata } from 'next';
import { ServiceWorkerRegistrar } from '@/components/ServiceWorkerRegistrar';
import './globals.css';

export const metadata: Metadata = {
  title: 'ldgr — Zero-Knowledge Personal Finance',
  description:
    'Private, encrypted double-entry bookkeeping. Your financial data never leaves your device.',
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="min-h-screen antialiased">
        {children}
        <ServiceWorkerRegistrar />
      </body>
    </html>
  );
}
