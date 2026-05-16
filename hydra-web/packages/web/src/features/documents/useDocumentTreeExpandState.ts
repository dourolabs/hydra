import { useState, useCallback } from "react";

const STORAGE_KEY = "hydra:document-tree-expanded";

function loadExpandedPaths(): Set<string> | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return new Set(parsed);
      }
    }
  } catch {
    // localStorage unavailable or corrupt data
  }
  return null;
}

function saveExpandedPaths(paths: Set<string>): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...paths]));
  } catch {
    // Quota exceeded or localStorage unavailable
  }
}

export interface DocumentTreeExpandState {
  expandedPaths: Set<string>;
  onToggle: (path: string) => void;
  /**
   * Auto-expand the given paths if and only if no expansion state has ever
   * been persisted. Intended for first-visit defaults once top-level paths
   * are known.
   */
  autoExpand: (paths: string[]) => void;
}

export function useDocumentTreeExpandState(): DocumentTreeExpandState {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(
    () => loadExpandedPaths() ?? new Set(),
  );

  const onToggle = useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      saveExpandedPaths(next);
      return next;
    });
  }, []);

  const autoExpand = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    if (localStorage.getItem(STORAGE_KEY) !== null) return;
    setExpandedPaths((prev) => {
      if (prev.size > 0) return prev;
      const next = new Set(paths);
      saveExpandedPaths(next);
      return next;
    });
  }, []);

  return { expandedPaths, onToggle, autoExpand };
}
