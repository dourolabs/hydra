import { useRef, useCallback, type RefObject } from "react";

const SWIPE_THRESHOLD = 10;
const COMMIT_THRESHOLD = 100;

interface SwipeState {
  startX: number;
  startY: number;
  swiping: boolean;
  decided: boolean;
}

export function useSwipeToArchive(
  onArchive: () => void,
): {
  rowRef: RefObject<HTMLDivElement | null>;
  onTouchStart: (e: React.TouchEvent) => void;
  onTouchMove: (e: React.TouchEvent) => void;
  onTouchEnd: () => void;
} {
  const rowRef = useRef<HTMLDivElement | null>(null);
  const stateRef = useRef<SwipeState>({
    startX: 0,
    startY: 0,
    swiping: false,
    decided: false,
  });

  const onTouchStart = useCallback((e: React.TouchEvent) => {
    const touch = e.touches[0];
    stateRef.current = {
      startX: touch.clientX,
      startY: touch.clientY,
      swiping: false,
      decided: false,
    };
    const el = rowRef.current;
    if (el) {
      el.style.transition = "none";
    }
  }, []);

  const onTouchMove = useCallback((e: React.TouchEvent) => {
    const state = stateRef.current;
    const touch = e.touches[0];
    const dx = state.startX - touch.clientX;
    const dy = Math.abs(touch.clientY - state.startY);

    if (!state.decided) {
      if (dy > SWIPE_THRESHOLD) {
        state.decided = true;
        state.swiping = false;
        return;
      }
      if (dx > SWIPE_THRESHOLD) {
        state.decided = true;
        state.swiping = true;
      } else {
        return;
      }
    }

    if (!state.swiping) return;

    const offset = Math.max(0, dx);
    const el = rowRef.current;
    if (el) {
      el.style.transform = `translateX(-${offset}px)`;
    }
  }, []);

  const onTouchEnd = useCallback(() => {
    const state = stateRef.current;
    const el = rowRef.current;

    if (!el) return;

    if (!state.swiping) {
      el.style.transform = "";
      el.style.transition = "";
      return;
    }

    const current = el.style.transform;
    const match = current.match(/translateX\(-(\d+(?:\.\d+)?)px\)/);
    const offset = match ? parseFloat(match[1]) : 0;

    if (offset >= COMMIT_THRESHOLD) {
      el.style.transition = "transform 200ms ease-out, opacity 200ms ease-out";
      el.style.transform = "translateX(-100%)";
      el.style.opacity = "0";
      el.addEventListener(
        "transitionend",
        () => {
          onArchive();
        },
        { once: true },
      );
    } else {
      el.style.transition = "transform 200ms ease-out";
      el.style.transform = "translateX(0)";
      el.addEventListener(
        "transitionend",
        () => {
          el.style.transition = "";
          el.style.transform = "";
        },
        { once: true },
      );
    }

    stateRef.current = { startX: 0, startY: 0, swiping: false, decided: false };
  }, [onArchive]);

  return { rowRef, onTouchStart, onTouchMove, onTouchEnd };
}
