import { describe, it, expect } from "vitest";
import { formatDuration, getRuntime } from "./time";

describe("formatDuration", () => {
  it("formats seconds", () => {
    expect(formatDuration(5000)).toBe("5s");
  });

  it("formats minutes and seconds", () => {
    expect(formatDuration(125000)).toBe("2m 5s");
  });

  it("formats hours and minutes", () => {
    expect(formatDuration(3720000)).toBe("1h 2m");
  });

  it("returns 0s for zero", () => {
    expect(formatDuration(0)).toBe("0s");
  });

  it("clamps negative values to 0s", () => {
    expect(formatDuration(-1000)).toBe("0s");
    expect(formatDuration(-500)).toBe("0s");
    expect(formatDuration(-999999)).toBe("0s");
  });
});

describe("getRuntime", () => {
  it("returns em dash when startTime is null", () => {
    expect(getRuntime(null, null)).toBe("\u2014");
  });

  it("returns em dash when startTime is undefined", () => {
    expect(getRuntime(undefined, undefined)).toBe("\u2014");
  });

  it("formats duration between start and end", () => {
    const start = "2026-01-01T00:00:00Z";
    const end = "2026-01-01T00:01:05Z";
    expect(getRuntime(start, end)).toBe("1m 5s");
  });

  it("never returns a negative runtime", () => {
    // endTime before startTime simulates clock skew
    const start = "2026-01-01T00:01:00Z";
    const end = "2026-01-01T00:00:00Z";
    expect(getRuntime(start, end)).toBe("0s");
  });
});
