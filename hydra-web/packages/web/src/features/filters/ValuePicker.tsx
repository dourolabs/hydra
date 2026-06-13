import { useMemo, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { Icons } from "@hydra/ui";
import type { Filter, FilterDefinition } from "./types";
import { useOutsideClick } from "./hooks/useOutsideClick";
import styles from "./ValuePicker.module.css";

const SEARCH_THRESHOLD = 6;

interface ValuePickerProps<T> {
  filter: Filter;
  definition: FilterDefinition<T>;
  anchorStyle: CSSProperties;
  onChange: (next: Filter) => void;
  onClose: () => void;
}

export function ValuePicker<T>({
  filter,
  definition,
  anchorStyle,
  onChange,
  onClose,
}: ValuePickerProps<T>) {
  const pickerRef = useRef<HTMLDivElement | null>(null);
  const [search, setSearch] = useState("");
  useOutsideClick(pickerRef, onClose, true);

  const Icon = definition.icon;
  const showSearch = definition.options.length > SEARCH_THRESHOLD;
  const showNotInToggle = definition.notInSupported === true;

  const visibleOptions = useMemo(() => {
    if (!search.trim()) return definition.options;
    const q = search.trim().toLowerCase();
    return definition.options.filter((opt) => {
      const hay = `${opt.label} ${opt.value} ${opt.sub ?? ""}`.toLowerCase();
      return hay.includes(q);
    });
  }, [definition.options, search]);

  const selectedSet = useMemo(() => new Set(filter.values), [filter.values]);

  function toggleValue(value: string) {
    if (definition.singleSelect) {
      // Radio behaviour: picking a new value replaces the selection; picking
      // the currently-selected value clears it. Single-select filters back
      // server params that take a single value (e.g. `?status=open`), so we
      // must enforce one-at-a-time at the picker UI to avoid drifting from
      // what the server can express.
      const nextValues = selectedSet.has(value) ? [] : [value];
      onChange({ ...filter, values: nextValues });
      return;
    }
    const nextValues = selectedSet.has(value)
      ? filter.values.filter((v) => v !== value)
      : [...filter.values, value];
    onChange({ ...filter, values: nextValues });
  }

  function setOp(op: Filter["op"]) {
    onChange({ ...filter, op });
  }

  function clearValues() {
    onChange({ ...filter, values: [] });
  }

  return createPortal(
    <>
      <div
        className={styles.mobileBackdrop}
        onClick={onClose}
        aria-hidden="true"
        data-testid={`value-picker-backdrop-${filter.id}`}
      />
      <div
        ref={pickerRef}
        className={styles.picker}
        style={anchorStyle}
        data-testid={`value-picker-${filter.id}`}
      >
        <div className={styles.header}>
          <span className={styles.headerIcon}>
            <Icon size={14} />
          </span>
          <span className={styles.headerLabel}>{definition.label}</span>
          {showNotInToggle && (
            <div className={styles.segmented} role="tablist" aria-label="Match mode">
              <button
                type="button"
                role="tab"
                aria-selected={filter.op === "in"}
                className={`${styles.segmentedButton}${
                  filter.op === "in" ? ` ${styles.segmentedActive}` : ""
                }`}
                onClick={() => setOp("in")}
              >
                is
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={filter.op === "not_in"}
                className={`${styles.segmentedButton}${
                  filter.op === "not_in" ? ` ${styles.segmentedActive}` : ""
                }`}
                onClick={() => setOp("not_in")}
              >
                is not
              </button>
            </div>
          )}
        </div>

        {showSearch && (
          <div className={styles.searchBox}>
            <span className={styles.searchIcon}>
              <Icons.IconSearch size={12} />
            </span>
            <input
              type="text"
              className={styles.searchInput}
              placeholder={`Search ${definition.label.toLowerCase()}…`}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              aria-label={`Search ${definition.label}`}
              autoFocus
            />
          </div>
        )}

        <div className={styles.list}>
          {visibleOptions.length === 0 && <div className={styles.empty}>No matches.</div>}
          {visibleOptions.map((opt) => {
            const isChecked = selectedSet.has(opt.value);
            return (
              <button
                key={opt.value}
                type="button"
                className={`${styles.row}${isChecked ? ` ${styles.rowChecked}` : ""}`}
                onClick={() => toggleValue(opt.value)}
                aria-pressed={isChecked}
                data-testid={`value-option-${opt.value}`}
              >
                <span className={styles.rowCheck}>
                  <Icons.IconCheck size={12} />
                </span>
                <span className={styles.rowRender}>{opt.render}</span>
                {opt.sub && <span className={styles.rowSub}>{opt.sub}</span>}
              </button>
            );
          })}
        </div>

        {filter.values.length > 0 && (
          <div className={styles.footer}>
            <button type="button" className={styles.footerButton} onClick={clearValues}>
              Clear values
            </button>
            <button
              type="button"
              className={`${styles.footerButton} ${styles.footerButtonPrimary}`}
              onClick={onClose}
            >
              Done
            </button>
          </div>
        )}
      </div>
    </>,
    document.body,
  );
}
