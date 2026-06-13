import { useMemo, useRef, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import type { FilterDefinitions, FilterGroup, FilterKind } from "./types";
import { useOutsideClick } from "./hooks/useOutsideClick";
import styles from "./AddFilterMenu.module.css";

interface AddFilterMenuProps<T> {
  definitions: FilterDefinitions<T>;
  anchorStyle: CSSProperties;
  onPick: (id: string) => void;
  onClose: () => void;
}

const GROUP_ORDER: FilterGroup[] = ["properties", "people", "context", "relations"];

const GROUP_LABELS: Record<FilterGroup, string> = {
  properties: "Properties",
  people: "People",
  context: "Context",
  relations: "Relations",
};

function hintForKind<T>(kind: FilterKind, def: FilterDefinitions<T>[string]): string {
  if (kind === "enum") return `${def.options.length} options`;
  if (kind === "user") return "person or agent";
  if (kind === "presence") return "flag";
  return def.entityLabel ?? "relation";
}

export function AddFilterMenu<T>({
  definitions,
  anchorStyle,
  onPick,
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
