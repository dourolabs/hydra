import { useEffect } from "react";
import { useIsMobile } from "../../hooks/useIsMobile";

/**
 * Register a global Cmd/Ctrl-K listener that toggles the search modal.
 * Ignores presses that are already consumed by the focused element
 * (e.g. a code editor that handles its own shortcut). Disabled on
 * mobile viewports, where there is typically no physical keyboard.
 */
export function useGlobalSearchShortcut(toggle: () => void): void {
  const isMobile = useIsMobile();
  useEffect(() => {
    if (isMobile) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      const isK = event.key === "k" || event.key === "K";
      if (!isK) return;
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.altKey || event.shiftKey) return;
      const target = event.target as HTMLElement | null;
      if (target?.isContentEditable) return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggle, isMobile]);
}
