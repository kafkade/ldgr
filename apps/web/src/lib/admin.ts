/**
 * Admin API client for the ldgr sync server.
 *
 * The sync server (`crates/ldgr-server`, AGPL-3.0) is headless and API-only: its
 * admin surface (`/api/v1/admin/*`, #176) returns JSON that this Apache-2.0 web
 * app renders (ADR-008 §7). This module is the typed HTTP boundary — plain
 * `fetch` with a `Bearer` session token — used by the admin panel screens (#179).
 *
 * It deliberately does no auth handshake: sign-in happens through the shared WASM
 * SRP client (see `AdminContext`), which yields the bearer token passed here.
 */

// ── Response types (mirror the server's serde views) ─────────────────────────

/** `admin::UserView`. */
export interface AdminUser {
  id: string;
  username: string;
  email: string | null;
  role: string;
  status: string;
  /** Per-user quota override in bytes; `null` means "use the server default". */
  quota_bytes: number | null;
  usage_bytes: number;
  created_at: string;
}

/** `admin::InviteView`. */
export interface AdminInvite {
  id: string;
  email: string | null;
  role: string;
  created_by: string | null;
  created_at: string;
  expires_at: string | null;
  redeemed_at: string | null;
  redeemed_by: string | null;
  /** Derived lifecycle: `pending`, `redeemed`, or `expired`. */
  status: string;
}

/** `admin::CreateInviteResponse` — the raw token is returned exactly once. */
export interface CreateInviteResult {
  token: string;
  id: string;
  role: string;
  email: string | null;
  expires_at: string | null;
}

/** `admin::SettingsView`. */
export interface ServerSettings {
  registration_policy: string;
  default_quota_bytes: number;
  max_blob_bytes: number;
}

/** `admin::PerUserUsage`. */
export interface PerUserUsage {
  id: string;
  username: string;
  usage_bytes: number;
  quota_bytes: number | null;
}

/** `admin::StatsView`. */
export interface ServerStats {
  user_count: number;
  total_storage_bytes: number;
  per_user: PerUserUsage[];
}

/** `server::ServerInfo` (unauthenticated discovery document). */
export interface ServerInfo {
  name: string;
  version: string;
  protocol_version: number;
  min_protocol_version: number;
  max_protocol_version: number;
  registration_policy: string;
  public_registration: boolean;
  two_secret_auth: boolean;
}

// ── Request payloads ─────────────────────────────────────────────────────────

export interface UpdateUserInput {
  status?: 'active' | 'disabled';
  role?: 'admin' | 'user';
  /**
   * `undefined` → leave quota unchanged; `null` → clear the override (use server
   * default); a number → set the override. Mirrors the server's double-option.
   */
  quota_bytes?: number | null;
}

export interface CreateInviteInput {
  email?: string;
  role?: 'admin' | 'user';
  /** Hours from now; omit for a non-expiring invite. */
  expires_in_hours?: number;
}

export interface UpdateSettingsInput {
  registration_policy?: string;
  default_quota_bytes?: number;
  max_blob_bytes?: number;
}

// ── Error type ───────────────────────────────────────────────────────────────

/** An error carrying the HTTP status and the server's `{ error }` message. */
export class AdminApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'AdminApiError';
    this.status = status;
  }

  /** A 401/403 means the session isn't (or is no longer) an active admin. */
  get isAuthError(): boolean {
    return this.status === 401 || this.status === 403;
  }
}

// ── Client ───────────────────────────────────────────────────────────────────

const API_PREFIX = '/api/v1';

function stripTrailingSlash(url: string): string {
  return url.replace(/\/+$/, '');
}

/**
 * Typed admin API client. Construct with the server base URL and a bearer token
 * (obtained via SRP sign-in). All mutating calls surface the server's error
 * message (e.g. "cannot demote the last active admin") as an {@link AdminApiError}.
 */
export class AdminClient {
  private readonly base: string;
  private readonly token: string;

  constructor(serverUrl: string, token: string) {
    this.base = stripTrailingSlash(serverUrl);
    this.token = token;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const headers: Record<string, string> = {
      authorization: `Bearer ${this.token}`,
    };
    const init: RequestInit = { method, headers };
    if (body !== undefined) {
      headers['content-type'] = 'application/json';
      init.body = JSON.stringify(body);
    }

    let resp: Response;
    try {
      resp = await fetch(`${this.base}${API_PREFIX}${path}`, init);
    } catch (err) {
      throw new AdminApiError(
        0,
        `Could not reach the server at ${this.base}. ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }

    if (!resp.ok) {
      throw new AdminApiError(resp.status, await extractError(resp));
    }

    if (resp.status === 204) return undefined as T;
    const text = await resp.text();
    return (text ? JSON.parse(text) : undefined) as T;
  }

  // Users
  listUsers(): Promise<AdminUser[]> {
    return this.request('GET', '/admin/users');
  }

  updateUser(id: string, input: UpdateUserInput): Promise<AdminUser> {
    return this.request('PATCH', `/admin/users/${encodeURIComponent(id)}`, input);
  }

  deleteUser(id: string): Promise<void> {
    return this.request('DELETE', `/admin/users/${encodeURIComponent(id)}`);
  }

  // Invites
  listInvites(): Promise<AdminInvite[]> {
    return this.request('GET', '/admin/invites');
  }

  createInvite(input: CreateInviteInput): Promise<CreateInviteResult> {
    return this.request('POST', '/admin/invites', input);
  }

  deleteInvite(id: string): Promise<void> {
    return this.request('DELETE', `/admin/invites/${encodeURIComponent(id)}`);
  }

  // Settings
  getSettings(): Promise<ServerSettings> {
    return this.request('GET', '/admin/settings');
  }

  updateSettings(input: UpdateSettingsInput): Promise<ServerSettings> {
    return this.request('PATCH', '/admin/settings', input);
  }

  // Stats
  getStats(): Promise<ServerStats> {
    return this.request('GET', '/admin/stats');
  }
}

/**
 * Fetch the unauthenticated server discovery document. Also serves as a cheap
 * "is this an ldgr server?" probe during sign-in.
 */
export async function fetchServerInfo(serverUrl: string): Promise<ServerInfo> {
  const base = stripTrailingSlash(serverUrl);
  let resp: Response;
  try {
    resp = await fetch(`${base}${API_PREFIX}/server/info`);
  } catch (err) {
    throw new AdminApiError(
      0,
      `Could not reach the server at ${base}. ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
  }
  if (!resp.ok) throw new AdminApiError(resp.status, await extractError(resp));
  return (await resp.json()) as ServerInfo;
}

/** Pull the server's `{ error }` message out of a failed response. */
async function extractError(resp: Response): Promise<string> {
  try {
    const data = (await resp.clone().json()) as { error?: unknown };
    if (data && typeof data.error === 'string' && data.error) return data.error;
  } catch {
    // Fall through to text/status.
  }
  try {
    const text = (await resp.text()).trim();
    if (text) return text;
  } catch {
    // Ignore.
  }
  return `Request failed (${resp.status})`;
}
