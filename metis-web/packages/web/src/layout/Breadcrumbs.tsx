import { Link } from "react-router-dom";
import styles from "./Breadcrumbs.module.css";

export interface BreadcrumbItem {
  label: string;
  to: string;
}

export interface BreadcrumbsProps {
  items: BreadcrumbItem[];
  current: string;
}

export function Breadcrumbs({ items, current }: BreadcrumbsProps) {
  return (
    <nav aria-label="Breadcrumb" className={styles.breadcrumbs}>
      {items.map((item, i) => (
        <span key={item.to}>
          {i > 0 && <span className={styles.separator}>/</span>}{" "}
          <Link to={item.to} className={styles.link}>
            {item.label}
          </Link>{" "}
        </span>
      ))}
      <span className={styles.separator}>/</span>{" "}
      <span className={styles.current}>{current}</span>
    </nav>
  );
}
