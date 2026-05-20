import { describe, it, expect } from "vitest";
import { formatTokenCount } from "./tokens";

describe("formatTokenCount", () => {
  it("returns 0 for null/undefined", () => {
    expect(formatTokenCount(null)).toBe("0");
    expect(formatTokenCount(undefined)).toBe("0");
  });

  it("returns plain integers under 1k", () => {
    expect(formatTokenCount(0)).toBe("0");
    expect(formatTokenCount(42)).toBe("42");
    expect(formatTokenCount(999)).toBe("999");
  });

  it("formats thousands with one decimal under 10k", () => {
    expect(formatTokenCount(4000)).toBe("4k");
    expect(formatTokenCount(4900)).toBe("4.9k");
    expect(formatTokenCount(9990)).toBe("10k");
  });

  it("rounds thousands at and above 10k", () => {
    expect(formatTokenCount(42000)).toBe("42k");
    expect(formatTokenCount(999_499)).toBe("999k");
  });

  it("formats millions", () => {
    expect(formatTokenCount(1_500_000)).toBe("1.5M");
    expect(formatTokenCount(12_000_000)).toBe("12M");
  });

  it("accepts bigint", () => {
    expect(formatTokenCount(4000n)).toBe("4k");
  });

  it("clamps negatives to 0", () => {
    expect(formatTokenCount(-1)).toBe("0");
  });
});
