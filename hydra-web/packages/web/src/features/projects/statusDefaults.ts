import type { StatusDefinition } from "@hydra/api";
import { LABEL_COLOR_PALETTE } from "../../components/ColorPicker";

export function blankStatus(index: number): StatusDefinition {
  return {
    key: `status-${index + 1}`,
    label: "",
    color: LABEL_COLOR_PALETTE[index % LABEL_COLOR_PALETTE.length],
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
    on_enter: null,
    prompt_path: null,
    position: (index + 1) * 100,
  };
}

const STATUS_KEY_RE = /^[a-z0-9-]+$/;

export function validateStatusKey(
  key: string,
  existingKeys: ReadonlySet<string>,
): string | null {
  const trimmed = key.trim();
  if (!trimmed) return "Status key is required";
  if (!STATUS_KEY_RE.test(trimmed)) {
    return "Status key must be lowercase letters, digits, and dashes only";
  }
  if (existingKeys.has(trimmed)) {
    return `Status key '${trimmed}' already exists in this project`;
  }
  return null;
}

// Lowercase, dash-separated slug stripped of any character outside
// STATUS_KEY_RE. Collapsed and trimmed so callers get a key the wire layer
// already accepts.
export function slugifyStatusKey(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
