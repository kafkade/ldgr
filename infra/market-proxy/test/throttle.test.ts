import { afterEach, describe, expect, it } from "vitest";
import {
  __resetConcurrency,
  dedupe,
  MIN_UPSTREAM_INTERVAL_MS,
  throttle,
} from "../src/throttle.js";

afterEach(() => __resetConcurrency());

describe("dedupe", () => {
  it("coalesces concurrent calls for the same key into one invocation", async () => {
    let calls = 0;
    const fn = async () => {
      calls += 1;
      await new Promise((r) => setTimeout(r, 20));
      return "value";
    };

    const [a, b, c] = await Promise.all([
      dedupe("k", fn),
      dedupe("k", fn),
      dedupe("k", fn),
    ]);

    expect(calls).toBe(1);
    expect([a, b, c]).toEqual(["value", "value", "value"]);
  });

  it("runs a fresh invocation after the previous one settled", async () => {
    let calls = 0;
    const fn = async () => {
      calls += 1;
      return calls;
    };
    await dedupe("k", fn);
    await dedupe("k", fn);
    expect(calls).toBe(2);
  });

  it("clears in-flight state even when the fn rejects", async () => {
    await expect(
      dedupe("k", async () => {
        throw new Error("boom");
      }),
    ).rejects.toThrow("boom");
    // A subsequent call should invoke the fn again (not a cached rejection).
    await expect(dedupe("k", async () => "ok")).resolves.toBe("ok");
  });
});

describe("throttle", () => {
  it("spaces successive upstream fetches to the same provider by ~1s", async () => {
    const times: number[] = [];
    const record = async () => {
      times.push(Date.now());
      return true;
    };

    await throttle("yahoo", record);
    await throttle("yahoo", record);

    expect(times).toHaveLength(2);
    expect(times[1] - times[0]).toBeGreaterThanOrEqual(MIN_UPSTREAM_INTERVAL_MS - 50);
  });

  it("returns the fn result", async () => {
    await expect(throttle("ecb", async () => 42)).resolves.toBe(42);
  });
});
