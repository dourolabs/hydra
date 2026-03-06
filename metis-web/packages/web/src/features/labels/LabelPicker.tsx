import { useState, useCallback, useRef, useEffect } from "react";
import type { LabelRecord } from "@metis/api";
import { useLabels } from "./useLabels";
import { LabelChip } from "./LabelChip";
import styles from "./LabelPicker.module.css";

interface LabelPickerProps {
  selectedNames: string[];
  onChange: (names: string[]) => void;
}

export function LabelPicker({ selectedNames, onChange }: LabelPickerProps) {
  const { data: labels } = useLabels();
  const [inputValue, setInputValue] = useState("");
  const [isOpen, setIsOpen] = useState(false);
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
    },
    [selectedNames, onChange],
  );

  const removeLabel = useCallback(
    (name: string) => {
      onChange(selectedNames.filter((n) => n !== name));
    },
    [selectedNames, onChange],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        const trimmed = inputValue.trim();
        if (trimmed) {
          addLabel(trimmed);
        }
      }
      if (e.key === "Backspace" && !inputValue && selectedNames.length > 0) {
        removeLabel(selectedNames[selectedNames.length - 1]);
      }
    },
    [inputValue, selectedNames, addLabel, removeLabel],
  );

  const getLabelColor = (name: string): string => {
    const label = (labels ?? []).find((l: LabelRecord) => l.name === name);
    if (label) return label.color;
    // Default color for new labels
    return "#6b7280";
  };

  return (
    <div className={styles.container} ref={containerRef}>
      <label className={styles.label}>Labels</label>
      <div className={styles.inputWrapper} onClick={() => setIsOpen(true)}>
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
              className={styles.option}
              onClick={() => addLabel(inputValue.trim())}
            >
              <span className={styles.createLabel}>
                Create &ldquo;{inputValue.trim()}&rdquo;
              </span>
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
