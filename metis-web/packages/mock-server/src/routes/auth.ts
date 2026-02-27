import { Hono } from "hono";
import { DEV_USERNAME, DEV_GITHUB_USER_ID, DEV_TOKEN } from "../auth.js";
import type {
  WhoAmIResponse,
  UserSummary,
  GithubTokenResponse,
  LoginResponse,
} from "@metis/api";

export function createAuthRoutes(): Hono {
  const app = new Hono();

  // POST /v1/login — accepts any body, returns dev token
  app.post("/v1/login", async (c) => {
    const resp: LoginResponse = {
      login_token: DEV_TOKEN,
      user: { username: DEV_USERNAME, github_user_id: DEV_GITHUB_USER_ID },
    };
    return c.json(resp);
  });

  // GET /v1/whoami
  app.get("/v1/whoami", (c) => {
    const resp: WhoAmIResponse = {
      actor: { type: "user", username: DEV_USERNAME },
    };
    return c.json(resp);
  });

  // GET /v1/users/:username
  app.get("/v1/users/:username", (c) => {
    const username = c.req.param("username");
    const resp: UserSummary = {
      username,
      github_user_id: DEV_GITHUB_USER_ID,
    };
    return c.json(resp);
  });

  // GET /v1/github/token
  app.get("/v1/github/token", (c) => {
    const resp: GithubTokenResponse = {
      github_token: "ghp_mock_token_for_dev",
    };
    return c.json(resp);
  });

  return app;
}
