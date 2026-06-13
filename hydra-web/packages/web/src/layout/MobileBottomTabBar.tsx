import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { Icons } from "@hydra/ui";
import { actorDisplayName } from "../api/auth";
import { useAuth } from "../features/auth/useAuth";
import { useActiveSessionCount } from "../features/sessions/useActiveSessionCount";
import { getActiveTabId, type MobileBottomTabId } from "./getActiveTabId";
import styles from "./MobileBottomTabBar.module.css";

interface PrimaryTab {
  id: Exclude<MobileBottomTabId, "more">;
  label: string;
  to: string;
  icon: ReactNode;
  badge?: number | "dot";
}

interface MobileBottomTabBarProps {
  onOpenSidebar: () => void;
}

export function MobileBottomTabBar({ onOpenSidebar }: MobileBottomTabBarProps) {
  const { pathname } = useLocation();
  const activeId = getActiveTabId(pathname);
  const { user } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;
  const { data: activeSessionCount = 0 } = useActiveSessionCount(displayName);

  const primaryTabs: PrimaryTab[] = [
    { id: "issues", label: "Issues", to: "/", icon: <Icons.IconIssue size={22} /> },
    { id: "patches", label: "Patches", to: "/patches", icon: <Icons.IconPatch size={22} /> },
    {
      id: "sessions",
      label: "Sessions",
      to: "/sessions",
      icon: <Icons.IconPlay size={22} />,
      badge: activeSessionCount > 0 ? activeSessionCount : undefined,
    },
    { id: "chat", label: "Chat", to: "/chat", icon: <Icons.IconChat size={22} /> },
  ];

  return (
    <nav
      className={styles.bar}
      aria-label="Primary mobile navigation"
      data-testid="mobile-bottom-tab-bar"
    >
      {primaryTabs.map((tab) => {
        const isActive = activeId === tab.id;
        return (
          <Link
            key={tab.id}
            to={tab.to}
            className={`${styles.tab}${isActive ? ` ${styles.tabActive}` : ""}`}
            aria-current={isActive ? "page" : undefined}
            aria-label={tabAriaLabel(tab)}
            data-testid={`mobile-bottom-tab-${tab.id}`}
            data-active={isActive ? "true" : undefined}
          >
            <span className={styles.icon} aria-hidden="true">
              {tab.icon}
              {tab.badge !== undefined && (
                <TabBadge
                  value={tab.badge}
                  testId={`mobile-bottom-tab-${tab.id}-badge`}
                />
              )}
            </span>
            <span className={styles.label}>{tab.label}</span>
          </Link>
        );
      })}
      <button
        type="button"
        className={`${styles.tab}${activeId === "more" ? ` ${styles.tabActive}` : ""}`}
        aria-label="More navigation"
        aria-current={activeId === "more" ? "page" : undefined}
        onClick={onOpenSidebar}
        data-testid="mobile-bottom-tab-more"
        data-active={activeId === "more" ? "true" : undefined}
      >
        <span className={styles.icon} aria-hidden="true">
          <Icons.IconMore size={22} />
        </span>
        <span className={styles.label}>More</span>
      </button>
    </nav>
  );
}

interface TabBadgeProps {
  value: number | "dot";
  testId: string;
}

function TabBadge({ value, testId }: TabBadgeProps) {
  if (value === "dot") {
    return (
      <span
        className={`${styles.badge} ${styles.badgeDot}`}
        data-testid={testId}
        aria-hidden="true"
      />
    );
  }
  // Numeric badges truncate at 99+ to keep the pill width predictable.
  const display = value > 99 ? "99+" : String(value);
  return (
    <span className={styles.badge} data-testid={testId} aria-hidden="true">
      {display}
    </span>
  );
}

function tabAriaLabel(tab: PrimaryTab): string | undefined {
  if (typeof tab.badge === "number" && tab.badge > 0) {
    return `${tab.label}, ${tab.badge} active`;
  }
  if (tab.badge === "dot") {
    return `${tab.label}, active`;
  }
  return undefined;
}
