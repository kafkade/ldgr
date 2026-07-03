/**
 * KV-backed response cache with concurrent-miss de-duplication.
 *
 * A cached entry stores the upstream body plus its content type, keyed by the
 * normalized route+symbol cache key. Concurrent misses for the same key are
 * coalesced (see `dedupe`) so only one upstream fetch happens, and the write to
 * KV is deferred via `ctx.waitUntil` so it never delays the response.
 */

import { dedupe } from "./throttle.js";
import type { Env, UpstreamResponse } from "./types.js";

export interface CacheResult {
  response: UpstreamResponse;
  /** Whether the response was served from KV (true) or freshly fetched. */
  cached: boolean;
}

/**
 * Return a cached response for `key`, or fetch it via `fetcher`, cache it with
 * the given TTL, and return it.
 */
export async function getOrFetch(
  env: Env,
  ctx: ExecutionContext,
  key: string,
  ttlSeconds: number,
  fetcher: () => Promise<UpstreamResponse>,
): Promise<CacheResult> {
  const hit = await env.MARKET_KV.get(key, "json");
  if (hit) {
    return { response: hit as UpstreamResponse, cached: true };
  }

  const response = await dedupe(key, async () => {
    const fresh = await fetcher();
    ctx.waitUntil(
      env.MARKET_KV.put(key, JSON.stringify(fresh), { expirationTtl: ttlSeconds }),
    );
    return fresh;
  });

  return { response, cached: false };
}
