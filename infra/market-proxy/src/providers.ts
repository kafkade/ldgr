/**
 * Upstream provider fetchers.
 *
 * Each function fetches raw data from a free market-data provider and returns
 * it in the exact shape ldgr-core's parsers already expect, so the proxy is a
 * drop-in cache in front of the same endpoints the client would otherwise call
 * directly (see crates/ldgr-core/src/market/{yahoo,coingecko,ecb}.rs):
 *
 *   - Yahoo quotes/history  -> `{ "chart": { "result": [...] } }` JSON
 *   - CoinGecko crypto      -> `simple/price` JSON, passed through verbatim
 *   - ECB forex             -> `eurofxref-daily.xml`, passed through verbatim
 */

import { throttle } from "./throttle.js";
import type { Env, UpstreamResponse } from "./types.js";

const YAHOO_CHART_BASE = "https://query1.finance.yahoo.com/v8/finance/chart/";
const COINGECKO_BASE = "https://api.coingecko.com/api/v3";
const ECB_DAILY_URL = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";

// Yahoo rejects requests without a browser-like User-Agent.
const USER_AGENT =
  "Mozilla/5.0 (compatible; ldgr-market-proxy/0.1; +https://github.com/kafkade/ldgr)";

/** Raised when an upstream provider returns a non-2xx response. */
export class UpstreamError extends Error {
  constructor(
    public readonly provider: string,
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "UpstreamError";
  }
}

async function getText(
  provider: "yahoo" | "coingecko" | "ecb",
  url: string,
  headers: Record<string, string> = {},
): Promise<string> {
  const res = await throttle(provider, () =>
    fetch(url, { headers: { "User-Agent": USER_AGENT, ...headers } }),
  );
  if (!res.ok) {
    throw new UpstreamError(provider, res.status, `${provider} responded ${res.status}`);
  }
  return res.text();
}

/** Convert `YYYY-MM-DD` to a Unix timestamp in seconds (UTC midnight). */
export function dateToUnix(date: string): number {
  const ms = Date.parse(`${date}T00:00:00Z`);
  if (Number.isNaN(ms)) throw new Error(`invalid date: ${date}`);
  return Math.floor(ms / 1000);
}

/**
 * Fetch current quotes for one or more symbols from Yahoo Finance.
 *
 * Yahoo's v8 chart endpoint is single-symbol, so we fetch each symbol and
 * combine the individual `chart.result[0]` entries into one array — matching
 * the multi-result shape ldgr-core's `parse_quotes` iterates over.
 */
export async function fetchYahooQuotes(symbols: string[]): Promise<UpstreamResponse> {
  const results: unknown[] = [];
  for (const symbol of symbols) {
    const url = `${YAHOO_CHART_BASE}${encodeURIComponent(symbol)}?interval=1d&range=1d`;
    const text = await getText("yahoo", url);
    const json = JSON.parse(text) as { chart?: { result?: unknown[] } };
    const result = json.chart?.result?.[0];
    if (result) results.push(result);
  }
  return {
    body: JSON.stringify({ chart: { result: results, error: null } }),
    contentType: "application/json",
  };
}

/** Fetch daily OHLCV history for a single symbol from Yahoo Finance. */
export async function fetchYahooHistorical(
  symbol: string,
  start: string,
  end: string,
): Promise<UpstreamResponse> {
  const period1 = dateToUnix(start);
  const period2 = dateToUnix(end);
  const url =
    `${YAHOO_CHART_BASE}${encodeURIComponent(symbol)}` +
    `?period1=${period1}&period2=${period2}&interval=1d`;
  const body = await getText("yahoo", url);
  return { body, contentType: "application/json" };
}

/** Fetch crypto spot prices from CoinGecko's `simple/price` endpoint. */
export async function fetchCoinGeckoPrices(
  ids: string[],
  env: Env,
): Promise<UpstreamResponse> {
  const url =
    `${COINGECKO_BASE}/simple/price?ids=${encodeURIComponent(ids.join(","))}` +
    `&vs_currencies=usd&include_24hr_change=true&include_market_cap=true&include_24hr_vol=true`;
  const headers: Record<string, string> = {};
  if (env.COINGECKO_API_KEY) headers["x-cg-demo-api-key"] = env.COINGECKO_API_KEY;
  const body = await getText("coingecko", url, headers);
  return { body, contentType: "application/json" };
}

/** Fetch EUR-based daily reference rates from the ECB (XML). */
export async function fetchEcbForex(): Promise<UpstreamResponse> {
  const body = await getText("ecb", ECB_DAILY_URL);
  return { body, contentType: "application/xml" };
}
