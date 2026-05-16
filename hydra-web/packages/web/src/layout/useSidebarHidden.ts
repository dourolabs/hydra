import { useCallback, useState } from "react";

export const SIDEBAR_HIDDEN_STORAGE_KEY = "hydra-sidebar-hidden";

function readHidden(): boolean {
  if (typeof window === "undefined") return false;
  try {
    const stored = window.localStorage.getItem(SIDEBAR_HIDDEN_STORAGE_KEY);
    if (stored === "1") return true;
    if (stored === "0") return false;
    // Default visible on every breakpoint. The mobile drawer CSS starts the
    // sidebar off-screen anyway; we don't need React state to track that.
    return false;
  } catch {
    return false;
  }
}

function writeHidden(hidden: boolean): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(SIDEBAR_HIDDEN_STORAGE_KEY, hidden ? "1" : "0");
  } catch {
    /* localStorage unavailable; ignore */
  }
}

export function useSidebarHidden(): {
  hidden: boolean;
  hide: () => void;
  show: () => void;
} {
  const [hidden, setHidden] = useState<boolean>(readHidden);
  const hide = useCallback(() => {
    setHidden(true);
    writeHidden(true);
  }, []);
  const show = useCallback(() => {
    setHidden(false);
    writeHidden(false);
  }, []);
  return { hidden, hide, show };
}
