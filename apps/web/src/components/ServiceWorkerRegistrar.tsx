'use client';

import { useEffect } from 'react';

export function ServiceWorkerRegistrar() {
  useEffect(() => {
    if ('serviceWorker' in navigator) {
      navigator.serviceWorker.register('/sw.js').catch(() => {
        // Service worker registration is optional — fails silently in dev
      });
    }
  }, []);

  return null;
}
