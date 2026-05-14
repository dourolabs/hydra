import { useCallback, useState } from "react";
import { Navigate, Outlet } from "react-router-dom";
import { Spinner, Tooltip } from "@hydra/ui";
import { useAuth } from "../features/auth/useAuth";
import { useSSE } from "../hooks/useSSE";
import { GlobalSearchModal } from "../features/search/GlobalSearchModal";
import { useGlobalSearchShortcut } from "../features/search/useGlobalSearchShortcut";
import { Sidebar } from "./Sidebar";
import { useSidebarHidden } from "./useSidebarHidden";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  const { user, loading } = useAuth();
  const sseState = useSSE();
  const { hidden, hide, show } = useSidebarHidden();
  const [searchOpen, setSearchOpen] = useState(false);

  const openSearch = useCallback(() => setSearchOpen(true), []);
  const closeSearch = useCallback(() => setSearchOpen(false), []);
  const toggleSearch = useCallback(() => setSearchOpen((prev) => !prev), []);

  useGlobalSearchShortcut(toggleSearch);

  if (loading) {
    return (
      <div className={styles.loading}>
        <Spinner size="lg" />
      </div>
    );
  }

  if (!user) {
    return <Navigate to="/login" replace />;
  }

  return (
    <div className={styles.layout}>
      <Sidebar
        connectionState={sseState}
        hidden={hidden}
        onHide={hide}
        onOpenSearch={openSearch}
      />
      {hidden && (
        <Tooltip content="Show sidebar" position="right">
          <button
            type="button"
            className={styles.floatingRestore}
            onClick={show}
            aria-label="Show sidebar"
            data-testid="sidebar-restore"
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
      )}
      <main className={styles.main}>
        <Outlet />
      </main>
      <GlobalSearchModal open={searchOpen} onClose={closeSearch} />
    </div>
  );
}
