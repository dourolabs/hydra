import { useState, useEffect, useCallback, useRef, type ReactNode } from "react";
import type { WhoAmIResponse, DeviceStartResponse } from "@hydra/api";
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
  const [deviceFlowInfo, setDeviceFlowInfo] = useState<DeviceStartResponse | null>(null);
  const cancelledRef = useRef(false);

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

  const cancelDeviceFlow = useCallback(() => {
    cancelledRef.current = true;
    setDeviceFlowInfo(null);
  }, []);

  const loginWithDevice = useCallback(async () => {
    setError(null);
    cancelledRef.current = false;
    try {
      const startResp = await deviceStart();
      if (cancelledRef.current) return;
      setDeviceFlowInfo(startResp);

      const interval = startResp.interval * 1000;
      const expiresAt = Date.now() + startResp.expires_in * 1000;

      while (Date.now() < expiresAt) {
        await new Promise((r) => setTimeout(r, interval));
        if (cancelledRef.current) return;

        const pollResp = await devicePoll(startResp.device_session_id);
        if (cancelledRef.current) return;

        if (pollResp.status === "complete") {
          const u = await fetchMe();
          setUser(u);
          setDeviceFlowInfo(null);
          return;
        }

        if (pollResp.status === "error") {
          throw new Error(pollResp.error ?? "Device flow failed");
        }
        // status === "pending" — keep polling
      }

      throw new Error("Device code expired. Please try again.");
    } catch (err) {
      if (cancelledRef.current) return;
      const msg = err instanceof Error ? err.message : "Login failed";
      setError(msg);
      setDeviceFlowInfo(null);
      throw err;
    }
  }, []);

  const logout = useCallback(async () => {
    await apiLogout();
    setUser(null);
  }, []);

  return (
    <AuthContext.Provider value={{ user, loading, error, login, loginWithDevice, cancelDeviceFlow, logout, githubAuthAvailable, deviceFlowInfo }}>
      {children}
    </AuthContext.Provider>
  );
}
