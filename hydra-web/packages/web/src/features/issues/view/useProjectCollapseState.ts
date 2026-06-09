import { useCallback, useState } from "react";

export const PROJECT_COLLAPSE_STORAGE_KEY = "hydra:board-project-collapsed";

function loadCollapsedIds(): Set<string> {
  if (typeof window === "undefined") return new Set();
  try {
    const raw = window.localStorage.getItem(PROJECT_COLLAPSE_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return new Set(parsed.filter((x): x is string => typeof x === "string"));
      }
    }
  } catch {
    /* localStorage unavailable or corrupt — fall back to empty set */
  }
  return new Set();
}

function saveCollapsedIds(ids: Set<string>): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      PROJECT_COLLAPSE_STORAGE_KEY,
      JSON.stringify([...ids]),
    );
  } catch {
    /* Quota exceeded or unavailable — continue with in-memory state */
  }
}

export function useProjectCollapseState(): {
  isCollapsed: (projectId: string) => boolean;
  onToggle: (projectId: string) => void;
} {
  const [collapsedIds, setCollapsedIds] = useState<Set<string>>(loadCollapsedIds);

  const onToggle = useCallback((projectId: string) => {
    setCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(projectId)) {
        next.delete(projectId);
      } else {
        next.add(projectId);
      }
      saveCollapsedIds(next);
      return next;
    });
  }, []);

  const isCollapsed = useCallback(
    (projectId: string) => collapsedIds.has(projectId),
    [collapsedIds],
  );

  return { isCollapsed, onToggle };
}
