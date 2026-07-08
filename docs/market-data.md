# Market Data

ldgr fetches prices from free public APIs to value your investment holdings —
stocks, ETFs, crypto, and foreign-currency balances — so your net worth reflects
current market values. This document explains the providers, how caching works,
what the shared proxy can and cannot see, how to configure it, and how to run
your own proxy.

> **ldgr is a net worth tracker, not a trading platform.** Market data is used
> only to value the holdings you already own as part of your overall financial
> picture. Prices are intentionally refreshed on the order of minutes, not
> seconds — there are no live quotes, order books, alerts, or trade execution.
> For investment decisions or real-time data, use a specialized tool (your
> brokerage, Bloomberg, etc.). See [ADR-007](adr/007-market-data-caching.md).

## Providers

ldgr ships with three built-in providers. All are free and require **no API
key**. The parsing lives in `crates/ldgr-core/src/market/`; the actual HTTP
fetching is done by platform code (CLI/iOS/web), never by the core library.

| Provider | Asset classes | Rate limit | API key |
| --- | --- | --- | --- |
| **Yahoo Finance** | Stocks, ETFs, mutual funds, indices, forex, crypto | ~2000 req/hr (unofficial, subject to change) | No |
| **CoinGecko** | Crypto | 5–15 req/min (anonymous), 30/min (free Demo key) | No |
| **ECB** | Forex (EUR-based daily reference rates) | No limit (single daily XML file) | No |

Notes and caveats:

- **Yahoo Finance** is an *unofficial* API with no published SLA. Rate limits can
  change without notice, and Yahoo may block IPs or require CAPTCHA at any time.
  Commercial use may violate Yahoo's Terms of Service — this provider is
  community-provided and not affiliated with Yahoo.
- **CoinGecko** returns daily close prices for historical data (not true OHLCV);
  the `open`/`high`/`low` fields are set equal to `close`. A free Demo API key
  raises the anonymous rate limit.
- **ECB** publishes official EUR-based reference rates once per business day.
  USD/X conversions are computed as `(1/EUR_USD) * EUR_X`.

Adding new providers (Alpha Vantage, Twelve Data, etc.) is documented in the
[Provider Development Guide](provider-development-guide.md).

## Caching

Caching happens at two layers (ADR-007). Together they mean most price lookups
never touch a provider at all, which keeps everyone comfortably inside the free
rate limits.

### Layer 1 — client-side cache (local, per device)

Every ldgr client caches prices locally with a time-to-live (TTL). Within the
TTL, a lookup returns the cached value with **no network request**.

| Data type | TTL | Rationale |
| --- | --- | --- |
| Intraday quotes | 15 minutes | Adequate for net worth tracking |
| Daily OHLCV (historical) | 24 hours | Historical data doesn't change |
| ECB forex rates | 24 hours | ECB publishes once daily |

The client cache is a standalone SQLite database at `~/.ldgr/market_cache.db`
(table `market_cache`). It is kept **separate from the encrypted `vault.db`** and
holds only public market data, so it needs no encryption and works even for
commands that never unlock a vault (e.g. `ldgr watch`).

### Layer 2 — shared proxy (edge cache, across all users)

An optional Cloudflare Worker at `https://api.ldgr.dev/market/` caches upstream
responses so that many users requesting the same symbols are served from a
single cached fetch. Symbol lists are normalized (case-folded, de-duplicated,
sorted) so `AAPL,MSFT` and `MSFT,AAPL` resolve to the same cache key. Proxy TTLs
match the client TTLs (15 min for quotes/crypto, 24 hr for historical/forex).

### Request flow

```text
1. Check local market_cache.db (TTL-based)
   → fresh hit? return immediately, no network.

2. Miss → fetch the shared proxy (api.ldgr.dev/market/…)
   → write the result through to the local cache, return.

3. Proxy unavailable (offline, non-2xx, or disabled)?
   → fall back to fetching the provider directly, cache that, return.
```

The proxy is an **optimization, not a dependency** — if it is unreachable or you
opt out, the client fetches providers directly, exactly as it would without the
proxy.

### Clearing / inspecting the cache

The local cache is managed with the `ldgr cache` command:

```sh
ldgr cache status   # show entry count, fresh vs expired, and hit rate
ldgr cache clear    # remove all cached prices
```

These commands operate only on `market_cache.db` and never touch your vault.

## Privacy

The shared proxy only ever sees **public market data**. It has no accounts, no
authentication, and no visibility into your finances.

| What the proxy sees | Sensitive? |
| --- | --- |
| Which symbols are requested (e.g. `AAPL`, `bitcoin`) | No — public market data |
| Request IP address | Minimal — ordinary web traffic, no login |
| How often prices are refreshed | Marginal — largely hidden by the local cache |

**What the proxy never sees:**

- Account balances, transaction history, or any vault data
- Your portfolio composition or allocation — a request for `AAPL` does **not**
  reveal that you hold AAPL, only that *someone* asked for the AAPL price
- Which user made a request (there is no authentication or user identity)
- Net worth or any derived figure

Only symbol names leave your device, and only when a lookup misses the local
cache. This is no different from any CDN-fronted public API.

## Configuration

Proxy behavior is controlled by the `LDGR_MARKET_PROXY` environment variable and
the `--no-proxy` flag. The flag applies to the CLI commands that fetch market
data (`ldgr watch`, `ldgr portfolio`) and always wins over the env var.

| Setting | Effect |
| --- | --- |
| `LDGR_MARKET_PROXY` unset (or empty) | Use the default shared proxy `https://api.ldgr.dev/market` |
| `LDGR_MARKET_PROXY=https://…` | Use a custom proxy (e.g. your own Worker) |
| `LDGR_MARKET_PROXY=none` | Disable the proxy; fetch providers directly (case-insensitive) |
| `--no-proxy` flag | Disable the proxy for a single invocation; overrides the env var |

```sh
# Use the default shared proxy (nothing to configure)
ldgr portfolio

# Point at your own self-hosted proxy
LDGR_MARKET_PROXY=https://market.example.com/market ldgr portfolio

# Never use any proxy — always fetch providers directly
LDGR_MARKET_PROXY=none ldgr watch AAPL MSFT BTC-USD

# Bypass the proxy for just this one run
ldgr portfolio --no-proxy
```

## Self-hosting the proxy

You don't need the shared proxy at all — with `LDGR_MARKET_PROXY=none` the client
fetches providers directly. But if you want the caching/rate-limit benefits
without relying on `api.ldgr.dev`, you can deploy the same Cloudflare Worker
yourself. The source lives in [`infra/market-proxy/`](../infra/market-proxy/).

```sh
cd infra/market-proxy
npm install
npm run typecheck   # tsc --noEmit
npm test            # vitest unit tests (mocked KV + fetch)

# First-time deployment:
# 1. Create the KV namespaces and paste the ids into wrangler.toml
wrangler kv namespace create MARKET_KV
wrangler kv namespace create MARKET_KV --preview

# 2. (Optional) add a CoinGecko Demo key to raise the crypto rate limit
wrangler secret put COINGECKO_API_KEY

# 3. Deploy
npm run deploy
```

Then point ldgr at it:

```sh
export LDGR_MARKET_PROXY=https://your-worker.example.com/market
```

The Worker exposes `/quote`, `/crypto`, `/forex`, `/historical`, and `/health`,
returning responses in the exact provider shapes ldgr-core already parses. It
runs comfortably within Cloudflare's free tier (100K Worker requests/day, 100K KV
reads/day, 1K KV writes/day). See the
[proxy README](../infra/market-proxy/README.md) for full route, caching, and
deployment details.

## Related documents

- [ADR-007: Market Data Caching](adr/007-market-data-caching.md) — the full design rationale
- [Provider Development Guide](provider-development-guide.md) — adding new market data providers
- [market-proxy README](../infra/market-proxy/README.md) — Worker routes, caching, and deployment
