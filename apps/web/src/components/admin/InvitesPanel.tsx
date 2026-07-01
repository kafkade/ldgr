'use client';

import { useCallback, useEffect, useState } from 'react';
import { useAdmin } from '@/contexts/AdminContext';
import type {
  AdminInvite,
  CreateInviteInput,
  CreateInviteResult,
} from '@/lib/admin';
import {
  btnPrimary,
  btnSecondary,
  cardClass,
  errorMessage,
  formatDate,
  inputClass,
  labelClass,
} from './ui';

export function InvitesPanel() {
  const { client, handleAuthError } = useAdmin();
  const [invites, setInvites] = useState<AdminInvite[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Create form
  const [email, setEmail] = useState('');
  const [role, setRole] = useState<'user' | 'admin'>('user');
  const [expires, setExpires] = useState('');
  const [creating, setCreating] = useState(false);
  const [created, setCreated] = useState<CreateInviteResult | null>(null);
  const [copied, setCopied] = useState(false);

  const refresh = useCallback(async () => {
    if (!client) return;
    setLoading(true);
    setError(null);
    try {
      setInvites(await client.listInvites());
    } catch (err) {
      if (!handleAuthError(err)) setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [client, handleAuthError]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const createInvite = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!client) return;
    setCreating(true);
    setError(null);
    setCreated(null);
    setCopied(false);
    try {
      const input: CreateInviteInput = { role };
      if (email.trim()) input.email = email.trim();
      if (expires.trim()) {
        const hours = Number(expires.trim());
        if (!Number.isFinite(hours) || hours <= 0) {
          throw new Error('Expiry must be a positive number of hours.');
        }
        input.expires_in_hours = hours;
      }
      setCreated(await client.createInvite(input));
      setEmail('');
      setExpires('');
      await refresh();
    } catch (err) {
      if (!handleAuthError(err)) setError(errorMessage(err));
    } finally {
      setCreating(false);
    }
  };

  const revoke = async (invite: AdminInvite) => {
    if (!client) return;
    if (!window.confirm('Revoke this invite? The token will stop working.')) {
      return;
    }
    setError(null);
    try {
      await client.deleteInvite(invite.id);
      await refresh();
    } catch (err) {
      if (!handleAuthError(err)) setError(errorMessage(err));
    }
  };

  const copyToken = async () => {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.token);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <section className="space-y-4">
      <div>
        <h2 className="text-xl font-semibold">Invites</h2>
        <p className="text-sm text-[var(--color-text-secondary)]">
          Issue an invite token, then share it with the invitee — they redeem it
          during self-registration. The token is shown only once.
        </p>
      </div>

      {error && (
        <p className="text-sm text-[var(--color-danger)]" role="alert">
          {error}
        </p>
      )}

      <form onSubmit={createInvite} className={cardClass}>
        <div className="grid gap-4 sm:grid-cols-3">
          <div>
            <label className={labelClass} htmlFor="invite-email">
              Email (optional)
            </label>
            <input
              id="invite-email"
              className={inputClass}
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="user@example.com"
            />
          </div>
          <div>
            <label className={labelClass} htmlFor="invite-role">
              Role
            </label>
            <select
              id="invite-role"
              className={inputClass}
              value={role}
              onChange={(e) => setRole(e.target.value as 'user' | 'admin')}
            >
              <option value="user">user</option>
              <option value="admin">admin</option>
            </select>
          </div>
          <div>
            <label className={labelClass} htmlFor="invite-expires">
              Expires in (hours, optional)
            </label>
            <input
              id="invite-expires"
              className={inputClass}
              inputMode="numeric"
              value={expires}
              onChange={(e) => setExpires(e.target.value)}
              placeholder="never"
            />
          </div>
        </div>
        <button type="submit" className={btnPrimary} disabled={creating}>
          {creating ? 'Creating…' : 'Create invite'}
        </button>
      </form>

      {created && (
        <div className={`${cardClass} border-[var(--color-accent)]`}>
          <p className="text-sm font-medium">
            Invite created — copy this token now. It will not be shown again.
          </p>
          <code className="block break-all rounded bg-[var(--color-bg)] p-3 text-xs">
            {created.token}
          </code>
          <div className="flex items-center gap-3">
            <button type="button" className={btnSecondary} onClick={copyToken}>
              {copied ? 'Copied' : 'Copy token'}
            </button>
            <span className="text-xs text-[var(--color-text-secondary)]">
              role: {created.role}
              {created.expires_at
                ? ` · expires ${formatDate(created.expires_at)}`
                : ' · no expiry'}
            </span>
          </div>
        </div>
      )}

      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">Existing invites</h3>
        <button
          type="button"
          className={btnSecondary}
          onClick={refresh}
          disabled={loading}
        >
          {loading ? 'Refreshing…' : 'Refresh'}
        </button>
      </div>

      {loading && !invites ? (
        <p className="text-sm text-[var(--color-text-secondary)]">Loading…</p>
      ) : invites && invites.length > 0 ? (
        <div className="space-y-2">
          {invites.map((inv) => (
            <div
              key={inv.id}
              className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] px-4 py-3"
            >
              <div className="min-w-0 text-sm">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{inv.email ?? '(no email)'}</span>
                  <span className="rounded bg-[var(--color-bg)] px-1.5 py-0.5 text-xs text-[var(--color-text-secondary)]">
                    {inv.role}
                  </span>
                  <InviteStatus status={inv.status} />
                </div>
                <div className="mt-0.5 text-xs text-[var(--color-text-secondary)]">
                  created {formatDate(inv.created_at)} ·{' '}
                  {inv.expires_at
                    ? `expires ${formatDate(inv.expires_at)}`
                    : 'no expiry'}
                </div>
              </div>
              {inv.status === 'pending' && (
                <button
                  type="button"
                  className={btnSecondary}
                  onClick={() => revoke(inv)}
                >
                  Revoke
                </button>
              )}
            </div>
          ))}
        </div>
      ) : (
        <p className="text-sm text-[var(--color-text-secondary)]">
          No invites yet.
        </p>
      )}
    </section>
  );
}

function InviteStatus({ status }: { status: string }) {
  const color =
    status === 'redeemed'
      ? 'text-[var(--color-accent)]'
      : status === 'expired'
        ? 'text-[var(--color-danger)]'
        : 'text-[var(--color-text-secondary)]';
  return <span className={`text-xs ${color}`}>{status}</span>;
}
