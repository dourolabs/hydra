import { useState, useCallback, useRef, useEffect } from "react";
import type { LabelRecord } from "@hydra/api";
import { useLabels } from "./useLabels";
import { LabelChip } from "./LabelChip";
import styles from "./LabelPicker.module.css";

/**
 * Default color palette matching the backend's DEFAULT_COLORS in labels.rs.
 */
export const LABEL_COLOR_PALETTE = [
  "#e74c3c", // red
  "#e67e22", // orange
  "#f1c40f", // yellow
  "#2ecc71", // green
  "#1abc9c", // teal
  "#3498db", // blue
  "#9b59b6", // purple
  "#e91e63", // pink
  "#795548", // brown
  "#607d8b", // blue grey
];

interface LabelPickerProps {
  selectedNames: string[];
  onChange: (names: string[]) => void;
  /** Map of new label name -> chosen color. Updated when a new label is created. */
  newLabelColors?: Map<string, string>;
  onNewLabelColorsChange?: (colors: Map<string, string>) => void;
}

export function LabelPicker({
  selectedNames,
  onChange,
  newLabelColors,
  onNewLabelColorsChange,
}: LabelPickerProps) {
  const { data: labels } = useLabels();
  const [inputValue, setInputValue] = useState("");
  const [isOpen, setIsOpen] = useState(false);
  const [selectedColor, setSelectedColor] = useState(LABEL_COLOR_PALETTE[0]);
  const containerRef = useRef<HTMLDivElement>(null);

  const handleClickOutside = useCallback((e: MouseEvent) => {
    if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
      setIsOpen(false);
    }
  }, []);

  useEffect(() => {
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [handleClickOutside]);

  const filteredLabels = (labels ?? []).filter(
    (label: LabelRecord) =>
      !label.hidden &&
      !selectedNames.includes(label.name) &&
      label.name.toLowerCase().includes(inputValue.toLowerCase()),
  );

  const showCreateOption =
    inputValue.trim() &&
    !selectedNames.includes(inputValue.trim()) &&
    !(labels ?? []).some(
      (l: LabelRecord) => l.name.toLowerCase() === inputValue.trim().toLowerCase(),
    );

  const addLabel = useCallback(
    (name: string) => {
      if (!selectedNames.includes(name)) {
        onChange([...selectedNames, name]);
      }
      setInputValue("");
      setIsOpen(false);
    },
    [selectedNames, onChange],
  );

  const addNewLabel = useCallback(
    (name: string, color: string) => {
      if (!selectedNames.includes(name)) {
        onChange([...selectedNames, name]);
      }
      if (onNewLabelColorsChange && newLabelColors) {
        const updated = new Map(newLabelColors);
        updated.set(name, color);
        onNewLabelColorsChange(updated);
      }
      setInputValue("");
      setIsOpen(false);
      setSelectedColor(LABEL_COLOR_PALETTE[0]);
    },
    [selectedNames, onChange, newLabelColors, onNewLabelColorsChange],
  );

  const removeLabel = useCallback(
    (name: string) => {
      onChange(selectedNames.filter((n) => n !== name));
      if (onNewLabelColorsChange && newLabelColors?.has(name)) {
        const updated = new Map(newLabelColors);
        updated.delete(name);
        onNewLabelColorsChange(updated);
      }
    },
    [selectedNames, onChange, newLabelColors, onNewLabelColorsChange],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        const trimmed = inputValue.trim();
        if (trimmed) {
          const isNew = !(labels ?? []).some(
            (l: LabelRecord) => l.name.toLowerCase() === trimmed.toLowerCase(),
          );
          if (isNew) {
            addNewLabel(trimmed, selectedColor);
          } else {
            addLabel(trimmed);
          }
        }
      }
      if (e.key === "Backspace" && !inputValue && selectedNames.length > 0) {
        removeLabel(selectedNames[selectedNames.length - 1]);
      }
    },
    [inputValue, selectedNames, labels, selectedColor, addLabel, addNewLabel, removeLabel],
  );

  const getLabelColor = (name: string): string => {
    const label = (labels ?? []).find((l: LabelRecord) => l.name === name);
    if (label) return label.color;
    if (newLabelColors?.has(name)) return newLabelColors.get(name)!;
    return "#6b7280";
  };

  return (
    <div className={styles.container} ref={containerRef}>
      <label className={styles.label}>Labels</label>
      <div className={styles.inputWrapper} onClick={() => setIsOpen((prev) => !prev)}>
        {selectedNames.map((name) => (
          <LabelChip
            key={name}
            name={name}
            color={getLabelColor(name)}
            onRemove={() => removeLabel(name)}
          />
        ))}
        <input
          className={styles.input}
          value={inputValue}
          onChange={(e) => {
            setInputValue(e.target.value);
            setIsOpen(true);
          }}
          onFocus={() => setIsOpen(true)}
          onKeyDown={handleKeyDown}
          placeholder={selectedNames.length === 0 ? "Add labels..." : ""}
        />
      </div>
      {isOpen && (filteredLabels.length > 0 || showCreateOption) && (
        <ul className={styles.dropdown}>
          {filteredLabels.map((label: LabelRecord) => (
            <li
              key={label.label_id}
              className={styles.option}
              onClick={() => addLabel(label.name)}
            >
              <span
                className={styles.optionDot}
                style={{ backgroundColor: label.color }}
              />
              {label.name}
            </li>
          ))}
          {showCreateOption && (
            <li
              className={styles.createOption}
              onClick={() => addNewLabel(inputValue.trim(), selectedColor)}
            >
              <div className={styles.createRow}>
                <span
                  className={styles.optionDot}
                  style={{ backgroundColor: selectedColor }}
                />
                <span className={styles.createLabel}>
                  Create &ldquo;{inputValue.trim()}&rdquo;
                </span>
              </div>
              <div className={styles.colorPalette}>
                {LABEL_COLOR_PALETTE.map((color) => (
                  <button
                    key={color}
                    type="button"
                    className={`${styles.colorSwatch} ${color === selectedColor ? styles.colorSwatchSelected : ""}`}
                    style={{ backgroundColor: color }}
                    onClick={(e) => {
                      e.stopPropagation();
                      setSelectedColor(color);
                    }}
                    aria-label={`Select color ${color}`}
                  />
                ))}
              </div>
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
