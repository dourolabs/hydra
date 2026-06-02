import type { SearchConversationsQuery } from "@hydra/api";
import type { Filter } from "../filters";

/**
 * Translate FilterBar state + free-text `q` into the
 * `Partial<SearchConversationsQuery>` shape consumed by `useConversations` /
 * `apiClient.listConversations`. This is the only mapping from FilterBar →
 * server query: the Chats page no longer narrows client-side.
 *
 * Filter shapes:
 *   - `status` is single-select and forwards verbatim as `?status=<value>`.
 *   - `creator` is single-select; the option value is a Principal path
 *     (`users/<name>` / `agents/<name>`) but `SearchConversationsQuery.creator`
 *     expects a bare username, so we strip the prefix.
 *   - `op: "not_in"` is not server-applicable for either filter
 *     (`notInSupported` defaults to false on both definitions); such filters
 *     are silently dropped.
 *
 * Unlike the Issues page, there are no relation filters to resolve into an
 * `ids[]` set — `SearchConversationsQuery` has no `ids[]` field today. See
 * PR-4 issue for the deferred-relation-filter write-up.
 */
export interface BuildConversationsQueryArgs {
  filters: Filter[];
  q: string;
}

function stripUserPrefix(value: string): string {
  if (value.startsWith("users/")) return value.slice("users/".length);
  if (value.startsWith("agents/")) return value.slice("agents/".length);
  return value;
}

export function filtersToConversationsQuery({
  filters,
  q,
}: BuildConversationsQueryArgs): Partial<SearchConversationsQuery> {
  const out: Partial<SearchConversationsQuery> = {};
  for (const filter of filters) {
    if (filter.op !== "in") continue;
    if (filter.values.length === 0) continue;
    switch (filter.id) {
      case "status":
        out.status = filter.values[0] as SearchConversationsQuery["status"];
        break;
      case "creator":
        out.creator = stripUserPrefix(filter.values[0]);
        break;
      default:
        break;
    }
  }
  if (q.trim()) {
    out.q = q.trim();
  }
  return out;
}
