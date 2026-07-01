/**
 * Shared styling tokens and formatting helpers for the admin panel. Mirrors the
 * card/input/button classes used elsewhere in the web app (see `SyncSettings`)
 * so the admin screens look native.
 */

export const cardClass =
  'rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5 space-y-4';
export const inputClass =
  'w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm';
export const labelClass =
  'block text-xs font-medium text-[var(--color-text-secondary)] mb-1';
export const btnPrimary =
  'rounded-lg border border-[var(--color-accent)] bg-[var(--color-accent-light)] px-3 py-2 text-sm font-medium text-[var(--color-accent)] disabled:opacity-50 transition-colors';
export const btnSecondary =
  'rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm hover:bg-[var(--color-bg)] disabled:opacity-50 transition-colors';
export const btnDanger =
  'rounded-lg border border-[var(--color-danger)] px-3 py-2 text-sm font-medium text-[var(--color-danger)] hover:bg-[var(--color-danger)]/10 disabled:opacity-50 transition-colors';

/** Human-readable byte size (binary units). */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return '—';
  if (bytes < 0) return '—';
  if (bytes === 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const exp = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const value = bytes / 1024 ** exp;
  const rounded = value >= 100 || exp === 0 ? Math.round(value) : value.toFixed(1);
  return `${rounded} ${units[exp]}`;
}

/** Format an ISO timestamp as a locale date-time, or a dash when absent. */
export function formatDate(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString();
}

/** Extract a displayable message from any thrown value. */
export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
