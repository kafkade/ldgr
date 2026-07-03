import { describe, expect, it } from "vitest";
import { cacheKey, normalizeSymbols } from "../src/normalize.js";

describe("normalizeSymbols", () => {
  it("uppercases, sorts and de-dupes tickers", () => {
    expect(normalizeSymbols("msft,aapl,MSFT")).toEqual(["AAPL", "MSFT"]);
  });

  it("treats AAPL,MSFT and MSFT,AAPL identically", () => {
    expect(normalizeSymbols("AAPL,MSFT")).toEqual(normalizeSymbols("MSFT,AAPL"));
  });

  it("trims surrounding whitespace and drops empties", () => {
    expect(normalizeSymbols(" aapl , , msft ")).toEqual(["AAPL", "MSFT"]);
  });

  it("lowercases when upper=false (CoinGecko ids)", () => {
    expect(normalizeSymbols("Bitcoin,ETHEREUM", false)).toEqual(["bitcoin", "ethereum"]);
  });

  it("returns an empty array for null or blank input", () => {
    expect(normalizeSymbols(null)).toEqual([]);
    expect(normalizeSymbols("   ")).toEqual([]);
    expect(normalizeSymbols(",,")).toEqual([]);
  });
});

describe("cacheKey", () => {
  it("joins prefix and parts with colons", () => {
    expect(cacheKey("quote", "AAPL,MSFT")).toBe("quote:AAPL,MSFT");
    expect(cacheKey("historical", "AAPL", "2024-01-01", "2024-12-31")).toBe(
      "historical:AAPL:2024-01-01:2024-12-31",
    );
  });
});
