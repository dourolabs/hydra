import type { SseEventType } from "./generated/SseEventType";
import type { EntityEventData } from "./generated/EntityEventData";
import type { HeartbeatEventData } from "./generated/HeartbeatEventData";
import type { ResyncEventData } from "./generated/ResyncEventData";
import type { SnapshotEventData } from "./generated/SnapshotEventData";

/** A parsed SSE event from the metis-server /v1/events stream. */
export type MetisEvent =
  | { type: "entity"; eventType: SseEventType; data: EntityEventData; id: number }
  | { type: "snapshot"; data: SnapshotEventData; id: number }
  | { type: "resync"; data: ResyncEventData; id: number }
  | { type: "heartbeat"; data: HeartbeatEventData; id: number };

/** Callback invoked for each parsed event. */
export type MetisEventHandler = (event: MetisEvent) => void;

/** Callback invoked when the connection encounters an error. */
export type MetisEventErrorHandler = (error: Error) => void;

export interface EventSubscriptionOptions {
  /** SSE event types to filter (e.g. "issues,jobs"). */
  types?: string;
  /** Comma-separated issue IDs to filter. */
  issueIds?: string;
  /** Comma-separated job IDs to filter. */
  jobIds?: string;
  /** Comma-separated patch IDs to filter. */
  patchIds?: string;
  /** Comma-separated document IDs to filter. */
  documentIds?: string;
  /** Resume from this event ID. */
  lastEventId?: number;
}

const SNAPSHOT_EVENTS: ReadonlySet<string> = new Set(["snapshot"]);
const RESYNC_EVENTS: ReadonlySet<string> = new Set(["resync"]);
const HEARTBEAT_EVENTS: ReadonlySet<string> = new Set(["heartbeat"]);

function parseEvent(eventType: string, data: string, id: string): MetisEvent | null {
  const parsedId = id ? Number(id) : 0;

  if (SNAPSHOT_EVENTS.has(eventType)) {
    return { type: "snapshot", data: JSON.parse(data) as SnapshotEventData, id: parsedId };
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
export class MetisEventSource {
  private eventSource: EventSource;
  private closed = false;

  constructor(
    url: string,
    private onEvent: MetisEventHandler,
    private onError?: MetisEventErrorHandler,
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
      "job_created",
      "job_updated",
      "document_created",
      "document_updated",
      "document_deleted",
      "label_created",
      "label_updated",
      "label_deleted",
      "message_created",
      "message_updated",
      "snapshot",
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
  if (options?.jobIds) params.set("job_ids", options.jobIds);
  if (options?.patchIds) params.set("patch_ids", options.patchIds);
  if (options?.documentIds) params.set("document_ids", options.documentIds);

  const query = params.toString();
  const url = `${baseUrl}/v1/events${query ? `?${query}` : ""}`;
  return url;
}
