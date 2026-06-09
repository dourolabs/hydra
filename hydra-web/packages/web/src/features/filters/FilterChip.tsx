import type { Ref } from "react";
import { Icons } from "@hydra/ui";
import type { Filter, FilterDefinition } from "./types";
import styles from "./FilterChip.module.css";

interface FilterChipProps<T> {
  filter: Filter;
  definition: FilterDefinition<T>;
  open: boolean;
  // `undefined` for presence filters — clicking the chip body should not
  // open the value picker because there are no values to pick.
  onOpen?: () => void;
  onRemove: () => void;
  chipRef?: Ref<HTMLDivElement>;
}

const MAX_VALUE_PREVIEW = 2;

export function FilterChip<T>({
  filter,
  definition,
  open,
  onOpen,
  onRemove,
  chipRef,
}: FilterChipProps<T>) {
  const Icon = definition.icon;
  const isPresence = definition.kind === "presence";
  const selected = filter.values
    .map((v) => definition.options.find((opt) => opt.value === v))
    .filter((opt): opt is NonNullable<typeof opt> => opt !== undefined);
  const preview = selected.slice(0, MAX_VALUE_PREVIEW);
  const overflow = selected.length - preview.length;

  const bodyContent = (
    <>
      <span className={styles.icon}>
        <Icon size={12} />
      </span>
      <span className={styles.label}>{definition.label}</span>
      {!isPresence && definition.notInSupported && (
        <span className={styles.op}>
          {filter.op === "in" ? "is" : "is not"}
        </span>
      )}
      {!isPresence && (
        <span className={styles.values}>
          {preview.length === 0 && (
            <span className={styles.placeholder}>any…</span>
          )}
          {preview.map((opt) => (
            <span key={opt.value}>{opt.chip}</span>
          ))}
          {overflow > 0 && (
            <span className={styles.overflow}>+{overflow}</span>
          )}
        </span>
      )}
    </>
  );

  return (
    <div
      ref={chipRef}
      className={`${styles.chip}${open ? ` ${styles.chipActive}` : ""}`}
      data-testid={`filter-chip-${filter.id}`}
    >
      {isPresence ? (
        <span className={`${styles.body} ${styles.bodyStatic}`}>{bodyContent}</span>
      ) : (
        <button
          type="button"
          className={styles.body}
          onClick={onOpen}
          aria-label={`${definition.label} filter — click to edit`}
          aria-expanded={open}
        >
          {bodyContent}
        </button>
      )}
      <button
        type="button"
        className={styles.remove}
        onClick={onRemove}
        aria-label={`Remove ${definition.label} filter`}
      >
        <Icons.IconX size={10} />
      </button>
    </div>
  );
}
