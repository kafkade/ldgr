'use client';

import { useCallback, useEffect, useState } from 'react';
import { useAdmin } from '@/contexts/AdminContext';
import type { ServerStats } from '@/lib/admin';
import {
  btnSecondary,
  cardClass,
  errorMessage,
  formatBytes,
} from './ui';

export function StoragePanel() {
  const { client, handleAuthError } = useAdmin();
  const [stats, setStats] = useState<ServerStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client) return;
    setLoading(true);
    setError(null);
    try {
      setStats(await client.getStats());
    } catch (err) {
      if (!handleAuthError(err)) setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [client, handleAuthError]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold">Storage &amp; usage</h2>
          <p className="text-sm text-[var(--color-text-secondary)]">
            Encrypted blob storage consumed per account.
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

      {loading && !stats ? (
        <p className="text-sm text-[var(--color-text-secondary)]">Loading…</p>
      ) : stats ? (
        <>
          <div className="grid gap-3 sm:grid-cols-2">
            <Metric label="Users" value={String(stats.user_count)} />
            <Metric
              label="Total storage used"
              value={formatBytes(stats.total_storage_bytes)}
            />
          </div>

          <div className={cardClass}>
            <h3 className="text-sm font-semibold">Per-user usage</h3>
            {stats.per_user.length === 0 ? (
              <p className="text-sm text-[var(--color-text-secondary)]">
                No users yet.
              </p>
            ) : (
              <div className="space-y-3">
                {stats.per_user.map((u) => (
                  <UsageBar key={u.id} row={u} />
                ))}
              </div>
            )}
          </div>
        </>
      ) : null}
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4">
      <div className="text-xs text-[var(--color-text-secondary)]">{label}</div>
      <div className="mt-1 text-2xl font-semibold">{value}</div>
    </div>
  );
}

function UsageBar({
  row,
}: {
  row: ServerStats['per_user'][number];
}) {
  const hasQuota = row.quota_bytes != null && row.quota_bytes > 0;
  const pct = hasQuota
    ? Math.min(100, (row.usage_bytes / (row.quota_bytes as number)) * 100)
    : null;
  const over = pct != null && pct >= 100;

  return (
    <div>
      <div className="mb-1 flex items-center justify-between text-xs">
        <span className="font-medium">{row.username}</span>
        <span className="text-[var(--color-text-secondary)]">
          {formatBytes(row.usage_bytes)}
          {hasQuota ? ` / ${formatBytes(row.quota_bytes)}` : ' (server default)'}
        </span>
      </div>
      <div className="h-2 w-full overflow-hidden rounded bg-[var(--color-bg)]">
        <div
          className="h-full rounded transition-all"
          style={{
            width: pct != null ? `${Math.max(2, pct)}%` : '0%',
            backgroundColor: over
              ? 'var(--color-danger)'
              : 'var(--color-accent)',
          }}
        />
      </div>
    </div>
  );
}
