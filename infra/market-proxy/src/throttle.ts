/**
 * Concurrency control: in-flight request de-duplication and per-provider
 * upstream throttling.
 *
 * Cloudflare runs many requests inside a single V8 isolate, so module-level
 * state is shared across concurrent requests handled by the same isolate. That
 * makes these best-effort but effective: a burst of identical requests hitting
 * one POP collapses to a single upstream fetch, and upstream fetches to a given
 * provider are spaced out to avoid tripping rate limits.
 */

import type { Provider } from "./types.js";

/** Minimum spacing between upstream fetches to the same provider (ms). */
export const MIN_UPSTREAM_INTERVAL_MS = 1000;

/** In-flight upstream fetches, keyed by cache key. */
const inFlight = new Map<string, Promise<unknown>>();

/**
 * Coalesce concurrent calls that share a key onto a single in-flight promise.
 * The first caller runs `fn`; concurrent callers await the same result. The
 * entry is cleared once settled so the next miss triggers a fresh fetch.
 */
export function dedupe<T>(key: string, fn: () => Promise<T>): Promise<T> {
  const existing = inFlight.get(key) as Promise<T> | undefined;
  if (existing) return existing;

  const promise = (async () => {
    try {
      return await fn();
    } finally {
      inFlight.delete(key);
    }
  })();

  inFlight.set(key, promise);
  return promise;
}

/** Per-provider gate: resolves when the next fetch to that provider may start. */
const providerGate = new Map<Provider, Promise<void>>();

/**
 * Throttle upstream fetches to a provider to at most ~1 request/second.
 *
 * Calls are serialized per provider: each waits for the previous call's spacing
 * window to elapse before its fetch begins. Crucially, the spacing delay gates
 * only the *next* call — it never delays the current caller's response, so a
 * cache miss returns as soon as its own upstream fetch completes.
 */
export function throttle<T>(provider: Provider, fn: () => Promise<T>): Promise<T> {
  const prev = providerGate.get(provider) ?? Promise.resolve();

  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  // The next caller must wait until this call's spacing window closes.
  providerGate.set(provider, gate);

  return (async () => {
    await prev;
    const start = Date.now();
    try {
      return await fn();
    } finally {
      // Open the gate for the next caller after the spacing window, without
      // blocking resolution of this call's result.
      const wait = Math.max(0, MIN_UPSTREAM_INTERVAL_MS - (Date.now() - start));
      setTimeout(release, wait);
    }
  })();
}

/** Test helper: clear all module-level concurrency state. */
export function __resetConcurrency(): void {
  inFlight.clear();
  providerGate.clear();
}
