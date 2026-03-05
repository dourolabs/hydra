import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@metis/ui";
import type { LabelSummary, LabelRecord } from "@metis/api";
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
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const { data: allLabels } = useLabels();

  const startEditing = useCallback(() => {
    setSelectedNames(labels.map((l) => l.name));
    setEditing(true);
  }, [labels]);

  const saveMutation = useMutation({
    mutationFn: async (names: string[]) => {
      const currentNames = new Set(labels.map((l) => l.name));
      const targetNames = new Set(names);

      // Labels to add
      const toAdd = names.filter((n) => !currentNames.has(n));
      // Labels to remove
      const toRemove = labels.filter((l) => !targetNames.has(l.name));

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
          const created = await apiClient.createLabel({ label: { name } });
          await apiClient.addLabelToObject(created.label_id, issueId, true);
        }
      }
    },
    onSuccess: () => {
      setEditing(false);
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
        <LabelPicker selectedNames={selectedNames} onChange={setSelectedNames} />
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
    <div className={styles.display}>
      <span className={styles.labelHeader}>
        <span className={styles.labelTitle}>Labels</span>
        <Button variant="secondary" size="sm" onClick={startEditing}>
          Edit
        </Button>
      </span>
      {labels.length > 0 ? (
        <span className={styles.chips}>
          {labels.map((label) => (
            <LabelChip
              key={label.label_id}
              name={label.name}
              color={label.color}
            />
          ))}
        </span>
      ) : (
        <span className={styles.noLabels}>No labels</span>
      )}
    </div>
  );
}
