import { useCallback, useState, type ReactNode } from "react";
import { Link, NavLink, useLocation, useSearchParams } from "react-router-dom";
import { Avatar, Tooltip } from "@hydra/ui";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import type { SSEConnectionState } from "../hooks/useSSE";
import styles from "./Sidebar.module.css";

interface SidebarProps {
  connectionState: SSEConnectionState;
}

const CONNECTION_LABELS: Record<SSEConnectionState, string> = {
  connected: "Connected",
  connecting: "Connecting",
  disconnected: "Disconnected",
};

const SECTION_STORAGE_PREFIX = "hydra.sidebar.section.";

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
    window.localStorage.setItem(
      `${SECTION_STORAGE_PREFIX}${id}`,
      String(expanded),
    );
  } catch {
    /* localStorage unavailable; ignore */
  }
}

function useSectionExpanded(
  id: string,
  defaultValue = true,
): [boolean, () => void] {
  const [expanded, setExpanded] = useState(() =>
    readSectionExpanded(id, defaultValue),
  );
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
  children: ReactNode;
}

function SidebarSection({ id, label, children }: SidebarSectionProps) {
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

function moreLinkClass({ isActive }: { isActive: boolean }) {
  return `${styles.moreLink}${isActive ? ` ${styles.navItemActive}` : ""}`;
}

export function Sidebar({ connectionState }: SidebarProps) {
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;
  const { pathname } = useLocation();
  const [searchParams] = useSearchParams();
  const selectedParam = searchParams.get("selected");
  const isDashboard = pathname === "/";
  const issuesMoreActive = isDashboard && selectedParam === "your-issues";
  const patchesActive = isDashboard && selectedParam === "patches";

  return (
    <nav className={styles.sidebar} aria-label="Primary">
      <div className={styles.header}>
        <Tooltip content="Active sessions" position="bottom">
          <button
            type="button"
            className={styles.headerSlot}
            data-testid="sidebar-header-sessions"
            aria-label="Active sessions"
          >
            <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
              <path
                fillRule="evenodd"
                d="M10 18a8 8 0 100-16 8 8 0 000 16zm.75-13a.75.75 0 00-1.5 0v5c0 .2.08.39.22.53l3 3a.75.75 0 101.06-1.06L10.75 9.69V5z"
                clipRule="evenodd"
              />
            </svg>
          </button>
        </Tooltip>

        <div className={styles.headerSpacer} />

        <Tooltip content="Search" position="bottom">
          <button
            type="button"
            className={styles.headerSlot}
            data-testid="sidebar-header-search"
            aria-label="Search"
          >
            <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
              <path
                fillRule="evenodd"
                d="M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z"
                clipRule="evenodd"
              />
            </svg>
          </button>
        </Tooltip>

        <Tooltip content="Hide sidebar" position="bottom">
          <button
            type="button"
            className={styles.headerSlot}
            data-testid="sidebar-header-hide"
            aria-label="Hide sidebar"
          >
            <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
              <path
                fillRule="evenodd"
                d="M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z"
                clipRule="evenodd"
              />
            </svg>
          </button>
        </Tooltip>
      </div>

      <div className={styles.sections}>
        <SidebarSection id="chats" label="Chats">
          <NavLink
            to="/chat"
            className={moreLinkClass}
            data-testid="sidebar-section-chats-more"
          >
            More
          </NavLink>
        </SidebarSection>

        <SidebarSection id="issues" label="Issues">
          <Link
            to="/?selected=your-issues"
            className={`${styles.moreLink}${issuesMoreActive ? ` ${styles.navItemActive}` : ""}`}
            aria-current={issuesMoreActive ? "page" : undefined}
            data-testid="sidebar-section-issues-more"
          >
            More
          </Link>
        </SidebarSection>

        <SidebarSection id="documents" label="Documents">
          <NavLink
            to="/documents"
            className={moreLinkClass}
            data-testid="sidebar-section-documents-more"
          >
            More
          </NavLink>
        </SidebarSection>

        <Link
          to="/?selected=patches"
          className={`${styles.navItem}${patchesActive ? ` ${styles.navItemActive}` : ""}`}
          aria-current={patchesActive ? "page" : undefined}
          data-testid="sidebar-patches"
        >
          Patches
        </Link>

        <NavLink
          to="/settings"
          className={navItemClass}
          data-testid="sidebar-agents"
        >
          Agents
        </NavLink>

        <SidebarSection id="context" label="Context">
          <NavLink
            to="/settings"
            className={moreLinkClass}
            data-testid="sidebar-context-repositories"
          >
            Repositories
          </NavLink>
          <NavLink
            to="/settings"
            className={moreLinkClass}
            data-testid="sidebar-context-secrets"
          >
            Secrets
          </NavLink>
        </SidebarSection>
      </div>

      <div className={styles.bottom}>
        <Tooltip
          content={`SSE: ${CONNECTION_LABELS[connectionState]}`}
          position="top"
        >
          <div className={styles.connectionIndicator}>
            <span
              className={`${styles.connectionDot} ${styles[connectionState]}`}
            />
            <span className={styles.connectionLabel}>
              {CONNECTION_LABELS[connectionState]}
            </span>
          </div>
        </Tooltip>

        {user && displayName && (
          <div className={styles.userSection}>
            <Avatar name={displayName} size="sm" />
            <span className={styles.userName} title={displayName}>
              {displayName}
            </span>
            <Tooltip content="Logout" position="top">
              <button
                className={styles.logoutButton}
                onClick={logout}
                aria-label="Logout"
              >
                <svg
                  className={styles.logoutIcon}
                  viewBox="0 0 20 20"
                  fill="currentColor"
                >
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
      </div>
    </nav>
  );
}
