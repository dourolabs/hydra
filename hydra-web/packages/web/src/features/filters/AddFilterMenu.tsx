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
   * Currently-active filters. Used to (a) hide already-active filter types
   * from the add list when applying twice doesn't make sense (anything outside
   * the `relations` group — an item has at most one status/type/creator/…),
   * and (b) populate the mobile "Active" section when the caller wires up the
   * edit/remove handlers below.
   */
  filters?: Filter[];
  anchorStyle: CSSProperties;
  onPick: (id: string) => void;
  /**
   * When provided (mobile only — desktop shows the chips inline on the bar),
   * the menu prepends an "Active" section that lets the user edit, remove,
   * or clear existing filters without leaving the sheet.
   */
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

  // Filter types where applying twice doesn't make sense — an item has at most
  // one of each (one status, one type, one creator, …). Relations are exempt
  // because an item can relate to many issues / patches / sessions / chats.
  const activeNonRelationIds = useMemo(() => {
    const set = new Set<string>();
    for (const f of filters ?? []) {
      const def = definitions[f.id];
      if (def && def.group !== "relations") set.add(f.id);
    }
    return set;
  }, [filters, definitions]);

  const grouped = useMemo(() => {
    const out: Record<FilterGroup, { id: string; def: FilterDefinitions<T>[string] }[]> = {
      properties: [],
      people: [],
      context: [],
      relations: [],
    };
    for (const [id, def] of Object.entries(definitions)) {
      if (activeNonRelationIds.has(id)) continue;
      out[def.group].push({ id, def });
    }
    return out;
  }, [definitions, activeNonRelationIds]);

  const activeFilters = filters?.filter((f) => definitions[f.id]) ?? [];
  // Active section is mobile-only — gated on the caller wiring up edit/remove
  // handlers. Desktop renders chips inline on the bar instead.
  const showActive = activeFilters.length > 0 && !!onPickExisting;

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
