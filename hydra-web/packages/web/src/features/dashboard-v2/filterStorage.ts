import type { IssueStatus, PatchStatus } from "@hydra/api";

const STORAGE_KEY = "hydra:v2:dashboard:filters";

export interface PersistedFilterState {
  filterRootId: string;
  selectedIssueStatuses: IssueStatus[];
  selectedPatchStatuses: PatchStatus[];
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
    if (!Array.isArray(parsed.selectedIssueStatuses)) return null;
    if (!Array.isArray(parsed.selectedPatchStatuses)) return null;
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
