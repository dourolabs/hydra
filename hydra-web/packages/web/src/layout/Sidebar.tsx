import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import { Link, NavLink, useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Conversation, LabelRecord, VersionResponse } from "@hydra/api";
import { Avatar, Tooltip } from "@hydra/ui";
import type { ConversationSummary } from "@hydra/api";
import { apiClient } from "../api/client";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { useConversations } from "../features/chat/useConversations";
import { conversationTitle } from "../features/chat/conversationTitle";
import { useIssueCount, type IssueFilters } from "../features/issues/usePaginatedIssues";
import { useLabels } from "../features/labels/useLabels";
import { useMediaQuery } from "../hooks/useMediaQuery";
import type { SSEConnectionState } from "../hooks/useSSE";
import { SidebarDocumentTree } from "./SidebarDocumentTree";
import {
  AgentsIcon,
  ChatIcon,
  ContextIcon,
  DocumentsIcon,
  IssuesIcon,
  PatchesIcon,
  PlusIcon,
} from "./SidebarIcons";
import styles from "./Sidebar.module.css";

const CHATS_SECTION_LIMIT = 3;
const MOBILE_MEDIA_QUERY = "(max-width: 768px)";

interface SidebarProps {
  connectionState: SSEConnectionState;
  hidden: boolean;
  onHide: () => void;
}

const CONNECTION_LABELS: Record<SSEConnectionState, string> = {
  connected: "Connected",
  connecting: "Connecting",
  disconnected: "Disconnected",
};

const SECTION_STORAGE_PREFIX = "hydra:sidebar:section:";

function readSectionExpanded(id: string, defaultValue: boolean): boolean {
  if (typeof window === "undefined") return defaultValue;
  try {
    const raw = window.localStorage.getItem(`${SECTION_STORAGE_PREFIX}${id}`);
    if (raw === null) return defaultValue;
    return raw === "true";
  } catch {
    return defaultValue;
  }
}

function writeSectionExpanded(id: string, expanded: boolean): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(`${SECTION_STORAGE_PREFIX}${id}`, String(expanded));
  } catch {
    /* localStorage unavailable; ignore */
  }
}

function useSectionExpanded(id: string, defaultValue = true): [boolean, () => void] {
  const [expanded, setExpanded] = useState(() => readSectionExpanded(id, defaultValue));
  const toggle = useCallback(() => {
    setExpanded((prev) => {
      const next = !prev;
      writeSectionExpanded(id, next);
      return next;
    });
  }, [id]);
  return [expanded, toggle];
}

function ChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={`${styles.chevron}${expanded ? ` ${styles.chevronOpen}` : ""}`}
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-hidden="true"
    >
      <path
        fillRule="evenodd"
        d="M7.21 14.77a.75.75 0 01.02-1.06L11.168 10 7.23 6.29a.75.75 0 111.04-1.08l4.5 4.25a.75.75 0 010 1.08l-4.5 4.25a.75.75 0 01-1.06-.02z"
        clipRule="evenodd"
      />
    </svg>
  );
}

interface SidebarSectionProps {
  id: string;
  label: string;
  icon: ReactNode;
  children: ReactNode;
}

function SidebarSection({ id, label, icon, children }: SidebarSectionProps) {
  const [expanded, toggle] = useSectionExpanded(id);
  const bodyId = `sidebar-section-${id}-body`;
  return (
    <div className={styles.section}>
      <button
        type="button"
        className={styles.sectionHeader}
        onClick={toggle}
        aria-expanded={expanded}
        aria-controls={bodyId}
        data-testid={`sidebar-section-${id}`}
      >
        <ChevronIcon expanded={expanded} />
        {icon}
        <span className={styles.sectionLabel}>{label}</span>
      </button>
      {expanded && (
        <div id={bodyId} className={styles.sectionBody}>
          {children}
        </div>
      )}
    </div>
  );
}

function navItemClass({ isActive }: { isActive: boolean }) {
  return `${styles.navItem}${isActive ? ` ${styles.navItemActive}` : ""}`;
}

function seeAllLinkClass({ isActive }: { isActive: boolean }) {
  return `${styles.seeAllLink}${isActive ? ` ${styles.navItemActive}` : ""}`;
}

function topRecentLabels(labels: readonly LabelRecord[] | undefined): LabelRecord[] {
  if (!labels || labels.length === 0) return [];
  return [...labels].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, 3);
}

interface IssuesSectionContentProps {
  username: string | null;
  isDashboard: boolean;
  selectedParam: string | null;
  labelParam: string | null;
  onNewIssue: () => void;
}

function IssuesSectionContent({
  username,
  isDashboard,
  selectedParam,
  labelParam,
  onNewIssue,
}: IssuesSectionContentProps) {
  const assignedFilters = useMemo<IssueFilters>(
    () => (username ? { assignee: username, status: "open" } : {}),
    [username],
  );
  const { data: assignedCount = 0 } = useIssueCount(assignedFilters, !!username);
  const { data: labels } = useLabels();
  const recentLabels = useMemo(() => topRecentLabels(labels), [labels]);

  const assignedActive = isDashboard && selectedParam === "assigned";
  const allActive = isDashboard && selectedParam === "all" && !labelParam;

  return (
    <>
      <button
        type="button"
        className={styles.actionButton}
        onClick={onNewIssue}
        data-testid="sidebar-issues-new"
      >
        <PlusIcon />
        <span className={styles.navItemLabel}>New Issue</span>
      </button>
      <Link
        to="/?selected=assigned"
        className={`${styles.navItem}${assignedActive ? ` ${styles.navItemActive}` : ""}`}
        aria-current={assignedActive ? "page" : undefined}
        data-testid="sidebar-issues-assigned"
      >
        <span className={styles.navItemLabel}>Assigned to you</span>
        {assignedCount > 0 && (
          <span className={styles.badge} data-testid="sidebar-issues-assigned-badge">
            {assignedCount}
          </span>
        )}
      </Link>
      {recentLabels.map((label) => {
        const labelActive = isDashboard && selectedParam === "all" && labelParam === label.label_id;
        return (
          <Link
            key={label.label_id}
            to={`/?selected=all&label=${encodeURIComponent(label.label_id)}`}
            className={`${styles.navItem}${labelActive ? ` ${styles.navItemActive}` : ""}`}
            aria-current={labelActive ? "page" : undefined}
            data-testid={`sidebar-issues-label-${label.label_id}`}
          >
            <span
              className={styles.labelSwatch}
              style={{ backgroundColor: label.color }}
              aria-hidden="true"
            />
            <span className={styles.navItemLabel}>{label.name}</span>
          </Link>
        );
      })}
      <Link
        to="/?selected=all"
        className={`${styles.navItem}${allActive ? ` ${styles.navItemActive}` : ""}`}
        aria-current={allActive ? "page" : undefined}
        data-testid="sidebar-issues-all"
      >
        <span className={styles.navItemLabel}>All issues</span>
      </Link>
    </>
  );
}

export function Sidebar({ connectionState, hidden, onHide }: SidebarProps) {
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;
  const { pathname } = useLocation();
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const selectedParam = searchParams.get("selected");
  const labelParam = searchParams.get("label");
  const isDashboard = pathname === "/";
  const patchesActive = isDashboard && selectedParam === "patches";
  const isMobile = useMediaQuery(MOBILE_MEDIA_QUERY);

  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    apiClient
      .getVersion()
      .then((res: VersionResponse) => setVersion(res.version))
      .catch(() => {
        /* silently ignore -- badge stays hidden */
      });
  }, []);

  const { data: conversations } = useConversations();
  const recentChats = useMemo<ConversationSummary[]>(() => {
    if (!conversations) return [];
    return [...conversations]
      .sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime())
      .slice(0, CHATS_SECTION_LIMIT);
  }, [conversations]);

  const createChatMutation = useMutation({
    mutationFn: () => apiClient.createConversation({}),
    onSuccess: (conversation: Conversation) => {
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate(`/chat/${conversation.conversation_id}`);
    },
  });

  const handleNewIssue = useCallback(() => {
    navigate("/?create-issue=1");
  }, [navigate]);

  // On mobile, auto-close the drawer when the user taps a navigation link
  // (anchor). Section toggle buttons live inside the same <nav> but are
  // `<button>` elements, so the positive selector below leaves them alone.
  const handleNavClick = useCallback(
    (event: ReactMouseEvent<HTMLElement>) => {
      if (!isMobile) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest("a")) {
        onHide();
      }
    },
    [isMobile, onHide],
  );

  // On mobile, Escape closes the drawer. Listen on `window` so it works even
  // when focus is outside the sidebar (e.g. just after a touch tap).
  useEffect(() => {
    if (!isMobile || hidden) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onHide();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [isMobile, hidden, onHide]);

  return (
    <>
      {isMobile && !hidden && (
        <div
          className={styles.backdrop}
          onClick={onHide}
          aria-hidden="true"
          data-testid="sidebar-backdrop"
        />
      )}
      <nav
        className={`${styles.sidebar}${hidden ? ` ${styles.sidebarHidden}` : ""}`}
        aria-label="Primary"
        aria-hidden={hidden || undefined}
        inert={hidden || undefined}
        data-testid="sidebar"
        onClick={handleNavClick}
      >
        <div className={styles.sections}>
          <SidebarSection id="chats" label="Chats" icon={<ChatIcon />}>
            <button
              type="button"
              className={styles.actionButton}
              onClick={() => createChatMutation.mutate()}
              disabled={createChatMutation.isPending}
              data-testid="sidebar-chat-new"
            >
              <PlusIcon />
              <span className={styles.navItemLabel}>
                {createChatMutation.isPending ? "Creating…" : "New Chat"}
              </span>
            </button>
            {recentChats.map((c) => {
              const title = conversationTitle(c);
              return (
                <NavLink
                  key={c.conversation_id}
                  to={`/chat/${c.conversation_id}`}
                  className={navItemClass}
                  data-testid={`sidebar-chat-row-${c.conversation_id}`}
                  title={title}
                >
                  <span className={styles.navItemLabel}>{title}</span>
                </NavLink>
              );
            })}
            <NavLink
              to="/chat"
              end
              className={seeAllLinkClass}
              data-testid="sidebar-section-chats-more"
            >
              See All
            </NavLink>
          </SidebarSection>

          <SidebarSection id="issues" label="Issues" icon={<IssuesIcon />}>
            <IssuesSectionContent
              username={displayName}
              isDashboard={isDashboard}
              selectedParam={selectedParam}
              labelParam={labelParam}
              onNewIssue={handleNewIssue}
            />
          </SidebarSection>

          <SidebarSection id="documents" label="Documents" icon={<DocumentsIcon />}>
            <SidebarDocumentTree />
            <NavLink
              to="/documents"
              end
              className={seeAllLinkClass}
              data-testid="sidebar-section-documents-more"
            >
              See All
            </NavLink>
          </SidebarSection>

          <Link
            to="/?selected=patches"
            className={`${styles.navItem}${patchesActive ? ` ${styles.navItemActive}` : ""}`}
            aria-current={patchesActive ? "page" : undefined}
            data-testid="sidebar-patches"
          >
            <PatchesIcon />
            <span className={styles.navItemLabel}>Patches</span>
          </Link>

          <NavLink to="/agents" className={navItemClass} data-testid="sidebar-agents">
            <AgentsIcon />
            <span className={styles.navItemLabel}>Agents</span>
          </NavLink>

          <SidebarSection id="context" label="Context" icon={<ContextIcon />}>
            <NavLink
              to="/repositories"
              className={navItemClass}
              data-testid="sidebar-context-repositories"
            >
              Repositories
            </NavLink>
            <NavLink to="/secrets" className={navItemClass} data-testid="sidebar-context-secrets">
              Secrets
            </NavLink>
          </SidebarSection>
        </div>

        <div className={styles.bottom}>
          <Tooltip content={`SSE: ${CONNECTION_LABELS[connectionState]}`} position="top">
            <div className={styles.connectionIndicator}>
              <span className={`${styles.connectionDot} ${styles[connectionState]}`} />
              <span className={styles.connectionLabel}>{CONNECTION_LABELS[connectionState]}</span>
            </div>
          </Tooltip>

          {user && displayName && (
            <div className={styles.userSection}>
              <Avatar name={displayName} size="sm" />
              <span className={styles.userName} title={displayName}>
                {displayName}
              </span>
              <Tooltip content="Logout" position="top">
                <button className={styles.logoutButton} onClick={logout} aria-label="Logout">
                  <svg className={styles.logoutIcon} viewBox="0 0 20 20" fill="currentColor">
                    <path
                      fillRule="evenodd"
                      d="M3 3a1 1 0 00-1 1v12a1 1 0 001 1h5a1 1 0 100-2H4V5h4a1 1 0 100-2H3zm11.293 3.293a1 1 0 011.414 0l3 3a1 1 0 010 1.414l-3 3a1 1 0 01-1.414-1.414L15.586 11H8a1 1 0 110-2h7.586l-1.293-1.293a1 1 0 010-1.414z"
                      clipRule="evenodd"
                    />
                  </svg>
                </button>
              </Tooltip>
            </div>
          )}

          {version && (
            <div className={styles.version} data-testid="sidebar-version">
              {version}
            </div>
          )}
        </div>
      </nav>
    </>
  );
}
