import { createContext, useContext, useEffect } from "react";
import type { BreadcrumbItem } from "./Breadcrumbs";

export interface BreadcrumbsState {
  items: BreadcrumbItem[];
  current: string | null;
}

export interface BreadcrumbsContextValue {
  state: BreadcrumbsState;
  setBreadcrumbs: (items: BreadcrumbItem[], current: string) => void;
  clearBreadcrumbs: () => void;
}

export const BreadcrumbsContext = createContext<BreadcrumbsContextValue | null>(
  null,
);

export function useBreadcrumbsState(): BreadcrumbsState {
  const ctx = useContext(BreadcrumbsContext);
  if (!ctx) {
    throw new Error(
      "useBreadcrumbsState must be used within a BreadcrumbsProvider",
    );
  }
  return ctx.state;
}

/**
 * Pages call this to publish breadcrumbs to the SiteHeader. Inputs are
 * serialized to a stable key so callers don't have to memoize.
 */
export function useBreadcrumbs(items: BreadcrumbItem[], current: string): void {
  const ctx = useContext(BreadcrumbsContext);
  if (!ctx) {
    throw new Error(
      "useBreadcrumbs must be used within a BreadcrumbsProvider",
    );
  }
  const { setBreadcrumbs, clearBreadcrumbs } = ctx;
  const key = JSON.stringify({ items, current });
  useEffect(() => {
    setBreadcrumbs(items, current);
    return () => clearBreadcrumbs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, setBreadcrumbs, clearBreadcrumbs]);
}
