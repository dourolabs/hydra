import { useState, useCallback, useMemo } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input, MarkdownViewer, Select, Textarea } from "@hydra/ui";
import type {
  Form,
  FormResponse,
  Field,
  Action,
  ActionStyle,
  JsonValue,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { formatTimestamp } from "../../utils/time";
import styles from "./FormPanel.module.css";

interface FormPanelProps {
  issueId: string;
  form: Form;
  formResponse?: FormResponse | null;
}

type FormValues = Record<string, JsonValue>;
type FormErrors = Record<string, string>;

function getDefaultValue(field: Field): JsonValue {
  if (field.default !== undefined && field.default !== null) {
    return field.default;
  }
  switch (field.input.type) {
    case "checkbox":
      return false;
    case "number":
      return null;
    default:
      return "";
  }
}

function initValues(fields: Field[]): FormValues {
  const values: FormValues = {};
  for (const field of fields) {
    values[field.key] = getDefaultValue(field);
  }
  return values;
}

function validateField(field: Field, value: JsonValue): string | null {
  const input = field.input;
  switch (input.type) {
    case "text": {
      const str = typeof value === "string" ? value : "";
      if (input.min_length && str.length < input.min_length) {
        return `Must be at least ${input.min_length} characters`;
      }
      if (input.max_length && str.length > input.max_length) {
        return `Must be at most ${input.max_length} characters`;
      }
      if (input.pattern) {
        const re = new RegExp(input.pattern);
        if (!re.test(str)) {
          return `Must match pattern: ${input.pattern}`;
        }
      }
      return null;
    }
    case "textarea": {
      const str = typeof value === "string" ? value : "";
      if (input.min_length && str.length < input.min_length) {
        return `Must be at least ${input.min_length} characters`;
      }
      if (input.max_length && str.length > input.max_length) {
        return `Must be at most ${input.max_length} characters`;
      }
      return null;
    }
    case "select": {
      const str = typeof value === "string" ? value : "";
      if (str && !input.options.some((o) => o.value === str)) {
        return "Invalid selection";
      }
      return null;
    }
    case "checkbox":
      return null;
    case "number": {
      if (value === null || value === "") return null;
      const num = typeof value === "number" ? value : Number(value);
      if (isNaN(num)) return "Must be a number";
      if (input.min !== undefined && input.min !== null && num < input.min) {
        return `Must be at least ${input.min}`;
      }
      if (input.max !== undefined && input.max !== null && num > input.max) {
        return `Must be at most ${input.max}`;
      }
      return null;
    }
    default:
      return null;
  }
}

function isFieldEmpty(field: Field, value: JsonValue): boolean {
  switch (field.input.type) {
    case "text":
    case "textarea":
    case "select":
      return value === "" || value === null || value === undefined;
    case "checkbox":
      return false; // checkboxes always have a value
    case "number":
      return value === null || value === undefined || value === "";
    default:
      return value === null || value === undefined || value === "";
  }
}

function isActionEnabled(
  action: Action,
  fields: Field[],
  values: FormValues,
): boolean {
  for (const key of action.requires) {
    const field = fields.find((f) => f.key === key);
    if (!field) return false;
    const value = values[key];
    if (isFieldEmpty(field, value ?? null)) return false;
    if (validateField(field, value ?? null)) return false;
  }
  return true;
}

function actionVariant(style: ActionStyle): "primary" | "danger" | "secondary" {
  switch (style) {
    case "primary":
      return "primary";
    case "danger":
      return "danger";
    default:
      return "secondary";
  }
}

function FieldInput({
  field,
  value,
  error,
  onChange,
}: {
  field: Field;
  value: JsonValue;
  error?: string;
  onChange: (key: string, value: JsonValue) => void;
}) {
  const input = field.input;
  const fieldId = `form-field-${field.key}`;

  switch (input.type) {
    case "text":
      return (
        <Input
          id={fieldId}
          value={typeof value === "string" ? value : ""}
          placeholder={input.placeholder ?? undefined}
          maxLength={input.max_length ?? undefined}
          error={error}
          onChange={(e) => onChange(field.key, e.target.value)}
        />
      );
    case "textarea":
      return (
        <Textarea
          id={fieldId}
          value={typeof value === "string" ? value : ""}
          placeholder={input.placeholder ?? undefined}
          maxLength={input.max_length ?? undefined}
          rows={input.rows}
          error={error}
          onChange={(e) => onChange(field.key, e.target.value)}
        />
      );
    case "select":
      if (input.radio) {
        return (
          <div className={styles.radioGroup}>
            {input.options.map((opt) => (
              <label key={opt.value} className={styles.radioOption}>
                <input
                  type="radio"
                  className={styles.radio}
                  name={fieldId}
                  value={opt.value}
                  checked={value === opt.value}
                  onChange={() => onChange(field.key, opt.value)}
                />
                <span className={styles.radioLabel}>{opt.label}</span>
              </label>
            ))}
            {error && <p className={styles.fieldError}>{error}</p>}
          </div>
        );
      }
      return (
        <>
          <Select
            id={fieldId}
            value={typeof value === "string" ? value : ""}
            options={input.options}
            placeholder="Select..."
            onChange={(e) => onChange(field.key, e.target.value)}
          />
          {error && <p className={styles.fieldError}>{error}</p>}
        </>
      );
    case "checkbox":
      return (
        <div className={styles.checkboxRow}>
          <input
            id={fieldId}
            type="checkbox"
            className={styles.checkbox}
            checked={value === true}
            onChange={(e) => onChange(field.key, e.target.checked)}
          />
          <label htmlFor={fieldId} className={styles.checkboxLabel}>
            {field.label}
          </label>
          {error && <p className={styles.fieldError}>{error}</p>}
        </div>
      );
    case "number":
      return (
        <Input
          id={fieldId}
          type="number"
          value={value !== null && value !== undefined ? String(value) : ""}
          min={input.min ?? undefined}
          max={input.max ?? undefined}
          step={input.step ?? undefined}
          error={error}
          onChange={(e) => {
            const raw = e.target.value;
            onChange(field.key, raw === "" ? null : Number(raw));
          }}
        />
      );
    default:
      return null;
  }
}

function ReadOnlyField({ field, value }: { field: Field; value: JsonValue }) {
  const displayValue = useMemo(() => {
    if (value === null || value === undefined) return "—";
    if (field.input.type === "checkbox") return value ? "Yes" : "No";
    if (field.input.type === "select" && typeof value === "string") {
      const input = field.input;
      const opt = input.options.find((o) => o.value === value);
      return opt ? opt.label : String(value);
    }
    return String(value);
  }, [field, value]);

  return <div className={styles.readOnlyValue}>{displayValue}</div>;
}

export function FormPanel({ issueId, form, formResponse }: FormPanelProps) {
  const queryClient = useQueryClient();
  const isReadOnly = !!formResponse;

  const [values, setValues] = useState<FormValues>(() =>
    isReadOnly ? (formResponse.values as FormValues) : initValues(form.fields),
  );
  const [errors, setErrors] = useState<FormErrors>({});
  const [submitError, setSubmitError] = useState<string | null>(null);

  const handleChange = useCallback((key: string, value: JsonValue) => {
    setValues((prev) => ({ ...prev, [key]: value }));
    setErrors((prev) => {
      if (!prev[key]) return prev;
      const next = { ...prev };
      delete next[key];
      return next;
    });
  }, []);

  const mutation = useMutation({
    mutationFn: (action: Action) => {
      const submitValues: Record<string, unknown> = {};
      for (const field of form.fields) {
        const v = values[field.key];
        if (v !== null && v !== undefined && v !== "") {
          submitValues[field.key] = v;
        }
      }
      return apiClient.submitForm(issueId, action.id, submitValues);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
    },
    onError: (err: Error) => {
      setSubmitError(err.message);
    },
  });

  const handleAction = useCallback(
    (action: Action) => {
      setSubmitError(null);

      // Validate required fields
      const newErrors: FormErrors = {};
      for (const key of action.requires) {
        const field = form.fields.find((f) => f.key === key);
        if (!field) continue;
        const value = values[key] ?? null;
        if (isFieldEmpty(field, value)) {
          newErrors[key] = "Required";
          continue;
        }
        const validationError = validateField(field, value);
        if (validationError) {
          newErrors[key] = validationError;
        }
      }

      // Also validate non-required fields that have values
      for (const field of form.fields) {
        if (newErrors[field.key]) continue;
        const value = values[field.key] ?? null;
        if (!isFieldEmpty(field, value)) {
          const validationError = validateField(field, value);
          if (validationError) {
            newErrors[field.key] = validationError;
          }
        }
      }

      if (Object.keys(newErrors).length > 0) {
        setErrors(newErrors);
        return;
      }

      mutation.mutate(action);
    },
    [form.fields, values, mutation],
  );

  // Find the action that was taken (for read-only view)
  const submittedAction = isReadOnly
    ? form.actions.find((a) => a.id === formResponse.action_id)
    : null;

  return (
    <div className={styles.formPanel}>
      {/* Prompt */}
      <div className={styles.prompt}>
        <MarkdownViewer content={form.prompt} />
      </div>

      {/* Fields */}
      {form.fields.length > 0 && (
        <div className={styles.fields}>
          {form.fields.map((field) => {
            // Checkbox renders its label inline
            if (field.input.type === "checkbox") {
              return (
                <div key={field.key} className={styles.fieldGroup}>
                  {isReadOnly ? (
                    <>
                      <span className={styles.fieldLabel}>{field.label}</span>
                      <ReadOnlyField
                        field={field}
                        value={values[field.key] ?? null}
                      />
                    </>
                  ) : (
                    <FieldInput
                      field={field}
                      value={values[field.key] ?? null}
                      error={errors[field.key]}
                      onChange={handleChange}
                    />
                  )}
                  {field.description && (
                    <p className={styles.fieldDescription}>
                      {field.description}
                    </p>
                  )}
                </div>
              );
            }

            return (
              <div key={field.key} className={styles.fieldGroup}>
                <label
                  htmlFor={`form-field-${field.key}`}
                  className={styles.fieldLabel}
                >
                  {field.label}
                </label>
                {isReadOnly ? (
                  <ReadOnlyField
                    field={field}
                    value={values[field.key] ?? null}
                  />
                ) : (
                  <FieldInput
                    field={field}
                    value={values[field.key] ?? null}
                    error={errors[field.key]}
                    onChange={handleChange}
                  />
                )}
                {field.description && (
                  <p className={styles.fieldDescription}>
                    {field.description}
                  </p>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* Actions (only if not read-only) */}
      {!isReadOnly && (
        <div className={styles.actions}>
          {submitError && <p className={styles.submitError}>{submitError}</p>}
          {form.actions.map((action) => (
            <Button
              key={action.id}
              variant={actionVariant(action.style)}
              disabled={
                !isActionEnabled(action, form.fields, values) ||
                mutation.isPending
              }
              onClick={() => handleAction(action)}
            >
              {action.label}
            </Button>
          ))}
        </div>
      )}

      {/* Response metadata (read-only view) */}
      {isReadOnly && formResponse && (
        <div className={styles.responseMeta}>
          {submittedAction && <span>Action: {submittedAction.label}</span>}
          <span>Submitted by: {formResponse.actor}</span>
          <span>{formatTimestamp(formResponse.submitted_at)}</span>
        </div>
      )}
    </div>
  );
}
