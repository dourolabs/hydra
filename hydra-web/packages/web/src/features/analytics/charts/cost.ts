import type { AgentName } from "@hydra/api";

/** Shared USD formatter for cost widgets — 2 decimals, e.g. `$1,234.56`. */
const USD = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

export function formatUsd(value: number): string {
  return USD.format(value);
}

/** UI label for the ad-hoc bucket (sessions without an `agent_name`). */
export const AD_HOC_LABEL = "Ad-hoc";

export function agentDisplayName(name: AgentName | null): string {
  return name ?? AD_HOC_LABEL;
}

/**
 * Deterministic jitter in `[-0.1, +0.1]` from a session id. Used by the
 * per-session scatter so points within an agent row spread out without
 * changing across re-renders or Playwright runs.
 */
export function sessionJitter(sessionId: string): number {
  let hash = 0;
  for (let i = 0; i < sessionId.length; i++) {
    hash = (hash * 31 + sessionId.charCodeAt(i)) | 0;
  }
  const normalized = ((hash % 1000) + 1000) % 1000;
  return (normalized / 1000) * 0.2 - 0.1;
}
