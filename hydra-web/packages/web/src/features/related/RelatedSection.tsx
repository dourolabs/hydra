import type { ReactNode } from "react";
import styles from "./RelatedSection.module.css";

interface RelatedSectionProps {
  title: string;
  count: number | null;
  children: ReactNode;
}

export function RelatedSection({ title, count, children }: RelatedSectionProps) {
  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>
        <span>{title}</span>
        {count !== null && <span className={styles.count}>({count})</span>}
      </h3>
      {children}
    </section>
  );
}

interface RelatedEmptyProps {
  children: ReactNode;
}

export function RelatedEmpty({ children }: RelatedEmptyProps) {
  return <p className={styles.empty}>{children}</p>;
}

interface LoadMoreProps {
  isFetching: boolean;
  onClick: () => void;
}

export function LoadMore({ isFetching, onClick }: LoadMoreProps) {
  return (
    <div className={styles.loadMore}>
      <button
        type="button"
        className={styles.loadMoreButton}
        onClick={onClick}
        disabled={isFetching}
      >
        {isFetching ? "Loading…" : "Load more"}
      </button>
    </div>
  );
}
