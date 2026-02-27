import { createMiddleware } from "hono/factory";

export const DEV_USERNAME = "dev-user";
export const DEV_GITHUB_USER_ID = BigInt(12345);

export const DEV_TOKEN = "dev-token-12345";

export const authMiddleware = createMiddleware(async (c, next) => {
  const auth = c.req.header("Authorization");
  if (!auth || !auth.startsWith("Bearer ")) {
    return c.json({ error: "missing or invalid Authorization header" }, 401);
  }
  c.set("username", DEV_USERNAME);
  await next();
});
