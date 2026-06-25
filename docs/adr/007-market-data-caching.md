# ADR-007: Market Data Caching — Shared Proxy with Cloudflare Workers

**Status**: Proposed  
**Date**: 2026-05-06  
**Decision makers**: @kafkade  

## Context

ldgr fetches market data from free-tier APIs (Yahoo Finance, CoinGecko, ECB) to value investment holdings for net worth tracking. These providers have rate limits:

| Provider | Free Limit | Data Type |
| --- | --- | --- |
| Yahoo Finance | ~2000 req/hr (unofficial) | Stocks, ETFs, indices |
| CoinGecko | 5–15 req/min (no key) | Crypto |
| ECB | Daily update | Forex |

### The problem

- **Multiple users tracking the same assets**: If 100 users all track AAPL, BTC, and EUR/USD, each client hitting providers directly means 100× the requests for identical data.
- **Rate limit exhaustion**: Free tiers are per-IP. A web app (single server IP) or popular CLI tool (many IPs but same API) can exhaust limits quickly.
- **Stale data is acceptable**: ldgr is a net worth tracker, not a trading platform. Prices refreshed every 15–60 minutes are perfectly adequate. Users don't need sub-second quotes.
- **Zero-knowledge constraint**: The caching layer sees *which symbols are popular* but NOT any user's financial data. Symbol popularity is not sensitive — it's public market data.

### Options considered

1. **No cache (current)**: Each client fetches directly. Simple but doesn't scale.
2. **Client-side SQLite cache**: Each device caches locally. Reduces repeat requests from the same device but doesn't help across users.
3. **Self-hosted proxy (Azure)**: A shared proxy that caches responses. Works but adds hosting cost and operational burden.
4. **Cloudflare Workers + KV**: A lightweight edge proxy that caches market data at the CDN level. Minimal cost, global distribution, no server to manage.

## Decision

**Hybrid: client-side TTL cache + Cloudflare Workers shared proxy.**

### Layer 1: Client-side cache (local SQLite)

Every ldgr client caches market data locally with a TTL:

| Data Type | TTL | Rationale |
| --- | --- | --- |
| Intraday quotes | 15 minutes | Adequate for net worth tracking |
| Daily OHLCV | 24 hours | Historical data doesn't change |
| ECB forex rates | 24 hours | ECB publishes once daily |

On cache hit within TTL → return cached data, no network request. This alone eliminates most repeat requests from a single device.

**Implementation note:** The client cache is a standalone `SQLite` database
(`~/.ldgr/market_cache.db`, table `market_cache(key, data, stored_at, ttl_secs)`),
kept separate from the encrypted `vault.db`. It holds only public market data, so it
needs no encryption and remains usable by standalone commands (e.g. `ldgr watch`) that
never unlock a vault. Entries are hydrated on startup, evicted when expired, and
written through on every fetch. `ldgr cache status` / `ldgr cache clear` manage it.

### Layer 2: Shared proxy (Cloudflare Workers + KV)

A Cloudflare Worker at `api.ldgr.dev/market/` acts as a caching proxy:

```text
Client → api.ldgr.dev/market/quote?symbols=AAPL,MSFT
  → Worker checks KV cache (key: "quote:AAPL,MSFT")
  → Cache hit & fresh? Return cached response (< 1ms)
  → Cache miss? Fetch from Yahoo Finance → store in KV → return
```

**Why Cloudflare Workers:**

- **Free tier**: 100,000 requests/day, 1,000 KV reads/day (far exceeds needs for a niche open-source app).
- **Global edge**: Responses served from the nearest Cloudflare POP — low latency worldwide.
- **No server to manage**: No Azure VM, no Docker, no uptime monitoring. Workers are serverless.
- **Already using Cloudflare**: Domain and hosting already on Cloudflare — zero new vendor onboarding.
- **KV storage**: Key-value store with built-in TTL. Write a response, set expiration, done.

**Worker architecture:**

```text
┌─────────────────────────────────────────┐
│  Cloudflare Worker: api.ldgr.dev/market │
│                                         │
│  Routes:                                │
│    GET /quote?symbols=X,Y,Z             │
│    GET /historical?symbol=X&range=...   │
│    GET /forex                           │
│    GET /crypto?ids=bitcoin,ethereum     │
│                                         │
│  Logic:                                 │
│    1. Normalize & sort symbol list      │
│    2. Check KV for cached response      │
│    3. If fresh → return (fast path)     │
│    4. If stale/miss → fetch upstream    │
│    5. Store in KV with TTL              │
│    6. Return response                   │
│                                         │
│  Rate limiting:                         │
│    - Upstream fetch throttled to 1/sec  │
│    - Concurrent dedup (only one fetch   │
│      per symbol set in flight)          │
│                                         │
│  KV TTLs:                               │
│    - Quotes: 15 min                     │
│    - Historical: 24 hours               │
│    - Forex: 24 hours                    │
└─────────────────────────────────────────┘
```

**Symbol batching**: Yahoo Finance supports multi-symbol queries. The Worker normalizes and sorts symbol lists so `AAPL,MSFT` and `MSFT,AAPL` hit the same cache key.

**Concurrent request dedup**: If 10 clients request AAPL at the same instant (cache miss), the Worker makes ONE upstream request and serves all 10 from the result. This uses Cloudflare's `waitUntil` pattern or a simple in-memory lock.

### Client integration

The ldgr client uses a two-level lookup:

```text
1. Check local SQLite cache (TTL-based)
   → Hit? Return. Done.

2. Fetch from api.ldgr.dev/market/... (shared proxy)
   → Response is already cached at the edge for other users
   → Store in local SQLite cache
   → Return
```

If the shared proxy is unavailable (offline, rate limited, user opts out), the client falls back to direct provider requests — the same behavior as today. The proxy is an optimization, not a hard dependency.

### Privacy analysis

| What the proxy sees | Sensitive? |
| --- | --- |
| Which symbols are requested | No — public market data |
| Request IP addresses | Minimal — standard web traffic, no login |
| How often a user refreshes | Marginal — mitigated by client-side cache |
| User's portfolio composition | **No** — the proxy sees popular symbols, not which user holds what. A request for "AAPL" doesn't reveal that you own AAPL. |

**What the proxy never sees:**

- Account balances, transaction history, vault data
- Which user made which request (no authentication)
- Portfolio allocation or net worth

### Cost analysis (Cloudflare free tier)

| Resource | Free Limit | Expected Usage | Headroom |
| --- | --- | --- | --- |
| Worker requests | 100,000/day | ~1,000/day (early) | 100× |
| KV reads | 100,000/day | ~500/day | 200× |
| KV writes | 1,000/day | ~100/day | 10× |
| KV storage | 1 GB | < 1 MB | 1000× |

Even with 1,000 daily active users, the free tier is more than sufficient. The client-side cache ensures most requests never reach the proxy at all.

## Consequences

- **Positive**: Market data scales to thousands of users without hitting provider rate limits. Zero hosting cost. Global low-latency responses.
- **Positive**: Client-side cache provides offline support — prices available even without network.
- **Positive**: Graceful degradation — proxy is optional, clients fall back to direct fetching.
- **Negative**: Adds a shared service dependency (api.ldgr.dev) — but it's optional and the app works without it.
- **Negative**: Cloudflare free tier could change — but the Worker is ~50 lines of code, trivially portable to any serverless platform (Deno Deploy, AWS Lambda@Edge, Vercel Edge Functions).
- **Negative**: Symbol-level usage analytics could theoretically be inferred by Cloudflare — but this is no different from any CDN-fronted API.

## Future considerations

- **Self-hosted option**: Users who don't want to use the shared proxy can set `LDGR_MARKET_PROXY=none` or point to their own Worker deployment.
- **Azure fallback**: If Cloudflare free tier becomes insufficient, an Azure Functions equivalent is straightforward.
- **WebSocket streaming**: For future TUI watchlist features, consider a WebSocket-based push model instead of polling.
