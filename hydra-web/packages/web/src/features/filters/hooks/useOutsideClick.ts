import { useEffect, type RefObject } from "react";

/**
 * Calls `onClose` on document mousedown outside `ref` AND on Escape keydown.
 * Listeners only attach while `enabled` is true.
 */
export function useOutsideClick(
  ref: RefObject<HTMLElement | null>,
  onClose: () => void,
  enabled: boolean,
): void {
  useEffect(() => {
    if (!enabled) return;
    const handleMouseDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (!target) return;
      if (ref.current?.contains(target)) return;
      onClose();
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [ref, onClose, enabled]);
}
