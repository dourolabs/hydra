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

function SessionsIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <path
        fillRule="evenodd"
        d="M10 18a8 8 0 100-16 8 8 0 000 16zm.75-13a.75.75 0 00-1.5 0v5c0 .2.08.39.22.53l3 3a.75.75 0 101.06-1.06L10.75 9.69V5z"
        clipRule="evenodd"
      />
    </svg>
  );
}

export function SiteHeader({ hidden, onHide, onShow, onOpenSearch }: SiteHeaderProps) {
  const { items, current } = useBreadcrumbsState();
  const { data: activeSessionCount = 0 } = useActiveSessionCount();
  const isMobile = useMediaQuery(MOBILE_MEDIA_QUERY);
  const onToggleSidebar = hidden ? onShow : onHide;
  const toggleLabel = hidden ? "Show sidebar" : "Hide sidebar";
  const showHamburger = isMobile || hidden;

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

      <Tooltip content="Active sessions" position="left">
        <Link
          to="/sessions"
          className={styles.iconSlot}
          aria-label="Active sessions"
          data-testid="site-header-sessions"
        >
          <SessionsIcon />
          {activeSessionCount > 0 && (
            <span className={styles.badge} data-testid="site-header-sessions-badge">
              {activeSessionCount}
            </span>
          )}
        </Link>
      </Tooltip>
    </header>
  );
}
