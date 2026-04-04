import type { IssueStatus, PatchStatus } from "@hydra/api";

const STORAGE_KEY = "hydra:v2:dashboard:filters";

export interface PersistedFilterState {
  filterRootId: string;
  selectedIssueStatus: IssueStatus | null;
  selectedPatchStatus: PatchStatus | null;
  selectedLabelId: string | null;
  searchValue: string;
}

export function readFilterState(): PersistedFilterState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return null;
    if (typeof parsed.filterRootId !== "string") return null;
    // Accept both new single-value format and legacy array format
    if (
      parsed.selectedIssueStatus !== null &&
      parsed.selectedIssueStatus !== undefined &&
      typeof parsed.selectedIssueStatus !== "string"
    )
      return null;
    if (
      parsed.selectedPatchStatus !== null &&
      parsed.selectedPatchStatus !== undefined &&
      typeof parsed.selectedPatchStatus !== "string"
    )
      return null;
    return parsed as PersistedFilterState;
  } catch {
    return null;
  }
}

export function writeFilterState(state: PersistedFilterState) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // ignore
  }
}
