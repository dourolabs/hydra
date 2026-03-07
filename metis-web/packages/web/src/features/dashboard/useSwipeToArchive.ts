import { useEffect, useRef, useCallback } from "react";
import styles from "./ItemRow.module.css";

const DEFAULT_COMMIT_THRESHOLD = 100;
const FALLBACK_TIMEOUT_MS = 250;

interface UseSwipeToArchiveOptions {
  onArchive: () => void;
  commitThreshold?: number;
  enabled?: boolean;
}

export function useSwipeToArchive(
  ref: React.RefObject<HTMLElement | null>,
  { onArchive, commitThreshold = DEFAULT_COMMIT_THRESHOLD, enabled = true }: UseSwipeToArchiveOptions,
) {
  const startXRef = useRef(0);
  const currentXRef = useRef(0);
  const swipingRef = useRef(false);
  const committedRef = useRef(false);
  const onArchiveRef = useRef(onArchive);
  onArchiveRef.current = onArchive;

  const cleanupRef = useRef<(() => void) | null>(null);

  const handleTouchStart = useCallback(
    (e: TouchEvent) => {
      if (!enabled) return;
      startXRef.current = e.touches[0].clientX;
      currentXRef.current = 0;
      swipingRef.current = true;
      committedRef.current = false;
      const el = ref.current;
      if (el) {
        el.classList.remove(styles.swipeCommit, styles.swipeSnapBack);
        el.classList.add(styles.swiping);
      }
    },
    [enabled, ref],
  );

  const handleTouchMove = useCallback(
    (e: TouchEvent) => {
      if (!swipingRef.current) return;
      const el = ref.current;
      if (!el) return;
      const deltaX = e.touches[0].clientX - startXRef.current;
      const clamped = Math.min(0, deltaX);
      currentXRef.current = clamped;
      el.style.setProperty("--swipe-x", `${clamped}px`);
    },
    [ref],
  );

  const handleTouchEnd = useCallback(() => {
    if (!swipingRef.current) return;
    swipingRef.current = false;
    const el = ref.current;
    if (!el) return;

    const delta = currentXRef.current;
    el.classList.remove(styles.swiping);
    el.style.removeProperty("--swipe-x");

    if (Math.abs(delta) >= commitThreshold) {
      committedRef.current = true;
      el.classList.add(styles.swipeCommit);

      let fired = false;
      const fire = () => {
        if (fired) return;
        fired = true;
        clearTimeout(timeoutId);
        el.removeEventListener("transitionend", onTransitionEnd);
        cleanupRef.current = null;
        onArchiveRef.current();
      };

      const onTransitionEnd = (e: TransitionEvent) => {
        if (e.propertyName === "transform") {
          fire();
        }
      };
      el.addEventListener("transitionend", onTransitionEnd);
      const timeoutId = setTimeout(fire, FALLBACK_TIMEOUT_MS);

      cleanupRef.current = () => {
        clearTimeout(timeoutId);
        el.removeEventListener("transitionend", onTransitionEnd);
      };
    } else {
      el.classList.add(styles.swipeSnapBack);
    }
  }, [ref, commitThreshold]);

  useEffect(() => {
    const el = ref.current;
    if (!el || !enabled) return;

    el.addEventListener("touchstart", handleTouchStart, { passive: true });
    el.addEventListener("touchmove", handleTouchMove, { passive: true });
    el.addEventListener("touchend", handleTouchEnd);

    return () => {
      el.removeEventListener("touchstart", handleTouchStart);
      el.removeEventListener("touchmove", handleTouchMove);
      el.removeEventListener("touchend", handleTouchEnd);
      if (cleanupRef.current) {
        cleanupRef.current();
        cleanupRef.current = null;
      }
      // Only fire onArchive on unmount if a swipe was committed but hasn't fired yet
      // (e.g., optimistic update removed the element during transition).
      // Do NOT fire if no swipe was committed — the user may have navigated away.
    };
  }, [ref, enabled, handleTouchStart, handleTouchMove, handleTouchEnd]);
}
