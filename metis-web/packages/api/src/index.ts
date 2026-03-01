export { MetisApiClient } from "./client";
export type { MetisApiClientOptions } from "./client";

export { ApiError } from "./errors";

export {
  MetisEventSource,
  buildEventsUrl,
} from "./sse";
export type {
  MetisEvent,
  MetisEventHandler,
  MetisEventErrorHandler,
  EventSubscriptionOptions,
} from "./sse";

export * from "./types";

export type {
  Notification,
  NotificationResponse,
  ListNotificationsResponse,
  ListNotificationsQuery,
  UnreadCountResponse,
  MarkReadResponse,
} from "./notifications";
