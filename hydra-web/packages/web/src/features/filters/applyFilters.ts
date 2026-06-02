import type { Filter, FilterDefinitions } from "./types";

/**
 * Filters `items` according to every active `filter`. AND-across filters.
 *
 * - When `filters` is empty, returns the input array unchanged (referentially)
 *   so downstream `useMemo` consumers stay stable.
 * - Filters whose `id` is unknown or whose `values` array is empty are skipped
 *   (a new chip with no selection must not hide everything).
 * - `op === "not_in"` negates the per-item result.
 */
export function applyFilters<T>(
  items: T[],
  filters: Filter[],
  defs: FilterDefinitions<T>,
): T[] {
  if (filters.length === 0) return items;

  const active = filters
    .map((f) => {
      const def = defs[f.id];
      if (!def) return null;
      if (f.values.length === 0) return null;
      return { def, filter: f };
    })
    .filter((x): x is { def: FilterDefinitions<T>[string]; filter: Filter } => x !== null);

  if (active.length === 0) return items;

  return items.filter((item) => {
    for (const { def, filter } of active) {
      const matches = def.apply(item, filter);
      const passes = filter.op === "not_in" ? !matches : matches;
      if (!passes) return false;
    }
    return true;
  });
}
