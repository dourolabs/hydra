import { useState, type FormEvent } from "react";
import { Button, Input } from "@metis/ui";
import { useAuth } from "./useAuth";
import styles from "./LoginForm.module.css";

export function LoginForm() {
  const { login } = useAuth();
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: FormEvent) {
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

  return (
    <form className={styles.form} onSubmit={handleSubmit}>
      <Input
        label="Metis Token"
        type="password"
        value={token}
        onChange={(e) => setToken(e.target.value)}
        placeholder="Enter your metis token"
        error={error ?? undefined}
        autoFocus
      />
      <Button type="submit" variant="primary" disabled={submitting || !token.trim()}>
        {submitting ? "Logging in\u2026" : "Log in"}
      </Button>
    </form>
  );
}
