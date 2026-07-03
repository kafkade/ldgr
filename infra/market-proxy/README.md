# ldgr market data proxy

A Cloudflare Worker that acts as a **shared caching proxy** for market data. It
sits in front of the free-tier providers ldgr already uses (Yahoo Finance,
CoinGecko, ECB) so that many users requesting the same symbols are served from
one cached upstream response, keeping the app well within provider rate limits.

Deployed at **`https://api.ldgr.dev/market/`**. See
[ADR-007](../../docs/adr/007-market-data-caching.md) for the full design.

> The proxy only ever sees **public market data** — never any user's vault,
> balances, or portfolio. It caches which *symbols* are popular, not who holds
> what. It is an optimization: clients fall back to fetching providers directly
> if it is unavailable.

## Routes

| Route | Upstream | Cache TTL |
| --- | --- | --- |
| `GET /health` | — | — |
| `GET /quote?symbols=AAPL,MSFT` | Yahoo Finance (v8 chart) | 15 min |
| `GET /crypto?ids=bitcoin,ethereum` | CoinGecko (`simple/price`) | 15 min |
| `GET /forex` | ECB daily reference rates (XML) | 24 hr |
| `GET /historical?symbol=AAPL&start=2024-01-01&end=2024-12-31` | Yahoo Finance | 24 hr |

Responses are returned in the exact shape ldgr-core's parsers expect
(`crates/ldgr-core/src/market/`), so the proxy is a drop-in replacement for the
direct provider URLs. Every response includes:

- `X-Cache: HIT | MISS` — whether it was served from KV
- `Cache-Control: public, max-age=<ttl>`
- permissive CORS headers (`Access-Control-Allow-Origin: *`) for the web app

## How caching works

1. **Normalize** the symbol list — uppercased (tickers) or lowercased
   (CoinGecko ids), trimmed, de-duplicated and **sorted**, so `AAPL,MSFT` and
   `MSFT,AAPL` resolve to the same cache key.
2. **Look up** the key in Cloudflare KV. Fresh hit → return immediately.
3. **On a miss**, fetch upstream, store in KV with the route's TTL (via
   `waitUntil`, so the write never delays the response), and return.
4. **Concurrent misses** for the same key are de-duplicated onto a single
   upstream fetch within the isolate.
5. **Upstream fetches** to each provider are throttled to ~1 request/second.

## Local development

```sh
cd infra/market-proxy
npm install

npm run typecheck   # tsc --noEmit
npm test            # vitest unit tests (mocked KV + fetch)
npm run dev         # wrangler dev (needs a preview KV namespace)
```

## Deployment

The domain `ldgr.dev` is already on Cloudflare. First-time setup:

```sh
# 1. Create the KV namespaces and paste the ids into wrangler.toml
wrangler kv namespace create MARKET_KV
wrangler kv namespace create MARKET_KV --preview

# 2. (Optional) add a CoinGecko Demo key to raise the crypto rate limit
wrangler secret put COINGECKO_API_KEY

# 3. Deploy
npm run deploy
```

`wrangler.toml` binds the worker to the route `api.ldgr.dev/market/*`. The
worker strips the `/market` prefix internally, so `/market/quote` is served by
the `/quote` handler.

## Client integration

The ldgr client uses this proxy as the primary fetch endpoint, with the local
SQLite cache in front of it and direct provider requests as a fallback
(ADR-007 Layer 1 + Layer 2). For each market-data request the client:

1. checks its local `market_cache.db` (TTL-based) — a hit returns immediately;
2. on a miss, fetches `api.ldgr.dev/market/...` (this proxy) and writes the
   result through to the local cache;
3. if the proxy is unavailable (network error or non-2xx), falls back to the
   direct provider URL and caches that instead.

Because the proxy returns responses in the exact provider shapes, the same
`ldgr-core` parsers handle both proxy and direct responses.

Configuration (applies to `ldgr watch` and `ldgr portfolio`):

| Setting | Effect |
| --- | --- |
| `LDGR_MARKET_PROXY` unset | Use the default proxy `https://api.ldgr.dev/market` |
| `LDGR_MARKET_PROXY=https://…` | Use a custom proxy (e.g. your own Worker deployment) |
| `LDGR_MARKET_PROXY=none` | Disable the proxy; fetch providers directly |
| `--no-proxy` flag | Disable the proxy for a single invocation (overrides the env var) |

Only symbol names are ever sent to the proxy — never vault, balance, or
portfolio data.

## Cost

Comfortably within Cloudflare's free tier (100K Worker req/day, 100K KV
reads/day, 1K KV writes/day). The client-side cache means most requests never
reach the proxy at all — see ADR-007's cost analysis.
