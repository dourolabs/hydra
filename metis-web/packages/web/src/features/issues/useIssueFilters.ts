import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";

export interface IssueFilterValues {
  statuses: string[];
  assignee: string;
  type: string;
  q: string;
}

export function useIssueFilters() {
  const [searchParams, setSearchParams] = useSearchParams();

  const filters: IssueFilterValues = useMemo(() => {
    const statusParam = searchParams.get("status");
    const statuses = statusParam ? statusParam.split(",").filter(Boolean) : [];
    const assignee = searchParams.get("assignee") ?? "";
    const type = searchParams.get("type") ?? "";
    const q = searchParams.get("q") ?? "";

    return { statuses, assignee, type, q };
  }, [searchParams]);

  const setFilters = useCallback(
    (updates: Partial<IssueFilterValues>) => {
      setSearchParams((prev) => {
        const next = new URLSearchParams(prev);

        if (updates.statuses !== undefined) {
          if (updates.statuses.length > 0) {
            next.set("status", updates.statuses.join(","));
          } else {
            next.delete("status");
          }
        }

        if (updates.assignee !== undefined) {
          if (updates.assignee) {
            next.set("assignee", updates.assignee);
          } else {
            next.delete("assignee");
          }
        }

        if (updates.type !== undefined) {
          if (updates.type) {
            next.set("type", updates.type);
          } else {
            next.delete("type");
          }
        }

        if (updates.q !== undefined) {
          if (updates.q) {
            next.set("q", updates.q);
          } else {
            next.delete("q");
          }
        }

        return next;
      });
    },
    [setSearchParams],
  );

  return { filters, setFilters };
}
