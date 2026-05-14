import { useCallback, useState } from "react";
import { Navigate, Outlet } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { useAuth } from "../features/auth/useAuth";
import { useSSE } from "../hooks/useSSE";
import { GlobalSearchModal } from "../features/search/GlobalSearchModal";
import { useGlobalSearchShortcut } from "../features/search/useGlobalSearchShortcut";
import { Sidebar } from "./Sidebar";
import { SiteHeader } from "./SiteHeader";
import { BreadcrumbsProvider } from "./BreadcrumbsProvider";
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
    <BreadcrumbsProvider>
      <div className={styles.layout}>
        <Sidebar connectionState={sseState} hidden={hidden} onHide={hide} />
        <div className={styles.contentColumn}>
          <SiteHeader
            hidden={hidden}
            onHide={hide}
            onShow={show}
            onOpenSearch={openSearch}
          />
          <main className={styles.main}>
            <Outlet />
          </main>
        </div>
        <GlobalSearchModal open={searchOpen} onClose={closeSearch} />
      </div>
    </BreadcrumbsProvider>
  );
}
