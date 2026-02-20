import { createContext } from "react";
import type { WhoAmIResponse } from "@metis/api";

export interface AuthState {
  user: WhoAmIResponse | null;
  loading: boolean;
  error: string | null;
  login: (token: string) => Promise<void>;
  logout: () => Promise<void>;
}

export const AuthContext = createContext<AuthState | null>(null);
