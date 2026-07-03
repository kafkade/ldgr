/**
 * ldgr market data proxy — Cloudflare Worker entrypoint.
 *
 * A shared caching proxy in front of free market-data providers. Multiple
 * clients requesting the same symbols are served from one cached upstream
 * response, keeping ldgr well within provider rate limits. See ADR-007.
 *
 * Routes (served under api.ldgr.dev/market/*):
 *   GET /health
 *   GET /quote?symbols=AAPL,MSFT
 *   GET /crypto?ids=bitcoin,ethereum
 *   GET /forex
 *   GET /historical?symbol=AAPL&start=2024-01-01&end=2024-12-31
 */

import { getOrFetch } from "./cache.js";
import { cacheKey, normalizeSymbols } from "./normalize.js";
import {
  fetchCoinGeckoPrices,
  fetchEcbForex,
  fetchYahooHistorical,
  fetchYahooQuotes,
  UpstreamError,
} from "./providers.js";
import { TTL, type Env } from "./types.js";

const CORS_HEADERS: Record<string, string> = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
  "Access-Control-Max-Age": "86400",
};

/** Thrown for client input errors; surfaced as HTTP 400. */
class BadRequestError extends Error {}

function json(body: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...CORS_HEADERS, ...extra },
  });
}

function proxied(
  body: string,
  contentType: string,
  cached: boolean,
  ttlSeconds: number,
): Response {
  return new Response(body, {
    status: 200,
    headers: {
      "Content-Type": contentType,
      "Cache-Control": `public, max-age=${ttlSeconds}`,
      "X-Cache": cached ? "HIT" : "MISS",
      ...CORS_HEADERS,
    },
  });
}

/** Strip a leading `/market` prefix and any trailing slash. */
function routePath(pathname: string): string {
  let p = pathname;
  if (p === "/market") p = "/";
  else if (p.startsWith("/market/")) p = p.slice("/market".length);
  if (p.length > 1 && p.endsWith("/")) p = p.replace(/\/+$/, "");
  return p || "/";
}

function requireParam(url: URL, name: string): string {
  const value = url.searchParams.get(name);
  if (!value || !value.trim()) {
    throw new BadRequestError(`missing required query parameter: ${name}`);
  }
  return value.trim();
}

async function route(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url);
  const path = routePath(url.pathname);

  if (path === "/health") {
    return json({ status: "ok", service: "ldgr-market-proxy", time: new Date().toISOString() });
  }

  if (path === "/quote") {
    const symbols = normalizeSymbols(requireParam(url, "symbols"), true);
    if (symbols.length === 0) throw new BadRequestError("no valid symbols provided");
    const key = cacheKey("quote", symbols.join(","));
    const { response, cached } = await getOrFetch(env, ctx, key, TTL.QUOTE, () =>
      fetchYahooQuotes(symbols),
    );
    return proxied(response.body, response.contentType, cached, TTL.QUOTE);
  }

  if (path === "/crypto") {
    const ids = normalizeSymbols(requireParam(url, "ids"), false);
    if (ids.length === 0) throw new BadRequestError("no valid ids provided");
    const key = cacheKey("crypto", ids.join(","));
    const { response, cached } = await getOrFetch(env, ctx, key, TTL.CRYPTO, () =>
      fetchCoinGeckoPrices(ids, env),
    );
    return proxied(response.body, response.contentType, cached, TTL.CRYPTO);
  }

  if (path === "/forex") {
    const key = cacheKey("forex", "eur");
    const { response, cached } = await getOrFetch(env, ctx, key, TTL.FOREX, () =>
      fetchEcbForex(),
    );
    return proxied(response.body, response.contentType, cached, TTL.FOREX);
  }

  if (path === "/historical") {
    const symbol = normalizeSymbols(requireParam(url, "symbol"), true)[0];
    if (!symbol) throw new BadRequestError("no valid symbol provided");
    const start = requireParam(url, "start");
    const end = requireParam(url, "end");
    const key = cacheKey("historical", symbol, start, end);
    const { response, cached } = await getOrFetch(env, ctx, key, TTL.HISTORICAL, () =>
      fetchYahooHistorical(symbol, start, end),
    );
    return proxied(response.body, response.contentType, cached, TTL.HISTORICAL);
  }

  return json({ error: "not found", path }, 404);
}

export async function handleRequest(
  request: Request,
  env: Env,
  ctx: ExecutionContext,
): Promise<Response> {
  if (request.method === "OPTIONS") {
    return new Response(null, { status: 204, headers: CORS_HEADERS });
  }
  if (request.method !== "GET") {
    return json({ error: "method not allowed" }, 405, { Allow: "GET, OPTIONS" });
  }

  try {
    return await route(request, env, ctx);
  } catch (err) {
    if (err instanceof BadRequestError) {
      return json({ error: err.message }, 400);
    }
    if (err instanceof UpstreamError) {
      return json({ error: err.message, provider: err.provider }, 502);
    }
    const message = err instanceof Error ? err.message : "internal error";
    return json({ error: "internal error", detail: message }, 500);
  }
}

export default { fetch: handleRequest } satisfies ExportedHandler<Env>;
