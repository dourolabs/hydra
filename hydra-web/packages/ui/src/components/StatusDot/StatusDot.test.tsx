import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import { StatusDot } from "./StatusDot";
import type { BadgeStatus } from "../Badge/Badge";

vi.mock("./StatusDot.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const TONE_BY_STATUS: Array<[BadgeStatus, string]> = [
  ["open", "toneOpen"],
  ["in-progress", "toneInProgress"],
  ["closed", "toneClosed"],
  ["issue-closed", "toneClosed"],
  ["approved", "toneClosed"],
  ["failed", "toneFailed"],
  ["dropped", "toneDropped"],
  ["blocked", "toneBlocked"],
  ["pending", "toneInProgress"],
  ["running", "toneInProgress"],
  ["complete", "toneClosed"],
  ["changes-requested", "toneRejected"],
  ["merged", "toneClosed"],
  ["conv-active", "toneInProgress"],
  ["conv-idle", "toneOpen"],
  ["conv-closed", "toneClosed"],
];

const NEUTRAL_FALLBACK_STATUSES: BadgeStatus[] = ["created", "success", "unknown"];

describe("StatusDot", () => {
  it("renders an aria-hidden span with the base dot class", () => {
    const { container } = render(<StatusDot status="open" />);
    const span = container.querySelector("span");
    expect(span).not.toBeNull();
    expect(span?.getAttribute("aria-hidden")).toBe("true");
    expect(span?.className).toContain("dot");
  });

  it.each(TONE_BY_STATUS)("applies the %s tone class", (status, expectedTone) => {
    const { container } = render(<StatusDot status={status} />);
    const span = container.querySelector("span");
    expect(span?.className).toContain(expectedTone);
  });

  it.each(NEUTRAL_FALLBACK_STATUSES)(
    "falls back to toneNeutral for unmapped status %s",
    (status) => {
      const { container } = render(<StatusDot status={status} />);
      const span = container.querySelector("span");
      expect(span?.className).toContain("toneNeutral");
    },
  );

  it("appends a caller-supplied className", () => {
    const { container } = render(<StatusDot status="open" className="extra" />);
    const span = container.querySelector("span");
    expect(span?.className).toContain("extra");
    expect(span?.className).toContain("dot");
    expect(span?.className).toContain("toneOpen");
  });
});
