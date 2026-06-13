import { useCallback, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type { Issue, IssueVersionRecord, SessionSettings } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./IssueSettingsEditor.module.css";

interface IssueSettingsEditorProps {
  issueId: string;
  issue: Issue;
}

type StringField =
  | "repo_name"
  | "remote_url"
  | "image"
  | "model"
  | "branch"
  | "cpu_limit"
  | "memory_limit";

interface FieldSpec {
  key: StringField;
  label: string;
  placeholder: string;
}

const STRING_FIELDS: FieldSpec[] = [
  { key: "repo_name", label: "Repository", placeholder: "owner/name" },
  { key: "remote_url", label: "Remote URL", placeholder: "https://github.com/owner/name.git" },
  { key: "image", label: "Image", placeholder: "Agent default" },
  { key: "model", label: "Model", placeholder: "Agent default" },
  { key: "branch", label: "Branch", placeholder: "main" },
  { key: "cpu_limit", label: "CPU limit", placeholder: "Agent default" },
  { key: "memory_limit", label: "Memory limit", placeholder: "Agent default" },
];

interface DraftState {
  repo_name: string;
  remote_url: string;
  image: string;
  model: string;
  branch: string;
  cpu_limit: string;
  memory_limit: string;
  max_retries: string;
  secrets: string;
}

function settingsToDraft(s: SessionSettings | undefined | null): DraftState {
  return {
    repo_name: s?.repo_name ?? "",
    remote_url: s?.remote_url ?? "",
    image: s?.image ?? "",
    model: s?.model ?? "",
    branch: s?.branch ?? "",
    cpu_limit: s?.cpu_limit ?? "",
    memory_limit: s?.memory_limit ?? "",
    max_retries: s?.max_retries != null ? String(s.max_retries) : "",
    secrets: (s?.secrets ?? []).join(", "),
  };
}

function draftToSettings(
  draft: DraftState,
  base: SessionSettings | undefined | null,
): { settings: SessionSettings; error: string | null } {
  const next: SessionSettings = { ...(base ?? {}) };

  const trim = (v: string) => v.trim();
  const setOrNull = (key: StringField, value: string) => {
    const t = trim(value);
    next[key] = t === "" ? null : t;
  };

  setOrNull("repo_name", draft.repo_name);
  setOrNull("remote_url", draft.remote_url);
  setOrNull("image", draft.image);
  setOrNull("model", draft.model);
  setOrNull("branch", draft.branch);
  setOrNull("cpu_limit", draft.cpu_limit);
  setOrNull("memory_limit", draft.memory_limit);

  const retriesRaw = trim(draft.max_retries);
  if (retriesRaw === "") {
    next.max_retries = null;
  } else {
    const n = Number(retriesRaw);
    if (!Number.isInteger(n) || n < 0) {
      return {
        settings: next,
        error: "Max retries must be a non-negative integer.",
      };
    }
    next.max_retries = n;
  }

  const secretsRaw = trim(draft.secrets);
  if (secretsRaw === "") {
    next.secrets = null;
  } else {
    const parts = secretsRaw
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    next.secrets = parts.length > 0 ? parts : null;
  }

  return { settings: next, error: null };
}

function DisplayRow({
  testKey,
  label,
  value,
}: {
  testKey: string;
  label: string;
  value: string | null;
}) {
  return (
    <div className={styles.row} data-testid={`issue-settings-row-${testKey}`}>
      <dt className={styles.rowLabel}>{label}</dt>
      <dd className={styles.rowValue}>
        {value != null ? (
          <span className={styles.rowMono}>{value}</span>
        ) : (
          <span className={styles.rowEmpty}>Agent default</span>
        )}
      </dd>
    </div>
  );
}

export function IssueSettingsEditor({ issueId, issue }: IssueSettingsEditorProps) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const settings = issue.session_settings;

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<DraftState>(() => settingsToDraft(settings));
  const [validationError, setValidationError] = useState<string | null>(null);

  const startEditing = useCallback(() => {
    setDraft(settingsToDraft(settings));
    setValidationError(null);
    setEditing(true);
  }, [settings]);

  const cancel = useCallback(() => {
    setDraft(settingsToDraft(settings));
    setValidationError(null);
    setEditing(false);
  }, [settings]);

  const mutation = useMutation<
    unknown,
    Error,
    SessionSettings,
    { previous?: IssueVersionRecord }
  >({
    mutationFn: (nextSettings) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: issue.status.key,
          session_settings: nextSettings,
        },
        session_id: null,
      }),
    onMutate: async (nextSettings) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: { ...previous.issue, session_settings: nextSettings },
        });
      }
      return { previous };
    },
    onSuccess: () => {
      setEditing(false);
      addToast("Session settings updated.", "success");
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(err.message || "Failed to update settings", "error");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
    },
  });

  const onSave = useCallback(() => {
    const { settings: nextSettings, error } = draftToSettings(draft, settings);
    if (error) {
      setValidationError(error);
      return;
    }
    setValidationError(null);
    mutation.mutate(nextSettings);
  }, [draft, settings, mutation]);

  const displayedSecrets = useMemo(() => {
    const list = (settings?.secrets ?? []).filter(Boolean);
    return list.length > 0 ? list.join(", ") : null;
  }, [settings?.secrets]);

  if (!editing) {
    return (
      <div className={styles.wrapper} data-testid="issue-settings-display">
        <dl className={styles.list}>
          {STRING_FIELDS.map((field) => (
            <DisplayRow
              key={field.key}
              testKey={field.key}
              label={field.label}
              value={(settings?.[field.key] as string | null | undefined) ?? null}
            />
          ))}
          <DisplayRow
            testKey="max_retries"
            label="Max retries"
            value={
              settings?.max_retries != null ? String(settings.max_retries) : null
            }
          />
          <DisplayRow
            testKey="secrets"
            label="Secrets"
            value={displayedSecrets}
          />
        </dl>
        <div className={styles.actions}>
          <Button
            variant="secondary"
            size="sm"
            onClick={startEditing}
            data-testid="issue-settings-edit"
          >
            Edit settings
          </Button>
        </div>
      </div>
    );
  }

  const inputClass = (extra?: string) =>
    [styles.input, extra].filter(Boolean).join(" ");

  return (
    <div className={styles.wrapper} data-testid="issue-settings-editor">
      <div className={styles.form}>
        {STRING_FIELDS.map((field) => (
          <label key={field.key} className={styles.field}>
            <span className={styles.fieldLabel}>{field.label}</span>
            <input
              type="text"
              className={inputClass()}
              value={draft[field.key]}
              placeholder={field.placeholder}
              onChange={(e) =>
                setDraft((d) => ({ ...d, [field.key]: e.target.value }))
              }
              disabled={mutation.isPending}
              data-testid={`issue-settings-input-${field.key}`}
            />
          </label>
        ))}
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Max retries</span>
          <input
            type="number"
            inputMode="numeric"
            min={0}
            step={1}
            className={inputClass()}
            value={draft.max_retries}
            placeholder="Agent default"
            onChange={(e) =>
              setDraft((d) => ({ ...d, max_retries: e.target.value }))
            }
            disabled={mutation.isPending}
            data-testid="issue-settings-input-max_retries"
          />
        </label>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Secrets</span>
          <input
            type="text"
            className={inputClass()}
            value={draft.secrets}
            placeholder="comma-separated names"
            onChange={(e) =>
              setDraft((d) => ({ ...d, secrets: e.target.value }))
            }
            disabled={mutation.isPending}
            data-testid="issue-settings-input-secrets"
          />
        </label>
      </div>
      {validationError && (
        <div
          className={styles.error}
          role="alert"
          data-testid="issue-settings-error"
        >
          {validationError}
        </div>
      )}
      <div className={styles.actions}>
        <Button
          variant="primary"
          size="sm"
          onClick={onSave}
          disabled={mutation.isPending}
          data-testid="issue-settings-save"
        >
          {mutation.isPending ? "Saving…" : "Save"}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={cancel}
          disabled={mutation.isPending}
        >
          Cancel
        </Button>
        <span className={styles.hint}>
          Leave a field blank to inherit the agent default.
        </span>
      </div>
    </div>
  );
}
