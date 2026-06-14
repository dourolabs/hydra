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
export function useScrollRestoration(
  key: string | null,
  ref: RefObject<HTMLElement | null>,
): void {
  useEffect(() => {
    if (!key) return;
    const el = ref.current;
    if (!el) return;

    try {
      const raw = window.sessionStorage.getItem(key);
      if (raw) {
        const parsed = JSON.parse(raw) as { left?: number; top?: number };
        if (typeof parsed.left === "number") el.scrollLeft = parsed.left;
        if (typeof parsed.top === "number") el.scrollTop = parsed.top;
      }
    } catch {
      /* swallow parse / storage errors — restoration is best-effort */
    }

    let rafId = 0;
    const save = () => {
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
      save();
    };
  }, [key, ref]);
}
