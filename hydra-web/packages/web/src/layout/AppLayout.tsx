import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Navigate, Outlet } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { useAuth } from "../features/auth/useAuth";
import { useSSE } from "../hooks/useSSE";
import { useAgents } from "../hooks/useAgents";
import { useUsers } from "../hooks/useUsers";
import { useMediaQuery } from "../hooks/useMediaQuery";
import { GlobalSearchModal } from "../features/search/GlobalSearchModal";
import { useGlobalSearchShortcut } from "../features/search/useGlobalSearchShortcut";
import { IssueCreateModal } from "../features/dashboard/IssueCreateModal";
import {
  IssueCreateModalProvider,
  useIssueCreateModal,
} from "../features/dashboard/useIssueCreateModal";
import { Sidebar } from "./Sidebar";
import { SiteHeader } from "./SiteHeader";
import { BreadcrumbsProvider } from "./BreadcrumbsProvider";
import { useSidebarHidden } from "./useSidebarHidden";
import styles from "./AppLayout.module.css";

const MOBILE_MEDIA_QUERY = "(max-width: 768px)";

function GlobalIssueCreateModal() {
  const { isOpen, close } = useIssueCreateModal();
  const { data: agents } = useAgents();
  const { data: users } = useUsers();
  const assignees = useMemo(() => {
    const agentNames = Array.from(
      new Set((agents ?? []).map((a) => a.name)),
    ).sort();
    const userNames = Array.from(
      new Set((users ?? []).map((u) => u.username)),
    ).sort();
    return { agents: agentNames, users: userNames };
  }, [agents, users]);
  return <IssueCreateModal open={isOpen} onClose={close} assignees={assignees} />;
}

export function AppLayout() {
  const { user, loading } = useAuth();
  const sseState = useSSE();
  const { hidden, hide, show } = useSidebarHidden();
  const [searchOpen, setSearchOpen] = useState(false);
  const isMobile = useMediaQuery(MOBILE_MEDIA_QUERY);

  const openSearch = useCallback(() => setSearchOpen(true), []);
  const closeSearch = useCallback(() => setSearchOpen(false), []);
  const toggleSearch = useCallback(() => setSearchOpen((prev) => !prev), []);

  useGlobalSearchShortcut(toggleSearch);

  // When crossing the mobile→desktop boundary, auto-close any open drawer.
  // Without this, an open mobile drawer (hidden=false) translates into
  // sidebarMode="wide" on desktop — fine — but a closed mobile drawer
  // (hidden=true) leaves the sidebar collapsed on desktop with no obvious
  // way to recover. The hamburger in the topbar handles the recovery case,
  // but we still snap state cleanly when the breakpoint flips.
  const wasMobileRef = useRef(isMobile);
  useEffect(() => {
    if (wasMobileRef.current && !isMobile && hidden) {
      // Coming back to desktop with a hidden sidebar — show it so the user
      // doesn't end up with no sidebar and no visible toggle.
      show();
    }
    wasMobileRef.current = isMobile;
  }, [isMobile, hidden, show]);

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

  // On desktop, "hidden" collapses the sidebar to 0 width.
  // On mobile, the sidebar is a drawer — "hidden" hides it, otherwise it slides in.
  const sidebarMode = isMobile ? (hidden ? "hidden" : "open") : hidden ? "hidden" : "wide";

  return (
    <BreadcrumbsProvider>
      <IssueCreateModalProvider>
        <div className={styles.layout} data-sidebar={sidebarMode}>
          {isMobile && !hidden && (
            <div
              className={styles.backdrop}
              onClick={hide}
              aria-hidden="true"
              data-testid="sidebar-backdrop"
            />
          )}
          <div className={styles.sidebarSlot}>
            <Sidebar
              connectionState={sseState}
              hidden={hidden}
              onHide={hide}
              onOpenSearch={openSearch}
            />
          </div>
          <div className={styles.contentColumn}>
            <SiteHeader hidden={hidden} onHide={hide} onShow={show} onOpenSearch={openSearch} />
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
