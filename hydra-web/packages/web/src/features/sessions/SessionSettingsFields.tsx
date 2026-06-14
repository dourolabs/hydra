import { useState } from "react";
import { Icons, Input, Picker, PickerRow } from "@hydra/ui";
import type { SessionSettings, Timeout } from "@hydra/api";
import styles from "./SessionSettingsFields.module.css";

interface SessionSettingsFieldsProps {
  testIdPrefix: string;
  value: SessionSettings;
  onChange: (next: SessionSettings) => void;
  /**
   * Per-entity copy under the collapsible header. Wording differs across
   * agent / project / status, so the caller supplies the line.
   */
  helpText: string;
}

// Shared "Default session settings" collapsible used by the agent create/edit
// modals and the project form. Renders the six surfaced fields (CPU, memory,
// image, model, max retries, idle timeout); the CLI-only fields
// (`repo_name` / `remote_url` / `branch` / `secrets`) ride through `value`
// untouched so a CLI-managed entity round-trips without dropping overrides.
export function SessionSettingsFields({
  testIdPrefix,
  value,
  onChange,
  helpText,
}: SessionSettingsFieldsProps) {
  const [open, setOpen] = useState(false);
  const [idleTimeoutPickerOpen, setIdleTimeoutPickerOpen] = useState(false);

  const patch = (updates: Partial<SessionSettings>) => {
    onChange({ ...value, ...updates });
  };

  const setSessionString = (
    field: "image" | "model" | "cpu_limit" | "memory_limit",
    raw: string,
  ) => {
    patch({ [field]: raw === "" ? null : raw } as Partial<SessionSettings>);
  };

  const setMaxRetries = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      patch({ max_retries: null });
      return;
    }
    const n = Number(trimmed);
    if (!Number.isFinite(n) || n < 0 || !Number.isInteger(n)) return;
    patch({ max_retries: n });
  };

  // Idle timeout has three discrete modes: Default (None), Infinite, or an
  // explicit seconds count. The picker chooses the mode; the seconds input is
  // only meaningful in the "seconds" mode and converts on the fly.
  const idleTimeoutMode: "default" | "infinite" | "seconds" = (() => {
    const t = value.idle_timeout;
    if (t == null) return "default";
    if (t.kind === "infinite") return "infinite";
    return "seconds";
  })();
  const idleTimeoutSeconds =
    value.idle_timeout?.kind === "seconds"
      ? String(value.idle_timeout.value)
      : "";

  const setIdleTimeoutMode = (mode: "default" | "infinite" | "seconds") => {
    if (mode === "default") {
      patch({ idle_timeout: null });
      return;
    }
    if (mode === "infinite") {
      patch({ idle_timeout: { kind: "infinite" } });
      return;
    }
    const current =
      value.idle_timeout?.kind === "seconds"
        ? value.idle_timeout.value
        : (60n as unknown as bigint);
    patch({
      idle_timeout: { kind: "seconds", value: current } as Timeout,
    });
  };

  const setIdleTimeoutSeconds = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      patch({ idle_timeout: null });
      return;
    }
    const n = Number(trimmed);
    if (!Number.isFinite(n) || n < 1 || !Number.isInteger(n)) return;
    patch({
      idle_timeout: { kind: "seconds", value: n as unknown as bigint } as Timeout,
    });
  };

  return (
    <div className={styles.sessionSettings}>
      <button
        type="button"
        className={styles.collapsibleSummary}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        data-testid={`${testIdPrefix}-session-settings-toggle`}
      >
        <span className={styles.collapsibleChevron} aria-hidden="true">
          {open ? (
            <Icons.IconChevronDown size={10} />
          ) : (
            <Icons.IconChevronRight size={10} />
          )}
        </span>
        <span className={styles.sectionTitle}>Default session settings</span>
      </button>
      {open && (
        <div
          className={styles.collapsibleContent}
          data-testid={`${testIdPrefix}-session-settings-content`}
        >
          <span className={styles.helpText}>{helpText}</span>
          <div className={styles.sessionInputs}>
            <Input
              label="CPU limit"
              value={value.cpu_limit ?? ""}
              onChange={(e) => setSessionString("cpu_limit", e.target.value)}
              placeholder="e.g. 500m, 2"
              data-testid={`${testIdPrefix}-cpu-limit`}
            />
            <Input
              label="Memory limit"
              value={value.memory_limit ?? ""}
              onChange={(e) => setSessionString("memory_limit", e.target.value)}
              placeholder="e.g. 1Gi, 512Mi"
              data-testid={`${testIdPrefix}-memory-limit`}
            />
          </div>
          <div className={styles.sessionInputs}>
            <Input
              label="Container image"
              value={value.image ?? ""}
              onChange={(e) => setSessionString("image", e.target.value)}
              placeholder="ghcr.io/org/image:tag"
              data-testid={`${testIdPrefix}-image`}
            />
            <Input
              label="Model"
              value={value.model ?? ""}
              onChange={(e) => setSessionString("model", e.target.value)}
              placeholder="e.g. claude-opus-4-7"
              data-testid={`${testIdPrefix}-model`}
            />
          </div>
          <div className={styles.sessionInputs}>
            <Input
              label="Max retries"
              type="number"
              min={0}
              step={1}
              value={value.max_retries == null ? "" : String(value.max_retries)}
              onChange={(e) => setMaxRetries(e.target.value)}
              placeholder="Inherit"
              data-testid={`${testIdPrefix}-max-retries`}
            />
            <span className={styles.spacer} />
          </div>
          <div
            className={styles.idleTimeout}
            data-testid={`${testIdPrefix}-idle-timeout`}
          >
            <label className={styles.label}>Idle timeout</label>
            <div className={styles.idleTimeoutInputs}>
              <Input
                type="number"
                min={1}
                step={1}
                value={idleTimeoutSeconds}
                onChange={(e) => setIdleTimeoutSeconds(e.target.value)}
                disabled={idleTimeoutMode !== "seconds"}
                placeholder={
                  idleTimeoutMode === "infinite" ? "Never" : "Seconds"
                }
                aria-label="Idle timeout seconds"
                data-testid={`${testIdPrefix}-idle-timeout-seconds`}
              />
              <Picker
                label="Idle timeout"
                hideLabel
                open={idleTimeoutPickerOpen}
                onToggle={() => setIdleTimeoutPickerOpen((v) => !v)}
                value={
                  idleTimeoutMode === "default" ? (
                    <span className={styles.pillEmpty}>Server default</span>
                  ) : idleTimeoutMode === "infinite" ? (
                    <span>Never</span>
                  ) : (
                    <span>Custom</span>
                  )
                }
                data-testid={`${testIdPrefix}-idle-timeout-mode`}
              >
                <PickerRow
                  active={idleTimeoutMode === "default"}
                  onClick={() => {
                    setIdleTimeoutMode("default");
                    setIdleTimeoutPickerOpen(false);
                  }}
                >
                  <span>Server default</span>
                  <span className={styles.popSpacer} />
                </PickerRow>
                <PickerRow
                  active={idleTimeoutMode === "seconds"}
                  onClick={() => {
                    setIdleTimeoutMode("seconds");
                    setIdleTimeoutPickerOpen(false);
                  }}
                >
                  <span>Custom (seconds)</span>
                  <span className={styles.popSpacer} />
                </PickerRow>
                <PickerRow
                  active={idleTimeoutMode === "infinite"}
                  onClick={() => {
                    setIdleTimeoutMode("infinite");
                    setIdleTimeoutPickerOpen(false);
                  }}
                >
                  <span>Never</span>
                  <span className={styles.popSpacer} />
                </PickerRow>
              </Picker>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// Collapse the session_settings payload back to `undefined` only when every
// subfield is empty — surfaced AND un-surfaced. CLI users can set
// `repo_name` / `remote_url` / `branch` / `secrets` (not on the form), so
// preserving them is required to round-trip the modal without dropping
// CLI-only overrides. Mirrors the StatusSettingsModal patchSession check.
export function collapseSessionSettings(
  value: SessionSettings,
): SessionSettings | undefined {
  const allEmpty =
    (value.image ?? null) == null &&
    (value.model ?? null) == null &&
    (value.cpu_limit ?? null) == null &&
    (value.memory_limit ?? null) == null &&
    (value.max_retries ?? null) == null &&
    (value.idle_timeout ?? null) == null &&
    (value.repo_name ?? null) == null &&
    (value.remote_url ?? null) == null &&
    (value.branch ?? null) == null &&
    !(value.secrets && value.secrets.length > 0);
  return allEmpty ? undefined : value;
}
