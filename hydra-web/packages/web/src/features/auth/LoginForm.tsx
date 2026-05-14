import { useState, useCallback, useRef, useEffect } from "react";
import { Button, fallbackCopyText } from "@hydra/ui";
import { useAuth } from "./useAuth";
import styles from "./LoginForm.module.css";

export function LoginForm() {
  const { loginWithDevice, cancelDeviceFlow, error: authError, githubAuthAvailable, deviceFlowInfo } = useAuth();
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
  }, []);

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
    if (copyTimerRef.current) {
      clearTimeout(copyTimerRef.current);
      copyTimerRef.current = null;
    }
    copyTimerRef.current = setTimeout(() => {
      setCopyState("idle");
      copyTimerRef.current = null;
    }, 2000);
  }

  // Loading state while checking auth mode
  if (githubAuthAvailable === null) {
    return null;
  }

  // GitHub auth not configured — single-player mode auto-authenticates before
  // reaching this page, so this only renders in misconfigured multi-player setups.
  if (!githubAuthAvailable) {
    return (
      <div className={styles.form}>
        <p className={styles.instructions}>GitHub authentication is not configured.</p>
      </div>
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
    </div>
  );
}
