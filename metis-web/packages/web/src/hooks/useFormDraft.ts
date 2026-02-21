import { useState, useCallback } from "react";

/**
 * A useState wrapper that persists state to sessionStorage.
 * Falls back to regular useState behavior if sessionStorage is unavailable.
 */
export function useFormDraft<T>(
  key: string,
  initialValue: T,
): [T, (value: T) => void, () => void] {
  const [state, setState] = useState<T>(() => {
    try {
      const stored = sessionStorage.getItem(key);
      if (stored !== null) {
        return JSON.parse(stored) as T;
      }
    } catch {
      // sessionStorage unavailable or corrupt data — use initial value
    }
    return initialValue;
  });

  const setValue = useCallback(
    (value: T) => {
      setState(value);
      try {
        sessionStorage.setItem(key, JSON.stringify(value));
      } catch {
        // Quota exceeded or sessionStorage unavailable — continue with in-memory state
      }
    },
    [key],
  );

  const clearDraft = useCallback(() => {
    try {
      sessionStorage.removeItem(key);
    } catch {
      // sessionStorage unavailable — nothing to clear
    }
  }, [key]);

  return [state, setValue, clearDraft];
}
