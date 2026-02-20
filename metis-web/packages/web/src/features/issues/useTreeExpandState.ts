import { useState, useCallback } from "react";

const STORAGE_KEY = "metis:issue-tree-collapsed";

function loadCollapsedIds(): Set<string> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return new Set(parsed);
      }
    }
  } catch {
    // localStorage unavailable or corrupt data — fall back to empty set
  }
  return new Set();
}

function saveCollapsedIds(ids: Set<string>): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...ids]));
  } catch {
    // Quota exceeded or localStorage unavailable — continue with in-memory state
  }
}

export function useTreeExpandState(): {
  collapsedIds: Set<string>;
  onToggle: (id: string) => void;
} {
  const [collapsedIds, setCollapsedIds] = useState<Set<string>>(loadCollapsedIds);

  const onToggle = useCallback((id: string) => {
    setCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      saveCollapsedIds(next);
      return next;
    });
  }, []);

  return { collapsedIds, onToggle };
}
