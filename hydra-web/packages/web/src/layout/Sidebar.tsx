import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import { NavLink, useLocation, useSearchParams } from "react-router-dom";
import { HydraMark, Avatar, Kbd, Icons, Tooltip } from "@hydra/ui";
import type {
  ConversationSummary,
  SessionSummaryRecord,
  VersionResponse,
} from "@hydra/api";
import { apiClient } from "../api/client";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { useConversations } from "../features/chat/useConversations";
import { conversationTitle } from "../features/chat/conversationTitle";
import { useIssueCount, type IssueFilters } from "../features/issues/usePaginatedIssues";
import { useActiveSessions } from "../features/sessions/useActiveSessions";
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

function formatRelativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "";
  const diffSec = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (diffSec < 60) return `${diffSec}s`;
  const min = Math.floor(diffSec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day}d`;
  const mo = Math.floor(day / 30);
  return `${mo}mo`;
}

function chatDotClass(status: ConversationSummary["status"]): string {
  if (status === "active") return styles.chatDotInProgress!;
  if (status === "closed") return styles.chatDotClosed!;
  return styles.chatDotOpen!;
}

function sessionTitle(s: SessionSummaryRecord["session"]): string {
  const prompt = (s.prompt || "").trim();
  if (prompt.length === 0) {
    return s.spawned_from ?? "Session";
  }
  return prompt.length > 50 ? `${prompt.slice(0, 50)}…` : prompt;
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

  const { pathname } = useLocation();
  const [searchParams] = useSearchParams();
  const selectedParam = searchParams.get("selected");
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
    () => (displayName ? { assignee: displayName, status: "open" } : {}),
    [displayName],
  );
  const inProgressFilters = useMemo<IssueFilters>(() => ({ status: "in_progress" }), []);
  const { data: assignedCount = 0 } = useIssueCount(assignedFilters, !!displayName);
  const { data: inProgressCount = 0 } = useIssueCount(inProgressFilters, true);

  // ── Recent chats ──
  const { data: conversations } = useConversations();
  const recentChats = useMemo<ConversationSummary[]>(() => {
    if (!conversations) return [];
    return [...conversations]
      .sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime())
      .slice(0, CHATS_SECTION_LIMIT);
  }, [conversations]);

  // ── Active sessions ──
  const { data: activeSessions } = useActiveSessions(SESSIONS_SECTION_LIMIT);

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
      testId: "sidebar-issues",
      icon: <Icons.IconIssue />,
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
      icon: <Icons.IconSpark />,
    },
    {
      to: "/documents",
      label: "Docs",
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
      to: "/repositories",
      label: "Repos",
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
            className={({ isActive }) =>
              `${styles.item}${isActive ? ` ${styles.itemActive}` : ""}`
            }
            data-testid="sidebar-chats"
          >
            <span className={styles.itemIcon}>
              <Icons.IconChat />
            </span>
            <span className={styles.itemLabel}>All chats</span>
            {conversations && (
              <span className={styles.itemMeta}>{conversations.length}</span>
            )}
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
                <span className={styles.chatTime}>{formatRelativeTime(c.updated_at)}</span>
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
              to="/?selected=assigned"
              className={({ isActive }) => {
                const active = isActive && selectedParam === "assigned";
                return `${styles.item}${active ? ` ${styles.itemActive}` : ""}`;
              }}
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
              to="/?selected=in_progress"
              className={({ isActive }) => {
                const active = isActive && selectedParam === "in_progress";
                return `${styles.item}${active ? ` ${styles.itemActive}` : ""}`;
              }}
              data-testid="sidebar-issues-in-progress"
            >
              <span className={styles.itemIcon}>
                <Icons.IconTime />
              </span>
              <span className={styles.itemLabel}>In progress</span>
              {inProgressCount > 0 && (
                <span className={styles.itemMeta}>{inProgressCount}</span>
              )}
            </NavLink>
          </div>
        )}

        {/* ── Active sessions ── */}
        <div className={styles.section} data-testid="sidebar-active-sessions">
          <div className={styles.sectionHead}>
            <span>Active sessions</span>
            {activeSessions && activeSessions.length > 0 && (
              <span className={styles.itemMeta}>{activeSessions.length}</span>
            )}
          </div>
          {activeSessions && activeSessions.length === 0 && (
            <div className={styles.sectionEmpty}>No active sessions.</div>
          )}
          {(activeSessions ?? []).map((s) => {
            const title = sessionTitle(s.session);
            const startedAt = s.session.start_time ?? s.session.creation_time ?? s.timestamp;
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
                {startedAt && (
                  <span className={styles.sessionWhen}>{formatRelativeTime(startedAt)}</span>
                )}
              </NavLink>
            );
          })}
        </div>
      </div>

      <div className={styles.foot}>
        <Tooltip content={`SSE: ${CONNECTION_LABELS[connectionState]}`} position="top">
          <div className={styles.connectionStrip}>
            <span
              className={styles.connectionDot}
              data-state={connectionState}
              data-testid="sidebar-connection-dot"
            />
            <span>{CONNECTION_LABELS[connectionState]}</span>
            {version && (
              <span style={{ marginLeft: "auto" }} data-testid="sidebar-version">
                {version}
              </span>
            )}
          </div>
        </Tooltip>
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
