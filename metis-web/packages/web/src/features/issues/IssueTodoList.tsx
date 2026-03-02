import styles from "./IssueTodoList.module.css";

interface TodoItem {
  description: string;
  is_done: boolean;
}

interface IssueTodoListProps {
  items: TodoItem[];
}

export function IssueTodoList({ items }: IssueTodoListProps) {
  if (items.length === 0) {
    return (
      <div className={styles.empty}>
        <p className={styles.emptyText}>No todo items.</p>
      </div>
    );
  }

  return (
    <ul className={styles.list}>
      {items.map((item, i) => (
        <li key={i} className={styles.item}>
          <span className={item.is_done ? styles.checkboxDone : styles.checkbox}>
            {item.is_done && (
              <svg viewBox="0 0 12 12" className={styles.checkIcon}>
                <path d="M2.5 6l3 3 4.5-5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            )}
          </span>
          <span className={item.is_done ? styles.textDone : styles.text}>
            {item.description}
          </span>
        </li>
      ))}
    </ul>
  );
}
