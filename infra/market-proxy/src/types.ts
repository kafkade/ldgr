/**
 * Shared types and constants for the ldgr market data proxy.
 *
 * See ADR-007 (docs/adr/007-market-data-caching.md) for the design rationale.
 */

/** Cloudflare Worker environment bindings. */
export interface Env {
  /** KV namespace used to cache upstream responses. */
  MARKET_KV: KVNamespace;
  /** Optional CoinGecko Demo API key (set via `wrangler secret put`). */
  COINGECKO_API_KEY?: string;
}

/** Cache TTLs in seconds, per ADR-007. */
export const TTL = {
  /** Intraday quotes — adequate for net worth tracking. */
  QUOTE: 15 * 60,
  /** Crypto spot prices — treated like quotes. */
  CRYPTO: 15 * 60,
  /** Daily OHLCV history — does not change intraday. */
  HISTORICAL: 24 * 60 * 60,
  /** ECB reference rates — published once per day. */
  FOREX: 24 * 60 * 60,
} as const;

/** Logical upstream providers, used for per-provider rate limiting. */
export type Provider = "yahoo" | "coingecko" | "ecb";

/** A cached / freshly fetched upstream response ready to return to a client. */
export interface UpstreamResponse {
  body: string;
  contentType: string;
}
