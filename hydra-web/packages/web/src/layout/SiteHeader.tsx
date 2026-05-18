import { Link, useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Conversation } from "@hydra/api";
import { Icons, Kbd, Tooltip } from "@hydra/ui";
import { apiClient } from "../api/client";
import { useActiveSessionCount } from "../features/sessions/useActiveSessionCount";
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
  const { data: activeSessionCount = 0 } = useActiveSessionCount();
  const { open: openIssueCreate } = useIssueCreateModal();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const createConversation = useMutation({
    mutationFn: () => apiClient.createConversation({}),
    onSuccess: (conversation: Conversation) => {
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate(`/chat/${conversation.conversation_id}`);
    },
  });

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
          aria-label="Active sessions"
          data-testid="site-header-sessions"
        >
          <span
            className={styles.clusterDot}
            data-empty={sessionsActive ? undefined : "true"}
            data-testid="site-header-sessions-dot"
            data-active={sessionsActive ? "true" : "false"}
            aria-hidden="true"
          />
          <span data-testid="site-header-sessions-label">{sessionsLabel}</span>
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
              onSelect: () => createConversation.mutate(),
              testId: "site-header-new-conversation",
              disabled: createConversation.isPending,
            },
          ]}
        />
      </div>
    </header>
  );
}
