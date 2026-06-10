import { describe, it, expect } from "vitest";
import { formatBinRange } from "../duration";

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
