import type { ReactNode } from "react";
import styles from "./PageHead.module.css";

interface PageHeadProps {
  eyebrow: string;
  title: string;
  actions?: ReactNode;
}

// Shared list-page header. Desktop renders a tall column with eyebrow + H1
// over a row of actions. Mobile collapses to a single thin row of eyebrow +
// actions; the H1 is hidden because the breadcrumb in SiteHeader already
// names the page.
export function PageHead({ eyebrow, title, actions }: PageHeadProps) {
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
