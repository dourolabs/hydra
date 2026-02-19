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
    return <p className={styles.empty}>No todo items.</p>;
  }

  return (
    <ul className={styles.list}>
      {items.map((item, i) => (
        <li key={i} className={styles.item}>
          <span className={item.is_done ? styles.checkDone : styles.check}>
            {item.is_done ? "\u2611" : "\u2610"}
          </span>
          <span className={item.is_done ? styles.textDone : styles.text}>
            {item.description}
          </span>
        </li>
      ))}
    </ul>
  );
}
