import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { handleRequest } from "../src/index.js";
import { __resetConcurrency } from "../src/throttle.js";
import { makeEnv, TestExecutionContext, textResponse } from "./helpers.js";

const YAHOO_CHART = (symbol: string) =>
  JSON.stringify({ chart: { result: [{ meta: { symbol, regularMarketPrice: 100 } }] } });

const COINGECKO_PRICE = JSON.stringify({ bitcoin: { usd: 67500 } });
const ECB_XML = '<?xml version="1.0"?><Cube currency="USD" rate="1.08"/>';

function call(url: string, env: ReturnType<typeof makeEnv>["env"], ctx: TestExecutionContext) {
  return handleRequest(new Request(url), env, ctx as unknown as ExecutionContext);
}

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  __resetConcurrency();
  fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("routing & CORS", () => {
  it("responds to /health with ok status and CORS", async () => {
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/health", env, ctx);
    expect(res.status).toBe(200);
    expect(res.headers.get("Access-Control-Allow-Origin")).toBe("*");
    const body = (await res.json()) as { status: string; service: string };
    expect(body.status).toBe("ok");
    expect(body.service).toBe("ldgr-market-proxy");
  });

  it("handles the /market prefix and a bare path equivalently", async () => {
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const withPrefix = await call("https://api.ldgr.dev/market/health", env, ctx);
    const bare = await call("https://api.ldgr.dev/health", env, ctx);
    expect(withPrefix.status).toBe(200);
    expect(bare.status).toBe(200);
  });

  it("answers OPTIONS preflight with 204 and CORS headers", async () => {
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await handleRequest(
      new Request("https://api.ldgr.dev/market/quote", { method: "OPTIONS" }),
      env,
      ctx as unknown as ExecutionContext,
    );
    expect(res.status).toBe(204);
    expect(res.headers.get("Access-Control-Allow-Methods")).toContain("GET");
  });

  it("rejects non-GET methods with 405", async () => {
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await handleRequest(
      new Request("https://api.ldgr.dev/market/quote", { method: "POST" }),
      env,
      ctx as unknown as ExecutionContext,
    );
    expect(res.status).toBe(405);
  });

  it("returns 404 for unknown routes", async () => {
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/unknown", env, ctx);
    expect(res.status).toBe(404);
  });
});

describe("/quote", () => {
  it("fetches Yahoo on a miss and serves from KV on the next hit", async () => {
    fetchMock.mockImplementation((url: string) =>
      Promise.resolve(textResponse(YAHOO_CHART(url.includes("AAPL") ? "AAPL" : "MSFT"))),
    );
    const { env, kv } = makeEnv();
    const ctx = new TestExecutionContext();

    const miss = await call("https://api.ldgr.dev/market/quote?symbols=AAPL", env, ctx);
    expect(miss.status).toBe(200);
    expect(miss.headers.get("X-Cache")).toBe("MISS");
    await ctx.settle();
    expect(kv.store.has("quote:AAPL")).toBe(true);

    const hit = await call("https://api.ldgr.dev/market/quote?symbols=AAPL", env, ctx);
    expect(hit.headers.get("X-Cache")).toBe("HIT");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("normalizes symbol order so AAPL,MSFT and MSFT,AAPL share a cache entry", async () => {
    fetchMock.mockImplementation((url: string) =>
      Promise.resolve(textResponse(YAHOO_CHART(url.includes("AAPL") ? "AAPL" : "MSFT"))),
    );
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();

    const first = await call("https://api.ldgr.dev/market/quote?symbols=AAPL,MSFT", env, ctx);
    await ctx.settle();
    expect(first.headers.get("X-Cache")).toBe("MISS");

    const second = await call("https://api.ldgr.dev/market/quote?symbols=MSFT,AAPL", env, ctx);
    expect(second.headers.get("X-Cache")).toBe("HIT");
    // Two upstream fetches for the two symbols, none for the reversed request.
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("combines multiple symbols into one chart.result array", async () => {
    fetchMock.mockImplementation((url: string) =>
      Promise.resolve(textResponse(YAHOO_CHART(url.includes("AAPL") ? "AAPL" : "MSFT"))),
    );
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/quote?symbols=AAPL,MSFT", env, ctx);
    const body = (await res.json()) as { chart: { result: { meta: { symbol: string } }[] } };
    expect(body.chart.result.map((r) => r.meta.symbol).sort()).toEqual(["AAPL", "MSFT"]);
  });

  it("returns 400 when symbols is missing", async () => {
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/quote", env, ctx);
    expect(res.status).toBe(400);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("de-duplicates concurrent misses into a single upstream fetch", async () => {
    fetchMock.mockImplementation(
      () =>
        new Promise((resolve) =>
          setTimeout(() => resolve(textResponse(YAHOO_CHART("AAPL"))), 15),
        ),
    );
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const [a, b] = await Promise.all([
      call("https://api.ldgr.dev/market/quote?symbols=AAPL", env, ctx),
      call("https://api.ldgr.dev/market/quote?symbols=AAPL", env, ctx),
    ]);
    expect(a.status).toBe(200);
    expect(b.status).toBe(200);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});

describe("/crypto", () => {
  it("proxies CoinGecko and lowercases ids for the cache key", async () => {
    fetchMock.mockResolvedValue(textResponse(COINGECKO_PRICE));
    const { env, kv } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/crypto?ids=Bitcoin", env, ctx);
    expect(res.status).toBe(200);
    await ctx.settle();
    expect(kv.store.has("crypto:bitcoin")).toBe(true);
    const url = fetchMock.mock.calls[0][0] as string;
    expect(url).toContain("ids=bitcoin");
    expect(url).toContain("simple/price");
  });
});

describe("/forex", () => {
  it("proxies the ECB XML with an xml content type", async () => {
    fetchMock.mockResolvedValue(textResponse(ECB_XML));
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/forex", env, ctx);
    expect(res.status).toBe(200);
    expect(res.headers.get("Content-Type")).toContain("xml");
    expect(await res.text()).toContain("currency=");
  });
});

describe("/historical", () => {
  it("builds a Yahoo chart URL with period timestamps", async () => {
    fetchMock.mockResolvedValue(textResponse(YAHOO_CHART("AAPL")));
    const { env, kv } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call(
      "https://api.ldgr.dev/market/historical?symbol=AAPL&start=2024-01-01&end=2024-12-31",
      env,
      ctx,
    );
    expect(res.status).toBe(200);
    await ctx.settle();
    expect(kv.store.has("historical:AAPL:2024-01-01:2024-12-31")).toBe(true);
    const url = fetchMock.mock.calls[0][0] as string;
    expect(url).toContain("period1=1704067200"); // 2024-01-01T00:00:00Z
    expect(url).toContain("period2=1735603200"); // 2024-12-31T00:00:00Z
  });

  it("returns 400 when required params are missing", async () => {
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/historical?symbol=AAPL", env, ctx);
    expect(res.status).toBe(400);
  });
});

describe("upstream failures", () => {
  it("returns 502 when the provider responds non-2xx", async () => {
    fetchMock.mockResolvedValue(textResponse("rate limited", 429));
    const { env } = makeEnv();
    const ctx = new TestExecutionContext();
    const res = await call("https://api.ldgr.dev/market/crypto?ids=bitcoin", env, ctx);
    expect(res.status).toBe(502);
    const body = (await res.json()) as { provider: string };
    expect(body.provider).toBe("coingecko");
  });
});
