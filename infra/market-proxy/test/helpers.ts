/** Shared test doubles: an in-memory KV namespace and execution context. */

import type { Env } from "../src/types.js";

export interface StoredEntry {
  value: string;
  expirationTtl?: number;
}

/** Minimal in-memory KVNamespace supporting get(json) / put / delete. */
export class MemoryKV {
  store = new Map<string, StoredEntry>();

  async get(key: string, type?: "text" | "json"): Promise<unknown> {
    const entry = this.store.get(key);
    if (!entry) return null;
    return type === "json" ? JSON.parse(entry.value) : entry.value;
  }

  async put(
    key: string,
    value: string,
    options?: { expirationTtl?: number },
  ): Promise<void> {
    this.store.set(key, { value, expirationTtl: options?.expirationTtl });
  }

  async delete(key: string): Promise<void> {
    this.store.delete(key);
  }
}

/** Execution context that tracks and can await deferred `waitUntil` work. */
export class TestExecutionContext {
  pending: Promise<unknown>[] = [];
  waitUntil(promise: Promise<unknown>): void {
    this.pending.push(promise);
  }
  passThroughOnException(): void {}
  props = {};
  async settle(): Promise<void> {
    await Promise.all(this.pending);
  }
}

export function makeEnv(overrides: Partial<Env> = {}): {
  env: Env;
  kv: MemoryKV;
} {
  const kv = new MemoryKV();
  const env = { MARKET_KV: kv as unknown as KVNamespace, ...overrides } as Env;
  return { env, kv };
}

/** Build a plain Response for mocked fetch. */
export function textResponse(body: string, status = 200): Response {
  return new Response(body, { status });
}
