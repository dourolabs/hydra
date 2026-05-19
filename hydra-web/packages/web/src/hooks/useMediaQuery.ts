import { useState, useEffect } from "react";

function getMatchMedia(): ((query: string) => MediaQueryList) | null {
  if (typeof window === "undefined") return null;
  if (typeof window.matchMedia !== "function") return null;
  return window.matchMedia.bind(window);
}

export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => {
    const mm = getMatchMedia();
    return mm ? mm(query).matches : false;
  });

  useEffect(() => {
    const mm = getMatchMedia();
    if (!mm) return;
    const mql = mm(query);
    setMatches(mql.matches);
    const handler = (e: MediaQueryListEvent) => setMatches(e.matches);
    mql.addEventListener("change", handler);
    return () => mql.removeEventListener("change", handler);
  }, [query]);

  return matches;
}
