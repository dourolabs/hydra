import type { StatusDefinition } from "@hydra/api";

/**
 * Synthesize a placeholder {@link StatusDefinition} for tests that need
 * to populate the response-shaped `Issue.status` / `IssueSummary.status`
 * field without wiring up a real project status list. The key is the
 * only field most test assertions read; label/color/flags get neutral
 * defaults.
 */
export function makeStatusDef(key: string): StatusDefinition {
  return {
    key,
    label: "",
    color: "#888888",
    position: 0,
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
  };
}
