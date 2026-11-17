import { describe, expect, it } from "vitest";
import { backoffDelay } from "../../src/backoff.js";

describe("backoffDelay", () => {
  it("double à chaque tentative et plafonne", () => {
    const o = { initialDelayMs: 100, maxDelayMs: 1000, factor: 2, jitter: 0 };
    expect([0, 1, 2, 3, 4, 10].map((a) => backoffDelay(a, o))).toEqual([100, 200, 400, 800, 1000, 1000]);
  });

  it("la part aléatoire réduit le délai sans jamais dépasser le plafond", () => {
    const o = { initialDelayMs: 1000, maxDelayMs: 1000, jitter: 0.5 };
    expect(backoffDelay(0, o, () => 0)).toBe(1000);
    expect(backoffDelay(0, o, () => 1)).toBe(500);
    expect(backoffDelay(3, o, () => 0.5)).toBe(750);
  });

  it("valeurs par défaut raisonnables", () => {
    expect(backoffDelay(0, {}, () => 0)).toBe(500);
    expect(backoffDelay(100, {}, () => 0)).toBe(10_000);
  });
});
