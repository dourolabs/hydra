import type { SseEventType } from "@hydra/api";

/** Extracts the entity prefix P from SSE event type strings of the form `P_Suffix`. */
type SsePrefix<E extends string, Suffix extends string> = E extends `${infer P}_${Suffix}`
  ? P
  : never;

export interface VersionedEntity<T = unknown> {
  version: number;
  timestamp: string;
  data: T;
  deleted?: boolean;
}

export interface StoreEvent {
  id: number;
  eventType: SseEventType;
  entityType: string;
  entityId: string;
  version: number;
  timestamp: string;
  entity: unknown;
}

export type StoreEventListener = (event: StoreEvent) => void;

export class Store {
  private collections = new Map<string, Map<string, VersionedEntity[]>>();
  private eventSeq = 0;
  private events: StoreEvent[] = [];
  private listeners: Set<StoreEventListener> = new Set();

  private getCollection(collectionName: string): Map<string, VersionedEntity[]> {
    let col = this.collections.get(collectionName);
    if (!col) {
      col = new Map();
      this.collections.set(collectionName, col);
    }
    return col;
  }

  create<T>(
    collectionName: string,
    id: string,
    data: T,
    ssePrefix: SsePrefix<SseEventType, "created"> | null,
  ): VersionedEntity<T> {
    const col = this.getCollection(collectionName);
    if (col.has(id)) {
      throw new StoreError(409, `${collectionName} '${id}' already exists`);
    }
    const now = new Date().toISOString();
    const entry: VersionedEntity<T> = { version: 1, timestamp: now, data };
    col.set(id, [entry]);
    if (ssePrefix !== null) {
      this.emitEvent(`${ssePrefix}_created`, collectionName, id, entry);
    }
    return entry;
  }

  update<T>(
    collectionName: string,
    id: string,
    data: T,
    ssePrefix: SsePrefix<SseEventType, "updated"> | null,
  ): VersionedEntity<T> {
    const col = this.getCollection(collectionName);
    const versions = col.get(id);
    if (!versions || versions.length === 0) {
      throw new StoreError(404, `${collectionName} '${id}' not found`);
    }
    const latest = versions[versions.length - 1];
    if (latest.deleted) {
      throw new StoreError(404, `${collectionName} '${id}' not found`);
    }
    const now = new Date().toISOString();
    const entry: VersionedEntity<T> = {
      version: latest.version + 1,
      timestamp: now,
      data,
    };
    versions.push(entry);
    if (ssePrefix !== null) {
      this.emitEvent(`${ssePrefix}_updated`, collectionName, id, entry);
    }
    return entry;
  }

  get<T>(collectionName: string, id: string, includeDeleted = false): VersionedEntity<T> | null {
    const col = this.getCollection(collectionName);
    const versions = col.get(id);
    if (!versions || versions.length === 0) return null;
    const latest = versions[versions.length - 1];
    if (latest.deleted && !includeDeleted) return null;
    return latest as VersionedEntity<T>;
  }

  getVersion<T>(collectionName: string, id: string, version: number): VersionedEntity<T> | null {
    const col = this.getCollection(collectionName);
    const versions = col.get(id);
    if (!versions) return null;
    const entry = versions.find((v) => v.version === version);
    return (entry as VersionedEntity<T>) ?? null;
  }

  list<T>(collectionName: string, includeDeleted = false): { id: string; entry: VersionedEntity<T> }[] {
    const col = this.getCollection(collectionName);
    const results: { id: string; entry: VersionedEntity<T> }[] = [];
    for (const [id, versions] of col) {
      if (versions.length === 0) continue;
      const latest = versions[versions.length - 1];
      if (latest.deleted && !includeDeleted) continue;
      results.push({ id, entry: latest as VersionedEntity<T> });
    }
    return results;
  }

  listVersions<T>(collectionName: string, id: string): VersionedEntity<T>[] {
    const col = this.getCollection(collectionName);
    const versions = col.get(id);
    if (!versions) return [];
    return versions as VersionedEntity<T>[];
  }

  delete<T>(
    collectionName: string,
    id: string,
    ssePrefix: SsePrefix<SseEventType, "deleted"> | null,
  ): VersionedEntity<T> {
    const col = this.getCollection(collectionName);
    const versions = col.get(id);
    if (!versions || versions.length === 0) {
      throw new StoreError(404, `${collectionName} '${id}' not found`);
    }
    const latest = versions[versions.length - 1];
    if (latest.deleted) {
      throw new StoreError(404, `${collectionName} '${id}' not found`);
    }
    const now = new Date().toISOString();
    const deletedData = { ...latest.data as object, deleted: true } as T;
    const entry: VersionedEntity<T> = {
      version: latest.version + 1,
      timestamp: now,
      data: deletedData,
      deleted: true,
    };
    versions.push(entry);
    if (ssePrefix !== null) {
      this.emitEvent(`${ssePrefix}_deleted`, collectionName, id, entry);
    }
    return entry;
  }

  getCreationTime(collectionName: string, id: string): string | undefined {
    const col = this.getCollection(collectionName);
    const versions = col.get(id);
    if (!versions || versions.length === 0) return undefined;
    return versions[0].timestamp;
  }

  private emitEvent(
    eventType: SseEventType,
    entityType: string,
    entityId: string,
    entry: VersionedEntity,
  ): void {
    this.eventSeq++;
    const event: StoreEvent = {
      id: this.eventSeq,
      eventType,
      entityType,
      entityId,
      version: entry.version,
      timestamp: entry.timestamp,
      entity: entry.data,
    };
    this.events.push(event);
    for (const listener of this.listeners) {
      listener(event);
    }
  }

  subscribe(listener: StoreEventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  getEventsSince(lastEventId: number): StoreEvent[] {
    return this.events.filter((e) => e.id > lastEventId);
  }

  getCurrentSeq(): number {
    return this.eventSeq;
  }

  clear(): void {
    this.collections.clear();
    this.events = [];
    this.eventSeq = 0;
  }
}

export class StoreError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "StoreError";
  }
}
