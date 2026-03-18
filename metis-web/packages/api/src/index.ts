export { HydraApiClient } from "./client";
export type { HydraApiClientOptions } from "./client";
export type {
  RelationResponse,
  ListRelationsRequest,
  ListRelationsResponse,
} from "./client";

export { ApiError } from "./errors";

export {
  HydraEventSource,
  buildEventsUrl,
} from "./sse";
export type {
  HydraEvent,
  HydraEventHandler,
  HydraEventErrorHandler,
  EventSubscriptionOptions,
} from "./sse";

export * from "./types";
