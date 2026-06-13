import type { ReactNode } from "react";
import { useIsMobile } from "../hooks/useIsMobile";
import styles from "./PageHead.module.css";

interface PageHeadProps {
  eyebrow: string;
  title: string;
  actions?: ReactNode;
}

// Desktop list-page header (eyebrow + H1 + actions). On mobile the visible
// row is suppressed entirely — the SiteHeader breadcrumb names the page, and
// each consumer migrates its actions into its own toolbar / FAB. The H1
// stays in the DOM as an accessibility landmark and as the readiness signal
// integration tests use to wait for layout.
export function PageHead({ eyebrow, title, actions }: PageHeadProps) {
  const isMobile = useIsMobile();
  if (isMobile) {
    return <h1 className={styles.srOnlyTitle}>{title}</h1>;
  }
  return (
    <div className={styles.pageHead}>
      <div className={styles.headLeft}>
        <span className={styles.eyebrow}>{eyebrow}</span>
        <h1 className={styles.pageTitle}>{title}</h1>
      </div>
      <span className={styles.headSpacer} />
      {actions != null && <div className={styles.headRight}>{actions}</div>}
    </div>
  );
}
