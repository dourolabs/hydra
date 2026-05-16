import { Fragment } from "react";
import { Link } from "react-router-dom";
import { Icons } from "@hydra/ui";
import styles from "./Breadcrumbs.module.css";

export interface BreadcrumbItem {
  label: string;
  to: string;
  /** When set, render the label as a mono code-chip (used for IDs). */
  kind?: "code";
}

export interface BreadcrumbsProps {
  items: BreadcrumbItem[];
  current: string;
  /** When true, render the current crumb as a mono code-chip. */
  currentKind?: "code";
}

function Separator() {
  return (
    <span className={styles.separator} aria-hidden="true">
      <Icons.IconChevronRight />
    </span>
  );
}

export function Breadcrumbs({ items, current, currentKind }: BreadcrumbsProps) {
  return (
    <nav aria-label="Breadcrumb" className={styles.breadcrumbs}>
      {items.map((item, i) => (
        <Fragment key={`${item.to}-${i}`}>
          {i > 0 && <Separator />}
          <Link
            to={item.to}
            className={item.kind === "code" ? styles.code : styles.link}
          >
            {item.label}
          </Link>
        </Fragment>
      ))}
      {items.length > 0 && <Separator />}
      <span className={currentKind === "code" ? styles.currentCode : styles.current}>
        {current}
      </span>
    </nav>
  );
}
