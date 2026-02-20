import { useLocation, useNavigate } from "react-router-dom";
import { Tooltip } from "@metis/ui";
import type { SSEConnectionState } from "../hooks/useSSE";
import styles from "./IconSidebar.module.css";

interface NavItem {
  id: string;
  label: string;
  path: string;
  icon: React.ReactNode;
}

const NAV_ITEMS: NavItem[] = [
  {
    id: "dashboard",
    label: "Dashboard",
    path: "/",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M3 10.5L10 4l7 6.5" strokeLinecap="round" strokeLinejoin="round" />
        <path d="M5 9v7a1 1 0 001 1h3v-4h2v4h3a1 1 0 001-1V9" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    ),
  },
  {
    id: "issues",
    label: "Issues",
    path: "/issues",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M4 5h12M4 10h12M4 15h8" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    id: "documents",
    label: "Documents",
    path: "/documents",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M6 2h6l4 4v10a2 2 0 01-2 2H6a2 2 0 01-2-2V4a2 2 0 012-2z" strokeLinejoin="round" />
        <path d="M12 2v4h4" strokeLinejoin="round" />
        <path d="M7 10h6M7 13h4" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    id: "patches",
    label: "Patches",
    path: "/patches",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <circle cx="6" cy="6" r="2" />
        <circle cx="14" cy="6" r="2" />
        <circle cx="10" cy="16" r="2" />
        <path d="M6 8v2a4 4 0 004 4m4-6v2a4 4 0 01-4 4" />
      </svg>
    ),
  },
  {
    id: "settings",
    label: "Settings",
    path: "/settings",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <circle cx="10" cy="10" r="3" />
        <path d="M10 1.5v2M10 16.5v2M1.5 10h2M16.5 10h2M3.4 3.4l1.4 1.4M15.2 15.2l1.4 1.4M3.4 16.6l1.4-1.4M15.2 4.8l1.4-1.4" strokeLinecap="round" />
      </svg>
    ),
  },
];

interface IconSidebarProps {
  connectionState: SSEConnectionState;
}

function isActive(itemPath: string, currentPath: string): boolean {
  if (itemPath === "/") return currentPath === "/";
  return currentPath.startsWith(itemPath);
}

export function IconSidebar({ connectionState }: IconSidebarProps) {
  const location = useLocation();
  const navigate = useNavigate();

  return (
    <nav className={styles.sidebar}>
      <div className={styles.logo}>M</div>
      <div className={styles.navItems}>
        {NAV_ITEMS.map((item) => {
          const active = isActive(item.path, location.pathname);
          return (
            <Tooltip key={item.id} content={item.label} position="right">
              <button
                className={`${styles.navButton} ${active ? styles.active : ""}`}
                onClick={() => navigate(item.path)}
                aria-label={item.label}
              >
                {item.icon}
              </button>
            </Tooltip>
          );
        })}
      </div>
      <div className={styles.bottomSection}>
        <div className={`${styles.connectionDot} ${styles[connectionState]}`} title={`SSE: ${connectionState}`} />
      </div>
    </nav>
  );
}
