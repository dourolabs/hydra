import { TIME_RANGE_OPTIONS, type TimeRange } from "./slicerState";
import styles from "./TimeRangePicker.module.css";

const LABELS: Record<TimeRange, string> = {
  "7d": "7 days",
  "30d": "30 days",
  "90d": "90 days",
  "all-time": "All time",
};

export interface TimeRangePickerProps {
  value: TimeRange;
  onChange: (next: TimeRange) => void;
}

export function TimeRangePicker({ value, onChange }: TimeRangePickerProps) {
  return (
    <div className={styles.group} role="group" aria-label="Time range" data-testid="time-range-picker">
      {TIME_RANGE_OPTIONS.map((range) => {
        const active = range === value;
        return (
          <button
            key={range}
            type="button"
            className={`${styles.button}${active ? ` ${styles.buttonActive}` : ""}`}
            onClick={() => onChange(range)}
            aria-pressed={active}
            data-testid={`time-range-${range}`}
          >
            {LABELS[range]}
          </button>
        );
      })}
    </div>
  );
}
