import { useState, type ReactNode } from "react";
import { Button } from "@hydra/ui";
import sharedStyles from "../SettingsSection/SettingsSection.module.css";

interface ExpandableRowProps {
  name: string;
  onEdit: () => void;
  onDelete: () => void;
  children: ReactNode;
  className?: string;
  headerExtra?: ReactNode;
}

export function ExpandableRow({
  name,
  onEdit,
  onDelete,
  children,
  className,
  headerExtra,
}: ExpandableRowProps) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={className ? `${sharedStyles.item} ${className}` : sharedStyles.item}>
      <button
        type="button"
        className={sharedStyles.header}
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
      >
        <span className={sharedStyles.chevron} aria-hidden="true">
          {expanded ? "▾" : "▸"}
        </span>
        <span className={sharedStyles.name}>{name}</span>
        {headerExtra}
        <div className={sharedStyles.rowActions}>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onEdit();
            }}
          >
            Edit
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
          >
            Delete
          </Button>
        </div>
      </button>
      {expanded && (
        <div className={sharedStyles.details}>
          {children}
        </div>
      )}
    </div>
  );
}
