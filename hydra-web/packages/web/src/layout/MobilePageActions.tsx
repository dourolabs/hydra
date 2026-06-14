import type { ReactNode } from "react";
import { useIsMobile } from "../hooks/useIsMobile";
import styles from "./MobilePageActions.module.css";

// Slim mobile-only action row for controls that need to stay inline with the
// page (e.g. the analytics `TimeRangePicker`, which scopes the page's data
// and isn't a primary "create" action). Primary "new X" actions should use
// `FloatingActionButton` instead — that is the canonical mobile create
// affordance. Renders nothing on desktop; the shared `PageHead` carries
// those controls there.
export function MobilePageActions({ children }: { children: ReactNode }) {
  const isMobile = useIsMobile();
  if (!isMobile) return null;
  return <div className={styles.row}>{children}</div>;
}
