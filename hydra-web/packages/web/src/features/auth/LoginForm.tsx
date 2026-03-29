import { useState, useEffect, useCallback, type FormEvent } from "react";
import { Button, Input } from "@hydra/ui";
import type { DeviceStartResponse } from "@hydra/api";
import { useAuth } from "./useAuth";
import styles from "./LoginForm.module.css";

type LoginMode = "default" | "device-pending" | "token";

export function LoginForm() {
  const { login, loginWithDevice, error: authError, githubAuthAvailable } = useAuth();
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [mode, setMode] = useState<LoginMode>("default");
  const [deviceInfo, setDeviceInfo] = useState<DeviceStartResponse | null>(null);
  const [copied, setCopied] = useState(false);

  // Listen for device flow start event from AuthContext
  useEffect(() => {
    function handleDeviceStarted(e: Event) {
      const detail = (e as CustomEvent<DeviceStartResponse>).detail;
      setDeviceInfo(detail);
      setMode("device-pending");
    }
    window.addEventListener("hydra:device-flow-started", handleDeviceStarted);
    return () => window.removeEventListener("hydra:device-flow-started", handleDeviceStarted);
  }, []);

  // Sync auth errors
  useEffect(() => {
    if (authError) {
      setError(authError);
      setSubmitting(false);
      setMode("default");
    }
  }, [authError]);

  const handleDeviceLogin = useCallback(async () => {
    setSubmitting(true);
    setError(null);
    try {
      await loginWithDevice();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed");
      setMode("default");
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
    if (!deviceInfo) return;
    try {
      await navigator.clipboard.writeText(deviceInfo.user_code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Fallback: select text for manual copy
    }
  }

  // Loading state while checking auth mode
  if (githubAuthAvailable === null) {
    return null;
  }

  // Token-only mode (local auth, no GitHub)
  if (!githubAuthAvailable) {
    return (
      <form className={styles.form} onSubmit={handleTokenSubmit}>
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
      </form>
    );
  }

  // Device flow pending — show user code
  if (mode === "device-pending" && deviceInfo) {
    return (
      <div className={styles.form}>
        <p className={styles.instructions}>
          Enter this code at GitHub to sign in:
        </p>
        <div className={styles.codeContainer}>
          <code className={styles.userCode}>{deviceInfo.user_code}</code>
          <Button
            data-testid="copy-code-button"
            variant="secondary"
            onClick={handleCopyCode}
          >
            {copied ? "Copied!" : "Copy"}
          </Button>
        </div>
        <a
          href={deviceInfo.verification_uri}
          target="_blank"
          rel="noopener noreferrer"
          className={styles.verificationLink}
        >
          Open GitHub verification page
        </a>
        <p className={styles.waitingText}>Waiting for authorization…</p>
        {error && <p className={styles.error}>{error}</p>}
        <Button
          variant="secondary"
          onClick={() => {
            setMode("default");
            setDeviceInfo(null);
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
      <form className={styles.form} onSubmit={handleTokenSubmit}>
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
      </form>
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
      {error && <p className={styles.error}>{error}</p>}
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
