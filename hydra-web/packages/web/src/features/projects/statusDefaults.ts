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
