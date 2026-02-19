import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";

export type SortOption = "newest" | "oldest" | "updated" | "status";

export interface IssueFilters {
  statuses: string[];
  assignee: string;
  type: string;
  sort: SortOption;
}

const VALID_SORTS: Set<string> = new Set(["newest", "oldest", "updated", "status"]);

function parseSortParam(value: string | null): SortOption {
  if (value && VALID_SORTS.has(value)) return value as SortOption;
  return "newest";
}

export function useIssueFilters() {
  const [searchParams, setSearchParams] = useSearchParams();

  const filters: IssueFilters = useMemo(() => {
    const statusParam = searchParams.get("status");
    const statuses = statusParam ? statusParam.split(",").filter(Boolean) : [];
    const assignee = searchParams.get("assignee") ?? "";
    const type = searchParams.get("type") ?? "";
    const sort = parseSortParam(searchParams.get("sort"));

    return { statuses, assignee, type, sort };
  }, [searchParams]);

  const setFilters = useCallback(
    (updates: Partial<IssueFilters>) => {
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

        if (updates.sort !== undefined) {
          if (updates.sort !== "newest") {
            next.set("sort", updates.sort);
          } else {
            next.delete("sort");
          }
        }

        return next;
      });
    },
    [setSearchParams],
  );

  return { filters, setFilters };
}
