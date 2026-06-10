import { describe, it, expect } from "vitest";
import { formatDuration, formatDurationSeconds, getRuntime } from "./time";

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

describe("formatDurationSeconds", () => {
  it("formats sub-minute values as seconds", () => {
    expect(formatDurationSeconds(0)).toBe("0s");
    expect(formatDurationSeconds(45)).toBe("45s");
  });

  it("formats minutes / hours / days, rounding one decimal when small", () => {
    expect(formatDurationSeconds(60)).toBe("1m");
    expect(formatDurationSeconds(90)).toBe("1.5m");
    expect(formatDurationSeconds(3600)).toBe("1h");
    expect(formatDurationSeconds(18000)).toBe("5h");
    expect(formatDurationSeconds(86400)).toBe("1d");
    expect(formatDurationSeconds(86400 * 3)).toBe("3d");
  });

  it("returns an em dash for negative or non-finite input", () => {
    expect(formatDurationSeconds(-1)).toBe("—");
    expect(formatDurationSeconds(NaN)).toBe("—");
    expect(formatDurationSeconds(Infinity)).toBe("—");
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
