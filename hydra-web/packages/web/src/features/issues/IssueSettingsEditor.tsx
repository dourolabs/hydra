import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Issue, IssueVersionRecord, SessionSettings } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./IssueSettingsEditor.module.css";

interface IssueSettingsEditorProps {
  issueId: string;
  issue: Issue;
}

type FieldKind = "string" | "integer" | "secrets";

interface FieldSpec {
  key: keyof SessionSettings;
  label: string;
  kind: FieldKind;
  placeholder?: string;
}

const FIELDS: FieldSpec[] = [
  { key: "repo_name", label: "Repository", kind: "string", placeholder: "owner/name" },
  { key: "remote_url", label: "Remote URL", kind: "string", placeholder: "https://github.com/owner/name.git" },
  { key: "image", label: "Image", kind: "string" },
  { key: "model", label: "Model", kind: "string" },
  { key: "branch", label: "Branch", kind: "string", placeholder: "main" },
  { key: "max_retries", label: "Max retries", kind: "integer" },
  { key: "cpu_limit", label: "CPU limit", kind: "string" },
  { key: "memory_limit", label: "Memory limit", kind: "string" },
  { key: "secrets", label: "Secrets", kind: "secrets", placeholder: "comma-separated names" },
];

function displayValue(settings: SessionSettings | undefined | null, field: FieldSpec): string | null {
  if (!settings) return null;
  const raw = settings[field.key];
  if (raw == null) return null;
  if (field.kind === "secrets") {
    const list = (raw as string[]).filter(Boolean);
    return list.length > 0 ? list.join(", ") : null;
  }
  if (field.kind === "integer") {
    return String(raw);
  }
  return String(raw);
}

function draftFromSettings(
  settings: SessionSettings | undefined | null,
  field: FieldSpec,
): string {
  return displayValue(settings, field) ?? "";
}

interface ParsedCommit {
  value: SessionSettings[keyof SessionSettings] | null;
  error: string | null;
}

function parseDraft(draft: string, field: FieldSpec): ParsedCommit {
  const trimmed = draft.trim();
  if (trimmed === "") return { value: null, error: null };
  if (field.kind === "integer") {
    const n = Number(trimmed);
    if (!Number.isInteger(n) || n < 0) {
      return { value: null, error: `${field.label} must be a non-negative integer.` };
    }
    return { value: n, error: null };
  }
  if (field.kind === "secrets") {
    const parts = trimmed
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    return { value: parts.length > 0 ? parts : null, error: null };
  }
  return { value: trimmed, error: null };
}

function settingsEqualForField(
  a: SessionSettings | undefined | null,
  b: SessionSettings | undefined | null,
  field: FieldSpec,
): boolean {
  const av = a?.[field.key] ?? null;
  const bv = b?.[field.key] ?? null;
  if (field.kind === "secrets") {
    const al = (av as string[] | null) ?? [];
    const bl = (bv as string[] | null) ?? [];
    if (al.length !== bl.length) return false;
    return al.every((v, i) => v === bl[i]);
  }
  return av === bv;
}

interface RowProps {
  field: FieldSpec;
  settings: SessionSettings | undefined | null;
  editing: boolean;
  pending: boolean;
  errorMsg: string | null;
  onStartEdit: () => void;
  onCommit: (draft: string) => void;
  onCancel: () => void;
}

function Row({
  field,
  settings,
  editing,
  pending,
  errorMsg,
  onStartEdit,
  onCommit,
  onCancel,
}: RowProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState<string>(() => draftFromSettings(settings, field));

  useLayoutEffect(() => {
    if (editing) {
      setDraft(draftFromSettings(settings, field));
    }
  }, [editing, settings, field]);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  // If validation kept the field in edit mode after a blur attempt, re-focus
  // so the user can correct without an extra click.
  useEffect(() => {
    if (editing && errorMsg && inputRef.current) {
      inputRef.current.focus();
    }
  }, [editing, errorMsg]);

  const display = displayValue(settings, field);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        e.preventDefault();
        inputRef.current?.blur();
      } else if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    },
    [onCancel],
  );

  return (
    <div
      className={styles.row}
      data-testid={`issue-settings-row-${String(field.key)}`}
    >
      <dt className={styles.rowLabel}>{field.label}</dt>
      <dd className={styles.rowValue}>
        {editing ? (
          <>
            <input
              ref={inputRef}
              type={field.kind === "integer" ? "number" : "text"}
              inputMode={field.kind === "integer" ? "numeric" : undefined}
              min={field.kind === "integer" ? 0 : undefined}
              step={field.kind === "integer" ? 1 : undefined}
              className={styles.input}
              value={draft}
              placeholder={field.placeholder}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => onCommit(draft)}
              onKeyDown={handleKeyDown}
              disabled={pending}
              data-testid={`issue-settings-input-${String(field.key)}`}
              aria-label={field.label}
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              data-1p-ignore="true"
              data-lpignore="true"
              data-bwignore="true"
              data-form-type="other"
            />
            {errorMsg && (
              <div
                className={styles.error}
                role="alert"
                data-testid="issue-settings-error"
              >
                {errorMsg}
              </div>
            )}
          </>
        ) : (
          <button
            type="button"
            className={styles.displayButton}
            onClick={onStartEdit}
            data-testid={`issue-settings-value-${String(field.key)}`}
            title="Click to edit"
          >
            {display != null ? (
              <span className={styles.displayValue}>{display}</span>
            ) : (
              <span className={styles.displayEmpty}>Default</span>
            )}
          </button>
        )}
      </dd>
    </div>
  );
}

export function IssueSettingsEditor({ issueId, issue }: IssueSettingsEditorProps) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const settings = issue.session_settings;

  const [editingKey, setEditingKey] = useState<string | null>(null);
  const [fieldError, setFieldError] = useState<{ key: string; message: string } | null>(null);

  const mutation = useMutation<unknown, Error, SessionSettings>({
    mutationFn: (nextSettings) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: issue.status.key,
          session_settings: nextSettings,
        },
        session_id: null,
      }),
    onError: (err) => {
      addToast(err.message || "Failed to update settings", "error");
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
    },
  });

  const commitField = useCallback(
    (field: FieldSpec, draft: string) => {
      const parsed = parseDraft(draft, field);
      if (parsed.error) {
        setFieldError({ key: String(field.key), message: parsed.error });
        return;
      }
      setFieldError(null);

      const previous = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      const baseSettings = previous?.issue.session_settings ?? settings ?? {};
      const nextSettings: SessionSettings = {
        ...baseSettings,
        [field.key]: parsed.value,
      };

      setEditingKey(null);

      if (settingsEqualForField(baseSettings, nextSettings, field)) {
        return;
      }

      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: { ...previous.issue, session_settings: nextSettings },
        });
      }
      mutation.mutate(nextSettings);
    },
    [issueId, queryClient, settings, mutation],
  );

  const startEditing = useCallback((key: string) => {
    setFieldError(null);
    setEditingKey(key);
  }, []);

  const cancel = useCallback(() => {
    setFieldError(null);
    setEditingKey(null);
  }, []);

  return (
    <dl className={styles.list} data-testid="issue-settings-editor">
      {FIELDS.map((field) => {
        const isEditing = editingKey === String(field.key);
        const errMsg =
          isEditing && fieldError?.key === String(field.key)
            ? fieldError.message
            : null;
        return (
          <Row
            key={String(field.key)}
            field={field}
            settings={settings}
            editing={isEditing}
            pending={mutation.isPending}
            errorMsg={errMsg}
            onStartEdit={() => startEditing(String(field.key))}
            onCommit={(draft) => commitField(field, draft)}
            onCancel={cancel}
          />
        );
      })}
    </dl>
  );
}
