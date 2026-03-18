import { config } from "./config.js";

type LogLevel = "debug" | "info" | "warn" | "error";

const LEVEL_ORDER: Record<LogLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
};

function shouldLog(level: LogLevel): boolean {
  return LEVEL_ORDER[level] >= LEVEL_ORDER[config.logLevel];
}

function emit(level: LogLevel, msg: string, fields?: Record<string, unknown>) {
  if (!shouldLog(level)) return;
  const entry = {
    level,
    msg,
    timestamp: new Date().toISOString(),
    ...fields,
  };
  process.stdout.write(JSON.stringify(entry) + "\n");
}

export const logger = {
  debug: (msg: string, fields?: Record<string, unknown>) =>
    emit("debug", msg, fields),
  info: (msg: string, fields?: Record<string, unknown>) =>
    emit("info", msg, fields),
  warn: (msg: string, fields?: Record<string, unknown>) =>
    emit("warn", msg, fields),
  error: (msg: string, fields?: Record<string, unknown>) =>
    emit("error", msg, fields),
};
