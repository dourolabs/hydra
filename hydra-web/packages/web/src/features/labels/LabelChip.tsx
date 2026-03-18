import styles from "./LabelChip.module.css";

interface LabelChipProps {
  name: string;
  color: string;
  onRemove?: () => void;
}

export function LabelChip({ name, color, onRemove }: LabelChipProps) {
  return (
    <span className={styles.chip} style={{ borderColor: color }}>
      <span className={styles.dot} style={{ backgroundColor: color }} />
      <span className={styles.name}>{name}</span>
      {onRemove && (
        <button
          className={styles.remove}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          type="button"
          aria-label={`Remove label ${name}`}
        >
          &times;
        </button>
      )}
    </span>
  );
}
