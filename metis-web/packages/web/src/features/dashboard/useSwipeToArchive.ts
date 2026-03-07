import { useEffect, useRef, useCallback } from "react";

const DEFAULT_COMMIT_THRESHOLD = 100;
const TRANSITION_DURATION_MS = 200;
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
  const onArchiveRef = useRef(onArchive);
  onArchiveRef.current = onArchive;

  const cleanupRef = useRef<(() => void) | null>(null);

  const handleTouchStart = useCallback(
    (e: TouchEvent) => {
      if (!enabled) return;
      startXRef.current = e.touches[0].clientX;
      currentXRef.current = 0;
      swipingRef.current = true;
      const el = ref.current;
      if (el) {
        el.style.transition = "none";
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
      // Only allow left swipe (negative delta)
      const clamped = Math.min(0, deltaX);
      currentXRef.current = clamped;
      el.style.transform = `translateX(${clamped}px)`;
    },
    [ref],
  );

  const handleTouchEnd = useCallback(() => {
    if (!swipingRef.current) return;
    swipingRef.current = false;
    const el = ref.current;
    if (!el) return;

    const delta = currentXRef.current;

    if (Math.abs(delta) >= commitThreshold) {
      // Commit: slide off-screen
      el.style.transition = `transform ${TRANSITION_DURATION_MS}ms ease-out, opacity ${TRANSITION_DURATION_MS}ms ease-out`;
      el.style.transform = `translateX(-100%)`;
      el.style.opacity = "0";

      let fired = false;
      const fire = () => {
        if (fired) return;
        fired = true;
        clearTimeout(timeoutId);
        el.removeEventListener("transitionend", onTransitionEnd);
        onArchiveRef.current();
      };

      const onTransitionEnd = (e: TransitionEvent) => {
        if (e.propertyName === "transform") {
          fire();
        }
      };
      el.addEventListener("transitionend", onTransitionEnd);
      const timeoutId = setTimeout(fire, FALLBACK_TIMEOUT_MS);

      // Store cleanup for unmount
      cleanupRef.current = () => {
        clearTimeout(timeoutId);
        el.removeEventListener("transitionend", onTransitionEnd);
        // If unmounting before either fired, still archive
        if (!fired) {
          fired = true;
          onArchiveRef.current();
        }
      };
    } else {
      // Snap back
      el.style.transition = `transform ${TRANSITION_DURATION_MS}ms ease-out`;
      el.style.transform = "translateX(0)";
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
    };
  }, [ref, enabled, handleTouchStart, handleTouchMove, handleTouchEnd]);
}
