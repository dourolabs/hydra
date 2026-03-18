export const config = {
  hydraServerUrl:
    process.env.HYDRA_SERVER_URL ?? "http://server.hydra.svc.cluster.local",
  port: Number(process.env.PORT ?? 4000),
  cookieName: "hydra_token",
  cookieSecure: process.env.COOKIE_SECURE !== "false",
  logLevel: (process.env.LOG_LEVEL ?? "info") as
    | "debug"
    | "info"
    | "warn"
    | "error",
};
