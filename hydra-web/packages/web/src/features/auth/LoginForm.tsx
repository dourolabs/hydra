import { useState, useCallback, type FormEvent, type ReactNode } from "react";
import { Button, Input } from "@hydra/ui";
import { useAuth } from "./useAuth";
import styles from "./LoginForm.module.css";

function fallbackCopyText(text: string): boolean {
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  textarea.style.top = "-9999px";
  document.body.appendChild(textarea);
  textarea.focus();
  textarea.select();
  let success = false;
  try {
    success = document.execCommand("copy");
  } catch {
    success = false;
  }
  document.body.removeChild(textarea);
  return success;
}

type LoginMode = "default" | "token";

function TokenForm({
  onSubmit,
  token,
  setToken,
  error,
  submitting,
  footer,
}: {
  onSubmit: (e: FormEvent) => void;
  token: string;
  setToken: (v: string) => void;
  error: string | null;
  submitting: boolean;
  footer?: ReactNode;
}) {
  return (
    <form className={styles.form} onSubmit={onSubmit}>
      <Input
        data-testid="token-input"
        label="Hydra Token"
        type="password"
        value={token}
        onChange={(e) => setToken(e.target.value)}
        placeholder="Enter your hydra token"
        error={error ?? undefined}
        autoFocus
      />
      <Button data-testid="login-button" type="submit" variant="primary" disabled={submitting || !token.trim()}>
        {submitting ? "Logging in\u2026" : "Log in"}
      </Button>
      {footer}
    </form>
  );
}

export function LoginForm() {
  const { login, loginWithDevice, cancelDeviceFlow, error: authError, githubAuthAvailable, deviceFlowInfo } = useAuth();
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [mode, setMode] = useState<LoginMode>("default");
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");

  // Derive effective error from local + auth context
  const displayError = error ?? authError;

  const handleDeviceLogin = useCallback(async () => {
    setSubmitting(true);
    setError(null);
    try {
      await loginWithDevice();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed");
    } finally {
      setSubmitting(false);
    }
  }, [loginWithDevice]);

  async function handleTokenSubmit(e: FormEvent) {
    e.preventDefault();
    if (!token.trim()) return;

    setSubmitting(true);
    setError(null);
    try {
      await login(token.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed");
    } finally {
      setSubmitting(false);
    }
  }

  async function handleCopyCode() {
    if (!deviceFlowInfo) return;

    let success = false;
    try {
      if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
        await navigator.clipboard.writeText(deviceFlowInfo.user_code);
        success = true;
      } else {
        success = fallbackCopyText(deviceFlowInfo.user_code);
      }
    } catch {
      success = fallbackCopyText(deviceFlowInfo.user_code);
    }

    setCopyState(success ? "copied" : "failed");
    setTimeout(() => setCopyState("idle"), 2000);
  }

  // Loading state while checking auth mode
  if (githubAuthAvailable === null) {
    return null;
  }

  // Token-only mode (local auth, no GitHub)
  if (!githubAuthAvailable) {
    return (
      <TokenForm
        onSubmit={handleTokenSubmit}
        token={token}
        setToken={setToken}
        error={displayError}
        submitting={submitting}
      />
    );
  }

  // Device flow pending — show user code
  if (deviceFlowInfo) {
    return (
      <div className={styles.form}>
        <p className={styles.instructions}>
          Enter this code at GitHub to sign in:
        </p>
        <div className={styles.codeContainer}>
          <code className={styles.userCode}>{deviceFlowInfo.user_code}</code>
          <Button
            data-testid="copy-code-button"
            variant="secondary"
            onClick={handleCopyCode}
          >
            {copyState === "copied" ? "Copied!" : copyState === "failed" ? "Failed to copy" : "Copy"}
          </Button>
        </div>
        <a
          href={deviceFlowInfo.verification_uri}
          target="_blank"
          rel="noopener noreferrer"
          className={styles.verificationLink}
        >
          Open GitHub verification page
        </a>
        <p className={styles.waitingText}>Waiting for authorization…</p>
        {displayError && <p className={styles.error}>{displayError}</p>}
        <Button
          variant="secondary"
          onClick={() => {
            cancelDeviceFlow();
            setError(null);
          }}
        >
          Cancel
        </Button>
      </div>
    );
  }

  // Token input mode
  if (mode === "token") {
    return (
      <TokenForm
        onSubmit={handleTokenSubmit}
        token={token}
        setToken={setToken}
        error={displayError}
        submitting={submitting}
        footer={
          <button
            type="button"
            className={styles.switchLink}
            onClick={() => {
              setMode("default");
              setError(null);
            }}
          >
            Sign in with GitHub instead
          </button>
        }
      />
    );
  }

  // Default mode — show GitHub sign-in button
  return (
    <div className={styles.form}>
      <Button
        data-testid="github-login-button"
        variant="primary"
        onClick={handleDeviceLogin}
        disabled={submitting}
      >
        {submitting ? "Starting…" : "Sign in with GitHub"}
      </Button>
      {displayError && <p className={styles.error}>{displayError}</p>}
      <button
        type="button"
        className={styles.switchLink}
        onClick={() => {
          setMode("token");
          setError(null);
        }}
      >
        Sign in with token
      </button>
    </div>
  );
}
