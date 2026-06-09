import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import { NavLink, useLocation, useSearchParams } from "react-router-dom";
import { HydraMark, Avatar, Kbd, Icons, Tooltip } from "@hydra/ui";
import type { ConversationSummary, VersionResponse } from "@hydra/api";
import { apiClient } from "../api/client";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName, actorPrincipalPath } from "../api/auth";
import { useConversations } from "../features/chat/useConversations";
import { conversationTitle } from "../features/chat/conversationTitle";
import { compareConversationsByBucketThenUpdated } from "../utils/conversationOrder";
import { useIssueCount, type IssueFilters } from "../features/issues/usePaginatedIssues";
import { useActiveSessions } from "../features/sessions/useActiveSessions";
import { useActiveSessionCount } from "../features/sessions/useActiveSessionCount";
import { useSessionLinks } from "../features/sessions/useSessionLinks";
import { resolveSessionDisplay } from "../features/sessions/sessionDisplay";
import { useMediaQuery } from "../hooks/useMediaQuery";
import type { SSEConnectionState } from "../hooks/useSSE";
import styles from "./Sidebar.module.css";

const CHATS_SECTION_LIMIT = 4;
const SESSIONS_SECTION_LIMIT = 6;
const MOBILE_MEDIA_QUERY = "(max-width: 768px)";

interface SidebarProps {
  connectionState: SSEConnectionState;
  hidden: boolean;
  onHide: () => void;
  onOpenSearch: () => void;
}

const CONNECTION_LABELS: Record<SSEConnectionState, string> = {
  connected: "Connected",
  connecting: "Connecting",
  disconnected: "Disconnected",
};

function chatDotClass(status: ConversationSummary["status"]): string {
  if (status === "active") return styles.chatDotActive!;
  if (status === "closed") return styles.chatDotClosed!;
  return styles.chatDotIdle!;
}

interface NavItem {
  to: string;
  end?: boolean;
  label: string;
  testId: string;
  icon: React.ReactNode;
  meta?: React.ReactNode;
  isActive?: (pathname: string, search: URLSearchParams) => boolean;
}

export function Sidebar({ connectionState, hidden, onHide, onOpenSearch }: SidebarProps) {
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;
  const userMeta = user?.actor.type === "user" ? user.actor.username : null;
  // Phase 4b: the assignee filter on the wire is a Principal path
  // (`users/<name>` / `agents/<name>`); a bare username is rejected by the
  // server's deserializer. Build it once from the typed actor so every
  // "Assigned to me" surface (count, link, active check) speaks path form.
  const principalPath = user ? actorPrincipalPath(user.actor) : null;

  const { pathname } = useLocation();
  const [searchParams] = useSearchParams();
  const isMobile = useMediaQuery(MOBILE_MEDIA_QUERY);

  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    apiClient
      .getVersion()
      .then((res: VersionResponse) => setVersion(res.version))
      .catch(() => {
        /* ignore */
      });
  }, []);

  // ── Counts ──
  const assignedFilters = useMemo<IssueFilters>(
    () => (principalPath ? { assignee: principalPath, status: "open" } : {}),
    [principalPath],
  );
  const inProgressFilters = useMemo<IssueFilters>(() => ({ status: "in-progress" }), []);
  const { data: assignedCount = 0 } = useIssueCount(assignedFilters, !!principalPath);
  const { data: inProgressCount = 0 } = useIssueCount(inProgressFilters, true);

  // URL params used by the Workspace sidebar's links. Encoding the username
  // explicitly keeps the route shareable without a server-side lookup, but
  // means the `Issues` and `Assigned to me` links are computed per-user.
  const yourIssuesHref = displayName ? `/?creator=${encodeURIComponent(displayName)}` : "/";
  const assignedHref = principalPath ? `/?assignee=${encodeURIComponent(principalPath)}` : "/";
  const inProgressHref = "/?status=in-progress";

  // Active-link logic: the link is active iff the current URL matches the
  // single filter param it points at AND no other filter params are present,
  // so the dropdown filters below don't keep multiple sidebar items lit.
  const isOnlyParam = (key: string, value: string): boolean => {
    if (pathname !== "/") return false;
    if (searchParams.get(key) !== value) return false;
    for (const [k] of searchParams.entries()) {
      if (k === key) continue;
      if (k === "selected") continue;
      if (k === "q") continue;
      return false;
    }
    return true;
  };

  const isNoFilters = (): boolean => {
    if (pathname !== "/") return false;
    for (const [k] of searchParams.entries()) {
      if (k === "selected") continue;
      if (k === "q") continue;
      return false;
    }
    return true;
  };

  // A bare `/` means "all issues" (no filter applied), so the All issues
  // link is the one that lights up there. The Issues link is active only
  // when the URL carries the current user's creator filter.
  const isYourIssuesActive = displayName ? isOnlyParam("creator", displayName) : false;
  const isAllIssuesActive = isNoFilters();
  const isAssignedActive = principalPath ? isOnlyParam("assignee", principalPath) : false;
  const isInProgressActive = isOnlyParam("status", "in-progress");
  const isArchiveActive = isOnlyParam("includeArchived", "1");

  // ── Recent chats ──
  // Default to the logged-in user's own chats; mirrors the ChatListPage 'mine' default.
  const conversationsQuery = useMemo(
    () => (displayName ? { creator: displayName } : undefined),
    [displayName],
  );
  const { data: conversations } = useConversations(conversationsQuery, {
    enabled: !!displayName,
  });
  const recentChats = useMemo<ConversationSummary[]>(() => {
    if (!conversations) return [];
    return conversations
      .filter((c) => c.status !== "closed")
      .sort(compareConversationsByBucketThenUpdated)
      .slice(0, CHATS_SECTION_LIMIT);
  }, [conversations]);

  // ── Active sessions ──
  // List is capped at SESSIONS_SECTION_LIMIT rows, but the count badge shows
  // the true total (matching the top nav) so it doesn't appear to plateau at 6.
  // Scoped to the logged-in user's own sessions, matching the chats list above.
  const { data: activeSessions } = useActiveSessions(displayName, SESSIONS_SECTION_LIMIT);
  const { data: activeSessionCount = 0 } = useActiveSessionCount(displayName);
  const { issueMap, conversationMap } = useSessionLinks(activeSessions ?? []);

  // On mobile, close drawer when navigating.
  const handleNavClick = (event: ReactMouseEvent<HTMLElement>) => {
    if (!isMobile) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("a")) onHide();
  };

  // On mobile, Escape closes the drawer.
  useEffect(() => {
    if (!isMobile || hidden) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onHide();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [isMobile, hidden, onHide]);

  // ── Workspace items ── (Chats lives in its own top section)
  const workspaceItems: NavItem[] = [
    {
      to: "/",
      end: true,
      label: "Issues",
      testId: "sidebar-issues-all",
      icon: <Icons.IconIssue />,
      isActive: () => isAllIssuesActive,
    },
    {
      to: "/patches",
      label: "Patches",
      testId: "sidebar-patches",
      icon: <Icons.IconPatch />,
    },
    {
      to: "/sessions",
      label: "Sessions",
      testId: "sidebar-sessions",
      icon: <Icons.IconPlay />,
    },
    {
      to: "/documents",
      label: "Documents",
      testId: "sidebar-documents",
      icon: <Icons.IconDoc />,
    },
    {
      to: "/agents",
      label: "Agents",
      testId: "sidebar-agents",
      icon: <Icons.IconAgent />,
    },
    {
      to: "/triggers",
      label: "Triggers",
      testId: "sidebar-triggers",
      icon: <Icons.IconTime />,
    },
    {
      to: "/projects",
      label: "Projects",
      testId: "sidebar-projects",
      icon: <Icons.IconFolder />,
    },
    {
      to: "/repositories",
      label: "Repositories",
      testId: "sidebar-context-repositories",
      icon: <Icons.IconRepo />,
    },
    {
      to: "/secrets",
      label: "Secrets",
      testId: "sidebar-context-secrets",
      icon: <Icons.IconKey />,
    },
  ];

  const renderItem = (item: NavItem) => {
    const computedActive = item.isActive ? item.isActive(pathname, searchParams) : undefined;
    const className = ({ isActive }: { isActive: boolean }) => {
      const active = computedActive ?? isActive;
      return `${styles.item}${active ? ` ${styles.itemActive}` : ""}`;
    };
    return (
      <NavLink
        key={item.testId}
        to={item.to}
        end={item.end}
        className={className}
        data-testid={item.testId}
      >
        <span className={styles.itemIcon}>{item.icon}</span>
        <span className={styles.itemLabel}>{item.label}</span>
        {item.meta != null && <span className={styles.itemMeta}>{item.meta}</span>}
      </NavLink>
    );
  };

  return (
    <nav
      className={styles.sidebar}
      aria-label="Primary"
      aria-hidden={hidden || undefined}
      inert={hidden || undefined}
      data-testid="sidebar"
      onClick={handleNavClick}
    >
      <div className={styles.head}>
        <NavLink to="/" className={styles.brand} aria-label="Hydra" data-testid="hydra-brand">
          <HydraMark variant="borromean" size={20} className={styles.logoMark} />
          <span className={styles.wordmark}>Hydra</span>
        </NavLink>
      </div>

      <button
        type="button"
        className={styles.searchButton}
        onClick={onOpenSearch}
        data-testid="sidebar-search"
      >
        <Icons.IconSearch className={styles.searchIcon} />
        <span className={styles.searchPlaceholder}>Search…</span>
        <Kbd>⌘K</Kbd>
      </button>

      <div className={styles.scroll}>
        {/* ── Chats ── */}
        <div className={styles.section}>
          <div className={styles.sectionHead}>
            <span>Chats</span>
          </div>
          <NavLink
            to="/chat"
            end
            className={({ isActive }) => `${styles.item}${isActive ? ` ${styles.itemActive}` : ""}`}
            data-testid="sidebar-chats"
          >
            <span className={styles.itemIcon}>
              <Icons.IconChat />
            </span>
            <span className={styles.itemLabel}>My chats</span>
            {conversations && <span className={styles.itemMeta}>{conversations.length}</span>}
          </NavLink>
          {recentChats.map((c) => {
            const title = conversationTitle(c);
            return (
              <NavLink
                key={c.conversation_id}
                to={`/chat/${c.conversation_id}`}
                className={({ isActive }) =>
                  `${styles.chatRow}${isActive ? ` ${styles.chatRowActive}` : ""}`
                }
                data-testid={`sidebar-chat-row-${c.conversation_id}`}
                title={title}
              >
                <span className={`${styles.chatDot} ${chatDotClass(c.status)}`} />
                <span className={styles.chatTitle}>{title}</span>
              </NavLink>
            );
          })}
        </div>

        {/* ── Workspace ── */}
        <div className={styles.section}>
          <div className={styles.sectionHead}>
            <span>Workspace</span>
          </div>
          {workspaceItems.map(renderItem)}
        </div>

        {/* ── Views ── */}
        {displayName && (
          <div className={styles.section}>
            <div className={styles.sectionHead}>
              <span>Views</span>
            </div>
            <NavLink
              to={yourIssuesHref}
              className={() => `${styles.item}${isYourIssuesActive ? ` ${styles.itemActive}` : ""}`}
              data-testid="sidebar-issues-your-issues"
            >
              <span className={styles.itemIcon}>
                <Icons.IconIssue />
              </span>
              <span className={styles.itemLabel}>My issues</span>
            </NavLink>
            <NavLink
              to={assignedHref}
              className={() => `${styles.item}${isAssignedActive ? ` ${styles.itemActive}` : ""}`}
              data-testid="sidebar-issues-assigned"
            >
              <span className={styles.itemIcon}>
                <Icons.IconCheck />
              </span>
              <span className={styles.itemLabel}>Assigned to me</span>
              {assignedCount > 0 && (
                <span className={styles.itemMeta} data-testid="sidebar-issues-assigned-badge">
                  {assignedCount}
                </span>
              )}
            </NavLink>
            <NavLink
              to={inProgressHref}
              className={() => `${styles.item}${isInProgressActive ? ` ${styles.itemActive}` : ""}`}
              data-testid="sidebar-issues-in-progress"
            >
              <span className={styles.itemIcon}>
                <Icons.IconTime />
              </span>
              <span className={styles.itemLabel}>In progress</span>
              {inProgressCount > 0 && <span className={styles.itemMeta}>{inProgressCount}</span>}
            </NavLink>
            <NavLink
              to="/?includeArchived=1"
              className={() => `${styles.item}${isArchiveActive ? ` ${styles.itemActive}` : ""}`}
              data-testid="sidebar-issues-archive"
            >
              <span className={styles.itemIcon}>
                <Icons.IconArchive />
              </span>
              <span className={styles.itemLabel}>Archive</span>
            </NavLink>
          </div>
        )}

        {/* ── Active sessions ── */}
        <div className={styles.section} data-testid="sidebar-active-sessions">
          <div className={styles.sectionHead}>
            <span>Active sessions</span>
            {activeSessionCount > 0 && (
              <span className={styles.itemMeta} data-testid="sidebar-active-sessions-count">
                {activeSessionCount}
              </span>
            )}
          </div>
          {activeSessions && activeSessions.length === 0 && (
            <div className={styles.sectionEmpty}>No active sessions.</div>
          )}
          {(activeSessions ?? []).map((s) => {
            const { title } = resolveSessionDisplay(s, issueMap, conversationMap);
            return (
              <NavLink
                key={s.session_id}
                to={`/sessions/${s.session_id}`}
                className={({ isActive }) =>
                  `${styles.sessionRow}${isActive ? ` ${styles.sessionRowActive}` : ""}`
                }
                data-testid={`sidebar-session-row-${s.session_id}`}
                title={title}
              >
                <span className={`${styles.sessionDot}`} />
                <span className={styles.sessionTitle}>{title}</span>
              </NavLink>
            );
          })}
        </div>
      </div>

      <Tooltip content={`SSE: ${CONNECTION_LABELS[connectionState]}`} position="top">
        <div className={styles.connectionStrip}>
          <span
            className={styles.connectionDot}
            data-state={connectionState}
            data-testid="sidebar-connection-dot"
          />
          <span>
            {CONNECTION_LABELS[connectionState]}
            {version && (
              <>
                {" / "}
                <span data-testid="sidebar-version">v{version}</span>
              </>
            )}
          </span>
        </div>
      </Tooltip>

      <div className={styles.foot}>
        {user && displayName && (
          <div className={styles.userCard}>
            <Avatar name={displayName} kind="human" size="lg" />
            <div className={styles.userInfo}>
              <div className={styles.userName} title={displayName}>
                {displayName}
              </div>
              {userMeta && userMeta !== displayName && (
                <div className={styles.userMeta} title={userMeta}>
                  {userMeta}
                </div>
              )}
            </div>
            <Tooltip content="Logout" position="top">
              <button
                type="button"
                className={styles.logoutButton}
                onClick={logout}
                aria-label="Logout"
              >
                <Icons.IconX size={14} />
              </button>
            </Tooltip>
          </div>
        )}
      </div>
    </nav>
  );
}
