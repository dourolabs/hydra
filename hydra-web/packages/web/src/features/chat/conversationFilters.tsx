import { useMemo } from "react";
import { Icons, type BadgeStatus } from "@hydra/ui";
import type { ConversationStatus, ConversationSummary } from "@hydra/api";
import type { FilterDefinitions, FilterOption } from "../filters";
import { useUserOptions } from "../filters/options/userOptions";
import { statusOptions } from "../filters/options/statusOptions";
import {
  CONVERSATION_STATUS_LABELS,
  CONVERSATION_STATUS_TONES,
} from "./conversationStatusBadge";

// `unknown` is intentionally omitted: it is not a user-selectable filter value.
const CONVERSATION_STATUS_DISPLAY_ORDER: ConversationStatus[] = [
  "active",
  "idle",
  "closed",
];

const STATUS_OPTIONS: FilterOption[] = (() => {
  const tones: Record<string, BadgeStatus> = {};
  for (const status of CONVERSATION_STATUS_DISPLAY_ORDER) {
    tones[status] = CONVERSATION_STATUS_TONES[status];
  }
  return statusOptions(tones, CONVERSATION_STATUS_LABELS);
})();

// Server-side filtering means `apply` is not invoked on the Chats page (the
// page maps Filter[] → SearchConversationsQuery via `filtersToConversationsQuery`
// and lets the server narrow). `apply` stays defined for the foundation
// contract.
function valueIncludes(haystack: string | null, values: string[]): boolean {
  if (haystack === null) return false;
  return values.includes(haystack);
}

/**
 * Builds the `CONVERSATION_FILTERS` definition map for the Chats page. A hook
 * because the `creator` user/agent option list comes from live React Query
 * data via `useUserOptions`.
 *
 * Every entry maps to a server-side query param on `SearchConversationsQuery`:
 *   - `status`  → `?status=` (single ConversationStatus).
 *   - `creator` → `?creator=` (single bare Username).
 *
 * Both are `singleSelect: true` because the backing server params accept a
 * single value. `notInSupported` is left unset (server can't negate either),
 * so the ValuePicker hides the is/is-not toggle.
 *
 * Relation filters are intentionally omitted: `SearchConversationsQuery` has
 * no `ids[]` or relation-by-source param today, so any relation chip would
 * require a server-side change. See the PR-4 issue for the deferred-item
 * write-up.
 */
export function useConversationFilters(): FilterDefinitions<ConversationSummary> {
  const userOpts = useUserOptions();

  return useMemo<FilterDefinitions<ConversationSummary>>(() => {
    return {
      status: {
        label: "Status",
        icon: Icons.IconDot,
        group: "properties",
        kind: "enum",
        singleSelect: true,
        options: STATUS_OPTIONS,
        apply: (rec, filter) => valueIncludes(rec.status, filter.values),
      },
      creator: {
        label: "Creator",
        icon: Icons.IconAgent,
        group: "people",
        kind: "user",
        singleSelect: true,
        options: userOpts,
        // Creator on the wire is a bare username; strip the Principal-path
        // prefix to compare.
        apply: (rec, filter) => {
          return filter.values.some((v) => {
            const bare = v.startsWith("users/")
              ? v.slice("users/".length)
              : v.startsWith("agents/")
                ? v.slice("agents/".length)
                : v;
            return bare === rec.creator;
          });
        },
      },
    };
  }, [userOpts]);
}
