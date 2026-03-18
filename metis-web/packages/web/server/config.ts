export const config = {
  hydraServerUrl:
    process.env.METIS_SERVER_URL ?? "http://server.metis.svc.cluster.local",
  port: Number(process.env.PORT ?? 4000),
  cookieName: "metis_token",
  cookieSecure: process.env.COOKIE_SECURE !== "false",
  logLevel: (process.env.LOG_LEVEL ?? "info") as
    | "debug"
    | "info"
    | "warn"
    | "error",
};
