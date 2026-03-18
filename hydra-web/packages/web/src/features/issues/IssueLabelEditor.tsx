import { useState, useCallback, useMemo } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type { LabelSummary, LabelRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { LabelChip } from "../labels/LabelChip";
import { LabelPicker } from "../labels/LabelPicker";
import { useLabels } from "../labels/useLabels";
import { useToast } from "../toast/useToast";
import styles from "./IssueLabelEditor.module.css";

interface IssueLabelEditorProps {
  issueId: string;
  labels: LabelSummary[];
}

export function IssueLabelEditor({ issueId, labels }: IssueLabelEditorProps) {
  const [editing, setEditing] = useState(false);
  const [selectedNames, setSelectedNames] = useState<string[]>([]);
  const [newLabelColors, setNewLabelColors] = useState<Map<string, string>>(
    new Map(),
  );
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const { data: allLabels } = useLabels();

  const visibleLabels = useMemo(() => labels.filter((l) => !l.hidden), [labels]);

  const startEditing = useCallback(() => {
    setSelectedNames(visibleLabels.map((l) => l.name));
    setNewLabelColors(new Map());
    setEditing(true);
  }, [visibleLabels]);

  const saveMutation = useMutation({
    mutationFn: async (names: string[]) => {
      const currentNames = new Set(visibleLabels.map((l) => l.name));
      const targetNames = new Set(names);

      // Labels to add
      const toAdd = names.filter((n) => !currentNames.has(n));
      // Labels to remove (only consider visible labels; hidden labels are never touched)
      const toRemove = visibleLabels.filter((l) => !targetNames.has(l.name));

      // Remove labels
      for (const label of toRemove) {
        await apiClient.removeLabelFromObject(label.label_id, issueId);
      }

      // Add labels - for existing labels, use their ID; for new labels, create first
      for (const name of toAdd) {
        const existing = (allLabels ?? []).find(
          (l: LabelRecord) => l.name.toLowerCase() === name.toLowerCase(),
        );
        if (existing) {
          await apiClient.addLabelToObject(existing.label_id, issueId, true);
        } else {
          const color = newLabelColors.get(name);
          const created = await apiClient.createLabel({
            label: color ? { name, color } : { name },
          });
          await apiClient.addLabelToObject(created.label_id, issueId, true);
        }
      }
    },
    onSuccess: () => {
      setEditing(false);
      setNewLabelColors(new Map());
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["labels"] });
    },
    onError: (err) => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["labels"] });
      addToast(
        err instanceof Error ? err.message : "Failed to update labels",
        "error",
      );
    },
  });

  if (editing) {
    return (
      <div className={styles.editor}>
        <LabelPicker
          selectedNames={selectedNames}
          onChange={setSelectedNames}
          newLabelColors={newLabelColors}
          onNewLabelColorsChange={setNewLabelColors}
        />
        <div className={styles.editorActions}>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setEditing(false)}
            disabled={saveMutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={() => saveMutation.mutate(selectedNames)}
            disabled={saveMutation.isPending}
          >
            {saveMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.display} data-testid="label-editor">
      {visibleLabels.map((label) => (
        <LabelChip
          key={label.label_id}
          name={label.name}
          color={label.color}
        />
      ))}
      <button
        className={styles.editTrigger}
        onClick={startEditing}
        aria-label="Edit labels"
      >
        {visibleLabels.length > 0 ? "✎" : "+ Add label"}
      </button>
    </div>
  );
}
