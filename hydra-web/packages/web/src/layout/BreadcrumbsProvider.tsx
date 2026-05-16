import { useCallback, useMemo, useState, type ReactNode } from "react";
import type { BreadcrumbItem } from "./Breadcrumbs";
import { BreadcrumbsContext, type BreadcrumbsState } from "./useBreadcrumbs";

const EMPTY_STATE: BreadcrumbsState = { items: [], current: null };

export function BreadcrumbsProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<BreadcrumbsState>(EMPTY_STATE);

  const setBreadcrumbs = useCallback(
    (items: BreadcrumbItem[], current: string, currentKind?: "code") =>
      setState({ items, current, currentKind }),
    [],
  );
  const clearBreadcrumbs = useCallback(() => setState(EMPTY_STATE), []);

  const value = useMemo(
    () => ({ state, setBreadcrumbs, clearBreadcrumbs }),
    [state, setBreadcrumbs, clearBreadcrumbs],
  );

  return (
    <BreadcrumbsContext.Provider value={value}>{children}</BreadcrumbsContext.Provider>
  );
}
