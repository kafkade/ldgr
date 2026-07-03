/**
 * Symbol-list normalization and cache-key construction.
 *
 * Normalization is what makes the shared cache effective: `AAPL,MSFT` and
 * `MSFT,AAPL` (and ` aapl , msft `) must all resolve to the same cache key so
 * that every client requesting the same basket of symbols is served from a
 * single upstream fetch.
 */

/**
 * Normalize a comma-separated symbol list into a canonical, sorted, de-duped
 * array. Whitespace is trimmed, empties are dropped.
 *
 * @param raw the raw `symbols` / `ids` query parameter value
 * @param upper whether to uppercase entries (true for tickers, false for
 *   CoinGecko coin ids which are lowercase slugs like `bitcoin`)
 */
export function normalizeSymbols(raw: string | null, upper = true): string[] {
  if (!raw) return [];
  const seen = new Set<string>();
  for (const part of raw.split(",")) {
    const trimmed = part.trim();
    if (!trimmed) continue;
    seen.add(upper ? trimmed.toUpperCase() : trimmed.toLowerCase());
  }
  return [...seen].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/** Build a stable cache key from a route prefix and canonical parts. */
export function cacheKey(prefix: string, ...parts: string[]): string {
  return [prefix, ...parts].join(":");
}
