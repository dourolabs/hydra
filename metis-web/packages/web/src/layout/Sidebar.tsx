import { useState } from "react";
import { NavLink } from "react-router-dom";
import { Avatar } from "@metis/ui";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import type { SSEConnectionState } from "../hooks/useSSE";
import styles from "./Sidebar.module.css";

interface SidebarProps {
  connectionState: SSEConnectionState;
}

const CONNECTION_LABELS: Record<SSEConnectionState, string> = {
  connected: "Live",
  connecting: "Connecting",
  disconnected: "Offline",
};

const NAV_ITEMS = [
  { to: "/", icon: "\u25A3", label: "Dashboard", end: true },
  { to: "/issues", icon: "\u2630", label: "Issues", end: false },
  { to: "/documents", icon: "\u2637", label: "Documents", end: false },
  { to: "/patches", icon: "\u2387", label: "Patches", end: false },
  { to: "/settings", icon: "\u2699", label: "Settings", end: false },
] as const;

export function Sidebar({ connectionState }: SidebarProps) {
  const [collapsed, setCollapsed] = useState(false);
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;

  return (
    <aside className={`${styles.sidebar}${collapsed ? ` ${styles.collapsed}` : ""}`}>
      <div className={styles.header}>
        <span className={styles.logo}>{collapsed ? "m" : "metis"}</span>
        <button
          type="button"
          className={styles.collapseBtn}
          onClick={() => setCollapsed(true)}
          title="Collapse sidebar"
        >
          &laquo;
        </button>
      </div>

      <nav className={styles.nav}>
        {NAV_ITEMS.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.end}
            className={({ isActive }) =>
              `${styles.navItem}${isActive ? ` ${styles.active}` : ""}`
            }
            title={collapsed ? item.label : undefined}
          >
            <span className={styles.navIcon}>{item.icon}</span>
            <span className={styles.navLabel}>{item.label}</span>
          </NavLink>
        ))}
      </nav>

      <div className={styles.footer}>
        <div
          className={`${styles.connectionStatus} ${styles[connectionState]}`}
          title={`SSE: ${CONNECTION_LABELS[connectionState]}`}
        >
          <span className={styles.dot} />
          <span className={styles.connectionLabel}>
            {CONNECTION_LABELS[connectionState]}
          </span>
        </div>
        {user && displayName && (
          <div className={styles.userSection}>
            <Avatar name={displayName} size="sm" />
            <span className={styles.username}>{displayName}</span>
            <button type="button" className={styles.logoutBtn} onClick={logout}>
              Logout
            </button>
          </div>
        )}
      </div>

      {collapsed && (
        <button
          type="button"
          className={styles.expandBtn}
          onClick={() => setCollapsed(false)}
          title="Expand sidebar"
        >
          &raquo;
        </button>
      )}
    </aside>
  );
}
