import { useState, useEffect, useCallback, type ReactNode } from "react";
import type { WhoAmIResponse } from "@hydra/api";
import {
  fetchMe,
  login as apiLogin,
  logout as apiLogout,
  deviceStart,
  devicePoll,
  isGithubAuthAvailable,
} from "../../api/auth";
import { AuthContext } from "./auth-state";

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<WhoAmIResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [githubAuthAvailable, setGithubAuthAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    fetchMe()
      .then(setUser)
      .catch(() => setUser(null))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    isGithubAuthAvailable().then(setGithubAuthAvailable);
  }, []);

  const login = useCallback(async (token: string) => {
    setError(null);
    try {
      const u = await apiLogin(token);
      setUser(u);
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Login failed";
      setError(msg);
      throw err;
    }
  }, []);

  const loginWithDevice = useCallback(async () => {
    setError(null);
    try {
      const startResp = await deviceStart();
      // Dispatch a custom event so the LoginForm can show the user code
      window.dispatchEvent(
        new CustomEvent("hydra:device-flow-started", { detail: startResp }),
      );

      // Poll until complete or error
      const pollUntilDone = async (): Promise<void> => {
        const interval = startResp.interval * 1000;
        const expiresAt = Date.now() + startResp.expires_in * 1000;

        while (Date.now() < expiresAt) {
          await new Promise((r) => setTimeout(r, interval));
          const pollResp = await devicePoll(startResp.device_session_id);

          if (pollResp.status === "complete") {
            // BFF sets the cookie on successful poll; fetch user info
            const u = await fetchMe();
            setUser(u);
            return;
          }

          if (pollResp.status === "error") {
            throw new Error(pollResp.error ?? "Device flow failed");
          }
          // status === "pending" — keep polling
        }

        throw new Error("Device code expired. Please try again.");
      };

      await pollUntilDone();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Login failed";
      setError(msg);
      throw err;
    }
  }, []);

  const logout = useCallback(async () => {
    await apiLogout();
    setUser(null);
  }, []);

  return (
    <AuthContext.Provider value={{ user, loading, error, login, loginWithDevice, logout, githubAuthAvailable }}>
      {children}
    </AuthContext.Provider>
  );
}
