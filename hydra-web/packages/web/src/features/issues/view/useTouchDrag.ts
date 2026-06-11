import { useCallback, useEffect, useRef, useState } from "react";

interface TouchDragOptions<T> {
  /** Whether the drag interaction should be active (e.g. only on mobile). */
  enabled: boolean;
  /** Hold duration in ms before a touch becomes a drag. */
  delay?: number;
  /** Max movement in px during the hold before the timer cancels. */
  tolerance?: number;
  /** Payload to deliver to `onDrop` when the touch lands on a drop target. */
  payload: T;
  /** Returns the drop-target element (if any) for the given element under the touch. */
  resolveDropTarget: (el: Element | null) => Element | null;
  /** Called when a long-press drag ends over a resolved drop target. */
  onDrop: (payload: T, target: Element) => void;
}

interface TouchDragHandlers {
  onTouchStart: (e: React.TouchEvent<HTMLElement>) => void;
}

interface TouchDragState {
  isDragging: boolean;
  hoverTargetId: string | null;
}

/**
 * Long-press-then-drag for touch devices. HTML5 drag-and-drop does not fire
 * from touch events on mobile, so this hook synthesises an equivalent flow:
 * a hold (default 250ms) elevates a touch into "drag mode"; subsequent
 * touchmove updates a hover target via `document.elementFromPoint` hit
 * testing; `touchend` over a resolved drop target invokes `onDrop`.
 *
 * Returns `state` (so the caller can style the source as "dragging" and
 * highlight the hovered drop target via `data-touch-drag-over`) and
 * `handlers` (spread onto the source element).
 */
export function useTouchDrag<T>(opts: TouchDragOptions<T>): {
  state: TouchDragState;
  handlers: TouchDragHandlers;
} {
  const {
    enabled,
    delay = 250,
    tolerance = 5,
    payload,
    resolveDropTarget,
    onDrop,
  } = opts;

  const [state, setState] = useState<TouchDragState>({
    isDragging: false,
    hoverTargetId: null,
  });

  // Mutable refs hold the per-touch session so the document listeners (added
  // for `{ passive: false }`) close over fresh values without re-binding.
  const sessionRef = useRef<{
    startX: number;
    startY: number;
    holdTimer: ReturnType<typeof setTimeout> | null;
    dragging: boolean;
    lastTarget: Element | null;
    lastTargetId: string | null;
  } | null>(null);

  const cleanup = useCallback(() => {
    const s = sessionRef.current;
    if (!s) return;
    if (s.holdTimer) clearTimeout(s.holdTimer);
    sessionRef.current = null;
  }, []);

  // Document-level move/end listeners are bound once the hold elevates the
  // touch into a drag — keeping them out of React's passive synthetic system
  // so the page can be stopped from scrolling under the finger.
  useEffect(() => {
    if (!state.isDragging) return;

    const handleMove = (e: TouchEvent) => {
      const s = sessionRef.current;
      if (!s || !s.dragging) return;
      e.preventDefault();
      const t = e.touches[0];
      if (!t) return;
      const under = document.elementFromPoint(t.clientX, t.clientY);
      const dropTarget = resolveDropTarget(under);
      s.lastTarget = dropTarget;
      const id = dropTarget?.getAttribute("data-touch-drop-id") ?? null;
      if (id !== s.lastTargetId) {
        s.lastTargetId = id;
        setState((prev) => ({ ...prev, hoverTargetId: id }));
      }
    };

    const handleEnd = () => {
      const s = sessionRef.current;
      if (s && s.dragging && s.lastTarget) {
        onDrop(payload, s.lastTarget);
      }
      cleanup();
      setState({ isDragging: false, hoverTargetId: null });
    };

    const handleCancel = () => {
      cleanup();
      setState({ isDragging: false, hoverTargetId: null });
    };

    document.addEventListener("touchmove", handleMove, { passive: false });
    document.addEventListener("touchend", handleEnd);
    document.addEventListener("touchcancel", handleCancel);
    return () => {
      document.removeEventListener("touchmove", handleMove);
      document.removeEventListener("touchend", handleEnd);
      document.removeEventListener("touchcancel", handleCancel);
    };
  }, [state.isDragging, payload, onDrop, resolveDropTarget, cleanup]);

  const onTouchStart = useCallback(
    (e: React.TouchEvent<HTMLElement>) => {
      if (!enabled) return;
      // Multi-touch is treated as a pinch/zoom intent — bail out of any
      // pending hold so the user can interact with the page normally.
      if (e.touches.length > 1) {
        cleanup();
        return;
      }
      const t = e.touches[0];
      const session = {
        startX: t.clientX,
        startY: t.clientY,
        holdTimer: null as ReturnType<typeof setTimeout> | null,
        dragging: false,
        lastTarget: null as Element | null,
        lastTargetId: null as string | null,
      };
      sessionRef.current = session;

      // Pre-drag movement cancels the timer. Once `dragging` flips true the
      // document listeners take over and we no longer rely on these.
      const handlePreMove = (ev: TouchEvent) => {
        const s = sessionRef.current;
        if (!s || s.dragging) return;
        const pt = ev.touches[0];
        if (!pt) return;
        const dx = pt.clientX - s.startX;
        const dy = pt.clientY - s.startY;
        if (Math.hypot(dx, dy) > tolerance) {
          document.removeEventListener("touchmove", handlePreMove);
          document.removeEventListener("touchend", handlePreEnd);
          document.removeEventListener("touchcancel", handlePreEnd);
          cleanup();
        }
      };
      const handlePreEnd = () => {
        document.removeEventListener("touchmove", handlePreMove);
        document.removeEventListener("touchend", handlePreEnd);
        document.removeEventListener("touchcancel", handlePreEnd);
        cleanup();
      };
      document.addEventListener("touchmove", handlePreMove, { passive: true });
      document.addEventListener("touchend", handlePreEnd);
      document.addEventListener("touchcancel", handlePreEnd);

      session.holdTimer = setTimeout(() => {
        document.removeEventListener("touchmove", handlePreMove);
        document.removeEventListener("touchend", handlePreEnd);
        document.removeEventListener("touchcancel", handlePreEnd);
        const s = sessionRef.current;
        if (!s) return;
        s.dragging = true;
        // The state flip mounts the document-level move/end listeners.
        setState({ isDragging: true, hoverTargetId: null });
      }, delay);
    },
    [enabled, delay, tolerance, cleanup],
  );

  return { state, handlers: { onTouchStart } };
}
