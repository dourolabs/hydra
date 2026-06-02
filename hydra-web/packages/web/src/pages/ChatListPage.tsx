import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Badge, Button, Icons } from "@hydra/ui";
import type { Conversation } from "@hydra/api";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { useConversations } from "../features/chat/useConversations";
import { conversationTitle } from "../features/chat/conversationTitle";
import { useConversationFilters } from "../features/chat/conversationFilters";
import {
  filtersFromUrl,
  filtersToUrl,
  searchToUrl,
  legacyScopeRedirect,
  defaultCreatorFilter,
  SEARCH_URL_PARAM,
} from "../features/chat/conversationFilterUrlSync";
import { filtersToConversationsQuery } from "../features/chat/filtersToConversationsQuery";
import { FilterBar, type Filter } from "../features/filters";
import { compareConversationsByBucketThenUpdated } from "../utils/conversationOrder";
import { AgoTime } from "../components/Runtime/Runtime";
import { useMediaQuery } from "../hooks/useMediaQuery";
import { ChatRailRow } from "../features/related/RailRow";
import { apiClient } from "../api/client";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./ChatListPage.module.css";

const MOBILE_QUERY = "(max-width: 768px)";

// Canonical, uid-free string repr used to detect whether the URL state and
// the local FilterBar state are in sync. Empty-values filters represent an
// in-flight FilterBar add (user picked a definition from the menu but hasn't
// chosen values yet) and are deliberately invisible to the URL.
function filtersCanonicalRepr(filters: Filter[]): string {
  return filters
    .filter((f) => f.values.length > 0)
    .map((f) => `${f.id}:${f.op}:${[...f.values].sort().join(",")}`)
    .sort()
    .join("|");
}

export function ChatListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Chats");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;
  const [searchParams, setSearchParams] = useSearchParams();
  const definitions = useConversationFilters();
  const isMobile = useMediaQuery(MOBILE_QUERY);

  // Filters mirror between URL params and local state. Lazy init computes
  // either: (a) the URL's explicit filter params, (b) the legacy `?scope=`
  // shape resolved into a chip, or (c) the default Mine-by-default chip on
  // first visit. A ref captures any first-paint URL rewrite the lazy init
  // wants so the mount effect can apply it via setSearchParams.
  const initialRewriteRef = useRef<URLSearchParams | null>(null);
  const [filters, setFiltersState] = useState<Filter[]>(() => {
    const explicit = filtersFromUrl(searchParams);
    if (explicit.length > 0) return explicit;
    const redirect = legacyScopeRedirect(searchParams, displayName);
    if (redirect) {
      initialRewriteRef.current = redirect.nextParams;
      return redirect.filters;
    }
    const seeded = defaultCreatorFilter(displayName);
    if (seeded.length > 0) {
      const next = new URLSearchParams(searchParams);
      next.set("creator", seeded[0].values[0]);
      initialRewriteRef.current = next;
    }
    return seeded;
  });

  useEffect(() => {
    if (initialRewriteRef.current) {
      setSearchParams(initialRewriteRef.current, { replace: true });
      initialRewriteRef.current = null;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // External URL changes (sidebar nav, back/forward) win over local state.
  useEffect(() => {
    const fromUrl = filtersFromUrl(searchParams);
    if (filtersCanonicalRepr(filters) !== filtersCanonicalRepr(fromUrl)) {
      setFiltersState(fromUrl);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  // Debounced free-text search: `searchValue` is the user-typed string,
  // `searchQuery` is what we actually send to the server / write to the URL
  // after a 300ms quiet period.
  const [searchValue, setSearchValue] = useState(
    searchParams.get(SEARCH_URL_PARAM) ?? "",
  );
  const [searchQuery, setSearchQuery] = useState(
    searchParams.get(SEARCH_URL_PARAM) ?? "",
  );
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const handleSearchChange = useCallback(
    (value: string) => {
      setSearchValue(value);
      clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        setSearchQuery(value);
        setSearchParams((prev) => searchToUrl(prev, value), { replace: true });
      }, 300);
    },
    [setSearchParams],
  );

  useEffect(() => () => clearTimeout(debounceRef.current), []);

  useEffect(() => {
    const urlQ = searchParams.get(SEARCH_URL_PARAM) ?? "";
    if (urlQ !== searchQuery) {
      setSearchValue(urlQ);
      setSearchQuery(urlQ);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  const setFilters = useCallback(
    (next: Filter[]) => {
      setFiltersState(next);
      setSearchParams((prev) => filtersToUrl(prev, next), { replace: false });
    },
    [setSearchParams],
  );

  const query = useMemo(
    () => filtersToConversationsQuery({ filters, q: searchQuery }),
    [filters, searchQuery],
  );

  const { data, isLoading, error } = useConversations(query);

  const createMutation = useMutation({
    mutationFn: () => apiClient.createConversation({}),
    onSuccess: (conversation: Conversation) => {
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate(`/chat/${conversation.conversation_id}`);
    },
  });

  const sorted = useMemo(() => {
    if (!data) return [];
    return [...data].sort(compareConversationsByBucketThenUpdated);
  }, [data]);

  const totalLabel = sorted.length === 1 ? "1 CHAT" : `${sorted.length} CHATS`;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>WORK · {totalLabel}</span>
          <h1 className={styles.pageTitle}>Chats</h1>
        </div>
        <span className={styles.headSpacer} />
        <Button
          variant="primary"
          size="sm"
          onClick={() => createMutation.mutate()}
          disabled={createMutation.isPending}
        >
          <Icons.IconPlus />
          {createMutation.isPending ? "Creating…" : "New chat"}
        </Button>
      </div>

      <div className={styles.toolbar}>
        <div className={styles.searchBox}>
          <span className={styles.searchIcon}>
            <Icons.IconSearch size={14} />
          </span>
          <input
            type="text"
            placeholder="Search chats…"
            value={searchValue}
            onChange={(e) => handleSearchChange(e.target.value)}
            aria-label="Search chats"
            data-testid="chats-search"
          />
        </div>
        <FilterBar
          filters={filters}
          setFilters={setFilters}
          definitions={definitions}
          count={sorted.length}
          total={sorted.length}
        />
      </div>

      {createMutation.error && (
        <div className={styles.errorBanner}>
          Failed to create conversation: {createMutation.error.message}
        </div>
      )}

      {error && (
        <div className={styles.errorBanner}>Failed to load conversations: {error.message}</div>
      )}

      <div className={styles.body}>
        {isLoading && sorted.length === 0 && <div className={styles.empty}>Loading chats…</div>}

        {!isLoading && !error && sorted.length === 0 && (
          <div className={styles.empty}>No conversations match the current filters.</div>
        )}

        {sorted.length > 0 && isMobile && (
          <div className={styles.mobileList} data-testid="chats-list">
            {sorted.map((c) => (
              <ChatRailRow key={c.conversation_id} conversation={c} />
            ))}
          </div>
        )}

        {sorted.length > 0 && !isMobile && (
          <div className={styles.tableWrap}>
            <table className={styles.table} data-testid="chats-list">
              <thead>
                <tr>
                  <th className={styles.colTitle}>Title</th>
                  <th className={styles.colStatus}>Status</th>
                  <th className={styles.colCreator}>Creator</th>
                  <th className={styles.colMessages}>Messages</th>
                  <th className={styles.colUpdated}>Updated</th>
                </tr>
              </thead>
              <tbody>
                {sorted.map((c) => (
                  <tr
                    key={c.conversation_id}
                    onClick={() => navigate(`/chat/${c.conversation_id}`)}
                    data-testid={`chats-list-row-${c.conversation_id}`}
                  >
                    <td className={styles.colTitle}>
                      <div className={styles.titleCell}>
                        <span className={styles.titleText}>{conversationTitle(c)}</span>
                      </div>
                    </td>
                    <td className={styles.colStatus}>
                      <Badge status={`conv-${c.status}`} />
                    </td>
                    <td className={styles.colCreator}>{c.creator}</td>
                    <td className={styles.colMessages}>{c.event_count}</td>
                    <td className={styles.colUpdated}>
                      <AgoTime iso={c.updated_at} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
