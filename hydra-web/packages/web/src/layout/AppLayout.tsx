import { useCallback, useMemo, useState } from "react";
import { Navigate, Outlet } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { useAuth } from "../features/auth/useAuth";
import { useSSE } from "../hooks/useSSE";
import { useAgents } from "../hooks/useAgents";
import { GlobalSearchModal } from "../features/search/GlobalSearchModal";
import { useGlobalSearchShortcut } from "../features/search/useGlobalSearchShortcut";
import { IssueCreateModal } from "../features/dashboard/IssueCreateModal";
import {
  IssueCreateModalProvider,
  useIssueCreateModal,
} from "../features/dashboard/useIssueCreateModal";
import { Sidebar } from "./Sidebar";
import { SiteHeader } from "./SiteHeader";
import { AppChrome } from "./AppChrome";
import { BreadcrumbsProvider } from "./BreadcrumbsProvider";
import { useSidebarHidden } from "./useSidebarHidden";
import styles from "./AppLayout.module.css";

function GlobalIssueCreateModal() {
  const { isOpen, close } = useIssueCreateModal();
  const { data: agents } = useAgents();
  const assignees = useMemo(() => {
    const names = (agents ?? []).map((a) => a.name);
    return Array.from(new Set(names)).sort();
  }, [agents]);
  return (
    <IssueCreateModal open={isOpen} onClose={close} assignees={assignees} />
  );
}

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
      <IssueCreateModalProvider>
        <div className={styles.layout}>
          <AppChrome hidden={hidden} onHide={hide} onShow={show} />
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
          <GlobalIssueCreateModal />
        </div>
      </IssueCreateModalProvider>
    </BreadcrumbsProvider>
  );
}
