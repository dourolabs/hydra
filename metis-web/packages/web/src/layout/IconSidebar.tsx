import { Link, NavLink } from "react-router-dom";
import { Avatar, Tooltip } from "@metis/ui";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import type { SSEConnectionState } from "../hooks/useSSE";
import styles from "./IconSidebar.module.css";

interface IconSidebarProps {
  connectionState: SSEConnectionState;
}

const CONNECTION_LABELS: Record<SSEConnectionState, string> = {
  connected: "Connected",
  connecting: "Connecting",
  disconnected: "Disconnected",
};

const NAV_ITEMS = [
  {
    to: "/",
    label: "Dashboard",
    testId: "nav-dashboard",
    icon: (
      <svg className={styles.navIcon} viewBox="0 0 20 20" fill="currentColor">
        <path d="M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h2a1 1 0 001-1v-2a1 1 0 011-1h2a1 1 0 011 1v2a1 1 0 001 1h2a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z" />
      </svg>
    ),
    end: true,
  },
  {
    to: "/issues",
    label: "Issues",
    icon: (
      <svg className={styles.navIcon} viewBox="0 0 20 20" fill="currentColor">
        <path fillRule="evenodd" d="M3 4a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 4a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 4a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 4a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z" clipRule="evenodd" />
      </svg>
    ),
    end: false,
  },
  {
    to: "/documents",
    label: "Documents",
    icon: (
      <svg className={styles.navIcon} viewBox="0 0 20 20" fill="currentColor">
        <path fillRule="evenodd" d="M4 4a2 2 0 012-2h4.586A2 2 0 0112 2.586L15.414 6A2 2 0 0116 7.414V16a2 2 0 01-2 2H6a2 2 0 01-2-2V4zm2 6a1 1 0 011-1h6a1 1 0 110 2H7a1 1 0 01-1-1zm1 3a1 1 0 100 2h6a1 1 0 100-2H7z" clipRule="evenodd" />
      </svg>
    ),
    end: false,
  },
  {
    to: "/patches",
    label: "Patches",
    icon: (
      <svg className={styles.navIcon} viewBox="0 0 20 20" fill="currentColor">
        <path d="M3 4a2 2 0 1 1 4 0 2 2 0 0 1-4 0zM3 16a2 2 0 1 1 4 0 2 2 0 0 1-4 0zM13 4a2 2 0 1 1 4 0 2 2 0 0 1-4 0zM4 6h2v8H4zM14 6.5C14 10 10 13 6 14V12C9 11 12 9 12 6.5H14Z" />
      </svg>
    ),
    end: false,
  },
  {
    to: "/settings",
    label: "Settings",
    icon: (
      <svg className={styles.navIcon} viewBox="0 0 20 20" fill="currentColor">
        <path fillRule="evenodd" d="M11.49 3.17c-.38-1.56-2.6-1.56-2.98 0a1.532 1.532 0 01-2.286.948c-1.372-.836-2.942.734-2.106 2.106.54.886.061 2.042-.947 2.287-1.561.379-1.561 2.6 0 2.978a1.532 1.532 0 01.947 2.287c-.836 1.372.734 2.942 2.106 2.106a1.532 1.532 0 012.287.947c.379 1.561 2.6 1.561 2.978 0a1.533 1.533 0 012.287-.947c1.372.836 2.942-.734 2.106-2.106a1.533 1.533 0 01.947-2.287c1.561-.379 1.561-2.6 0-2.978a1.532 1.532 0 01-.947-2.287c.836-1.372-.734-2.942-2.106-2.106a1.532 1.532 0 01-2.287-.947zM10 13a3 3 0 100-6 3 3 0 000 6z" clipRule="evenodd" />
      </svg>
    ),
    end: false,
  },
];

export function IconSidebar({ connectionState }: IconSidebarProps) {
  const { user, logout } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;

  return (
    <nav className={styles.sidebar}>
      <div className={styles.top}>
        <Tooltip content="Dashboard" position="right">
          <Link to="/" className={styles.logo}>
            M
          </Link>
        </Tooltip>

        {NAV_ITEMS.map((item) => (
          <Tooltip key={item.to} content={item.label} position="right">
            <NavLink
              to={item.to}
              end={item.end}
              data-testid={item.testId}
              className={({ isActive }) =>
                `${styles.navItem}${isActive ? ` ${styles.active}` : ""}`
              }
            >
              {item.icon}
            </NavLink>
          </Tooltip>
        ))}
      </div>

      <div className={styles.bottom}>
        <Tooltip
          content={`SSE: ${CONNECTION_LABELS[connectionState]}`}
          position="right"
        >
          <div className={styles.connectionIndicator}>
            <span
              className={`${styles.connectionDot} ${styles[connectionState]}`}
            />
          </div>
        </Tooltip>

        {user && displayName && (
          <div className={styles.userSection}>
            <Tooltip content={displayName} position="right">
              <Avatar name={displayName} size="sm" />
            </Tooltip>
            <Tooltip content="Logout" position="right">
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
