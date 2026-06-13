import { useCallback, useState } from "react";

const STORAGE_PREFIX = "hydra-viewer-wrap:";
const MOBILE_QUERY = "(max-width: 768px)";

function storageKey(key: string): string {
  return STORAGE_PREFIX + key;
}

function readInitial(key: string): boolean {
  if (typeof window === "undefined") return false;
  try {
    const stored = window.localStorage.getItem(storageKey(key));
    if (stored === "true") return true;
    if (stored === "false") return false;
  } catch {
    // localStorage may throw in private mode or when disabled
  }
  if (typeof window.matchMedia === "function") {
    return window.matchMedia(MOBILE_QUERY).matches;
  }
  return false;
}

/**
 * Wrap-mode toggle for code/log viewers. Returns `[wrap, setWrap]`.
 *
 * Default: wrap on mobile (≤768px), scroll on desktop. Once the user picks
 * a mode it sticks across viewports — persisted in localStorage under the
 * supplied `key`, so each viewer (e.g. "diff", "log") gets its own preference.
 */
export function useViewerWrap(key: string): [boolean, (next: boolean) => void] {
  const [wrap, setWrap] = useState<boolean>(() => readInitial(key));

  const update = useCallback(
    (next: boolean) => {
      setWrap(next);
      try {
        window.localStorage.setItem(storageKey(key), String(next));
      } catch {
        // ignore write failure (private mode, quota, etc.)
      }
    },
    [key],
  );

  return [wrap, update];
}
