import { useEffect, type RefObject } from "react";

// Persist a scroll container's scrollLeft/scrollTop in sessionStorage so a
// route-level component (e.g. the Issues board) returns to the user's last
// horizontal/vertical scroll position after a round-trip through a detail
// page. Per-tab-session by design — opening the page in a new tab starts
// fresh; reloading within the same tab keeps the position.
//
// Restore-on-mount + save-on-unmount. Also saves on every scroll (rAF
// throttled) so external unmount paths that skip cleanup (HMR, dev
// reloads) still capture the latest position.
//
// Restore is retried via ResizeObserver on the inner content until the
// target position takes hold or 1.5s pass. This handles the common case
// where the component mounts before its async data has loaded: setting
// scrollLeft to 800 against a still-empty kanban clamps to 0, so we
// re-apply each time the content grows. Hard time bound prevents us from
// fighting the user if the destination page is genuinely shorter now.
export function useScrollRestoration(
  key: string | null,
  ref: RefObject<HTMLElement | null>,
): void {
  useEffect(() => {
    if (!key) return;
    const el = ref.current;
    if (!el) return;

    let saved: { left?: number; top?: number } | null = null;
    try {
      const raw = window.sessionStorage.getItem(key);
      if (raw) saved = JSON.parse(raw);
    } catch {
      /* swallow parse errors — restoration is best-effort */
    }

    const targetLeft = saved?.left ?? 0;
    const targetTop = saved?.top ?? 0;
    const needsRestore = !!saved && (targetLeft > 0 || targetTop > 0);

    let restoring = false;
    let observer: ResizeObserver | null = null;
    let deadline: ReturnType<typeof setTimeout> | undefined;

    const applyTarget = (): boolean => {
      if (targetLeft) el.scrollLeft = targetLeft;
      if (targetTop) el.scrollTop = targetTop;
      return (
        Math.abs(el.scrollLeft - targetLeft) < 1 &&
        Math.abs(el.scrollTop - targetTop) < 1
      );
    };

    const stopRestoring = () => {
      restoring = false;
      observer?.disconnect();
      observer = null;
      if (deadline) clearTimeout(deadline);
      deadline = undefined;
    };

    if (needsRestore) {
      restoring = true;
      if (applyTarget()) {
        restoring = false;
      } else if (typeof ResizeObserver !== "undefined") {
        // First attempt clamped — content not yet big enough. Re-apply each
        // time the inner content grows; once it accommodates the target,
        // stopRestoring tears down the observer. .body's own box doesn't
        // change as data loads, but its first child (the kanban) does — so
        // that's what we observe.
        observer = new ResizeObserver(() => {
          if (!restoring) return;
          if (applyTarget()) stopRestoring();
        });
        if (el.firstElementChild) observer.observe(el.firstElementChild);
        deadline = setTimeout(stopRestoring, 1500);
      } else {
        restoring = false;
      }
    }

    let rafId = 0;
    const save = () => {
      // Suppress saves during the restore window so the scroll events we
      // emit programmatically don't overwrite the saved position with the
      // clamped value.
      if (restoring) return;
      try {
        window.sessionStorage.setItem(
          key,
          JSON.stringify({ left: el.scrollLeft, top: el.scrollTop }),
        );
      } catch {
        /* quota / private-mode errors — ignore */
      }
    };
    const onScroll = () => {
      if (rafId) return;
      rafId = window.requestAnimationFrame(() => {
        rafId = 0;
        save();
      });
    };
    el.addEventListener("scroll", onScroll, { passive: true });

    return () => {
      el.removeEventListener("scroll", onScroll);
      if (rafId) window.cancelAnimationFrame(rafId);
      stopRestoring();
      save();
    };
  }, [key, ref]);
}
