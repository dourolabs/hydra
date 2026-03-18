import type { SseEventType } from "./generated/SseEventType";
import type { EntityEventData } from "./generated/EntityEventData";
import type { HeartbeatEventData } from "./generated/HeartbeatEventData";
import type { ResyncEventData } from "./generated/ResyncEventData";
import type { ConnectedEventData } from "./generated/ConnectedEventData";

/** A parsed SSE event from the hydra-server /v1/events stream. */
export type HydraEvent =
  | { type: "entity"; eventType: SseEventType; data: EntityEventData; id: number }
  | { type: "connected"; data: ConnectedEventData; id: number }
  | { type: "resync"; data: ResyncEventData; id: number }
  | { type: "heartbeat"; data: HeartbeatEventData; id: number };

/** Callback invoked for each parsed event. */
export type HydraEventHandler = (event: HydraEvent) => void;

/** Callback invoked when the connection encounters an error. */
export type HydraEventErrorHandler = (error: Error) => void;

export interface EventSubscriptionOptions {
  /** SSE event types to filter (e.g. "issues,jobs"). */
  types?: string;
  /** Comma-separated issue IDs to filter. */
  issueIds?: string;
  /** Comma-separated session IDs to filter. */
  sessionIds?: string;
  /** Comma-separated patch IDs to filter. */
  patchIds?: string;
  /** Comma-separated document IDs to filter. */
  documentIds?: string;
  /** Resume from this event ID. */
  lastEventId?: number;
}

const CONNECTED_EVENTS: ReadonlySet<string> = new Set(["connected"]);
const RESYNC_EVENTS: ReadonlySet<string> = new Set(["resync"]);
const HEARTBEAT_EVENTS: ReadonlySet<string> = new Set(["heartbeat"]);

function parseEvent(eventType: string, data: string, id: string): HydraEvent | null {
  const parsedId = id ? Number(id) : 0;

  if (CONNECTED_EVENTS.has(eventType)) {
    return { type: "connected", data: JSON.parse(data) as ConnectedEventData, id: parsedId };
  }
  if (RESYNC_EVENTS.has(eventType)) {
    return { type: "resync", data: JSON.parse(data) as ResyncEventData, id: parsedId };
  }
  if (HEARTBEAT_EVENTS.has(eventType)) {
    return { type: "heartbeat", data: JSON.parse(data) as HeartbeatEventData, id: parsedId };
  }

  // All other event types are entity events
  return {
    type: "entity",
    eventType: eventType as SseEventType,
    data: JSON.parse(data) as EntityEventData,
    id: parsedId,
  };
}

/**
 * An SSE event stream subscription that wraps EventSource.
 * Call `close()` to disconnect.
 */
export class HydraEventSource {
  private eventSource: EventSource;
  private closed = false;

  constructor(
    url: string,
    private onEvent: HydraEventHandler,
    private onError?: HydraEventErrorHandler,
  ) {
    this.eventSource = new EventSource(url);

    this.eventSource.onmessage = (e: MessageEvent) => {
      // Default event type for messages without an explicit event: field
      this.handleRawEvent("entity", e.data as string, e.lastEventId);
    };

    this.eventSource.onerror = () => {
      if (!this.closed) {
        this.onError?.(new Error("SSE connection error"));
      }
    };

    // Register named event handlers for each known SSE event type
    const sseEventTypes: SseEventType[] = [
      "issue_created",
      "issue_updated",
      "issue_deleted",
      "patch_created",
      "patch_updated",
      "patch_deleted",
      "session_created",
      "session_updated",
      "document_created",
      "document_updated",
      "document_deleted",
      "label_created",
      "label_updated",
      "label_deleted",
      "message_created",
      "message_updated",
      "connected",
      "resync",
      "heartbeat",
    ];

    for (const eventType of sseEventTypes) {
      this.eventSource.addEventListener(eventType, (e: Event) => {
        const messageEvent = e as MessageEvent;
        this.handleRawEvent(eventType, messageEvent.data as string, messageEvent.lastEventId);
      });
    }
  }

  private handleRawEvent(eventType: string, data: string, id: string): void {
    try {
      const parsed = parseEvent(eventType, data, id);
      if (parsed) {
        this.onEvent(parsed);
      }
    } catch (err) {
      this.onError?.(err instanceof Error ? err : new Error(String(err)));
    }
  }

  /** Close the underlying EventSource connection. */
  close(): void {
    this.closed = true;
    this.eventSource.close();
  }

  /** Returns the current connection state of the underlying EventSource. */
  get readyState(): number {
    return this.eventSource.readyState;
  }
}

/**
 * Build the SSE URL for the events endpoint.
 */
export function buildEventsUrl(baseUrl: string, options?: EventSubscriptionOptions): string {
  const params = new URLSearchParams();
  if (options?.types) params.set("types", options.types);
  if (options?.issueIds) params.set("issue_ids", options.issueIds);
  if (options?.sessionIds) params.set("session_ids", options.sessionIds);
  if (options?.patchIds) params.set("patch_ids", options.patchIds);
  if (options?.documentIds) params.set("document_ids", options.documentIds);

  const query = params.toString();
  const url = `${baseUrl}/v1/events${query ? `?${query}` : ""}`;
  return url;
}
