import { useMemo, useRef, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icons } from "@hydra/ui";
import type {
  Filter,
  FilterDefinition,
  FilterDefinitions,
  FilterGroup,
  FilterKind,
} from "./types";
import { useOutsideClick } from "./hooks/useOutsideClick";
import styles from "./AddFilterMenu.module.css";

interface AddFilterMenuProps<T> {
  definitions: FilterDefinitions<T>;
  /**
   * When provided (mobile only — desktop renders chips inline on the bar),
   * the menu prepends an "Active filters" section above the add list so the
   * user can edit or remove existing filters without leaving the sheet.
   */
  filters?: Filter[];
  anchorStyle: CSSProperties;
  onPick: (id: string) => void;
  onPickExisting?: (uid: string) => void;
  onRemoveExisting?: (uid: string) => void;
  onClearAll?: () => void;
  onClose: () => void;
}

const GROUP_ORDER: FilterGroup[] = ["properties", "people", "context", "relations"];

const GROUP_LABELS: Record<FilterGroup, string> = {
  properties: "Properties",
  people: "People",
  context: "Context",
  relations: "Relations",
};

const VALUE_PREVIEW = 2;

function hintForKind<T>(kind: FilterKind, def: FilterDefinitions<T>[string]): string {
  if (kind === "enum") return `${def.options.length} options`;
  if (kind === "user") return "person or agent";
  if (kind === "presence") return "flag";
  return def.entityLabel ?? "relation";
}

function renderValueChips<T>(filter: Filter, def: FilterDefinition<T>): ReactNode {
  if (def.kind === "presence") return null;
  const selected = filter.values
    .map((v) => def.options.find((opt) => opt.value === v))
    .filter((opt): opt is NonNullable<typeof opt> => opt !== undefined);
  if (selected.length === 0) {
    return <span className={styles.activeRowPlaceholder}>any…</span>;
  }
  const preview = selected.slice(0, VALUE_PREVIEW);
  const overflow = selected.length - preview.length;
  return (
    <>
      {preview.map((opt) => (
        <span key={opt.value}>{opt.chip}</span>
      ))}
      {overflow > 0 && <span className={styles.activeRowOverflow}>+{overflow}</span>}
    </>
  );
}

export function AddFilterMenu<T>({
  definitions,
  filters,
  anchorStyle,
  onPick,
  onPickExisting,
  onRemoveExisting,
  onClearAll,
  onClose,
}: AddFilterMenuProps<T>) {
  const menuRef = useRef<HTMLDivElement | null>(null);
  useOutsideClick(menuRef, onClose, true);

  const grouped = useMemo(() => {
    const out: Record<FilterGroup, { id: string; def: FilterDefinitions<T>[string] }[]> = {
      properties: [],
      people: [],
      context: [],
      relations: [],
    };
    for (const [id, def] of Object.entries(definitions)) {
      out[def.group].push({ id, def });
    }
    return out;
  }, [definitions]);

  const activeFilters = filters?.filter((f) => definitions[f.id]) ?? [];
  const showActive = activeFilters.length > 0;

  return createPortal(
    <>
      <div
        className={styles.mobileBackdrop}
        onClick={onClose}
        aria-hidden="true"
        data-testid="add-filter-menu-backdrop"
      />
      <div
        ref={menuRef}
        className={styles.menu}
        style={anchorStyle}
        role="menu"
        data-testid="add-filter-menu"
      >
        <div className={styles.list}>
          {showActive && (
            <div data-testid="add-filter-menu-active">
              <div className={styles.sectionRow}>
                <span className={styles.section}>Active</span>
                {onClearAll && (
                  <button
                    type="button"
                    className={styles.clearAll}
                    onClick={onClearAll}
                    data-testid="add-filter-menu-clear-all"
                  >
                    Clear all
                  </button>
                )}
              </div>
              {activeFilters.map((filter) => {
                const def = definitions[filter.id]!;
                const Icon = def.icon;
                const isPresence = def.kind === "presence";
                return (
                  <div
                    key={filter._uid}
                    className={styles.activeRow}
                    data-testid={`add-filter-menu-active-${filter.id}`}
                  >
                    {isPresence ? (
                      <span className={`${styles.activeRowMain} ${styles.activeRowStatic}`}>
                        <span className={styles.rowIcon}>
                          <Icon size={14} />
                        </span>
                        <span className={styles.rowLabel}>{def.label}</span>
                      </span>
                    ) : (
                      <button
                        type="button"
                        className={styles.activeRowMain}
                        onClick={() => onPickExisting?.(filter._uid)}
                        aria-label={`Edit ${def.label} filter`}
                      >
                        <span className={styles.rowIcon}>
                          <Icon size={14} />
                        </span>
                        <span className={styles.rowLabel}>{def.label}</span>
                        <span className={styles.activeRowValues}>
                          {renderValueChips(filter, def)}
                        </span>
                      </button>
                    )}
                    <button
                      type="button"
                      className={styles.activeRowRemove}
                      onClick={() => onRemoveExisting?.(filter._uid)}
                      aria-label={`Remove ${def.label} filter`}
                    >
                      <Icons.IconX size={12} />
                    </button>
                  </div>
                );
              })}
              <div className={styles.divider} aria-hidden="true" />
            </div>
          )}
          {GROUP_ORDER.map((group) => {
            const rows = grouped[group];
            if (rows.length === 0) return null;
            return (
              <div key={group}>
                <div className={styles.section}>{GROUP_LABELS[group]}</div>
                {rows.map(({ id, def }) => {
                  const Icon = def.icon;
                  return (
                    <button
                      key={id}
                      type="button"
                      className={styles.row}
                      onClick={() => onPick(id)}
                      data-testid={`add-filter-${id}`}
                    >
                      <span className={styles.rowIcon}>
                        <Icon size={14} />
                      </span>
                      <span className={styles.rowLabel}>{def.label}</span>
                      <span className={styles.rowHint}>{hintForKind(def.kind, def)}</span>
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
        <div className={styles.footer}>Combine to narrow results · AND across filters</div>
      </div>
    </>,
    document.body,
  );
}
