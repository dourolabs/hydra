import { useState, useCallback, useEffect } from "react";

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

export function useDocumentTreeExpandState(topLevelPaths: string[]): {
  expandedPaths: Set<string>;
  onToggle: (path: string) => void;
} {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => {
    const stored = loadExpandedPaths();
    if (stored !== null) {
      return stored;
    }
    // Default: expand top-level paths on first visit
    return new Set(topLevelPaths);
  });

  // When topLevelPaths arrive after initial render and nothing is stored,
  // expand them as defaults.
  useEffect(() => {
    if (topLevelPaths.length === 0) return;
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored !== null) return;
    setExpandedPaths((prev) => {
      if (prev.size > 0) return prev;
      const next = new Set(topLevelPaths);
      saveExpandedPaths(next);
      return next;
    });
  }, [topLevelPaths]);

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

  return { expandedPaths, onToggle };
}
