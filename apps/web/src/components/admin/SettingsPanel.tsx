'use client';

import { useCallback, useEffect, useState } from 'react';
import { useAdmin } from '@/contexts/AdminContext';
import type { ServerSettings, UpdateSettingsInput } from '@/lib/admin';
import {
  btnPrimary,
  cardClass,
  errorMessage,
  inputClass,
  labelClass,
} from './ui';

const POLICIES = [
  { value: 'open', label: 'Open — anyone may self-register' },
  { value: 'invite-only', label: 'Invite-only — requires an invite token' },
  { value: 'admin-only', label: 'Admin-only — only admins create accounts' },
];

export function SettingsPanel() {
  const { client, handleAuthError } = useAdmin();
  const [settings, setSettings] = useState<ServerSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const [policy, setPolicy] = useState('invite-only');
  const [defaultQuota, setDefaultQuota] = useState('');
  const [maxBlob, setMaxBlob] = useState('');

  const apply = useCallback((s: ServerSettings) => {
    setSettings(s);
    setPolicy(s.registration_policy);
    setDefaultQuota(String(s.default_quota_bytes));
    setMaxBlob(String(s.max_blob_bytes));
  }, []);

  const refresh = useCallback(async () => {
    if (!client) return;
    setLoading(true);
    setError(null);
    try {
      apply(await client.getSettings());
    } catch (err) {
      if (!handleAuthError(err)) setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [client, handleAuthError, apply]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const save = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!client || !settings) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const input: UpdateSettingsInput = {};
      if (policy !== settings.registration_policy) {
        input.registration_policy = policy;
      }
      const dq = Number(defaultQuota.trim());
      if (!Number.isInteger(dq) || dq <= 0) {
        throw new Error('Default quota must be a positive whole number of bytes.');
      }
      if (dq !== settings.default_quota_bytes) input.default_quota_bytes = dq;

      const mb = Number(maxBlob.trim());
      if (!Number.isInteger(mb) || mb <= 0) {
        throw new Error('Max blob size must be a positive whole number of bytes.');
      }
      if (mb !== settings.max_blob_bytes) input.max_blob_bytes = mb;

      if (Object.keys(input).length === 0) {
        setNotice('No changes to save.');
        return;
      }
      apply(await client.updateSettings(input));
      setNotice('Settings saved.');
    } catch (err) {
      if (!handleAuthError(err)) setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="space-y-4">
      <div>
        <h2 className="text-xl font-semibold">Server settings</h2>
        <p className="text-sm text-[var(--color-text-secondary)]">
          Control who can register and the default storage limits.
        </p>
      </div>

      {error && (
        <p className="text-sm text-[var(--color-danger)]" role="alert">
          {error}
        </p>
      )}
      {notice && <p className="text-sm text-[var(--color-accent)]">{notice}</p>}

      {loading && !settings ? (
        <p className="text-sm text-[var(--color-text-secondary)]">Loading…</p>
      ) : settings ? (
        <form onSubmit={save} className={cardClass}>
          <div>
            <label className={labelClass} htmlFor="reg-policy">
              Registration policy
            </label>
            <select
              id="reg-policy"
              className={inputClass}
              value={policy}
              onChange={(e) => setPolicy(e.target.value)}
            >
              {POLICIES.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div>
              <label className={labelClass} htmlFor="default-quota">
                Default quota (bytes)
              </label>
              <input
                id="default-quota"
                className={inputClass}
                inputMode="numeric"
                value={defaultQuota}
                onChange={(e) => setDefaultQuota(e.target.value)}
              />
            </div>
            <div>
              <label className={labelClass} htmlFor="max-blob">
                Max blob size (bytes)
              </label>
              <input
                id="max-blob"
                className={inputClass}
                inputMode="numeric"
                value={maxBlob}
                onChange={(e) => setMaxBlob(e.target.value)}
              />
            </div>
          </div>

          <button type="submit" className={btnPrimary} disabled={saving}>
            {saving ? 'Saving…' : 'Save settings'}
          </button>
        </form>
      ) : null}
    </section>
  );
}
