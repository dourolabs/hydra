import { Link } from "react-router-dom";
import { Icons, Kbd, Tooltip } from "@hydra/ui";
import { actorDisplayName } from "../api/auth";
import { useAuth } from "../features/auth/useAuth";
import { useActiveSessionCount } from "../features/sessions/useActiveSessionCount";
import { useChatCreateModal } from "../features/chat/useChatCreateModal";
import { useIssueCreateModal } from "../features/dashboard/useIssueCreateModal";
import { Breadcrumbs } from "./Breadcrumbs";
import { useBreadcrumbsState } from "./useBreadcrumbs";
import { HeaderActionMenu } from "./HeaderActionMenu";
import styles from "./SiteHeader.module.css";

interface SiteHeaderProps {
  hidden: boolean;
  onHide: () => void;
  onShow: () => void;
  onOpenSearch: () => void;
}

export function SiteHeader({ hidden, onHide, onShow, onOpenSearch }: SiteHeaderProps) {
  const { items, current, currentKind } = useBreadcrumbsState();
  const { user } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;
  const { data: activeSessionCount = 0 } = useActiveSessionCount(displayName);
  const { open: openIssueCreate } = useIssueCreateModal();
  const { open: openChatCreate } = useChatCreateModal();

  const onToggleSidebar = hidden ? onShow : onHide;
  const toggleLabel = hidden ? "Show sidebar" : "Hide sidebar";

  const sessionsLabel =
    activeSessionCount === 0
      ? "no sessions"
      : activeSessionCount === 1
        ? "1 session"
        : `${activeSessionCount} sessions`;
  const sessionsActive = activeSessionCount > 0;

  return (
    <header className={styles.topbar} data-testid="site-header">
      <Tooltip content={toggleLabel} position="right">
        <button
          type="button"
          className={styles.hamburger}
          onClick={onToggleSidebar}
          aria-label={toggleLabel}
          data-testid="site-header-toggle-sidebar"
        >
          <Icons.IconMenu />
        </button>
      </Tooltip>

      <div className={styles.breadcrumbs} data-testid="site-header-breadcrumbs">
        {current !== null && (
          <Breadcrumbs items={items} current={current} currentKind={currentKind} />
        )}
      </div>

      <div className={styles.right}>
        <Link
          to="/sessions"
          className={styles.clusterStatus}
          aria-label={`Active sessions: ${sessionsLabel}`}
          data-testid="site-header-sessions"
        >
          <span
            className={styles.clusterDot}
            data-empty={sessionsActive ? undefined : "true"}
            data-testid="site-header-sessions-dot"
            data-active={sessionsActive ? "true" : "false"}
            aria-hidden="true"
          />
          <span className={styles.clusterStatusLabel} data-testid="site-header-sessions-label">
            {sessionsLabel}
          </span>
        </Link>

        <button
          type="button"
          className={styles.searchButton}
          onClick={onOpenSearch}
          aria-label="Search"
          data-testid="site-header-search"
        >
          <Icons.IconSearch />
          <Kbd>⌘K</Kbd>
        </button>

        <HeaderActionMenu
          triggerLabel="Create new"
          triggerTestId="site-header-create"
          menuTestId="site-header-create-menu"
          items={[
            {
              key: "new-issue",
              label: "New issue",
              icon: <Icons.IconIssue size={14} />,
              onSelect: openIssueCreate,
              testId: "site-header-new-issue",
            },
            {
              key: "new-conversation",
              label: "New conversation",
              icon: <Icons.IconChat size={14} />,
              onSelect: openChatCreate,
              testId: "site-header-new-conversation",
            },
          ]}
        />
      </div>
    </header>
  );
}
