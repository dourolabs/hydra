import { Link } from "react-router-dom";
import { Tooltip } from "@hydra/ui";
import { useActiveSessionCount } from "../features/sessions/useActiveSessionCount";
import { useMediaQuery } from "../hooks/useMediaQuery";
import { Breadcrumbs } from "./Breadcrumbs";
import { useBreadcrumbsState } from "./useBreadcrumbs";
import styles from "./SiteHeader.module.css";

const MOBILE_MEDIA_QUERY = "(max-width: 768px)";

interface SiteHeaderProps {
  hidden: boolean;
  onHide: () => void;
  onShow: () => void;
  onOpenSearch: () => void;
}

function HamburgerIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <path
        fillRule="evenodd"
        d="M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z"
        clipRule="evenodd"
      />
    </svg>
  );
}

function SearchIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <path
        fillRule="evenodd"
        d="M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export function SiteHeader({
  hidden,
  onHide,
  onShow,
  onOpenSearch,
}: SiteHeaderProps) {
  const { items, current } = useBreadcrumbsState();
  const { data: activeSessionCount = 0 } = useActiveSessionCount();
  const isMobile = useMediaQuery(MOBILE_MEDIA_QUERY);
  const onToggleSidebar = hidden ? onShow : onHide;
  const toggleLabel = hidden ? "Show sidebar" : "Hide sidebar";
  const showHamburger = isMobile || hidden;
  const sessionsLabel =
    activeSessionCount === 0
      ? "no sessions"
      : activeSessionCount === 1
        ? "1 session"
        : `${activeSessionCount} sessions`;
  const sessionsActive = activeSessionCount > 0;

  return (
    <header className={styles.siteHeader} data-testid="site-header">
      {showHamburger && (
        <Tooltip content={toggleLabel} position="right" className={styles.hamburgerSlot}>
          <button
            type="button"
            className={styles.iconSlot}
            onClick={onToggleSidebar}
            aria-label={toggleLabel}
            data-testid="site-header-toggle-sidebar"
          >
            <HamburgerIcon />
          </button>
        </Tooltip>
      )}

      <div className={styles.breadcrumbsSlot} data-testid="site-header-breadcrumbs">
        {current !== null && <Breadcrumbs items={items} current={current} />}
      </div>

      <Tooltip content="Search" position="bottom">
        <button
          type="button"
          className={styles.iconSlot}
          onClick={onOpenSearch}
          aria-label="Search"
          data-testid="site-header-search"
        >
          <SearchIcon />
        </button>
      </Tooltip>

      <Link
        to="/sessions"
        className={styles.sessionsPill}
        aria-label="Active sessions"
        data-testid="site-header-sessions"
      >
        <span
          className={`${styles.sessionsDot} ${sessionsActive ? styles.sessionsDotActive : ""}`}
          data-testid="site-header-sessions-dot"
          data-active={sessionsActive ? "true" : "false"}
          aria-hidden="true"
        />
        <span data-testid="site-header-sessions-label">{sessionsLabel}</span>
      </Link>
    </header>
  );
}
