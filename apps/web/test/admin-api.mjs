/**
 * Unit tests for the admin API client (`src/lib/admin.ts`).
 *
 * These exercise request shaping (method, path, auth header, JSON body) and
 * error mapping against a stubbed `fetch` — no server or browser required.
 * `admin.ts` has no runtime imports, so Node's native type stripping loads it
 * directly.
 */

import { test, describe, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import {
  AdminApiError,
  AdminClient,
  fetchServerInfo,
} from '../src/lib/admin.ts';

const realFetch = globalThis.fetch;
let calls;

/** Install a fetch stub returning `status`/`body` and recording the request. */
function stubFetch(status, body, { asText = false } = {}) {
  globalThis.fetch = async (url, init = {}) => {
    calls.push({ url: String(url), init });
    const payload =
      body === undefined ? '' : asText ? body : JSON.stringify(body);
    return {
      ok: status >= 200 && status < 300,
      status,
      async text() {
        return payload;
      },
      async json() {
        return JSON.parse(payload);
      },
      clone() {
        return this;
      },
    };
  };
}

beforeEach(() => {
  calls = [];
});

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('AdminClient request shaping', () => {
  test('listUsers issues an authorized GET to the users endpoint', async () => {
    stubFetch(200, [{ id: 'u1', username: 'alice' }]);
    const client = new AdminClient('https://sync.example.com', 'tok123');
    const users = await client.listUsers();

    assert.equal(calls.length, 1);
    assert.equal(
      calls[0].url,
      'https://sync.example.com/api/v1/admin/users',
    );
    assert.equal(calls[0].init.method, 'GET');
    assert.equal(calls[0].init.headers.authorization, 'Bearer tok123');
    // GET carries no body / content-type.
    assert.equal(calls[0].init.body, undefined);
    assert.equal(users[0].username, 'alice');
  });

  test('trailing slashes in the base URL are normalized', async () => {
    stubFetch(200, []);
    await new AdminClient('https://sync.example.com///', 'tok').listUsers();
    assert.equal(
      calls[0].url,
      'https://sync.example.com/api/v1/admin/users',
    );
  });

  test('updateUser sends a PATCH with a JSON body and encodes the id', async () => {
    stubFetch(200, { id: 'a/b', username: 'bob' });
    const client = new AdminClient('https://s.example', 'tok');
    await client.updateUser('a/b', { role: 'admin', quota_bytes: null });

    const { url, init } = calls[0];
    assert.equal(url, 'https://s.example/api/v1/admin/users/a%2Fb');
    assert.equal(init.method, 'PATCH');
    assert.equal(init.headers['content-type'], 'application/json');
    assert.deepEqual(JSON.parse(init.body), {
      role: 'admin',
      quota_bytes: null,
    });
  });

  test('deleteUser sends a DELETE and tolerates a 204 with no body', async () => {
    stubFetch(204, undefined);
    const client = new AdminClient('https://s.example', 'tok');
    const result = await client.deleteUser('u1');
    assert.equal(result, undefined);
    assert.equal(calls[0].init.method, 'DELETE');
  });

  test('createInvite posts the invite payload', async () => {
    stubFetch(201, { token: 'raw', id: 'hash', role: 'user', email: null });
    const client = new AdminClient('https://s.example', 'tok');
    const res = await client.createInvite({ role: 'user', email: 'x@y.z' });
    assert.equal(calls[0].url, 'https://s.example/api/v1/admin/invites');
    assert.equal(calls[0].init.method, 'POST');
    assert.deepEqual(JSON.parse(calls[0].init.body), {
      role: 'user',
      email: 'x@y.z',
    });
    assert.equal(res.token, 'raw');
  });

  test('updateSettings PATCHes the settings endpoint', async () => {
    stubFetch(200, {
      registration_policy: 'open',
      default_quota_bytes: 10,
      max_blob_bytes: 20,
    });
    const client = new AdminClient('https://s.example', 'tok');
    const res = await client.updateSettings({ registration_policy: 'open' });
    assert.equal(calls[0].url, 'https://s.example/api/v1/admin/settings');
    assert.equal(calls[0].init.method, 'PATCH');
    assert.equal(res.registration_policy, 'open');
  });
});

describe('AdminClient error mapping', () => {
  test('surfaces the server {error} message and status', async () => {
    stubFetch(403, { error: 'cannot demote the last active admin' });
    const client = new AdminClient('https://s.example', 'tok');
    await assert.rejects(
      () => client.updateUser('u1', { role: 'user' }),
      (err) => {
        assert.ok(err instanceof AdminApiError);
        assert.equal(err.status, 403);
        assert.equal(err.message, 'cannot demote the last active admin');
        assert.equal(err.isAuthError, true);
        return true;
      },
    );
  });

  test('401 and 403 are auth errors; 400 is not', async () => {
    stubFetch(400, { error: 'bad request: nope' });
    const client = new AdminClient('https://s.example', 'tok');
    await assert.rejects(
      () => client.listUsers(),
      (err) => {
        assert.equal(err.status, 400);
        assert.equal(err.isAuthError, false);
        return true;
      },
    );
  });

  test('falls back to plain text when the body is not JSON', async () => {
    stubFetch(500, 'boom', { asText: true });
    const client = new AdminClient('https://s.example', 'tok');
    await assert.rejects(
      () => client.listUsers(),
      (err) => {
        assert.equal(err.status, 500);
        assert.equal(err.message, 'boom');
        return true;
      },
    );
  });
});

describe('fetchServerInfo', () => {
  test('GETs the unauthenticated discovery endpoint', async () => {
    stubFetch(200, {
      name: 'ldgr',
      version: '0.1.0',
      protocol_version: 1,
      min_protocol_version: 1,
      max_protocol_version: 1,
      registration_policy: 'invite-only',
      public_registration: false,
      two_secret_auth: true,
    });
    const info = await fetchServerInfo('https://s.example/');
    assert.equal(calls[0].url, 'https://s.example/api/v1/server/info');
    assert.equal(info.name, 'ldgr');
    assert.equal(info.two_secret_auth, true);
  });
});
