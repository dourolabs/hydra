import { createContext } from "react";
import type { WhoAmIResponse, DeviceStartResponse } from "@hydra/api";

export interface AuthState {
  user: WhoAmIResponse | null;
  loading: boolean;
  error: string | null;
  login: (token: string) => Promise<void>;
  loginWithDevice: () => Promise<void>;
  cancelDeviceFlow: () => void;
  logout: () => Promise<void>;
  githubAuthAvailable: boolean | null;
  deviceFlowInfo: DeviceStartResponse | null;
}

export const AuthContext = createContext<AuthState | null>(null);
