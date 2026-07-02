'use client';

import { useEffect, useState } from 'react';
import QRCode from 'qrcode';
import type { EmergencyKit } from '@/lib/wasm';

/**
 * Renders the account **Emergency Kit** shown once at two-secret (2SKD) sign-up.
 *
 * The Secret Key is a per-account server-auth secret required to sign in on a
 * new device together with the master password. It is displayed once, never
 * sent to the server, and does not unlock the local vault (ADR-008). The user
 * can copy it, download the kit, print it, or scan the QR with another device.
 */
export default function EmergencyKitView({
  kit,
  onDone,
}: {
  kit: EmergencyKit;
  onDone: () => void;
}) {
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    QRCode.toDataURL(kit.qrPayload, { margin: 2, width: 240 })
      .then((url) => {
        if (!cancelled) setQrDataUrl(url);
      })
      .catch(() => {
        if (!cancelled) setQrDataUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [kit.qrPayload]);

  const copySecretKey = async () => {
    try {
      await navigator.clipboard.writeText(kit.secretKey);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  };

  const download = () => {
    const lines = [
      'ldgr Emergency Kit',
      '==================',
      '',
      'KEEP THIS SAFE. The Secret Key below is required to sign in on a new',
      'device together with your master password. It does not unlock your',
      'local vault and is never sent to the server.',
      '',
      `Server:       ${kit.address}`,
      `Account:      ${kit.email}`,
      `Account hint: ${kit.accountHint}`,
      `Secret Key:   ${kit.secretKey}`,
      ...(kit.recoveryKey ? [`Recovery Key: ${kit.recoveryKey}`] : []),
      '',
      'QR payload (for import/scan):',
      kit.qrPayload,
      '',
    ].join('\n');
    const blob = new Blob([lines], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'ldgr-emergency-kit.txt';
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="rounded-lg border border-[var(--color-accent)] bg-[var(--color-accent-light)] p-5 space-y-4">
      <div className="space-y-1">
        <h3 className="text-sm font-bold text-[var(--color-accent)]">
          🔐 Your Emergency Kit
        </h3>
        <p className="text-xs text-[var(--color-text-secondary)]">
          Save this now — your <strong>Secret Key</strong> is shown{' '}
          <strong>only once</strong> and is required to sign in on a new device
          (together with your master password). It does not unlock your local
          vault and is never sent to the server.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-[auto_1fr] sm:items-center">
        <div className="flex justify-center">
          {qrDataUrl ? (
            // eslint-disable-next-line @next/next/no-img-element
            <img
              src={qrDataUrl}
              alt="Emergency Kit QR code"
              className="rounded-lg bg-white p-2"
              width={200}
              height={200}
            />
          ) : (
            <div className="flex h-[200px] w-[200px] items-center justify-center rounded-lg border border-[var(--color-border)] text-xs text-[var(--color-text-secondary)]">
              Generating QR…
            </div>
          )}
        </div>

        <dl className="space-y-2 text-xs">
          <div>
            <dt className="text-[var(--color-text-secondary)]">Server</dt>
            <dd className="font-medium break-all">{kit.address}</dd>
          </div>
          <div>
            <dt className="text-[var(--color-text-secondary)]">Account</dt>
            <dd className="font-medium break-all">{kit.email}</dd>
          </div>
          <div>
            <dt className="text-[var(--color-text-secondary)]">Secret Key</dt>
            <dd className="font-mono font-medium break-all select-all">
              {kit.secretKey}
            </dd>
          </div>
        </dl>
      </div>

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={copySecretKey}
          className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] px-3 py-2 text-sm hover:bg-[var(--color-bg)] transition-colors"
        >
          {copied ? '✓ Copied' : 'Copy Secret Key'}
        </button>
        <button
          type="button"
          onClick={download}
          className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] px-3 py-2 text-sm hover:bg-[var(--color-bg)] transition-colors"
        >
          Download kit
        </button>
        <button
          type="button"
          onClick={() => window.print()}
          className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] px-3 py-2 text-sm hover:bg-[var(--color-bg)] transition-colors"
        >
          Print
        </button>
      </div>

      <button
        type="button"
        onClick={onDone}
        className="w-full rounded-lg border border-[var(--color-accent)] bg-[var(--color-accent)] px-3 py-2 text-sm font-medium text-white transition-colors"
      >
        I&apos;ve saved my Secret Key — continue
      </button>
    </div>
  );
}
