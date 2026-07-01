'use client';

import { useCallback, useEffect, useState } from 'react';
import { useAdmin } from '@/contexts/AdminContext';
import type { AdminUser, UpdateUserInput } from '@/lib/admin';
import {
  btnDanger,
  btnSecondary,
  cardClass,
  errorMessage,
  formatBytes,
  formatDate,
  inputClass,
} from './ui';

export function UsersPanel() {
  const { client, handleAuthError } = useAdmin();
  const [users, setUsers] = useState<AdminUser[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client) return;
    setLoading(true);
    setError(null);
    try {
      setUsers(await client.listUsers());
    } catch (err) {
      if (!handleAuthError(err)) setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [client, handleAuthError]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const onAction = useCallback(
    async (label: string, fn: () => Promise<void>) => {
      setError(null);
      setNotice(null);
      try {
        await fn();
        setNotice(label);
        await refresh();
      } catch (err) {
        if (!handleAuthError(err)) setError(errorMessage(err));
      }
    },
    [refresh, handleAuthError],
  );

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold">Users</h2>
          <p className="text-sm text-[var(--color-text-secondary)]">
            Manage accounts, roles, quotas, and access. Add users from the{' '}
            <span className="font-medium">Invites</span> tab.
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
      {notice && (
        <p className="text-sm text-[var(--color-accent)]">{notice}</p>
      )}

      {loading && !users ? (
        <p className="text-sm text-[var(--color-text-secondary)]">Loading…</p>
      ) : users && users.length > 0 ? (
        <div className="space-y-3">
          {users.map((u) => (
            <UserRow key={u.id} user={u} onAction={onAction} />
          ))}
        </div>
      ) : (
        <p className="text-sm text-[var(--color-text-secondary)]">
          No users yet.
        </p>
      )}
    </section>
  );
}

function UserRow({
  user,
  onAction,
}: {
  user: AdminUser;
  onAction: (label: string, fn: () => Promise<void>) => Promise<void>;
}) {
  const { client } = useAdmin();
  const [quota, setQuota] = useState(
    user.quota_bytes != null ? String(user.quota_bytes) : '',
  );
  const [busy, setBusy] = useState(false);

  const disabled = user.status === 'disabled';

  const run = async (label: string, input: UpdateUserInput | 'delete') => {
    if (!client) return;
    setBusy(true);
    try {
      await onAction(label, () =>
        input === 'delete'
          ? client.deleteUser(user.id)
          : client.updateUser(user.id, input).then(() => undefined),
      );
    } finally {
      setBusy(false);
    }
  };

  const saveQuota = () => {
    const trimmed = quota.trim();
    if (trimmed === '') {
      void run('Quota cleared (using server default).', { quota_bytes: null });
      return;
    }
    const n = Number(trimmed);
    if (!Number.isInteger(n) || n < 0) return; // guarded by quotaValid/disabled
    void run('Quota updated.', { quota_bytes: n });
  };

  const quotaValid =
    quota.trim() === '' ||
    (Number.isInteger(Number(quota.trim())) && Number(quota.trim()) >= 0);

  return (
    <div className={cardClass}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium">{user.username}</span>
            <RoleBadge role={user.role} />
            <StatusBadge status={user.status} />
          </div>
          <div className="mt-1 text-xs text-[var(--color-text-secondary)]">
            {user.email ?? '—'} · created {formatDate(user.created_at)}
          </div>
          <div className="mt-1 text-xs text-[var(--color-text-secondary)]">
            Usage {formatBytes(user.usage_bytes)} · quota{' '}
            {user.quota_bytes != null
              ? formatBytes(user.quota_bytes)
              : 'server default'}
          </div>
        </div>
        <button
          type="button"
          className={btnDanger}
          disabled={busy}
          onClick={() => {
            if (
              window.confirm(
                `Delete user "${user.username}"? This cannot be undone.`,
              )
            ) {
              void run('User deleted.', 'delete');
            }
          }}
        >
          Delete
        </button>
      </div>

      <div className="flex flex-wrap items-end gap-4 border-t border-[var(--color-border)] pt-3">
        <div>
          <label className="mb-1 block text-xs text-[var(--color-text-secondary)]">
            Role
          </label>
          <select
            className={inputClass}
            value={user.role}
            disabled={busy}
            onChange={(e) =>
              void run(`Role set to ${e.target.value}.`, {
                role: e.target.value as 'admin' | 'user',
              })
            }
          >
            <option value="user">user</option>
            <option value="admin">admin</option>
          </select>
        </div>

        <button
          type="button"
          className={btnSecondary}
          disabled={busy}
          onClick={() =>
            void run(disabled ? 'User enabled.' : 'User disabled.', {
              status: disabled ? 'active' : 'disabled',
            })
          }
        >
          {disabled ? 'Enable' : 'Disable'}
        </button>

        <div className="flex items-end gap-2">
          <div>
            <label className="mb-1 block text-xs text-[var(--color-text-secondary)]">
              Quota (bytes, blank = default)
            </label>
            <input
              className={inputClass}
              inputMode="numeric"
              value={quota}
              disabled={busy}
              onChange={(e) => setQuota(e.target.value)}
              placeholder="server default"
            />
          </div>
          <button
            type="button"
            className={btnSecondary}
            disabled={busy || !quotaValid}
            onClick={saveQuota}
          >
            Save quota
          </button>
        </div>
      </div>
    </div>
  );
}

function RoleBadge({ role }: { role: string }) {
  const admin = role === 'admin';
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-xs ${
        admin
          ? 'bg-[var(--color-accent-light)] text-[var(--color-accent)]'
          : 'bg-[var(--color-bg)] text-[var(--color-text-secondary)]'
      }`}
    >
      {role}
    </span>
  );
}

function StatusBadge({ status }: { status: string }) {
  const active = status === 'active';
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-xs ${
        active
          ? 'bg-[var(--color-bg)] text-[var(--color-text-secondary)]'
          : 'text-[var(--color-danger)]'
      }`}
    >
      {status}
    </span>
  );
}
