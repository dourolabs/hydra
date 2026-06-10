import { describe, it, expect } from "vitest";
import { formatBinRange, formatDurationSeconds } from "../duration";

describe("formatDurationSeconds", () => {
  it("formats sub-minute values as seconds", () => {
    expect(formatDurationSeconds(0)).toBe("0s");
    expect(formatDurationSeconds(45)).toBe("45s");
  });

  it("formats minutes / hours / days with one decimal when small", () => {
    expect(formatDurationSeconds(60)).toBe("1m");
    expect(formatDurationSeconds(90)).toBe("1.5m");
    expect(formatDurationSeconds(3600)).toBe("1h");
    expect(formatDurationSeconds(18000)).toBe("5h");
    expect(formatDurationSeconds(86400)).toBe("1d");
    expect(formatDurationSeconds(86400 * 3)).toBe("3d");
  });

  it("returns a dash for invalid input", () => {
    expect(formatDurationSeconds(-1)).toBe("—");
    expect(formatDurationSeconds(NaN)).toBe("—");
  });
});

describe("formatBinRange", () => {
  it("formats closed bins as start–end", () => {
    expect(formatBinRange(3600, 14400)).toBe("1h–4h");
    expect(formatBinRange(0, 3600)).toBe("0s–1h");
    expect(formatBinRange(86400, 86400 * 3)).toBe("1d–3d");
  });

  it("formats the open-ended last bin with a `+` suffix", () => {
    expect(formatBinRange(86400 * 30, null)).toBe("30d+");
  });
});
