'use client';

import { useCallback, useEffect, useState } from 'react';
import { useAdmin } from '@/contexts/AdminContext';
import { fetchServerInfo, type ServerInfo } from '@/lib/admin';
import { btnSecondary, cardClass, errorMessage } from './ui';

export function ServerPanel() {
  const { session } = useAdmin();
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!session) return;
    setLoading(true);
    setError(null);
    try {
      setInfo(await fetchServerInfo(session.serverUrl));
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [session]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold">Server info</h2>
          <p className="text-sm text-[var(--color-text-secondary)]">
            Version and capabilities reported by the server.
          </p>
        </div>
        <button
          type="button"
          className={btnSecondary}
          onClick={refresh}
          disabled={loading}
        >
          {loading ? 'Refreshing…' : 'Refresh'}
        </button>
      </div>

      {error && (
        <p className="text-sm text-[var(--color-danger)]" role="alert">
          {error}
        </p>
      )}

      {loading && !info ? (
        <p className="text-sm text-[var(--color-text-secondary)]">Loading…</p>
      ) : info ? (
        <dl className={cardClass}>
          <Row label="Instance name" value={info.name} />
          <Row label="Software version" value={info.version} />
          <Row
            label="Protocol version"
            value={`${info.protocol_version} (supports ${info.min_protocol_version}–${info.max_protocol_version})`}
          />
          <Row label="Registration policy" value={info.registration_policy} />
          <Row
            label="Public registration"
            value={info.public_registration ? 'enabled' : 'disabled'}
          />
          <Row
            label="Two-secret auth"
            value={info.two_secret_auth ? 'available' : 'unavailable'}
          />
          <Row label="Server URL" value={session?.serverUrl ?? '—'} />
        </dl>
      ) : null}
    </section>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 border-b border-[var(--color-border)] pb-2 last:border-0 last:pb-0">
      <dt className="text-sm text-[var(--color-text-secondary)]">{label}</dt>
      <dd className="text-sm font-medium">{value}</dd>
    </div>
  );
}
